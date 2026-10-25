//! Fake campaign directory (TODO(S1): replaced by `df-campaign`). Campaigns, invitations, room
//! codes, the lobby and display pairing in memory, with the membership rules the screens
//! depend on: only members see a campaign, only the host or a co-host invites or starts, a
//! foreign id is `not_found`, and every invitation failure carries a closed reason.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use df_account_app::{
    wire::{DeviceKind, PlaySessionRow, PlaySessionsView},
    wire_campaign::{
        CampaignDraft, CampaignOverview, CampaignRole, CharacterRequest, ContentRating,
        DashboardView, InvitationCreated, InvitationKind, InvitePreview, JoinRequested, LobbyView,
        MicrophoneRequest, MicrophoneState, PairingLookup, WizardCatalog,
    },
};
use df_customer::{Clock, DeviceLabel};
use df_types::{AccountId, CampaignId, OperationId, SessionId};

use super::{Receipts, fresh_hex, lock};
use crate::app_api::ports::{
    Actor, CampaignDirectory, FundingTarget, IssuedChallenge, PortError, PortResult,
    RedeemedTicket, RenewedTicket, RequestDecision,
};

#[derive(Clone)]
pub(super) struct Member {
    pub account: AccountId,
    pub name: String,
    pub role: CampaignRole,
}

impl Member {
    pub(super) const fn has_seat(&self) -> bool {
        !matches!(self.role, CampaignRole::Contributor)
    }

    pub(super) const fn can_manage(&self) -> bool {
        matches!(self.role, CampaignRole::Host | CampaignRole::CoHost)
    }
}

pub(super) struct Invitation {
    pub id: String,
    pub kind: InvitationKind,
    pub token: String,
    pub uses: u32,
    pub maximum_uses: u32,
    pub expires_at_ms: u64,
    pub is_revoked: bool,
}

pub(super) struct JoinRequest {
    pub id: String,
    pub account: AccountId,
    pub name: String,
    pub requested_at_ms: u64,
}

pub(super) struct Campaign {
    pub id: CampaignId,
    pub name: String,
    pub members: Vec<Member>,
    pub invitations: Vec<Invitation>,
    pub requests: Vec<JoinRequest>,
    pub room_code: Option<String>,
    pub seats: u8,
    pub rating: ContentRating,
    pub live_session: Option<SessionId>,
    pub last_saved_at_ms: Option<u64>,
}

pub(super) struct Ticket {
    pub ticket_id: String,
    pub device: DeviceKind,
    pub paired_at_ms: u64,
}

pub(super) struct Lobby {
    pub campaign: CampaignId,
    pub ready: Vec<AccountId>,
    pub characters: HashMap<AccountId, df_account_app::wire_campaign::CharacterChoice>,
    pub microphones: HashMap<AccountId, MicrophoneState>,
    pub tickets: Vec<Ticket>,
    pub has_started: bool,
}

pub(super) enum PairState {
    Pending,
    Approved {
        session: SessionId,
    },
    Redeemed {
        session: SessionId,
        ticket: String,
        expires_at_ms: u64,
    },
    Cancelled,
}

pub(super) struct Pairing {
    pub id: String,
    pub user_code: String,
    pub device_secret: String,
    pub device: DeviceKind,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub state: PairState,
    pub ticket_id: Option<String>,
}

#[derive(Clone)]
pub(super) enum Receipt {
    Invitation(InvitationCreated),
    Joined(CampaignId),
    Requested(JoinRequested),
}

#[derive(Default)]
pub(super) struct State {
    pub campaigns: HashMap<CampaignId, Campaign>,
    pub lobbies: HashMap<SessionId, Lobby>,
    pub drafts: Vec<(String, AccountId, CampaignDraft)>,
    pub pairings: Vec<Pairing>,
    pub receipts: Receipts<Receipt>,
    /// Play history per account, newest first (seeded; S1's real adapter reads its store).
    pub play_sessions: Vec<(AccountId, PlaySessionRow)>,
}

pub struct FakeCampaigns {
    pub(super) origin: String,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
}

impl FakeCampaigns {
    pub fn new(origin: &str, clock: Arc<dyn Clock>) -> Self {
        Self {
            origin: origin.to_owned(),
            clock,
            state: Mutex::new(State::default()),
        }
    }

    fn run<T>(&self, work: impl FnOnce(&mut State, &Self, u64) -> PortResult<T>) -> PortResult<T> {
        work(&mut lock(&self.state), self, self.clock.now_ms())
    }

    /// Creates an active campaign hosted by `host` and returns its id.
    pub fn seed_campaign(&self, host: &Actor, name: &str) -> CampaignId {
        let now_ms = self.clock.now_ms();
        let mut state = lock(&self.state);
        state.create_campaign(host, name, 4, ContentRating::Teen, false, now_ms)
    }

