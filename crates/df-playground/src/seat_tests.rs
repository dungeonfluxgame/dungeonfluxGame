//! The phone's seat machine: it follows the host's answers and ignores what a state does not take.

use crate::seat::wire::{
    FriendCard, HelloReply, HeroCard, JoinStatus, PlayerKind, RoomReply, RoomState, SeatCard,
    SeatErrorKind, SeatIntentReply,
};
use crate::seat::{RefusedReason, SeatEvent, SeatMachine, SeatPoll, SeatState};

fn card(seat: u64, kind: PlayerKind) -> SeatCard {
    SeatCard {
        seat,
        name: "Mara".to_owned(),
        kind,
        hero: None,
    }
}

/// The seat once its player chose hero 4 for it.
fn with_hero(seat: SeatCard) -> SeatCard {
    SeatCard {
        hero: Some(4),
        ..seat
    }
}

fn room(state: RoomState) -> RoomReply {
    RoomReply {
        adventure: Some("The Sunken Bell".to_owned()),
        picture: None,
        players: Vec::new(),
        state,
    }
}

fn friends() -> Vec<FriendCard> {
    vec![FriendCard {
        id: 1,
        name: "Danny".to_owned(),
    }]
}

fn at_who_are_you() -> SeatMachine {
    let mut machine = SeatMachine::default();
    assert!(machine.apply(SeatEvent::RoomLoaded(room(RoomState::Gathering))));
    assert!(machine.apply(SeatEvent::HelloAnswered(HelloReply::Unknown {
        friends: friends()
    })));
    machine
}

#[test]
fn a_new_phone_goes_from_the_room_to_who_are_you_and_a_guest_join_leads_to_the_hero_list() {
    let mut machine = at_who_are_you();
    assert_eq!(machine.state(), &SeatState::WhoAreYou);
    assert_eq!(machine.friends(), friends().as_slice());
    assert!(machine.apply(SeatEvent::ChoseGuest));
    assert!(machine.apply(SeatEvent::JoinAnswered(JoinStatus::Seated {
        seat: card(2, PlayerKind::Guest)
    })));
    assert_eq!(
        machine.state(),
        &SeatState::ChooseHero {
            seat: card(2, PlayerKind::Guest),
            heroes: None
        }
    );
    assert_eq!(machine.poll(), Some(SeatPoll::Heroes));
}

#[test]
fn a_phone_whose_seat_has_a_hero_goes_straight_to_seated_and_waits_for_its_view() {
    let mut machine = SeatMachine::default();
    assert!(machine.apply(SeatEvent::HelloAnswered(HelloReply::Seated {
        seat: with_hero(card(3, PlayerKind::Guest))
    })));
    assert_eq!(machine.state().key(), "seated");
    assert_eq!(machine.poll(), Some(SeatPoll::SeatView));
}

#[test]
fn a_seated_player_with_no_hero_chosen_returns_to_your_hero_before_the_lobby() {
    let mut machine = SeatMachine::default();
    machine.apply(SeatEvent::HelloAnswered(HelloReply::Seated {
        seat: card(2, PlayerKind::Guest),
    }));
    assert_eq!(machine.state().key(), "choose_hero");
}

#[test]
fn continue_as_from_who_are_you_returns_to_the_table_without_the_hero_list() {
    let mut machine = SeatMachine::default();
    machine.apply(SeatEvent::HelloAnswered(HelloReply::Returning {
        seat: card(3, PlayerKind::Friend),
        friends: Vec::new(),
    }));
    assert_eq!(machine.returning(), Some(&card(3, PlayerKind::Friend)));
    assert!(machine.apply(SeatEvent::JoinAnswered(JoinStatus::Seated {
        seat: with_hero(card(3, PlayerKind::Friend))
    })));
    assert_eq!(machine.state().key(), "seated");
}

#[test]
fn a_friend_waits_for_approval_and_follows_the_hosts_answer() {
    let mut machine = at_who_are_you();
    machine.apply(SeatEvent::ChoseFriend);
    assert!(
        machine.apply(SeatEvent::JoinAnswered(JoinStatus::AwaitingApproval {
            request: 7
        }))
    );
    assert_eq!(machine.poll(), Some(SeatPoll::JoinStatus));
    assert!(
        machine.apply(SeatEvent::JoinAnswered(JoinStatus::AwaitingApproval {
            request: 7
        }))
    );
    assert!(machine.apply(SeatEvent::JoinAnswered(JoinStatus::Refused)));
    assert_eq!(machine.state(), &SeatState::Refused(RefusedReason::ByHost));
}

#[test]
fn signing_in_shows_the_hosts_refusal_and_back_returns_to_who_are_you() {
    let mut machine = at_who_are_you();
    machine.apply(SeatEvent::ChoseSignIn);
    assert!(machine.apply(SeatEvent::Refused {
        kind: Some(SeatErrorKind::AccountsNotAvailable),
        message: "signing in comes later on this table".to_owned(),
    }));
    assert_eq!(
        machine.state(),
        &SeatState::SigningIn {
            answer: Some("signing in comes later on this table".to_owned())
        }
    );
    assert!(machine.apply(SeatEvent::Back));
    assert_eq!(machine.state(), &SeatState::WhoAreYou);
}

