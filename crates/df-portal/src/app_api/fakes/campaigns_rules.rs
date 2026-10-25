//! The membership, draft and lobby rules of the fake campaign directory (TODO(S1)).

use df_account_app::wire_campaign::FundingNote;
use df_account_app::{
    wire::{ErrorCode, PacePreset, PresetOffer, Refusal},
    wire_campaign::{
        AdventureOption, CampaignCard, CampaignDraft, CampaignOverview, CampaignRole,
        ContentRating, DashboardView, FundingModel, HeroChoice, InvitationRow, JoinMode,
        JoinRequestRow, LobbyMember, LobbyView, MemberRow, MicrophonePermission, MicrophoneRequest,
        MicrophoneState, PictureDefault, Reason, RoomCodeState, StartBlocker, WizardCatalog,
    },
};
use df_types::{AccountId, CampaignId, OperationId, SessionId};

use super::{
    campaigns::{Campaign, Lobby, Member, Receipt, State, fresh_code},
    fresh_hex,
};
use crate::app_api::{
    ids::hex,
    ports::{Actor, FundingTarget, PortError, PortResult},
};

const MAXIMUM_SEATS: u8 = 6;
const MAXIMUM_DRAFTS: usize = 32;
const LOBBY_POLL_SECONDS: u32 = 3;
const CUSTOM_MINIMUM_CENTS_PER_HOUR: u32 = 100;
const CUSTOM_MAXIMUM_CENTS_PER_HOUR: u32 = 5_000;

const fn code_for(reason: Reason) -> ErrorCode {
    match reason {
        Reason::Expired
        | Reason::Revoked
        | Reason::Exhausted
        | Reason::NoLongerValid
        | Reason::NotTakingRequests => ErrorCode::NotFound,
        Reason::Full
        | Reason::ContributorsFull
        | Reason::AlreadyJoined
        | Reason::RequestsFull
        | Reason::AllowBackFirst
        | Reason::DisplayLimit => ErrorCode::Invalid,
        Reason::StaleRevision => ErrorCode::StaleForm,
        Reason::AccountClosing => ErrorCode::Forbidden,
        Reason::EmailUnverified => ErrorCode::NotVerified,
    }
}

pub(super) const fn because(reason: Reason) -> PortError {
    PortError::Because(code_for(reason), reason)
}

pub(super) fn default_draft() -> CampaignDraft {
    CampaignDraft {
        name: String::new(),
        adventure_id: None,
        content_rating: None,
        seat_count: 4,
        hero_choice: HeroChoice::InLobby,
        has_voice: true,
        pictures: PictureDefault::Automatic,
        pace: PacePreset::Story,
        custom_ceiling_cents_per_hour: None,
        co_hosts_can_invite: false,
        co_hosts_can_start: true,
        co_hosts_can_export: false,
        join_mode: JoinMode::LinkOnly,
        funding: FundingModel::Sponsor,
    }
}

impl State {
    pub(super) fn create_campaign(
        &mut self,
        host: &Actor,
        name: &str,
        seats: u8,
        rating: ContentRating,
        has_room_code: bool,
        now_ms: u64,
    ) -> CampaignId {
        let id = loop {
            if let Ok(id) = CampaignId::from_hex(&fresh_hex()) {
                break id;
            }
        };
        self.campaigns.insert(
            id,
            Campaign {
                id,
                name: name.to_owned(),
                members: vec![Member {
                    account: host.account,
                    name: host.display_name.clone(),
                    role: CampaignRole::Host,
                }],
                invitations: Vec::new(),
                requests: Vec::new(),
                room_code: has_room_code.then(fresh_code),
                seats,
                rating,
                live_session: None,
                last_saved_at_ms: Some(now_ms),
            },
        );
        id
    }

    /// The campaign and the caller's membership; anything else is `not_found`.
    fn membership(&self, who: &Actor, campaign: CampaignId) -> PortResult<(&Campaign, &Member)> {
        let found = self.campaigns.get(&campaign).ok_or(PortError::NotFound)?;
        let member = found
            .members
            .iter()
            .find(|member| member.account == who.account)
            .ok_or(PortError::NotFound)?;
        Ok((found, member))
    }

