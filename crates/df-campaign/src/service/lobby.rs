//! The lobby (8.4): opening it, the view every member polls, a seated member's character, ready
//! state and microphone, and Start. The session itself is O1's; the funding answer is F's; this
//! service owns the seated members' lobby state and the rule that decides whether Start may go.

use df_types::{CampaignId, MemberId, SessionId};

use super::{CampaignService, Ports, refuse};
use crate::{
    audit::{AuditLine, failure_word},
    errors::{Failure, Outcome, Refusal, StoreError},
    lobby::{CharacterChoice, LobbySeat, LobbySnapshot, StartBlocker, decide_start},
    lobby_state::{
        LobbyDisplay, LobbyMemberRow, LobbyRows, MicrophonePreference, MicrophoneReport,
        SeatedMember, StartReadiness,
    },
    records::{CampaignRow, CampaignStatus, MemberRow, MemberStatus},
    roles::Capability,
    store::{AccountDirectory, CampaignStore, FundingDirectory, SessionDirectory, SessionPhase},
};

/// One seated member as the lobby shows them. An unauthorized seated member (a closing or closed
/// account) is shown as unable to play and is left out of Start.
#[derive(Clone, Debug)]
pub struct LobbySeatView {
    pub member: MemberRow,
    pub display_name: Option<String>,
    pub authorized: bool,
    /// The lobby row, or the defaults (undecided, not ready) for a member who has done nothing.
    pub lobby: Option<LobbyMemberRow>,
}

impl LobbySeatView {
    pub fn character(&self) -> CharacterChoice {
        self.lobby
            .as_ref()
            .map_or(CharacterChoice::Undecided, |row| row.character)
    }

    pub fn ready(&self) -> bool {
        self.lobby.as_ref().is_some_and(|row| row.ready)
    }
}

#[derive(Clone, Debug)]
pub struct LobbyView {
    pub campaign: CampaignRow,
    pub session: SessionId,
    pub phase: SessionPhase,
    pub seats: Vec<LobbySeatView>,
    pub displays: Vec<LobbyDisplay>,
    /// Why Start would be refused right now; empty when it would go ahead.
    pub blockers: Vec<StartBlocker>,
    /// F's answer; `None` when F could not be read (then `FundingNotReady` blocks).
    pub funding: Option<StartReadiness>,
    pub you: MemberId,
    /// Whether the caller may open and start (host, or a co-host with the delegation).
    pub can_start: bool,
}

/// What Start did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartOutcome {
    Started { roster: Vec<SeatedMember> },
    Blocked(Vec<StartBlocker>),
}

impl<P: Ports> CampaignService<P> {
    fn lobby_change_allowed(&self, account: df_types::AccountId) -> Outcome<()> {
        let result = self
            .inner
            .throttles
            .lobby_changes
            .attempt(account, self.now_ms());
        self.window_outcome(result, "LOBBY", "lobby.change_saturated")
    }

    fn lobby_host_allowed(&self, account: df_types::AccountId) -> Outcome<()> {
        let result = self
            .inner
            .throttles
            .lobby_host
            .attempt(account, self.now_ms());
        self.window_outcome(result, "LOBBY", "lobby.host_saturated")
    }

    /// The session must belong to the campaign and still be in the lobby; a started or ended
    /// session takes no lobby change.
    async fn lobby_phase(&self, campaign: CampaignId, session: SessionId) -> Outcome<()> {
        match self.session_of(campaign, session).await? {
            SessionPhase::Lobby => Ok(()),
            SessionPhase::Ended => refuse(Refusal::SessionEnded),
            _ => refuse(Refusal::SessionInProgress),
        }
    }

    /// Opens the campaign's single non-ended session in the lobby, or returns the one that
    /// exists. A campaign that is not active opens nothing.
    pub async fn open_lobby(
        &self,
        actor: df_types::AccountId,
        campaign: CampaignId,
    ) -> Outcome<(SessionId, SessionPhase)> {
        let snapshot = self
            .require_full_capability(actor, campaign, Capability::StartOrPauseSession)
            .await?;
        self.lobby_host_allowed(actor)?;
        if snapshot.campaign_status != CampaignStatus::Active {
            return refuse(Refusal::CampaignUnavailable);
        }
        let opened = self.inner.sessions.open_session(campaign).await?;
        AuditLine::new("LOBBY", "opened")
            .campaign(campaign)
            .session(opened.0)
            .actor(actor)
            .reason("ok")
            .emit("lobby.opened");
        Ok(opened)
    }

