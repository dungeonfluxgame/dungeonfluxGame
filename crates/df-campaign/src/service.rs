//! The campaign service: the rules the routes call. It owns the throttle layers, token and room
//! code handling, the audit lines and the calls to the other owners' ports; the store owns every
//! table. Every method takes the caller's `AccountId` from the authenticated session, never from
//! a body, and authorizes before it reads anything about a session or a campaign's state.

mod campaigns;
mod display;
mod invites;
mod lobby;
mod members;
mod views;

use std::{net::IpAddr, sync::Arc};

use df_campaign_wire::PairingStateName;
use df_types::{AccountId, CampaignId};

use crate::{
    audit::{AuditLine, failure_word},
    clock::Clock,
    config::Tunables,
    display_pairing::PairingState,
    errors::{Failure, Outcome, Refusal},
    records::MemberSnapshot,
    roles::{Authorized, Capability, authorize},
    store::{AccountDirectory, CampaignStore, FundingDirectory, RetentionReport, SessionDirectory},
    throttles::{GateRefusal, Throttles},
};

pub use crate::throttles::{IpKeys, KeysRefusal, ServiceKeys, ip_keys};
pub use campaigns::CreateCampaign;
pub use display::{Challenge, DisplayLookup, RedeemedTicket};
pub use lobby::{LobbySeatView, LobbyView, StartBlocker, StartOutcome};
pub use views::{
    Dashboard, DashboardEntry, LiveSession, Overview, RosterEntry, SessionReading, Tab,
};

/// The collaborators of the service, bundled so signatures stay short.
pub trait Ports: Send + Sync + 'static {
    type Store: CampaignStore;
    type Accounts: AccountDirectory;
    type Sessions: SessionDirectory;
    type Funding: FundingDirectory;
    type Time: Clock;
}

struct Inner<P: Ports> {
    store: P::Store,
    accounts: P::Accounts,
    sessions: P::Sessions,
    funding: P::Funding,
    clock: P::Time,
    tunables: Tunables,
    keys: ServiceKeys,
    throttles: Throttles,
}

pub struct CampaignService<P: Ports> {
    inner: Arc<Inner<P>>,
}

impl<P: Ports> Clone for CampaignService<P> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub(crate) fn refuse<T>(refusal: Refusal) -> Outcome<T> {
    Err(Failure::Refused(refusal))
}

/// What the role matrix decided for a capability. The two arms cannot be confused: a caller must
/// match, so "something short of the action" (a request, an own-object right, a filtered view) is
/// never mistaken for permission to act.
#[derive(Clone, Debug)]
#[must_use = "match on the decision: Limited is not permission to act"]
pub enum CapabilityDecision {
    /// The caller may perform the action itself.
    Allowed(MemberSnapshot),
    /// A grant short of the action, with what it is.
    Limited(MemberSnapshot, Authorized),
}

/// The counts the dev view shows (`dev campaigns`).
#[derive(Clone, Debug)]
pub struct DevCampaigns {
    pub by_status: Vec<(String, u64)>,
    pub saturated_refusals: Vec<(&'static str, u64)>,
    /// For each address layer, the distinct address prefixes that fill its tight map in one window.
    pub sizing: Vec<crate::config::LayerSizing>,
}

impl DevCampaigns {
    pub fn render(&self) -> String {
        let mut text = String::from("campaigns by status\n");
        for (status, count) in &self.by_status {
            text.push_str(&format!("  {status}: {count}\n"));
        }
        text.push_str("throttle layers: refusals because the map was full\n");
        for (layer, count) in &self.saturated_refusals {
            text.push_str(&format!("  {layer}: {count}\n"));
        }
        text.push_str(
            "address layers: prefixes that fill the tight map in one window
",
        );
        for item in &self.sizing {
            text.push_str(&format!(
                "  {}: {} (loose limit {})
",
                item.layer, item.prefixes_to_fill_tight, item.loose_limit
            ));
        }
        text
    }
}

impl<P: Ports> CampaignService<P> {
    pub fn new(
        store: P::Store,
        accounts: P::Accounts,
        sessions: P::Sessions,
        funding: P::Funding,
        clock: P::Time,
        tunables: Tunables,
        keys: ServiceKeys,
    ) -> Self {
        let throttles = Throttles::new(&tunables);
        Self {
            inner: Arc::new(Inner {
                store,
                accounts,
                sessions,
                funding,
                clock,
                tunables,
                keys,
                throttles,
            }),
        }
    }