#[test]
fn the_phone_never_seats_itself_a_join_answer_out_of_turn_is_ignored() {
    let mut machine = SeatMachine::default();
    assert!(!machine.apply(SeatEvent::JoinAnswered(JoinStatus::Seated {
        seat: card(1, PlayerKind::Guest)
    })));
    assert_eq!(machine.state(), &SeatState::Room);
    let mut guest = at_who_are_you();
    guest.apply(SeatEvent::ChoseGuest);
    assert!(
        !guest.apply(SeatEvent::JoinAnswered(JoinStatus::AwaitingApproval {
            request: 1
        }))
    );
    assert_eq!(guest.state(), &SeatState::Guest);
}

#[test]
fn a_refused_name_stays_on_the_screen_with_the_hosts_sentence() {
    let mut machine = at_who_are_you();
    machine.apply(SeatEvent::ChoseGuest);
    machine.apply(SeatEvent::Refused {
        kind: Some(SeatErrorKind::InvalidName),
        message: "a name is letters".to_owned(),
    });
    assert_eq!(machine.state(), &SeatState::Guest);
    assert_eq!(machine.problem(), Some("a name is letters"));
}

#[test]
fn a_full_table_a_started_game_and_a_closed_room_end_the_way_in() {
    for (kind, expected) in [
        (
            SeatErrorKind::TableFull,
            SeatState::Refused(RefusedReason::TableFull),
        ),
        (
            SeatErrorKind::GameStarted,
            SeatState::Refused(RefusedReason::GameStarted),
        ),
        (SeatErrorKind::TableClosed, SeatState::RoomEnded),
    ] {
        let mut machine = at_who_are_you();
        machine.apply(SeatEvent::ChoseGuest);
        machine.apply(SeatEvent::Refused {
            kind: Some(kind),
            message: String::new(),
        });
        assert_eq!(machine.state(), &expected, "{kind:?}");
    }
    let mut closed = SeatMachine::default();
    closed.apply(SeatEvent::RoomLoaded(room(RoomState::Closed)));
    assert_eq!(closed.state(), &SeatState::RoomEnded);
}

#[test]
fn a_hero_from_the_list_seats_the_player_and_make_a_new_hero_leads_to_creation() {
    let hero = HeroCard {
        id: 4,
        name: "Vix".to_owned(),
        species: "Elf".to_owned(),
        class: "Fighter".to_owned(),
        portrait: None,
        last_played_ms: None,
    };
    let mut machine = at_who_are_you();
    machine.apply(SeatEvent::ChoseGuest);
    machine.apply(SeatEvent::JoinAnswered(JoinStatus::Seated {
        seat: card(1, PlayerKind::Guest),
    }));
    machine.apply(SeatEvent::HeroesLoaded(vec![hero]));
    let mut creating = machine.clone();
    assert!(
        !machine.apply(SeatEvent::HeroChosen(card(1, PlayerKind::Guest))),
        "only the host's record of a chosen hero seats the player"
    );
    assert!(machine.apply(SeatEvent::HeroChosen(with_hero(card(1, PlayerKind::Guest)))));
    assert_eq!(machine.state().key(), "seated");
    assert!(creating.apply(SeatEvent::MakeHero));
    assert_eq!(creating.state().key(), "creation");
}

#[test]
fn leaving_the_table_returns_to_who_are_you_without_continue_as() {
    let mut machine = SeatMachine::default();
    machine.apply(SeatEvent::HelloAnswered(HelloReply::Seated {
        seat: with_hero(card(2, PlayerKind::Guest)),
    }));
    assert!(machine.apply(SeatEvent::IntentAnswered(SeatIntentReply::Left)));
    assert_eq!(machine.state(), &SeatState::WhoAreYou);
    assert_eq!(machine.returning(), None);
}

mod screens {
    use super::*;
    use crate::phone_kit::shell::ButtonKind;
    use crate::phone_kit::shell::PillTone;
    use crate::seat::screen::{SeatTap, screen_of};
    use crate::seat::wire::{SeatAction, SeatView, TablePlayer, TableStage};

    fn player(name: &str, present: bool, ready: bool, you: bool) -> TablePlayer {
        TablePlayer {
            name: name.to_owned(),
            present,
            ready,
            you,
        }
    }

    fn lobby_view(ready: bool) -> SeatView {
        SeatView {
            revision: 3,
            stage: TableStage::Gathering,
            ready,
            can_toggle_ready: true,
            players: vec![
                player("Mara", true, ready, true),
                player("Danny", true, false, false),
                player("Vix", false, false, false),
            ],
        }
    }

    fn seated() -> SeatMachine {
        let mut machine = SeatMachine::default();
        machine.apply(SeatEvent::HelloAnswered(HelloReply::Seated {
            seat: with_hero(card(1, PlayerKind::Guest)),
        }));
        machine
    }

