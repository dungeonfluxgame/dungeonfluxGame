//! The reads the screens are built from: the dashboard, a campaign's overview, its settings and its
//! sessions tab. They change nothing. Authority comes from the member snapshot (an authorized,
//! active member), and anyone else sees a campaign as missing.

use df_types::{AccountId, CampaignId, SessionId};

use super::{CampaignService, Ports, refuse};
use crate::{
    draft::{CampaignSettings, DraftRow},
    errors::{Outcome, Refusal},
    records::{CampaignRow, DashboardRow, MemberRow, MemberStatus},
    roles::{Authorized, Capability, authorize},
    store::{AccountDirectory, CampaignStore, SessionDirectory, SessionPhase},
};

/// A campaign's live session as O1 reports it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveSession {
    pub session: SessionId,
    pub phase: SessionPhase,
}

/// What O1 said about a campaign's session. An unreadable session is `Unknown`, shown as such and
/// never as "no session".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionReading {
    Live(LiveSession),
    None,
    Unknown,
    /// Not asked: the dashboard asks about the most recently active campaigns only.
    NotChecked,
}

#[derive(Clone, Debug)]
pub struct DashboardEntry {
    pub row: DashboardRow,
    pub session: SessionReading,
}

#[derive(Clone, Debug)]
pub struct Dashboard {
    pub campaigns: Vec<DashboardEntry>,
    pub drafts: Vec<DraftRow>,
}

impl Dashboard {
    /// The plan's empty state: no campaign and no draft.
    pub fn is_empty(&self) -> bool {
        self.campaigns.is_empty() && self.drafts.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct RosterEntry {
    pub member: MemberRow,
    pub display_name: Option<String>,
}

/// The tabs of a campaign page that exist. Overview, Sessions and Settings are this service's;
/// the others link to another owner's screens and appear only when that owner is mounted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tab {
    Overview,
    Sessions,
    Characters,
    Assets,
    Funding,
    Settings,
}

#[derive(Clone, Debug)]
pub struct Overview {
    pub campaign: CampaignRow,
    pub settings: CampaignSettings,
    pub roster: Vec<RosterEntry>,
    pub me: MemberRow,
    pub seats_in_use: u32,
    pub contributors_in_use: u32,
    /// Pending room-code requests, for those who may decide them.
    pub pending_requests: Option<u32>,
    pub session: SessionReading,
    pub tabs: Vec<Tab>,
    pub is_owner: bool,
    pub can_invite: bool,
    pub can_start: bool,
    /// Only the host edits settings; everyone else reads them.
    pub can_edit_settings: bool,
}

impl<P: Ports> CampaignService<P> {
    async fn session_reading(&self, campaign: CampaignId) -> SessionReading {
        match self.inner.sessions.live_session_of(campaign).await {
            Ok(Some((session, phase))) => SessionReading::Live(LiveSession { session, phase }),
            Ok(None) => SessionReading::None,
            Err(_) => SessionReading::Unknown,
        }
    }

    /// The campaigns the account plays in or runs, and its unfinished drafts. The live session is
    /// asked of the `dashboard_session_lookups` most recently active active campaigns only, so
    /// the page costs a bounded number of reads from O1.
    pub async fn dashboard(&self, account: AccountId) -> Outcome<Dashboard> {
        let tunables = &self.inner.tunables;
        let rows = self
            .inner
            .store
            .dashboard_campaigns(account, tunables.dashboard_maximum_campaigns)
            .await?;
        let drafts = self.inner.store.list_drafts(account, self.now_ms()).await?;
        let mut asked = 0;
        let mut campaigns = Vec::with_capacity(rows.len());
        for row in rows {
            let session = if row.status == crate::records::CampaignStatus::Active
                && asked < tunables.dashboard_session_lookups
            {
                asked += 1;
                self.session_reading(row.id).await
            } else {
                SessionReading::NotChecked
            };
            campaigns.push(DashboardEntry { row, session });
        }
        Ok(Dashboard { campaigns, drafts })
    }

    fn tabs(&self) -> Vec<Tab> {
        let tunables = &self.inner.tunables;
        let mut tabs = vec![Tab::Overview, Tab::Sessions];
        if tunables.tab_characters_mounted {
            tabs.push(Tab::Characters);
        }
        if tunables.tab_assets_mounted {
            tabs.push(Tab::Assets);
        }
        if tunables.tab_funding_mounted {
            tabs.push(Tab::Funding);
        }
        tabs.push(Tab::Settings);
        tabs
    }

    /// The overview and settings of a campaign for one of its authorized members. A draft, a
    /// campaign being deleted, a stranger and a former member all answer as missing.
    pub async fn overview(&self, account: AccountId, campaign: CampaignId) -> Outcome<Overview> {
        let Some(snapshot) = self.inner.store.member_snapshot(account, campaign).await? else {
            return refuse(Refusal::NotFound);
        };
        if snapshot.status != MemberStatus::Active {
            return refuse(Refusal::NotFound);
        }
        if !snapshot.authorized {
            return refuse(Refusal::NotAllowed);
        }
        let Some(rows) = self
            .inner
            .store
            .campaign_overview(account, campaign)
            .await?
        else {
            return refuse(Refusal::NotFound);
        };
        let Some(me) = rows
            .members
            .iter()
            .find(|member| member.account == account)
            .cloned()
        else {
            return refuse(Refusal::NotFound);
        };
        let full = |capability| {
            authorize(snapshot.role, snapshot.delegations, capability) == Ok(Authorized::Full)
        };
        let can_invite = full(Capability::InviteOrRemovePlayers);
        let mut roster = Vec::with_capacity(rows.members.len());
        for member in &rows.members {
            // A name is a convenience; a profile that cannot be read leaves the entry unnamed.
            let display_name = match self.inner.accounts.profile(member.account).await {
                Ok(Some(profile)) => profile.display_name,
                _ => None,
            };
            roster.push(RosterEntry {
                member: member.clone(),
                display_name,
            });
        }
        let seats_in_use = rows
            .members
            .iter()
            .filter(|member| member.holds_seat)
            .count();
        let contributors_in_use = rows
            .members
            .iter()
            .filter(|member| member.role == crate::roles::CampaignRole::Contributor)
            .count();
        Ok(Overview {
            is_owner: rows.campaign.owner == account,
            settings: rows.settings,
            roster,
            me,
            seats_in_use: u32::try_from(seats_in_use).unwrap_or(u32::MAX),
            contributors_in_use: u32::try_from(contributors_in_use).unwrap_or(u32::MAX),
            pending_requests: can_invite.then_some(rows.pending_requests),
            session: self.session_reading(campaign).await,
            tabs: self.tabs(),
            can_invite,
            can_start: full(Capability::StartOrPauseSession),
            can_edit_settings: snapshot.role == crate::roles::CampaignRole::Host,
            campaign: rows.campaign,
        })
    }

    /// The sessions tab: the live session, if any. The history of past sessions is O1's and not
    /// read here yet (TODO(S1-10)).
    pub async fn sessions(
        &self,
        account: AccountId,
        campaign: CampaignId,
    ) -> Outcome<SessionReading> {
        let Some(snapshot) = self.inner.store.member_snapshot(account, campaign).await? else {
            return refuse(Refusal::NotFound);
        };
        if snapshot.status != MemberStatus::Active {
            return refuse(Refusal::NotFound);
        }
        if !snapshot.authorized {
            return refuse(Refusal::NotAllowed);
        }
        Ok(self.session_reading(campaign).await)
    }
}
