//! Fake campaign directory (TODO(S1): replaced by `df-campaign`). Campaigns, invitations, room
//! codes, the lobby and display pairing in memory, with the membership rules the screens
//! depend on: only members see a campaign, only the host or a co-host invites or starts, a
//! foreign id is `not_found`, and every invitation failure carries a closed reason.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use df_account_app::{
    wire::{PlaySessionRow, PlaySessionsView},
    wire_campaign::{
        CampaignDraft, CampaignOverview, CampaignRole, ContentRating, DashboardView,
        InvitationCreated, InvitationKind, InvitePreview, JoinRequested, LobbyView,
    },
};
use df_customer::Clock;
use df_types::{AccountId, CampaignId, OperationId, SessionId};

use super::{Receipts, fresh_hex, lock};
use crate::app_api::ports::{
    Actor, CampaignDirectory, FundingTarget, PortError, PortResult, RequestDecision,
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
    pub settings: CampaignDraft,
    pub members: Vec<Member>,
    pub invitations: Vec<Invitation>,
    pub requests: Vec<JoinRequest>,
    pub room_code: Option<String>,
    pub seats: u8,
    pub rating: ContentRating,
    pub live_session: Option<SessionId>,
    pub last_saved_at_ms: Option<u64>,
}

pub(super) struct Lobby {
    pub campaign: CampaignId,
    pub has_started: bool,
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
    pub receipts: Receipts<Receipt>,
    pub last_ratings: HashMap<AccountId, ContentRating>,
    pub game_links: HashMap<SessionId, df_account_app::wire_campaign::GameLinks>,
    /// A local preview's one running game, used for every started session without its own links.
    pub local_game: Option<df_account_app::wire_campaign::GameLinks>,
    pub launcher_urls: HashMap<SessionId, String>,
    pub fail_next_open: bool,
    /// Play history per account, newest first (seeded; S1's real adapter reads its store).
    pub play_sessions: Vec<(AccountId, PlaySessionRow)>,
}

pub struct FakeCampaigns {
    pub(super) origin: String,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    #[cfg(feature = "test-fakes")]
    pub(super) runtime: Mutex<Option<Arc<super::campaigns_runtime::LocalRuntime>>>,
}

impl FakeCampaigns {
    pub fn new(origin: &str, clock: Arc<dyn Clock>) -> Self {
        Self {
            origin: origin.to_owned(),
            clock,
            state: Mutex::new(State::default()),
            #[cfg(feature = "test-fakes")]
            runtime: Mutex::new(None),
        }
    }

    pub(super) fn run<T>(
        &self,
        work: impl FnOnce(&mut State, &Self, u64) -> PortResult<T>,
    ) -> PortResult<T> {
        work(&mut lock(&self.state), self, self.clock.now_ms())
    }

    pub fn fail_next_open(&self) {
        lock(&self.state).fail_next_open = true;
    }

    /// Points every started session without its own links at one running local game (the
    /// preview's DF_PREVIEW_GAME_DISPLAY and DF_PREVIEW_GAME_PLAYER), until hosted sessions get
    /// their own placed runtime.
    pub fn use_local_game(&self, links: df_account_app::wire_campaign::GameLinks) {
        lock(&self.state).local_game = Some(links);
    }

    /// Registers a fixture runtime's already placed session links.
    pub fn seed_game_links(
        &self,
        session: SessionId,
        links: df_account_app::wire_campaign::GameLinks,
    ) {
        let mut state = lock(&self.state);
        if state.lobbies.contains_key(&session) {
            state.game_links.insert(session, links);
        }
    }