    /// The seats as `decide_start` needs them, with each member's account checked.
    async fn seat_views(&self, rows: LobbyRows) -> Vec<LobbySeatView> {
        let mut seats = Vec::with_capacity(rows.seats.len());
        for (member, lobby) in rows.seats {
            // A profile that cannot be read leaves the seat unauthorized: fail closed.
            let (display_name, authorized) = match self.inner.accounts.profile(member.account).await
            {
                Ok(Some(profile)) => (profile.display_name, profile.status.is_active()),
                _ => (None, false),
            };
            seats.push(LobbySeatView {
                authorized: authorized && member.status == MemberStatus::Active,
                member,
                display_name,
                lobby,
            });
        }
        seats
    }

    fn blockers_of(
        &self,
        campaign: &CampaignRow,
        phase: SessionPhase,
        seats: &[LobbySeatView],
        displays: usize,
        funding: Option<StartReadiness>,
    ) -> Result<Vec<SeatedMember>, Vec<StartBlocker>> {
        let facts: Vec<LobbySeat> = seats
            .iter()
            .map(|seat| LobbySeat {
                member: seat.member.member,
                authorized: seat.authorized,
                character: seat.character(),
                ready: seat.ready(),
            })
            .collect();
        let snapshot = LobbySnapshot {
            campaign_active: campaign.status == CampaignStatus::Active,
            session_in_lobby: phase == SessionPhase::Lobby,
            seat_capacity: usize::try_from(campaign.seat_count).unwrap_or(usize::MAX),
            seats: &facts,
            display_required: self.inner.tunables.start_requires_display,
            displays_paired: displays,
            funding_ready: funding.is_some_and(|readiness| readiness.ready),
        };
        decide_start(&snapshot).map(|decision| {
            decision
                .roster
                .into_iter()
                .filter_map(|member| {
                    seats
                        .iter()
                        .find(|seat| seat.member.member == member)
                        .map(|seat| SeatedMember {
                            member,
                            account: seat.member.account,
                        })
                })
                .collect()
        })
    }

    /// What every seated member's screen polls. Any authorized active member may read it.
    pub async fn lobby_view(
        &self,
        actor: df_types::AccountId,
        campaign: CampaignId,
        session: SessionId,
    ) -> Outcome<LobbyView> {
        let Some(snapshot) = self.inner.store.member_snapshot(actor, campaign).await? else {
            return refuse(Refusal::NotFound);
        };
        if snapshot.status != MemberStatus::Active {
            return refuse(Refusal::NotFound);
        }
        if !snapshot.authorized {
            return refuse(Refusal::NotAllowed);
        }
        let phase = self.session_of(campaign, session).await?;
        let Some(rows) = self
            .inner
            .store
            .lobby_rows(session, campaign, self.now_ms())
            .await?
        else {
            return refuse(Refusal::NotFound);
        };
        let campaign_row = rows.campaign.clone();
        let displays = rows.displays.clone();
        let seats = self.seat_views(rows).await;
        let funding = self
            .inner
            .funding
            .start_readiness(campaign, session)
            .await
            .ok();
        let blockers = match self.blockers_of(&campaign_row, phase, &seats, displays.len(), funding)
        {
            Ok(_) => Vec::new(),
            Err(blockers) => blockers,
        };
        let can_start = crate::roles::authorize(
            snapshot.role,
            snapshot.delegations,
            Capability::StartOrPauseSession,
        ) == Ok(crate::roles::Authorized::Full);
        Ok(LobbyView {
            campaign: campaign_row,
            session,
            phase,
            seats,
            displays,
            blockers,
            funding,
            you: snapshot.member,
            can_start,
        })
    }

    pub async fn choose_character(
        &self,
        actor: df_types::AccountId,
        campaign: CampaignId,
        session: SessionId,
        choice: CharacterChoice,
        reference: Option<[u8; 16]>,
    ) -> Outcome<LobbyMemberRow> {
        self.lobby_change_allowed(actor)?;
        self.lobby_phase(campaign, session).await?;
        let result = self
            .inner
            .store
            .choose_character(actor, campaign, session, choice, reference, self.now_ms())
            .await;
        self.audit_lobby("character_chosen", campaign, session, actor, &result);
        result
    }