    pub(super) fn managed(
        &mut self,
        who: &Actor,
        campaign: CampaignId,
    ) -> PortResult<&mut Campaign> {
        let (_, member) = self.membership(who, campaign)?;
        if !member.can_manage() {
            return Err(PortError::Forbidden);
        }
        self.campaigns.get_mut(&campaign).ok_or(PortError::NotFound)
    }

    pub(super) fn dashboard(&self, who: &Actor) -> DashboardView {
        let mut campaigns: Vec<CampaignCard> = self
            .campaigns
            .values()
            .filter_map(|campaign| {
                let member = campaign
                    .members
                    .iter()
                    .find(|member| member.account == who.account)?;
                Some(CampaignCard {
                    campaign_id: hex(campaign.id.as_bytes()),
                    name: campaign.name.clone(),
                    role: member.role,
                    host_name: host_name(campaign),
                    party_size: seated(campaign),
                    last_played_at_ms: None,
                    last_saved_at_ms: campaign.last_saved_at_ms,
                    live_session_id: campaign.live_session.map(|id| hex(id.as_bytes())),
                    is_archived: false,
                })
            })
            .collect();
        campaigns.sort_by(|left, right| left.name.cmp(&right.name));
        DashboardView {
            campaigns,
            can_create: who.is_verified,
        }
    }

    pub(super) fn funding_targets(&self, account: AccountId) -> Vec<FundingTarget> {
        let mut targets: Vec<FundingTarget> = self
            .campaigns
            .values()
            .filter_map(|campaign| {
                let member = campaign
                    .members
                    .iter()
                    .find(|member| member.account == account)?;
                Some(FundingTarget {
                    campaign: campaign.id,
                    name: campaign.name.clone(),
                    can_choose_pace: member.role == CampaignRole::Host,
                    live_session: campaign.live_session,
                })
            })
            .collect();
        targets.sort_by(|left, right| left.name.cmp(&right.name));
        targets
    }

    pub(super) fn hosted_names(&self, account: AccountId) -> Vec<String> {
        self.campaigns
            .values()
            .filter(|campaign| {
                campaign
                    .members
                    .iter()
                    .any(|member| member.account == account && member.role == CampaignRole::Host)
            })
            .map(|campaign| campaign.name.clone())
            .collect()
    }

    pub(super) fn wizard_catalog() -> WizardCatalog {
        let option = |id: &str, name: &str, summary: &str| AdventureOption {
            adventure_id: id.to_owned(),
            name: name.to_owned(),
            summary: summary.to_owned(),
            ratings: vec![ContentRating::Teen, ContentRating::Mature],
            rules_text: "Plays by the fifth edition SRD 5.2.1 rules.".to_owned(),
        };
        WizardCatalog {
            adventures: vec![
                option(
                    "lantern-road",
                    "The Lantern Road",
                    "A caravan, a missing lamplighter and a road that changes at night.",
                ),
                option(
                    "ashes-of-vale",
                    "Ashes of Vale",
                    "A burned village, a sealed cellar and a stranger who knows your names.",
                ),
            ],
            maximum_seats: MAXIMUM_SEATS,
            presets: [
                (PacePreset::Story, Some(300)),
                (PacePreset::Immersive, Some(800)),
                (PacePreset::Cinematic, Some(2_000)),
                (PacePreset::Custom, None),
            ]
            .map(|(preset, ceiling_cents_per_hour)| PresetOffer {
                preset,
                ceiling_cents_per_hour,
            })
            .to_vec(),
            is_proposed_pricing: true,
            custom_minimum_cents_per_hour: CUSTOM_MINIMUM_CENTS_PER_HOUR,
            custom_maximum_cents_per_hour: CUSTOM_MAXIMUM_CENTS_PER_HOUR,
        }
    }