    /// Registers a fixture runtime's launcher address for one placed session.
    pub fn seed_launcher_url(&self, session: SessionId, url: String) {
        let mut state = lock(&self.state);
        if state.lobbies.contains_key(&session) {
            state.launcher_urls.insert(session, url);
        }
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
    async fn launcher_url(&self, who: &Actor, session: SessionId) -> PortResult<Option<String>> {
        self.run(|state, _, _| {
            state.lobby_for(who, session)?;
            Ok(state.launcher_urls.get(&session).cloned())
        })
    }

    async fn game_links(
        &self,
        who: &Actor,
        session: SessionId,
    ) -> PortResult<Option<df_account_app::wire_campaign::GameLinks>> {
        #[cfg(feature = "test-fakes")]
        self.allocate_runtime(who, session).await?;
        self.run(|state, _, _| {
            state.lobby_for(who, session)?;
            Ok(state
                .game_links
                .get(&session)
                .or(state.local_game.as_ref())
                .cloned())
        })
    }

    async fn play_now_campaign(
        &self,
        who: &Actor,
        operation: OperationId,
    ) -> PortResult<CampaignId> {
        self.run(|state, _, now_ms| {
            if let Some(Receipt::Joined(campaign)) = state.receipts.replay(who.account, operation) {
                return Ok(campaign);
            }
            let mut settings = super::campaigns_rules::default_draft();
            settings.name = "The Broken Seal".to_owned();
            settings.adventure_id = Some("harbor-journey".to_owned());
            settings.content_rating = Some(
                state
                    .last_ratings
                    .get(&who.account)
                    .copied()
                    .unwrap_or(ContentRating::Teen),
            );
            settings.seat_count = 6;
            settings.pace = df_account_app::wire::PacePreset::Immersive;
            settings.join_mode = df_account_app::wire_campaign::JoinMode::LinkAndRoomCode;
            let campaign = state.create_campaign(
                who,
                &settings.name,
                settings.seat_count,
                settings.content_rating.ok_or(PortError::Unavailable)?,
                true,
                now_ms,
            );
            if let Some(created) = state.campaigns.get_mut(&campaign) {
                created.settings = settings;
            }
            state
                .receipts
                .record(who.account, operation, Receipt::Joined(campaign));
            Ok(campaign)
        })
    }

    async fn campaign_settings(
        &self,
        who: &Actor,
        campaign: CampaignId,
    ) -> PortResult<df_account_app::wire_campaign::CampaignSettingsView> {
        self.run(|state, _, _| {
            let (found, me) = state.membership(who, campaign)?;
            Ok(df_account_app::wire_campaign::CampaignSettingsView {
                draft: found.settings.clone(),
                catalog: State::wizard_catalog(),
                can_edit: me.role == CampaignRole::Host,
            })
        })
    }

    async fn save_campaign_settings(
        &self,
        who: &Actor,
        campaign: CampaignId,
        draft: CampaignDraft,
    ) -> PortResult<df_account_app::wire_campaign::CampaignSettingsView> {
        self.run(|state, _, _| {
            let (_, me) = state.membership(who, campaign)?;
            if me.role != CampaignRole::Host {
                return Err(PortError::Forbidden);
            }
            super::campaigns_rules::validate_settings(&draft)?;
            let rating = draft.content_rating.ok_or(PortError::Refused(
                df_account_app::wire::Refusal::ContentRatingRequired,
            ))?;
            let found = state.managed(who, campaign)?;
            if draft.seat_count < super::campaigns_rules::seated(found) {
                return Err(PortError::Refused(
                    df_account_app::wire::Refusal::SeatsOutOfRange,
                ));
            }
            found.name = draft.name.trim().to_owned();
            found.rating = rating;
            found.seats = draft.seat_count;
            if draft.join_mode == df_account_app::wire_campaign::JoinMode::LinkAndRoomCode {
                if found.room_code.is_none() {
                    found.room_code = Some(fresh_code());
                }
            } else {
                found.room_code = None;
            }
            found.settings = draft;
            state.last_ratings.insert(who.account, rating);
            Ok(())
        })?;
        self.campaign_settings(who, campaign).await
    }

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

    async fn cancel_lobby(&self, who: &Actor, session: SessionId) -> PortResult<()> {
        self.run(|state, _, _| {
            let (lobby, campaign, member) = state.lobby_for(who, session)?;
            if !member.can_manage() || lobby.has_started {
                return Err(PortError::Forbidden);
            }
            let campaign = campaign.id;
            state.lobbies.remove(&session);
            if let Some(campaign) = state.campaigns.get_mut(&campaign) {
                campaign.live_session = None;
            }
            Ok(())
        })
    }

    async fn lobby(&self, who: &Actor, session: SessionId) -> PortResult<LobbyView> {
        self.run(|state, _, _| state.lobby_view(who, session))
    }

    async fn start_session(&self, who: &Actor, session: SessionId) -> PortResult<LobbyView> {
        let lobby = self.run(|state, _, _| state.start_session(who, session))?;
        #[cfg(feature = "test-fakes")]
        self.allocate_runtime(who, session).await?;
        Ok(lobby)
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
}

/// `ABCD-EFGH` style code from random hex digits (the real directory uses an unambiguous alphabet).
pub(super) fn fresh_code() -> String {
    let digits = fresh_hex().to_ascii_uppercase();
    format!("{}-{}", &digits[..4], &digits[4..8])
}