    pub fn store(&self) -> &P::Store {
        &self.inner.store
    }

    pub fn tunables(&self) -> &Tunables {
        &self.inner.tunables
    }

    pub fn now_ms(&self) -> u64 {
        self.inner.clock.now_ms()
    }

    pub fn ip_keys(&self, ip: IpAddr) -> IpKeys {
        ip_keys(self.inner.keys.ip_key(), ip)
    }

    // ---- Throttles -------------------------------------------------------------------------

    /// Turns a gate result into a bool and audits a refusal with the keyed address hash.
    fn passed(
        &self,
        result: Result<(), GateRefusal>,
        line: impl FnOnce() -> AuditLine,
        events: (&'static str, &'static str),
        keys: Option<&IpKeys>,
    ) -> bool {
        let Err(refusal) = result else {
            return true;
        };
        let mut line = line().reason(match refusal {
            GateRefusal::Throttled => "throttled",
            GateRefusal::Saturated => "saturated",
        });
        if refusal == GateRefusal::Saturated {
            line = line.alert();
        }
        if let Some(keys) = keys {
            line = line.ip_hash(&keys.tight);
        }
        line.emit(match refusal {
            GateRefusal::Throttled => events.0,
            GateRefusal::Saturated => events.1,
        });
        false
    }

    /// The gate every lookup of a code passes (room codes, display codes). A refusal looks like
    /// an unknown code to the caller.
    fn code_gate(&self, ip: IpAddr, account: Option<AccountId>) -> bool {
        let keys = self.ip_keys(ip);
        let result = self
            .inner
            .throttles
            .code_gate(&keys, account, self.now_ms());
        self.passed(
            result,
            || AuditLine::new("ROOMCODE", "refused"),
            ("roomcode.throttled", "roomcode.saturated"),
            Some(&keys),
        )
    }

    /// The answer for a per-key window (a limit keyed by an account, a pairing or a ticket): the
    /// caller sees `Throttled` either way, and a refusal because the map was full is an alert line.
    /// An ordinary throttle is not audited per request.
    fn window_outcome(
        &self,
        result: Result<(), GateRefusal>,
        family: &'static str,
        saturated_event: &'static str,
    ) -> Outcome<()> {
        match result {
            Ok(()) => Ok(()),
            Err(GateRefusal::Throttled) => refuse(Refusal::Throttled),
            Err(GateRefusal::Saturated) => {
                AuditLine::new(family, "refused")
                    .reason("saturated")
                    .alert()
                    .emit(saturated_event);
                refuse(Refusal::Throttled)
            }
        }
    }

    /// A per-account limit on a change; `Throttled` when spent.
    fn change_allowed(&self, account: AccountId) -> Outcome<()> {
        let result = self
            .inner
            .throttles
            .member_changes
            .attempt(account, self.now_ms());
        self.window_outcome(result, "CAMPAIGN", "campaign.change_saturated")
    }

    // ---- Authority -------------------------------------------------------------------------

    pub async fn member_snapshot(
        &self,
        account: AccountId,
        campaign: CampaignId,
    ) -> Outcome<Option<MemberSnapshot>> {
        Ok(self.inner.store.member_snapshot(account, campaign).await?)
    }