    pub(super) fn start_draft(&mut self, who: &Actor) -> (String, CampaignDraft) {
        let draft_id = fresh_hex();
        if self.drafts.len() >= MAXIMUM_DRAFTS {
            self.drafts.remove(0);
        }
        self.drafts
            .push((draft_id.clone(), who.account, default_draft()));
        (draft_id, default_draft())
    }

    pub(super) fn save_draft(
        &mut self,
        who: &Actor,
        draft_id: &str,
        draft: CampaignDraft,
    ) -> PortResult<CampaignDraft> {
        let stored = self
            .drafts
            .iter_mut()
            .find(|(id, account, _)| id == draft_id && *account == who.account)
            .ok_or(PortError::NotFound)?;
        stored.2 = draft.clone();
        Ok(draft)
    }

    pub(super) fn finish_draft(
        &mut self,
        who: &Actor,
        draft_id: &str,
        operation: OperationId,
        now_ms: u64,
    ) -> PortResult<CampaignId> {
        if let Some(Receipt::Joined(first)) = self.receipts.replay(who.account, operation) {
            return Ok(first);
        }
        let position = self
            .drafts
            .iter()
            .position(|(id, account, _)| id == draft_id && *account == who.account)
            .ok_or(PortError::NotFound)?;
        let draft = self.drafts[position].2.clone();
        let name = draft.name.trim();
        if name.is_empty() {
            return Err(PortError::Refused(Refusal::CampaignNameRequired));
        }
        if name.chars().count() > 60 {
            return Err(PortError::Refused(Refusal::CampaignNameTooLong));
        }
        if draft.adventure_id.is_none() {
            return Err(PortError::Refused(Refusal::AdventureRequired));
        }
        let rating = draft
            .content_rating
            .ok_or(PortError::Refused(Refusal::ContentRatingRequired))?;
        if !(1..=MAXIMUM_SEATS).contains(&draft.seat_count) {
            return Err(PortError::Refused(Refusal::SeatsOutOfRange));
        }
        let custom_is_valid = draft.custom_ceiling_cents_per_hour.is_some_and(|ceiling| {
            (CUSTOM_MINIMUM_CENTS_PER_HOUR..=CUSTOM_MAXIMUM_CENTS_PER_HOUR).contains(&ceiling)
        });
        if (draft.pace == PacePreset::Custom) != draft.custom_ceiling_cents_per_hour.is_some()
            || (draft.pace == PacePreset::Custom && !custom_is_valid)
        {
            return Err(PortError::Refused(Refusal::CeilingOutOfRange));
        }
        let has_code = draft.join_mode == JoinMode::LinkAndRoomCode;
        let id = self.create_campaign(who, name, draft.seat_count, rating, has_code, now_ms);
        self.drafts.remove(position);
        self.receipts
            .record(who.account, operation, Receipt::Joined(id));
        Ok(id)
    }

