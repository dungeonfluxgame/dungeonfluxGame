//! The lobby's pure rules (`planning/campaigns-and-table-research.md` 4.5): who counts toward
//! Start and what blocks it. No storage, no clock, no ports: the caller supplies the facts it read,
//! and the answer is either the roster to hand to the session owner or every blocker.
//!
//! Only seated, active members whose account is authorized count. A seated member whose account is
//! closing or closed is left out of the roster and out of the readiness counts, so Start never
//! waits for someone who cannot play; the lobby still shows them.

use df_auth::readiness::aggregate_current_readiness;
use df_types::MemberId;

/// How a member chose their hero. `Template` instantiates a template (the characters owner does it
/// idempotently); `Existing` binds a character the campaign already has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterChoice {
    Undecided,
    Template,
    Existing,
}

impl CharacterChoice {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Undecided => "undecided",
            Self::Template => "template",
            Self::Existing => "existing",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "undecided" => Some(Self::Undecided),
            "template" => Some(Self::Template),
            "existing" => Some(Self::Existing),
            _ => None,
        }
    }

    pub const fn is_chosen(self) -> bool {
        !matches!(self, Self::Undecided)
    }
}

/// One member who holds a seat, as the lobby read found them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LobbySeat {
    pub member: MemberId,
    /// The member snapshot's `authorized`: an active member of an active account.
    pub authorized: bool,
    pub character: CharacterChoice,
    pub ready: bool,
}

/// Everything `decide_start` needs, supplied by the caller.
#[derive(Clone, Copy, Debug)]
pub struct LobbySnapshot<'a> {
    /// The campaign is `active` (an archived or deleting campaign never starts).
    pub campaign_active: bool,
    /// The session is still in the lobby state.
    pub session_in_lobby: bool,
    /// The campaign's `seat_count`.
    pub seat_capacity: usize,
    /// Active members that hold a seat.
    pub seats: &'a [LobbySeat],
    /// Whether this deployment needs a paired display before Start.
    pub display_required: bool,
    pub displays_paired: usize,
    /// The funding owner's typed `start_readiness`, never computed here.
    pub funding_ready: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartBlocker {
    CampaignArchived,
    SessionNotInLobby,
    NoSeatedMembers,
    CharacterNotChosen,
    MemberNotReady,
    DisplayRequiredButUnpaired,
    FundingNotReady,
    /// The seats supplied contradict the capacity or each other (a duplicate member, or more
    /// seated members than seats). It cannot happen with the database constraints; Start refuses
    /// instead of guessing.
    RosterInconsistent,
}

impl StartBlocker {
    /// The closed word for audit lines and the wire.
    pub const fn word(self) -> &'static str {
        match self {
            Self::CampaignArchived => "campaign_archived",
            Self::SessionNotInLobby => "session_not_in_lobby",
            Self::NoSeatedMembers => "no_seated_members",
            Self::CharacterNotChosen => "character_not_chosen",
            Self::MemberNotReady => "member_not_ready",
            Self::DisplayRequiredButUnpaired => "display_required_but_unpaired",
            Self::FundingNotReady => "funding_not_ready",
            Self::RosterInconsistent => "roster_inconsistent",
        }
    }
}

/// Start may go ahead with this roster: the authorized seated members, in the order supplied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartDecision {
    pub roster: Vec<MemberId>,
}