    /// The helper O1 calls for the pause and emergency-pause checks: the caller's snapshot must be
    /// authorized, and the role matrix decides. A request-only or own-only grant comes back as
    /// `Limited`, never as `Allowed`.
    pub async fn authorize_capability(
        &self,
        account: AccountId,
        campaign: CampaignId,
        capability: Capability,
    ) -> Outcome<CapabilityDecision> {
        let Some(snapshot) = self.inner.store.member_snapshot(account, campaign).await? else {
            return refuse(Refusal::NotFound);
        };
        if !snapshot.authorized {
            return refuse(Refusal::NotAllowed);
        }
        match authorize(snapshot.role, snapshot.delegations, capability) {
            Ok(Authorized::Full) => Ok(CapabilityDecision::Allowed(snapshot)),
            Ok(other) => Ok(CapabilityDecision::Limited(snapshot, other)),
            Err(_) => refuse(Refusal::NotAllowed),
        }
    }

    /// Authorizes an action: the caller must be an authorized member whose matrix cell is a full
    /// grant. A non-member answers as missing; a member without the right is refused.
    pub async fn require_full_capability(
        &self,
        account: AccountId,
        campaign: CampaignId,
        capability: Capability,
    ) -> Outcome<MemberSnapshot> {
        match self
            .authorize_capability(account, campaign, capability)
            .await?
        {
            CapabilityDecision::Allowed(snapshot) => Ok(snapshot),
            CapabilityDecision::Limited(..) => refuse(Refusal::NotAllowed),
        }
    }

    async fn notify(&self, campaign: CampaignId) {
        // A notification, not a command: a failed head read only skips the push.
        if let Ok(heads) = self.inner.store.campaign_heads(&[campaign]).await
            && let Some(head) = heads.first()
        {
            self.inner
                .funding
                .membership_event(campaign, head.event_seq)
                .await;
        }
    }

    // ---- Feed reads ------------------------------------------------------------------------

    pub async fn events_after(
        &self,
        campaign: CampaignId,
        after_seq: u64,
        limit: u32,
    ) -> Outcome<Vec<crate::records::EventRow>> {
        Ok(self
            .inner
            .store
            .events_after(campaign, after_seq, limit)
            .await?)
    }

    // ---- Retention and the dev view --------------------------------------------------------

    /// One tick of the retention task: scheduled deletions under the campaign lock and expired
    /// pairings.
    pub async fn retention_tick(&self) -> Outcome<RetentionReport> {
        self.inner
            .store
            .execute_scheduled_deletions(self.now_ms())
            .await
    }

    /// The owned retention loop: ticks every `retention_tick_seconds` until `shutdown` changes.
    /// A failed tick is audited and the next one tries again. The composition keeps the task's
    /// handle and awaits it at shutdown.
    pub async fn run_retention(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(self.inner.tunables.retention_tick());
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(failure) = self.retention_tick().await {
                        AuditLine::new("CAMPAIGN", "retention_failed")
                            .reason(failure_word(failure))
                            .emit("campaign.retention_failed");
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return;
                    }
                }
            }
        }
    }

    /// `dev campaigns`: counts by status and the per-layer throttle saturation. Read only.
    pub async fn dev_campaigns(&self) -> Outcome<DevCampaigns> {
        Ok(DevCampaigns {
            by_status: self.inner.store.campaign_counts().await?,
            saturated_refusals: self.inner.throttles.saturation_report(),
            sizing: self.inner.tunables.throttle_sizing(),
        })
    }
}

/// The wire name of a pairing state.
pub fn pairing_state_name(state: PairingState) -> PairingStateName {
    match state {
        PairingState::Pending => PairingStateName::Pending,
        PairingState::Approved => PairingStateName::Approved,
        PairingState::Redeemed => PairingStateName::Redeemed,
        PairingState::Cancelled => PairingStateName::Cancelled,
        PairingState::Expired => PairingStateName::Expired,
    }
}

#[cfg(any(test, feature = "test-fakes"))]
impl<P: Ports> CampaignService<P> {
    /// Whether a code guess from `ip` (and `account`) would pass the gate now, counting it as the
    /// real lookups do. Tests of the throttle layers use it; a real caller cannot tell a throttled
    /// guess from an unknown code.
    pub fn probe_guess_gate(&self, ip: IpAddr, account: Option<AccountId>) -> bool {
        self.code_gate(ip, account)
    }
}
