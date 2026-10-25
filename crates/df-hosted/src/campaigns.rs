//! Native account API adapters; missing owner contracts refuse without changing state.

use df_account_app::{
    wire::PlaySessionsView,
    wire_campaign::{
        CampaignCard, CampaignDraft, CampaignOverview, CampaignRole, DashboardView, DisplayState,
        InvitationCreated, InvitationKind, InvitePreview, JoinRequested, LobbyView,
        MicrophoneRequest, PairingLookup, WizardCatalog,
    },
};
use df_campaign::{
    service::{CampaignService, Ports as ServicePorts},
    store::*,
};
use df_customer::{AccountStore, DeviceLabel};
use df_portal::app_api::ports::*;
use df_types::{AccountId, CampaignId, OperationId, SessionId};

pub(crate) struct Campaigns {
    service: CampaignService<Owners>,
}
struct Owners;
struct Accounts(df_persistence::account_store::PostgresAccountStore);
struct Unavailable;

impl ServicePorts for Owners {
    type Store = crate::campaign_store::PgCampaignStore;
    type Accounts = Accounts;
    type Sessions = Unavailable;
    type Funding = Unavailable;
    type Time = df_campaign::clock::SystemClock;
}

impl AccountDirectory for Accounts {
    async fn profile(
        &self,
        account: AccountId,
    ) -> df_campaign::errors::StoreResult<Option<AccountProfile>> {
        let record = self
            .0
            .account(account)
            .await
            .map_err(|_| df_campaign::errors::StoreError::Unavailable)?;
        let Some(record) = record else {
            return Ok(None);
        };
        // TODO(S1-02): B/O1 must export the verified account's canonical tenant binding.
        if record.is_verified {
            df_observe::record_audit_line(
                "hosted.tenant_binding_unavailable",
                "HOSTED tenant_binding_unavailable owner=S1-02".to_owned(),
            );
            return Err(df_campaign::errors::StoreError::Unavailable);
        }
        Ok(Some(AccountProfile {
            display_name: record.display_name.map(|name| name.as_str().to_owned()),
            verified: false,
            tenant: None,
            status: match record.status {
                df_customer::AccountStatus::Active => df_campaign::records::AccountStatus::Active,
                df_customer::AccountStatus::Closing => df_campaign::records::AccountStatus::Closing,
                df_customer::AccountStatus::Closed => df_campaign::records::AccountStatus::Closed,
            },
        }))
    }
}
impl SessionDirectory for Unavailable {
    async fn session(
        &self,
        _session: SessionId,
    ) -> df_campaign::errors::StoreResult<Option<SessionFacts>> {
        Err(df_campaign::errors::StoreError::Unavailable)
    }
    async fn live_session_of(
        &self,
        _campaign: CampaignId,
    ) -> df_campaign::errors::StoreResult<Option<(SessionId, SessionPhase)>> {
        Err(df_campaign::errors::StoreError::Unavailable)
    }
    async fn open_session(
        &self,
        _campaign: CampaignId,
    ) -> df_campaign::errors::StoreResult<(SessionId, SessionPhase)> {
        Err(df_campaign::errors::StoreError::Unavailable)
    }
    async fn start_session(
        &self,
        _session: SessionId,
        _campaign: CampaignId,
        _roster: &[df_campaign::lobby_state::SeatedMember],
    ) -> df_campaign::errors::StoreResult<()> {
        Err(df_campaign::errors::StoreError::Unavailable)
    }
}
impl FundingDirectory for Unavailable {
    async fn membership_event(&self, _campaign: CampaignId, _event_seq: u64) {}
    async fn start_readiness(
        &self,
        _campaign: CampaignId,
        _session: SessionId,
    ) -> df_campaign::errors::StoreResult<df_campaign::lobby_state::StartReadiness> {
        Err(df_campaign::errors::StoreError::Unavailable)
    }
    async fn open_session_funding(
        &self,
        _campaign: CampaignId,
        _session: SessionId,
    ) -> df_campaign::errors::StoreResult<()> {
        Err(df_campaign::errors::StoreError::Unavailable)
    }
}
impl Campaigns {
    pub(crate) fn new(
        store: crate::campaign_store::PgCampaignStore,
        accounts: df_persistence::account_store::PostgresAccountStore,
        keys: df_campaign::service::ServiceKeys,
    ) -> Self {
        Self {
            service: CampaignService::new(
                store,
                Accounts(accounts),
                Unavailable,
                Unavailable,
                df_campaign::clock::SystemClock,
                df_campaign::config::Tunables::default(),
                keys,
            ),
        }
    }
}