    pub fn seed_member(&self, campaign: CampaignId, member: &Actor, role: CampaignRole) {
        if let Some(campaign) = lock(&self.state).campaigns.get_mut(&campaign) {
            campaign.members.push(Member {
                account: member.account,
                name: member.display_name.clone(),
                role,
            });
        }
    }

    /// Opens the campaign's lobby as its host.
    pub fn seed_lobby(&self, campaign: CampaignId) -> Option<SessionId> {
        lock(&self.state).open_lobby_for(campaign)
    }

    /// Creates an invitation the way the host would and returns its once-shown link.
    pub fn seed_invitation(&self, campaign: CampaignId, kind: InvitationKind) -> Option<String> {
        let now_ms = self.clock.now_ms();
        lock(&self.state).add_invitation(campaign, kind, now_ms, &self.origin)
    }

    pub fn seed_join_request(&self, campaign: CampaignId, requester: &Actor) {
        let now_ms = self.clock.now_ms();
        lock(&self.state).add_request(campaign, requester, now_ms);
    }

    /// Seeds one finished or running play session in an account's history.
    pub fn seed_play_session(&self, account: AccountId, row: PlaySessionRow) {
        let mut state = lock(&self.state);
        state.play_sessions.push((account, row));
        state
            .play_sessions
            .sort_by_key(|(_, row)| std::cmp::Reverse(row.started_at_ms));
    }

    /// The room code of a campaign that takes join requests by code.
    pub fn room_code(&self, campaign: CampaignId) -> Option<String> {
        lock(&self.state)
            .campaigns
            .get(&campaign)
            .and_then(|campaign| campaign.room_code.clone())
    }
}

impl CampaignDirectory for FakeCampaigns {
    async fn dashboard(&self, who: &Actor) -> PortResult<DashboardView> {
        self.run(|state, _, _| Ok(state.dashboard(who)))
    }

    async fn play_sessions(
        &self,
        account: AccountId,
        cursor: Option<&str>,
    ) -> PortResult<PlaySessionsView> {
        let start = match cursor {
            None => 0,
            Some(text) => text.parse::<usize>().map_err(|_| PortError::NotFound)?,
        };
        self.run(|state, _, _| {
            let rows: Vec<PlaySessionRow> = state
                .play_sessions
                .iter()
                .filter(|(owner, _)| *owner == account)
                .map(|(_, row)| row.clone())
                .collect();
            let page: Vec<PlaySessionRow> = rows.iter().skip(start).take(20).cloned().collect();
            let end = start + page.len();
            Ok(PlaySessionsView {
                next_cursor: (end < rows.len()).then(|| end.to_string()),
                sessions: page,
            })
        })
    }

    async fn funding_targets(&self, account: AccountId) -> PortResult<Vec<FundingTarget>> {
        self.run(|state, _, _| Ok(state.funding_targets(account)))
    }

    async fn closure_blockers(&self, account: AccountId) -> PortResult<Vec<String>> {
        self.run(|state, _, _| Ok(state.hosted_names(account)))
    }

    async fn wizard_catalog(&self) -> PortResult<WizardCatalog> {
        Ok(State::wizard_catalog())
    }

    async fn start_draft(&self, who: &Actor) -> PortResult<(String, CampaignDraft)> {
        self.run(|state, _, _| Ok(state.start_draft(who)))
    }

    async fn save_draft(
        &self,
        who: &Actor,
        draft_id: &str,
        _step: u8,
        draft: CampaignDraft,
    ) -> PortResult<CampaignDraft> {
        self.run(|state, _, _| state.save_draft(who, draft_id, draft))
    }

    async fn finish_draft(
        &self,
        who: &Actor,
        draft_id: &str,
        operation: OperationId,
    ) -> PortResult<CampaignId> {
        self.run(|state, _, now_ms| state.finish_draft(who, draft_id, operation, now_ms))
    }

    async fn campaign(&self, who: &Actor, campaign: CampaignId) -> PortResult<CampaignOverview> {
        self.run(|state, _, _| state.overview(who, campaign))
    }

    async fn create_invitation(
        &self,
        who: &Actor,
        campaign: CampaignId,
        kind: InvitationKind,
        operation: OperationId,
    ) -> PortResult<InvitationCreated> {
        self.run(|state, fake, now_ms| {
            state.create_invitation(who, campaign, kind, operation, now_ms, &fake.origin)
        })
    }

    async fn revoke_invitation(
        &self,
        who: &Actor,
        campaign: CampaignId,
        invitation_id: &str,
    ) -> PortResult<CampaignOverview> {
        self.run(|state, _, _| state.revoke_invitation(who, campaign, invitation_id))
    }

    async fn rotate_code(&self, who: &Actor, campaign: CampaignId) -> PortResult<CampaignOverview> {
        self.run(|state, _, _| state.rotate_code(who, campaign))
    }