    #[test]
    fn every_tap_reads_back_from_its_action() {
        for tap in [
            SeatTap::Continue,
            SeatTap::Guest,
            SeatTap::Friend,
            SeatTap::SignIn,
            SeatTap::Back,
            SeatTap::JoinGuest,
            SeatTap::PickFriend(12),
            SeatTap::PlayHero(3),
            SeatTap::MakeHero,
            SeatTap::Act(SeatAction::Ready),
            SeatTap::Act(SeatAction::Leave),
            SeatTap::Retry,
        ] {
            assert_eq!(SeatTap::from_action(&tap.action()), Some(tap), "{tap:?}");
        }
        assert_eq!(SeatTap::from_action("toggle:sound"), None);
    }

    #[test]
    fn who_are_you_offers_continue_as_first_when_the_key_holds_a_seat() {
        let mut machine = SeatMachine::default();
        machine.apply(SeatEvent::HelloAnswered(HelloReply::Returning {
            seat: card(2, PlayerKind::Guest),
            friends: Vec::new(),
        }));
        let screen = screen_of(&machine).expect("a join screen");
        assert_eq!(screen.cards[0].card.label, "Continue as Mara");
        assert_eq!(screen.cards[0].tap, SeatTap::Continue);
        let taps: Vec<_> = screen.cards.iter().map(|card| card.tap).collect();
        assert_eq!(
            taps,
            [
                SeatTap::Continue,
                SeatTap::Guest,
                SeatTap::Friend,
                SeatTap::SignIn
            ]
        );
    }

    #[test]
    fn the_seated_lobby_shows_who_is_here_the_ready_toggle_and_leave() {
        let mut machine = seated();
        machine.apply(SeatEvent::ViewLoaded(lobby_view(false)));
        let screen = screen_of(&machine).expect("the lobby screen");
        assert_eq!(screen.roster.len(), 3);
        assert_eq!(screen.roster[0].tone, PillTone::Accent, "Mara is you");
        assert_eq!(screen.buttons[0].tap, SeatTap::Act(SeatAction::Ready));
        assert_eq!(screen.buttons[1].kind, ButtonKind::Danger);
        machine.apply(SeatEvent::ViewLoaded(lobby_view(true)));
        let ready = screen_of(&machine).expect("the lobby screen");
        assert_eq!(ready.buttons[0].tap, SeatTap::Act(SeatAction::Unready));
        assert_eq!(ready.status.map(|pill| pill.text), Some("Ready".to_owned()));
    }

    #[test]
    fn the_lobby_counts_only_who_is_at_the_table_and_marks_the_others_away() {
        let mut machine = seated();
        machine.apply(SeatEvent::ViewLoaded(lobby_view(true)));
        let screen = screen_of(&machine).expect("the lobby screen");
        assert_eq!(
            screen.lead.as_deref(),
            Some("Waiting for everyone to be ready: 1 of 2 ready.")
        );
        assert_eq!(screen.roster[2].text, "Vix (away)");
    }

    #[test]
    fn until_the_first_seat_view_arrives_the_phone_stays_on_the_loading_screen() {
        let screen = screen_of(&seated()).expect("a loading screen, never the play page");
        assert!(screen.loading.is_some());
    }

    #[test]
    fn once_the_table_moves_on_from_the_lobby_no_join_screen_is_shown() {
        let mut machine = seated();
        let mut view = lobby_view(false);
        view.stage = TableStage::Playing;
        machine.apply(SeatEvent::ViewLoaded(view));
        assert_eq!(screen_of(&machine), None);
    }

    #[test]
    fn every_state_but_the_moved_on_table_has_a_screen() {
        let mut machine = at_who_are_you();
        assert!(screen_of(&machine).is_some_and(|screen| screen.heading.is_some()));
        machine.apply(SeatEvent::ChoseGuest);
        assert!(screen_of(&machine).is_some_and(|screen| screen.name_field));
        machine.apply(SeatEvent::Back);
        machine.apply(SeatEvent::ChoseFriend);
        let friends = screen_of(&machine).expect("the friends screen");
        assert_eq!(friends.cards[0].tap, SeatTap::PickFriend(1));
    }
}

#[cfg(debug_assertions)]
#[test]
fn the_join_board_has_one_screen_for_every_join_state() {
    use crate::seat::samples::{join_samples, names_join_board};
    use crate::seat::screen::screen_of;

    let keys: Vec<&str> = join_samples()
        .iter()
        .map(|(title, machine)| {
            assert!(screen_of(machine).is_some(), "{title} has a screen");
            machine.state().key()
        })
        .collect();
    for state in [
        "room",
        "who_are_you",
        "guest",
        "friend",
        "signing_in",
        "awaiting_approval",
        "choose_hero",
        "creation",
        "seated",
        "refused",
        "room_ended",
    ] {
        assert!(keys.contains(&state), "{state} is on the board");
    }
    assert!(names_join_board("?phone_kit=join"));
    assert!(!names_join_board("?phone_kit=2"));
}