fn failure(error: df_campaign::errors::Failure) -> PortError {
    match error {
        df_campaign::errors::Failure::Store(df_campaign::errors::StoreError::Unknown) => {
            PortError::UnknownOutcome
        }
        df_campaign::errors::Failure::Store(_) => PortError::Unavailable,
        df_campaign::errors::Failure::Refused(df_campaign::errors::Refusal::NotFound) => {
            PortError::NotFound
        }
        df_campaign::errors::Failure::Refused(_) => PortError::Forbidden,
    }
}

impl CampaignDirectory for Campaigns {
    async fn dashboard(&self, who: &Actor) -> PortResult<DashboardView> {
        let dashboard = self.service.dashboard(who.account).await.map_err(failure)?;
        let mut campaigns = Vec::with_capacity(dashboard.campaigns.len());
        for entry in dashboard.campaigns {
            // The app's view has no Unknown session variant; never report an unreadable session as absent.
            let live_session_id = match entry.session {
                df_campaign::service::SessionReading::Live(live) => {
                    Some(hex(live.session.as_bytes()))
                }
                df_campaign::service::SessionReading::None => None,
                _ => return Err(PortError::Unavailable),
            };
            campaigns.push(CampaignCard {
                campaign_id: hex(entry.row.id.as_bytes()),
                name: entry.row.name,
                role: match entry.row.role {
                    df_campaign::roles::CampaignRole::Host => CampaignRole::Host,
                    df_campaign::roles::CampaignRole::CoHost => CampaignRole::CoHost,
                    df_campaign::roles::CampaignRole::Player => CampaignRole::Player,
                    df_campaign::roles::CampaignRole::Contributor => CampaignRole::Contributor,
                    df_campaign::roles::CampaignRole::PairedDisplay => {
                        return Err(PortError::Unavailable);
                    }
                },
                host_name: self
                    .service
                    .overview(who.account, entry.row.id)
                    .await
                    .map_err(failure)?
                    .roster
                    .into_iter()
                    .find(|member| member.member.role == df_campaign::roles::CampaignRole::Host)
                    .and_then(|member| member.display_name)
                    .ok_or(PortError::Unavailable)?,
                party_size: u8::try_from(entry.row.seats_in_use)
                    .map_err(|_| PortError::Unavailable)?,
                last_played_at_ms: None,
                last_saved_at_ms: None,
                live_session_id,
                is_archived: entry.row.status == df_campaign::records::CampaignStatus::Archived,
            });
        }
        Ok(DashboardView {
            campaigns,
            can_create: who.is_verified,
        })
    }

    async fn play_sessions(
        &self,
        _account: AccountId,
        _cursor: Option<&str>,
    ) -> PortResult<PlaySessionsView> {
        crate::ports::unavailable("S1")
    }

    async fn funding_targets(&self, _account: AccountId) -> PortResult<Vec<FundingTarget>> {
        crate::ports::unavailable("S1")
    }

    async fn closure_blockers(&self, _account: AccountId) -> PortResult<Vec<String>> {
        crate::ports::unavailable("S1")
    }

    async fn wizard_catalog(&self) -> PortResult<WizardCatalog> {
        crate::ports::unavailable("S1")
    }

    async fn start_draft(&self, who: &Actor) -> PortResult<(String, CampaignDraft)> {
        let _draft = self
            .service
            .start_draft(who.account)
            .await
            .map_err(failure)?;
        crate::ports::unavailable("S1")
    }

    async fn save_draft(
        &self,
        _who: &Actor,
        _draft_id: &str,
        _draft: CampaignDraft,
    ) -> PortResult<CampaignDraft> {
        crate::ports::unavailable("S1")
    }