/// Decides whether the lobby may start. Every blocker is returned, in a fixed order, so the lobby
/// view can list them all.
pub fn decide_start(snapshot: &LobbySnapshot<'_>) -> Result<StartDecision, Vec<StartBlocker>> {
    let mut blockers = Vec::new();
    if !snapshot.campaign_active {
        blockers.push(StartBlocker::CampaignArchived);
    }
    if !snapshot.session_in_lobby {
        blockers.push(StartBlocker::SessionNotInLobby);
    }
    let counted: Vec<&LobbySeat> = snapshot
        .seats
        .iter()
        .filter(|seat| seat.authorized)
        .collect();
    if counted.is_empty() {
        blockers.push(StartBlocker::NoSeatedMembers);
    } else {
        if counted.iter().any(|seat| !seat.character.is_chosen()) {
            blockers.push(StartBlocker::CharacterNotChosen);
        }
        let members: Vec<MemberId> = counted.iter().map(|seat| seat.member).collect();
        let ready: Vec<MemberId> = counted
            .iter()
            .filter(|seat| seat.ready)
            .map(|seat| seat.member)
            .collect();
        match aggregate_current_readiness(snapshot.seat_capacity, &members, &ready) {
            Ok(aggregate) if aggregate.all_ready() => {}
            Ok(_) => blockers.push(StartBlocker::MemberNotReady),
            Err(_) => blockers.push(StartBlocker::RosterInconsistent),
        }
    }
    if snapshot.display_required && snapshot.displays_paired == 0 {
        blockers.push(StartBlocker::DisplayRequiredButUnpaired);
    }
    if !snapshot.funding_ready {
        blockers.push(StartBlocker::FundingNotReady);
    }
    if blockers.is_empty() {
        Ok(StartDecision {
            roster: counted.iter().map(|seat| seat.member).collect(),
        })
    } else {
        Err(blockers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(number: u8) -> MemberId {
        MemberId::from_bytes(&[number; 16]).unwrap()
    }

    fn seat(number: u8, authorized: bool, character: CharacterChoice, ready: bool) -> LobbySeat {
        LobbySeat {
            member: member(number),
            authorized,
            character,
            ready,
        }
    }

    fn snapshot(seats: &[LobbySeat]) -> LobbySnapshot<'_> {
        LobbySnapshot {
            campaign_active: true,
            session_in_lobby: true,
            seat_capacity: 6,
            seats,
            display_required: false,
            displays_paired: 0,
            funding_ready: true,
        }
    }

    #[test]
    fn a_lobby_of_ready_members_with_characters_starts_with_exactly_them() {
        let seats = [
            seat(1, true, CharacterChoice::Template, true),
            seat(2, true, CharacterChoice::Existing, true),
        ];
        let decision = decide_start(&snapshot(&seats)).unwrap();
        assert_eq!(decision.roster, vec![member(1), member(2)]);
    }

    #[test]
    fn every_blocker_is_listed_in_a_fixed_order() {
        let seats = [
            seat(1, true, CharacterChoice::Undecided, false),
            seat(2, true, CharacterChoice::Template, true),
        ];
        let mut lobby = snapshot(&seats);
        lobby.campaign_active = false;
        lobby.session_in_lobby = false;
        lobby.display_required = true;
        lobby.funding_ready = false;
        assert_eq!(
            decide_start(&lobby),
            Err(vec![
                StartBlocker::CampaignArchived,
                StartBlocker::SessionNotInLobby,
                StartBlocker::CharacterNotChosen,
                StartBlocker::MemberNotReady,
                StartBlocker::DisplayRequiredButUnpaired,
                StartBlocker::FundingNotReady,
            ])
        );
    }

    #[test]
    fn an_empty_lobby_never_starts_and_is_never_all_ready() {
        assert_eq!(
            decide_start(&snapshot(&[])),
            Err(vec![StartBlocker::NoSeatedMembers])
        );
    }

    #[test]
    fn a_seated_member_whose_account_is_not_authorized_is_left_out_not_waited_for() {
        let seats = [
            seat(1, true, CharacterChoice::Template, true),
            // Closing: not ready, no character, and Start does not wait for them or hand them over.
            seat(2, false, CharacterChoice::Undecided, false),
        ];
        let decision = decide_start(&snapshot(&seats)).unwrap();
        assert_eq!(decision.roster, vec![member(1)]);
        // If only unauthorized members hold seats, nobody is seated for Start.
        let only = [seat(2, false, CharacterChoice::Template, true)];
        assert_eq!(
            decide_start(&snapshot(&only)),
            Err(vec![StartBlocker::NoSeatedMembers])
        );
    }

    #[test]
    fn a_display_blocks_only_when_the_deployment_requires_one_and_none_is_paired() {
        let seats = [seat(1, true, CharacterChoice::Template, true)];
        let mut lobby = snapshot(&seats);
        lobby.display_required = true;
        assert_eq!(
            decide_start(&lobby),
            Err(vec![StartBlocker::DisplayRequiredButUnpaired])
        );
        lobby.displays_paired = 1;
        assert!(decide_start(&lobby).is_ok());
        lobby.display_required = false;
        lobby.displays_paired = 0;
        assert!(decide_start(&lobby).is_ok());
    }

    #[test]
    fn contradictory_seats_are_refused_not_guessed_at() {
        let seats = [
            seat(1, true, CharacterChoice::Template, true),
            seat(1, true, CharacterChoice::Template, true),
        ];
        assert_eq!(
            decide_start(&snapshot(&seats)),
            Err(vec![StartBlocker::RosterInconsistent])
        );
        let crowded = [
            seat(1, true, CharacterChoice::Template, true),
            seat(2, true, CharacterChoice::Template, true),
        ];
        let mut lobby = snapshot(&crowded);
        lobby.seat_capacity = 1;
        assert_eq!(
            decide_start(&lobby),
            Err(vec![StartBlocker::RosterInconsistent])
        );
    }

    #[test]
    fn the_character_choice_text_round_trips_and_only_undecided_is_unchosen() {
        for choice in [
            CharacterChoice::Undecided,
            CharacterChoice::Template,
            CharacterChoice::Existing,
        ] {
            assert_eq!(CharacterChoice::parse(choice.text()), Some(choice));
        }
        assert!(!CharacterChoice::Undecided.is_chosen());
        assert!(CharacterChoice::Template.is_chosen());
        assert_eq!(CharacterChoice::parse("other"), None);
    }
}