    pub(super) fn overview(
        &self,
        who: &Actor,
        campaign: CampaignId,
    ) -> PortResult<CampaignOverview> {
        let (found, me) = self.membership(who, campaign)?;
        let manages = me.can_manage();
        Ok(CampaignOverview {
            campaign_id: hex(found.id.as_bytes()),
            name: found.name.clone(),
            my_role: me.role,
            can_invite: manages,
            can_open_lobby: manages && found.live_session.is_none(),
            members: found
                .members
                .iter()
                .map(|member| MemberRow {
                    display_name: member.name.clone(),
                    role: member.role,
                    is_me: member.account == who.account,
                })
                .collect(),
            invitations: if manages {
                found
                    .invitations
                    .iter()
                    .filter(|invitation| !invitation.is_revoked)
                    .map(|invitation| InvitationRow {
                        invitation_id: invitation.id.clone(),
                        kind: invitation.kind,
                        uses: invitation.uses,
                        maximum_uses: invitation.maximum_uses,
                        expires_at_ms: invitation.expires_at_ms,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            join_requests: if manages {
                found
                    .requests
                    .iter()
                    .map(|request| JoinRequestRow {
                        request_id: request.id.clone(),
                        display_name: request.name.clone(),
                        requested_at_ms: request.requested_at_ms,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            room_code: match (&found.room_code, manages) {
                (Some(code), true) => RoomCodeState::Active { code: code.clone() },
                _ => RoomCodeState::Off,
            },
            live_session_id: found.live_session.map(|id| hex(id.as_bytes())),
            last_saved_at_ms: found.last_saved_at_ms,
        })
    }

    pub(super) fn leave(&mut self, who: &Actor, campaign: CampaignId) -> PortResult<()> {
        let (_, me) = self.membership(who, campaign)?;
        if me.role == CampaignRole::Host {
            return Err(PortError::Forbidden);
        }
        let found = self
            .campaigns
            .get_mut(&campaign)
            .ok_or(PortError::NotFound)?;
        found.members.retain(|member| member.account != who.account);
        if let Some(lobby) = found.live_session.and_then(|id| self.lobbies.get_mut(&id)) {
            lobby.characters.remove(&who.account);
        }
        Ok(())
    }

    pub(super) fn open_lobby_for(&mut self, campaign: CampaignId) -> Option<SessionId> {
        let found = self.campaigns.get_mut(&campaign)?;
        if let Some(live) = found.live_session {
            return Some(live);
        }
        let session = SessionId::from_hex(&fresh_hex()).ok()?;
        found.live_session = Some(session);
        self.lobbies.insert(
            session,
            Lobby {
                campaign,
                ready: Vec::new(),
                characters: std::collections::HashMap::new(),
                microphones: std::collections::HashMap::new(),
                tickets: Vec::new(),
                has_started: false,
            },
        );
        Some(session)
    }

    pub(super) fn open_lobby_as(
        &mut self,
        who: &Actor,
        campaign: CampaignId,
    ) -> PortResult<SessionId> {
        self.managed(who, campaign)?;
        self.open_lobby_for(campaign).ok_or(PortError::Unavailable)
    }

    /// The lobby and its campaign, for a caller who is a member of that campaign.
    pub(super) fn lobby_for(
        &self,
        who: &Actor,
        session: SessionId,
    ) -> PortResult<(&Lobby, &Campaign, &Member)> {
        let lobby = self.lobbies.get(&session).ok_or(PortError::NotFound)?;
        let (campaign, member) = self.membership(who, lobby.campaign)?;
        Ok((lobby, campaign, member))
    }

    pub(super) fn lobby_view(&self, who: &Actor, session: SessionId) -> PortResult<LobbyView> {
        let (lobby, campaign, me) = self.lobby_for(who, session)?;
        let seated_members: Vec<&Member> = campaign
            .members
            .iter()
            .filter(|member| member.has_seat())
            .collect();
        let mut start_blockers = Vec::new();
        if lobby.has_started {
            start_blockers.push(StartBlocker::SessionNotInLobby);
        }
        if seated_members.is_empty() {
            start_blockers.push(StartBlocker::NoSeatedMembers);
        }
        if seated_members
            .iter()
            .any(|member| !lobby.ready.contains(&member.account))
        {
            start_blockers.push(StartBlocker::MemberNotReady);
        }
        if seated_members.iter().any(|member| {
            !lobby.characters.get(&member.account).is_some_and(|choice| {
                *choice != df_account_app::wire_campaign::CharacterChoice::Undecided
            })
        }) {
            start_blockers.push(StartBlocker::CharacterNotChosen);
        }
        Ok(LobbyView {
            session_id: hex(session.as_bytes()),
            campaign_id: hex(campaign.id.as_bytes()),
            campaign_name: campaign.name.clone(),
            can_manage: me.can_manage(),
            start_blockers,
            members: seated_members
                .iter()
                .map(|member| LobbyMember {
                    display_name: member.name.clone(),
                    character_name: lobby
                        .characters
                        .get(&member.account)
                        .filter(|choice| {
                            **choice != df_account_app::wire_campaign::CharacterChoice::Undecided
                        })
                        .map(|_| format!("{}'s hero", member.name)),
                    is_ready: lobby.ready.contains(&member.account),
                    microphone: lobby
                        .microphones
                        .get(&member.account)
                        .copied()
                        .unwrap_or(MicrophoneState::NotAsked),
                    is_me: member.account == who.account,
                })
                .collect(),
            displays: lobby
                .tickets
                .iter()
                .map(|ticket| df_account_app::wire_campaign::DisplayRow {
                    ticket_id: ticket.ticket_id.clone(),
                    device: ticket.device,
                    paired_at_ms: ticket.paired_at_ms,
                })
                .collect(),
            funding_note: FundingNote::NoChargeUntilHostStarts,
            has_started: lobby.has_started,
            poll_seconds: LOBBY_POLL_SECONDS,
        })
    }

    fn seated_lobby(&mut self, who: &Actor, session: SessionId) -> PortResult<&mut Lobby> {
        let (lobby, _, me) = self.lobby_for(who, session)?;
        if !me.has_seat() {
            return Err(PortError::Forbidden);
        }
        if lobby.has_started {
            return Err(because(Reason::StaleRevision));
        }
        self.lobbies.get_mut(&session).ok_or(PortError::NotFound)
    }

    pub(super) fn choose_character(
        &mut self,
        who: &Actor,
        session: SessionId,
        request: &df_account_app::wire_campaign::CharacterRequest,
    ) -> PortResult<LobbyView> {
        if (request.choice != df_account_app::wire_campaign::CharacterChoice::Undecided)
            != request.reference.is_some()
        {
            return Err(PortError::Forbidden);
        }
        if let Some(reference) = &request.reference {
            CampaignId::from_hex(reference).map_err(|_| PortError::NotFound)?;
        }
        let lobby = self.seated_lobby(who, session)?;
        lobby.characters.insert(who.account, request.choice);
        lobby.ready.retain(|account| *account != who.account);
        self.lobby_view(who, session)
    }

    pub(super) fn set_ready(
        &mut self,
        who: &Actor,
        session: SessionId,
        is_ready: bool,
    ) -> PortResult<LobbyView> {
        let lobby = self.seated_lobby(who, session)?;
        lobby.ready.retain(|account| *account != who.account);
        if is_ready {
            if !lobby.characters.get(&who.account).is_some_and(|choice| {
                *choice != df_account_app::wire_campaign::CharacterChoice::Undecided
            }) {
                return Err(because(Reason::StaleRevision));
            }
            lobby.ready.push(who.account);
        }
        self.lobby_view(who, session)
    }

    pub(super) fn report_microphone(
        &mut self,
        who: &Actor,
        session: SessionId,
        report: &MicrophoneRequest,
    ) -> PortResult<LobbyView> {
        let lobby = self.seated_lobby(who, session)?;
        let state = if report.prefers_typing {
            MicrophoneState::Typing
        } else {
            match report.permission {
                Some(MicrophonePermission::Granted) => MicrophoneState::Granted,
                Some(MicrophonePermission::Declined) => MicrophoneState::Declined,
                Some(MicrophonePermission::Unsupported) => MicrophoneState::Unsupported,
                None => MicrophoneState::NotAsked,
            }
        };
        lobby.microphones.insert(who.account, state);
        self.lobby_view(who, session)
    }

    pub(super) fn start_session(
        &mut self,
        who: &Actor,
        session: SessionId,
    ) -> PortResult<LobbyView> {
        let (_, _, me) = self.lobby_for(who, session)?;
        if !me.can_manage() {
            return Err(PortError::Forbidden);
        }
        if !self.lobby_view(who, session)?.start_blockers.is_empty() {
            return Err(because(Reason::StaleRevision));
        }
        if let Some(lobby) = self.lobbies.get_mut(&session) {
            lobby.has_started = true;
        }
        self.lobby_view(who, session)
    }
}

pub(super) fn seated(campaign: &Campaign) -> u8 {
    let count = campaign
        .members
        .iter()
        .filter(|member| member.has_seat())
        .count();
    u8::try_from(count).unwrap_or(u8::MAX)
}

pub(super) fn host_name(campaign: &Campaign) -> String {
    campaign
        .members
        .iter()
        .find(|member| member.role == CampaignRole::Host)
        .map_or_else(String::new, |member| member.name.clone())
}