    async fn finish_draft(
        &self,
        _who: &Actor,
        _draft_id: &str,
        _operation: OperationId,
    ) -> PortResult<CampaignId> {
        crate::ports::unavailable("S1")
    }

    async fn campaign(&self, _who: &Actor, _campaign: CampaignId) -> PortResult<CampaignOverview> {
        crate::ports::unavailable("S1")
    }

    async fn create_invitation(
        &self,
        _who: &Actor,
        _campaign: CampaignId,
        _kind: InvitationKind,
        _operation: OperationId,
    ) -> PortResult<InvitationCreated> {
        crate::ports::unavailable("S1")
    }

    async fn revoke_invitation(
        &self,
        _who: &Actor,
        _campaign: CampaignId,
        _invitation_id: &str,
    ) -> PortResult<CampaignOverview> {
        crate::ports::unavailable("S1")
    }

    async fn rotate_code(
        &self,
        _who: &Actor,
        _campaign: CampaignId,
    ) -> PortResult<CampaignOverview> {
        crate::ports::unavailable("S1")
    }

    async fn decide_join_request(
        &self,
        _who: &Actor,
        _campaign: CampaignId,
        _request_id: &str,
        _decision: RequestDecision,
    ) -> PortResult<CampaignOverview> {
        crate::ports::unavailable("S1")
    }

    async fn leave(&self, _who: &Actor, _campaign: CampaignId) -> PortResult<()> {
        crate::ports::unavailable("S1")
    }

    async fn open_lobby(&self, _who: &Actor, _campaign: CampaignId) -> PortResult<SessionId> {
        crate::ports::unavailable("S1")
    }

    async fn lobby(&self, _who: &Actor, _session: SessionId) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn set_ready(
        &self,
        _who: &Actor,
        _session: SessionId,
        _is_ready: bool,
    ) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn report_microphone(
        &self,
        _who: &Actor,
        _session: SessionId,
        _report: MicrophoneRequest,
    ) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn start_session(&self, _who: &Actor, _session: SessionId) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn lookup_display(
        &self,
        _who: &Actor,
        _session: SessionId,
        _code: &str,
    ) -> PortResult<PairingLookup> {
        crate::ports::unavailable("S1")
    }

    async fn approve_display(
        &self,
        _who: &Actor,
        _session: SessionId,
        _pairing_id: &str,
    ) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn unpair_display(
        &self,
        _who: &Actor,
        _session: SessionId,
        _ticket_id: &str,
    ) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn unpair_all_displays(
        &self,
        _who: &Actor,
        _session: SessionId,
    ) -> PortResult<LobbyView> {
        crate::ports::unavailable("S1")
    }

    async fn join_preview(&self, _token: &str) -> PortResult<InvitePreview> {
        crate::ports::unavailable("S1")
    }

    async fn join_accept(
        &self,
        _who: &Actor,
        _token: &str,
        _operation: OperationId,
    ) -> PortResult<CampaignId> {
        crate::ports::unavailable("S1")
    }

    async fn code_lookup(&self, _who: &Actor, _code: &str) -> PortResult<InvitePreview> {
        crate::ports::unavailable("S1")
    }

    async fn code_request(
        &self,
        _who: &Actor,
        _code: &str,
        _operation: OperationId,
    ) -> PortResult<JoinRequested> {
        crate::ports::unavailable("S1")
    }

    async fn display_challenge(&self, _device: DeviceLabel) -> PortResult<IssuedChallenge> {
        crate::ports::unavailable("S1")
    }

    async fn display_status(&self, _device_secret: &str) -> PortResult<DisplayState> {
        crate::ports::unavailable("S1")
    }

    async fn display_redeem(&self, _device_secret: &str) -> PortResult<RedeemedTicket> {
        crate::ports::unavailable("S1")
    }

    async fn display_renew(
        &self,
        _device_secret: &str,
        _ticket: &str,
    ) -> PortResult<RenewedTicket> {
        crate::ports::unavailable("S1")
    }
}

fn hex(bytes: &[u8; 16]) -> String {
    df_commerce::finance_ids::hex(bytes)
}