    pub async fn set_ready(
        &self,
        actor: df_types::AccountId,
        campaign: CampaignId,
        session: SessionId,
        ready: bool,
    ) -> Outcome<LobbyMemberRow> {
        self.lobby_change_allowed(actor)?;
        self.lobby_phase(campaign, session).await?;
        let result = self
            .inner
            .store
            .set_ready(actor, campaign, session, ready, self.now_ms())
            .await;
        self.audit_lobby(
            if ready { "ready" } else { "unready" },
            campaign,
            session,
            actor,
            &result,
        );
        result
    }

    /// The member's preference and the browser's permission result, sent together. `Declined`,
    /// `Unsupported` or a typed preference leaves typed play.
    pub async fn set_microphone(
        &self,
        actor: df_types::AccountId,
        campaign: CampaignId,
        session: SessionId,
        preference: MicrophonePreference,
        report: MicrophoneReport,
    ) -> Outcome<LobbyMemberRow> {
        self.lobby_change_allowed(actor)?;
        self.lobby_phase(campaign, session).await?;
        let result = self
            .inner
            .store
            .set_microphone(actor, campaign, session, preference, report, self.now_ms())
            .await;
        self.audit_lobby("microphone", campaign, session, actor, &result);
        result
    }

    fn audit_lobby(
        &self,
        verb: &'static str,
        campaign: CampaignId,
        session: SessionId,
        actor: df_types::AccountId,
        result: &Outcome<LobbyMemberRow>,
    ) {
        let line = AuditLine::new("LOBBY", verb)
            .campaign(campaign)
            .session(session)
            .actor(actor);
        match result {
            Ok(_) => line.reason("ok").emit(event_of(verb)),
            Err(failure) => line.reason(failure_word(*failure)).emit("lobby.refused"),
        }
    }

    /// Start: the rule decides, finance opens first, then the session owner starts with the
    /// seated, authorized roster (decision 29: finance, then O1, then campaigns). Every blocker
    /// is returned when the rule refuses.
    pub async fn start_session(
        &self,
        actor: df_types::AccountId,
        campaign: CampaignId,
        session: SessionId,
    ) -> Outcome<StartOutcome> {
        self.require_full_capability(actor, campaign, Capability::StartOrPauseSession)
            .await?;
        self.lobby_host_allowed(actor)?;
        let phase = self.session_of(campaign, session).await?;
        let Some(rows) = self
            .inner
            .store
            .lobby_rows(session, campaign, self.now_ms())
            .await?
        else {
            return refuse(Refusal::NotFound);
        };
        let campaign_row = rows.campaign.clone();
        let displays = rows.displays.len();
        let seats = self.seat_views(rows).await;
        let funding = self
            .inner
            .funding
            .start_readiness(campaign, session)
            .await
            .ok();
        let roster = match self.blockers_of(&campaign_row, phase, &seats, displays, funding) {
            Ok(roster) => roster,
            Err(blockers) => {
                for blocker in &blockers {
                    AuditLine::new("LOBBY", "start_blocked")
                        .campaign(campaign)
                        .session(session)
                        .actor(actor)
                        .reason(blocker.word())
                        .emit("lobby.start_blocked");
                }
                return Ok(StartOutcome::Blocked(blockers));
            }
        };
        self.inner
            .funding
            .open_session_funding(campaign, session)
            .await
            .map_err(Failure::Store)?;
        self.inner
            .sessions
            .start_session(session, campaign, &roster)
            .await
            .map_err(|error| match error {
                StoreError::Unknown => Failure::Store(StoreError::Unknown),
                other => Failure::Store(other),
            })?;
        AuditLine::new("LOBBY", "started")
            .campaign(campaign)
            .session(session)
            .actor(actor)
            .reason("ok")
            .emit("lobby.started");
        Ok(StartOutcome::Started { roster })
    }
}

fn event_of(verb: &'static str) -> &'static str {
    match verb {
        "character_chosen" => "lobby.character_chosen",
        "ready" => "lobby.ready",
        "unready" => "lobby.unready",
        _ => "lobby.microphone",
    }
}