    async fn decide_join_request(
        &self,
        who: &Actor,
        campaign: CampaignId,
        request_id: &str,
        decision: RequestDecision,
    ) -> PortResult<CampaignOverview> {
        self.run(|state, _, _| state.decide_request(who, campaign, request_id, decision))
    }

    async fn leave(&self, who: &Actor, campaign: CampaignId) -> PortResult<()> {
        self.run(|state, _, _| state.leave(who, campaign))
    }

    async fn open_lobby(&self, who: &Actor, campaign: CampaignId) -> PortResult<SessionId> {
        self.run(|state, _, _| state.open_lobby_as(who, campaign))
    }

    async fn lobby(&self, who: &Actor, session: SessionId) -> PortResult<LobbyView> {
        self.run(|state, _, _| state.lobby_view(who, session))
    }

    async fn choose_character(
        &self,
        who: &Actor,
        session: SessionId,
        request: CharacterRequest,
    ) -> PortResult<LobbyView> {
        self.run(|state, _, _| state.choose_character(who, session, &request))
    }

    async fn set_ready(
        &self,
        who: &Actor,
        session: SessionId,
        is_ready: bool,
    ) -> PortResult<LobbyView> {
        self.run(|state, _, _| state.set_ready(who, session, is_ready))
    }

    async fn report_microphone(
        &self,
        who: &Actor,
        session: SessionId,
        report: MicrophoneRequest,
    ) -> PortResult<LobbyView> {
        self.run(|state, _, _| state.report_microphone(who, session, &report))
    }

    async fn start_session(&self, who: &Actor, session: SessionId) -> PortResult<LobbyView> {
        self.run(|state, _, _| state.start_session(who, session))
    }

    async fn lookup_display(
        &self,
        who: &Actor,
        session: SessionId,
        code: &str,
    ) -> PortResult<PairingLookup> {
        self.run(|state, _, now_ms| state.lookup_display(who, session, code, now_ms))
    }

    async fn approve_display(
        &self,
        who: &Actor,
        session: SessionId,
        pairing_id: &str,
    ) -> PortResult<LobbyView> {
        self.run(|state, _, now_ms| state.approve_display(who, session, pairing_id, now_ms))
    }

    async fn unpair_display(
        &self,
        who: &Actor,
        session: SessionId,
        ticket_id: &str,
    ) -> PortResult<LobbyView> {
        self.run(|state, _, _| state.unpair_display(who, session, Some(ticket_id)))
    }

    async fn unpair_all_displays(&self, who: &Actor, session: SessionId) -> PortResult<LobbyView> {
        self.run(|state, _, _| state.unpair_display(who, session, None))
    }

    async fn join_preview(&self, token: &str) -> PortResult<InvitePreview> {
        self.run(|state, _, now_ms| state.preview_invitation(token, now_ms).map(|found| found.1))
    }

    async fn join_accept(
        &self,
        who: &Actor,
        token: &str,
        operation: OperationId,
    ) -> PortResult<CampaignId> {
        self.run(|state, _, now_ms| state.accept_invitation(who, token, operation, now_ms))
    }

    async fn code_lookup(&self, who: &Actor, code: &str) -> PortResult<InvitePreview> {
        self.run(|state, _, _| state.lookup_code(who, code).map(|found| found.1))
    }

    async fn code_request(
        &self,
        who: &Actor,
        code: &str,
        operation: OperationId,
    ) -> PortResult<JoinRequested> {
        self.run(|state, _, now_ms| state.request_by_code(who, code, operation, now_ms))
    }

    async fn display_challenge(&self, device: DeviceLabel) -> PortResult<IssuedChallenge> {
        self.run(|state, _, now_ms| Ok(state.issue_challenge(device, now_ms)))
    }

    async fn display_status(
        &self,
        device_secret: &str,
    ) -> PortResult<df_account_app::wire_campaign::DisplayState> {
        self.run(|state, _, now_ms| state.pairing_status(device_secret, now_ms))
    }

    async fn display_redeem(&self, device_secret: &str) -> PortResult<RedeemedTicket> {
        self.run(|state, _, now_ms| state.redeem_pairing(device_secret, now_ms))
    }

    async fn display_renew(&self, device_secret: &str, ticket: &str) -> PortResult<RenewedTicket> {
        self.run(|state, _, now_ms| state.renew_ticket(device_secret, ticket, now_ms))
    }
}

pub(super) fn device_kind(label: DeviceLabel) -> DeviceKind {
    match label {
        DeviceLabel::Phone => DeviceKind::Phone,
        DeviceLabel::Tablet => DeviceKind::Tablet,
        DeviceLabel::Desktop => DeviceKind::Desktop,
        DeviceLabel::Unknown => DeviceKind::Unknown,
    }
}

/// `ABCD-EFGH` style code from random hex digits (the real directory uses an unambiguous alphabet).
pub(super) fn fresh_code() -> String {
    let digits = fresh_hex().to_ascii_uppercase();
    format!("{}-{}", &digits[..4], &digits[4..8])
}
