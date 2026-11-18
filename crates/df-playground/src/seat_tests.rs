//! The phone's seat machine: it follows the host's answers and ignores what a state does not take.

use crate::look::wire::HeroPack;
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
        hero_locked: false,
        plays: None,
    }
}

/// The seat once its player chose hero 4 for it and locked in its look.
fn with_hero(seat: SeatCard) -> SeatCard {
    SeatCard {
        hero: Some(4),
        hero_locked: true,
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
fn a_hero_chosen_but_not_locked_in_returns_to_the_look_studio_on_hello_and_on_continue() {
    let unlocked = SeatCard {
        hero: Some(3),
        ..card(1, PlayerKind::Guest)
    };
    let mut machine = SeatMachine::default();
    machine.apply(SeatEvent::HelloAnswered(HelloReply::Seated {
        seat: unlocked.clone(),
    }));
    assert_eq!(machine.state().key(), "creation");
    let mut returning = SeatMachine::default();
    returning.apply(SeatEvent::HelloAnswered(HelloReply::Returning {
        seat: unlocked.clone(),
        friends: Vec::new(),
    }));
    returning.apply(SeatEvent::JoinAnswered(JoinStatus::Seated {
        seat: unlocked,
    }));
    assert_eq!(returning.state().key(), "creation");
    assert_eq!(
        returning.poll(),
        None,
        "the studio follows the host itself; the seat asks nothing"
    );
}

#[test]
fn a_key_the_host_lost_while_making_a_hero_returns_to_who_are_you() {
    let unlocked = SeatCard {
        hero: Some(3),
        ..card(1, PlayerKind::Guest)
    };
    let mut machine = SeatMachine::default();
    machine.apply(SeatEvent::HelloAnswered(HelloReply::Seated {
        seat: unlocked,
    }));
    assert_eq!(machine.state().key(), "creation");
    assert!(machine.apply(SeatEvent::Refused {
        kind: Some(SeatErrorKind::UnknownKey),
        message: "join again".to_owned(),
    }));
    assert_eq!(machine.state().key(), "who_are_you");
    assert_eq!(machine.returning(), None);
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
        species: "elf".to_owned(),
        class: "fighter".to_owned(),
        portrait: None,
        pack: HeroPack {
            headshot: crate::look::wire::PackStatus::Ready {
                picture: "vix-headshot.png".to_owned(),
            },
            ..HeroPack::default()
        },
        last_played_ms: None,
    };
    let mut machine = at_who_are_you();
    machine.apply(SeatEvent::ChoseGuest);
    machine.apply(SeatEvent::JoinAnswered(JoinStatus::Seated {
        seat: card(1, PlayerKind::Guest),
    }));
    machine.apply(SeatEvent::HeroesLoaded(vec![hero]));
    let list = crate::seat::screen::screen_of(&machine).expect("the hero list");
    assert_eq!(
        list.cards[0].card.explain.as_deref(),
        Some("Elf \u{b7} Fighter")
    );
    assert_eq!(list.cards[0].picture.as_deref(), Some("vix-headshot.png"));
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
    use crate::seat::screen::{SeatTap, screen_of};
    use crate::seat::wire::{DayEnd, LobbyStep, SeatView, TablePlayer, TableStage};

    fn player(name: &str, present: bool, ready: bool, you: bool) -> TablePlayer {
        TablePlayer {
            name: name.to_owned(),
            present,
            ready,
            you,
            hero: None,
        }
    }

    fn lobby_view(ready: bool) -> SeatView {
        SeatView {
            revision: 3,
            stage: TableStage::Gathering,
            ready,
            can_toggle_ready: true,
            can_start: true,
            players: vec![
                player("Mara", true, ready, true),
                player("Danny", true, false, false),
                player("Vix", false, false, false),
            ],
            ask: None,
            story: None,
            day_end: None,
            start_problem: None,
            talk_waiting: 0,
            stt: Default::default(),
            notice: None,
            pending_roll: None,
            last_roll: None,
            answered_by: None,
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
            SeatTap::Act(LobbyStep::Ready),
            SeatTap::Act(LobbyStep::Leave),
            SeatTap::Retry,
            SeatTap::Roll,
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
        let others: Vec<&str> = screen.table.iter().map(|card| card.name.as_str()).collect();
        assert_eq!(
            others,
            ["Danny", "Vix"],
            "the others at the table; Mara is this phone"
        );
        assert_eq!(screen.buttons[0].tap, SeatTap::Act(LobbyStep::Ready));
        assert_eq!(screen.buttons[1].kind, ButtonKind::Danger);
        machine.apply(SeatEvent::ViewLoaded(lobby_view(true)));
        let ready = screen_of(&machine).expect("the lobby screen");
        assert_eq!(ready.buttons[0].tap, SeatTap::Act(LobbyStep::Unready));
        assert!(
            ready
                .buttons
                .iter()
                .all(|button| button.tap != SeatTap::Act(LobbyStep::Start)),
            "Danny at the table is not ready, so no start is offered"
        );
        assert_eq!(ready.status.map(|pill| pill.text), Some("Ready".to_owned()));
    }

    #[test]
    fn after_a_day_the_lobby_says_how_it_ended_and_offers_ready_again() {
        let mut machine = seated();
        let mut view = lobby_view(false);
        view.day_end = Some(DayEnd::Finished);
        machine.apply(SeatEvent::ViewLoaded(view));
        let screen = screen_of(&machine).expect("the lobby screen, not the day-over page");
        let notice = screen.notice.expect("how the day ended").text;
        assert!(notice.starts_with("The day is over."), "{notice}");
        assert!(
            screen
                .buttons
                .iter()
                .any(|button| button.tap == SeatTap::Act(LobbyStep::Ready))
        );
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
        assert!(
            screen.table[1].away,
            "Vix holds a seat but is not at the table"
        );
    }

    #[test]
    fn with_everyone_at_the_table_ready_a_start_is_offered_and_an_away_player_does_not_hold_it_up()
    {
        let mut machine = seated();
        let mut view = lobby_view(true);
        view.players[1].ready = true;
        machine.apply(SeatEvent::ViewLoaded(view));
        let screen = screen_of(&machine).expect("the lobby screen");
        assert_eq!(screen.lead.as_deref(), Some("Everyone is ready."));
        assert_eq!(screen.buttons[0].label, "Start the game");
        assert_eq!(screen.buttons[0].tap, SeatTap::Act(LobbyStep::Start));
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

mod play {
    use crate::fixture::Status;
    use crate::seat::play::play_of;
    use crate::seat::screen::SeatTap;
    use crate::seat::wire::{
        AskKind, DayEnd, SeatAsk, SeatStory, SeatThought, SeatView, StoryLine, SuggestedLine,
        TableStage,
    };

    fn view(ask: Option<SeatAsk>) -> SeatView {
        SeatView {
            revision: 1,
            stage: TableStage::Gathering,
            ready: false,
            can_toggle_ready: true,
            can_start: true,
            players: Vec::new(),
            ask,
            story: Some(Box::new(SeatStory {
                hero: "Danny".to_owned(),
                place: Some("The Mill Ford".to_owned()),
                plate: Some("roam-01.png".to_owned()),
                lines: vec![
                    StoryLine {
                        id: 1,
                        speaker: Some("Danny".to_owned()),
                        you: true,
                        text: "I take the road north.".to_owned(),
                    },
                    StoryLine {
                        id: 1,
                        speaker: None,
                        you: false,
                        text: "Rain on the ford.".to_owned(),
                    },
                ],
                thought: Some(SeatThought {
                    thought: "The mill wheel is worth painting.".to_owned(),
                    saw: vec!["a mill wheel".to_owned()],
                    wanted: vec!["to paint it".to_owned()],
                }),
            })),
            day_end: None,
            start_problem: None,
            talk_waiting: 0,
            stt: Default::default(),
            notice: None,
            pending_roll: None,
            last_roll: None,
            answered_by: None,
        }
    }

    #[test]
    fn the_story_page_shows_the_place_the_lines_heard_and_the_heros_own_thought() {
        let play = play_of(&view(None)).expect("a seat that plays a character is in play");
        assert_eq!(play.phone.goal.as_deref(), Some("The Mill Ford"));
        assert_eq!(
            play.phone.narration,
            ["Danny: I take the road north.", "Rain on the ford."]
        );
        assert_eq!(
            play.phone.note.map(|note| note.text).as_deref(),
            Some("The mill wheel is worth painting.")
        );
    }

    #[test]
    fn while_nothing_is_asked_the_composer_waits_and_the_act_page_is_empty() {
        let play = play_of(&view(None)).expect("in play");
        assert_eq!(
            crate::copy::status_copy(&play.phone.status).as_deref(),
            Some("Waiting for your move to play out"),
            "the player's own hero acting is said from the player's side"
        );
        assert_eq!(play.act.prompt, None);
    }

    #[test]
    fn an_ask_offers_its_prompt_and_cards_that_send_their_lines_by_index() {
        let ask = SeatAsk {
            id: 4,
            kind: AskKind::Move,
            prompt: "Where do you go next?".to_owned(),
            cards: vec![SuggestedLine {
                label: "Take the road".to_owned(),
                line: "I take the road on.".to_owned(),
                thing: None,
            }],
            note: Some("the movement names no destination".to_owned()),
        };
        let play = play_of(&view(Some(ask))).expect("in play");
        assert_eq!(play.phone.status, Status::Ok);
        assert_eq!(play.act.prompt.as_deref(), Some("Where do you go next?"));
        assert_eq!(play.act.cards[0].tap, SeatTap::Say(0));
        assert_eq!(
            play.act.cards[0].card.explain.as_deref(),
            Some("I take the road on.")
        );
        assert!(play.act.note.is_some());
    }

    #[test]
    fn a_day_that_stopped_says_so_on_the_act_page_instead_of_waiting() {
        let mut stopped = view(None);
        stopped.day_end = Some(DayEnd::Failed);
        let play = play_of(&stopped).expect("in play");
        assert_eq!(play.phone.status, Status::Ok);
        assert_eq!(play.act.prompt.as_deref(), Some("The day stopped early."));
        assert!(play.act.note.is_some_and(|note| note.contains("host")));
    }

    #[test]
    fn a_word_from_the_table_shows_on_the_act_page_with_or_without_an_ask() {
        let mut waiting = view(None);
        waiting.notice = Some("The painter is still at work.".to_owned());
        let play = play_of(&waiting).expect("in play");
        assert_eq!(
            play.act.notice.as_deref(),
            Some("The painter is still at work.")
        );
        assert_eq!(play.act.prompt, None);
    }

    #[test]
    fn a_seat_that_plays_no_character_has_no_play_screen() {
        let mut plain = view(None);
        plain.story = None;
        assert!(play_of(&plain).is_none());
    }

    fn asked(id: u64) -> SeatAsk {
        SeatAsk {
            id,
            kind: AskKind::Line,
            prompt: "What do you do?".to_owned(),
            cards: Vec::new(),
            note: None,
        }
    }

    fn rolled(id: &str, ask: Option<u64>) -> SeatView {
        let roll: crate::fixture::RollView = serde_json::from_value(serde_json::json!({
            "id": id,
            "title": "Performance (Charisma)",
            "kind": "skill_check",
            "dice": [{"sides": 20, "face": 15, "role": "test"}],
            "total": 17,
            "outcome": "success"
        }))
        .expect("a roll");
        SeatView {
            last_roll: Some(Box::new(roll)),
            ..view(ask.map(asked))
        }
    }

    #[test]
    fn the_last_roll_shows_on_act_until_the_table_asks_something_else() {
        use crate::seat::play::RollSeen;
        let mut seen = RollSeen::default();
        assert!(
            seen.still_current(&rolled("roll-1", None)),
            "the result came in"
        );
        assert!(
            seen.still_current(&rolled("roll-1", None)),
            "the same moment"
        );
        assert!(
            !seen.still_current(&rolled("roll-1", Some(5))),
            "the next ask clears it"
        );
        assert!(
            !seen.still_current(&rolled("roll-1", None)),
            "and it stays cleared when that ask is answered"
        );
        assert!(
            !seen.still_current(&rolled("roll-1", Some(6))),
            "many turns later the host still sends it: it stays cleared"
        );
        assert!(
            seen.still_current(&rolled("roll-2", Some(6))),
            "a new roll shows beside the ask it came with"
        );
        assert!(!seen.still_current(&rolled("roll-2", Some(7))));
        assert!(!seen.still_current(&view(None)), "no roll, nothing shown");
        assert!(seen.still_current(&rolled("roll-2", Some(7))), "seen anew");
    }
}

/// The actions a phone sent whose answer has not come: the same tap again keeps its id, so the
/// host does it once, and an answer settles it.
mod unsettled {
    use crate::seat::Unsettled;
    use crate::seat::wire::{LobbyStep, SeatAction};

    fn roll(call: &str) -> SeatAction {
        SeatAction::Roll {
            said: 0,
            call: call.to_owned(),
        }
    }

    #[test]
    fn a_tap_again_before_the_answer_keeps_its_id_and_an_answer_settles_it() {
        let mut unsettled = Unsettled::default();
        let first = unsettled.take_up(&roll("check-3"), 11);
        assert_eq!(first.said(), 11);
        assert_eq!(
            unsettled.take_up(&roll("check-3"), 12).said(),
            11,
            "the same roll goes with the same id while its fate is not known"
        );
        let ready = SeatAction::Lobby {
            said: 0,
            step: LobbyStep::Ready,
        };
        assert_eq!(
            unsettled.take_up(&ready, 13).said(),
            13,
            "another action waits beside it with its own id"
        );
        unsettled.settle(11);
        assert_eq!(
            unsettled.take_up(&roll("check-3"), 14).said(),
            14,
            "once answered, a new tap is a new action"
        );
        assert_eq!(
            unsettled.take_up(&ready, 15).said(),
            13,
            "the other one still waits"
        );
    }
}

mod hero_header {
    use crate::look::wire::{HeroPack, PackStatus};
    use crate::seat::screen::{LobbyTap, hero_header};
    use crate::seat::wire::{SeatView, TableHero, TablePlayer, TableStage};

    fn view(portrait: Option<&str>) -> SeatView {
        SeatView {
            revision: 1,
            stage: TableStage::Playing,
            ready: true,
            can_toggle_ready: false,
            can_start: false,
            players: vec![TablePlayer {
                name: "Cam".to_owned(),
                present: true,
                ready: true,
                you: true,
                hero: Some(TableHero {
                    id: 3,
                    name: "Brynn Quarry".to_owned(),
                    species: "dwarf".to_owned(),
                    class: "fighter".to_owned(),
                    portrait: portrait.map(str::to_owned),
                    pack: HeroPack::default(),
                }),
            }],
            ask: None,
            story: None,
            day_end: None,
            start_problem: None,
            talk_waiting: 0,
            stt: Default::default(),
            notice: None,
            pending_roll: None,
            last_roll: None,
            answered_by: None,
        }
    }

    #[test]
    fn a_seated_phones_header_is_its_own_hero_and_a_tap_opens_the_portrait() {
        let hero = hero_header(&view(Some("hero-3.png"))).expect("the player's hero");
        assert_eq!(hero.header.name, "Brynn Quarry");
        assert_eq!(
            hero.header.subtitle.as_deref(),
            Some("Dwarf \u{b7} Fighter")
        );
        assert_eq!(hero.header.portrait.as_deref(), Some("hero-3.png"));
        assert_eq!(
            hero.action,
            Some(LobbyTap::View("hero-3.png".to_owned()).action())
        );
    }

    #[test]
    fn the_headshot_made_at_lock_in_fills_the_medallion_once_it_is_ready() {
        let mut with_headshot = view(Some("hero-3.png"));
        if let Some(hero) = with_headshot.players[0].hero.as_mut() {
            hero.pack.headshot = PackStatus::Ready {
                picture: "hero-3-head.png".to_owned(),
            };
        }
        let hero = hero_header(&with_headshot).expect("the player's hero");
        assert_eq!(hero.header.portrait.as_deref(), Some("hero-3-head.png"));
        assert_eq!(
            hero.action,
            Some(LobbyTap::View("hero-3.png".to_owned()).action())
        );
    }

    #[test]
    fn a_hero_with_no_portrait_yet_shows_its_name_and_opens_nothing() {
        let hero = hero_header(&view(None)).expect("the player's hero");
        assert_eq!(hero.action, None);
        let mut chosen_none = view(None);
        chosen_none.players[0].hero = None;
        assert!(hero_header(&chosen_none).is_none());
    }
}

/// The seat action contract: every action reads back from its JSON whole, carries its own id and
/// key, and every refusal tag of the family reads back as its kind.
mod action_contract {
    use crate::dev_reply::ErrorTag;
    use crate::pages::wire::{BagAct, NoteAsk, PagesErrorKind};
    use crate::seat::wire::{
        LobbyStep, RollTapErrorKind, SeatAction, SeatActionErrorKind, SeatErrorKind, SeatOutcome,
        SeatStt,
    };

    fn every_action() -> Vec<SeatAction> {
        vec![
            SeatAction::Answer {
                said: 1,
                ask: Some(4),
                text: "I take the road.".to_owned(),
                to: Some("mara".to_owned()),
                thing: Some("gate".to_owned()),
            },
            SeatAction::Talk {
                said: 2,
                text: "Good day!".to_owned(),
                to: None,
            },
            SeatAction::Lobby {
                said: 3,
                step: LobbyStep::Start,
            },
            SeatAction::Hero { said: 4, hero: 7 },
            SeatAction::Voice { said: 5, on: true },
            SeatAction::Note {
                said: 6,
                note: NoteAsk::Snap,
            },
            SeatAction::Bag {
                said: 7,
                bag: BagAct::Give {
                    item: "rags".to_owned(),
                    to: "mara".to_owned(),
                },
            },
            SeatAction::Roll {
                said: 8,
                call: "check-12".to_owned(),
            },
        ]
    }

    #[test]
    fn every_action_round_trips_and_names_its_id_and_key() {
        let actions = every_action();
        let mut keys: Vec<&str> = actions.iter().map(SeatAction::key).collect();
        keys.dedup();
        assert_eq!(keys.len(), actions.len(), "one sample of each action");
        for (index, action) in actions.iter().enumerate() {
            let json = serde_json::to_value(action).expect("writes");
            assert_eq!(json["action"], action.key(), "{json}");
            assert_eq!(
                serde_json::from_value::<SeatAction>(json).ok().as_ref(),
                Some(action)
            );
            assert_eq!(action.said(), u64::try_from(index + 1).unwrap());
            assert_eq!(action.with_said(99).said(), 99);
            assert_eq!(action.with_said(99).with_said(action.said()), *action);
        }
        assert!(
            serde_json::from_str::<SeatAction>(r#"{"action":"voice","said":1,"on":true,"seat":2}"#)
                .is_err(),
            "the seat is never read from the body"
        );
        let outcome = SeatOutcome::Voice {
            stt: SeatStt {
                on: true,
                available: true,
            },
        };
        let json = serde_json::to_value(&outcome).expect("writes");
        assert_eq!(json["done"], "voice");
        assert_eq!(
            serde_json::from_value::<SeatOutcome>(json).ok(),
            Some(outcome)
        );
    }

    #[test]
    fn every_refusal_tag_of_the_family_reads_back_as_its_kind() {
        for kind in SeatErrorKind::ALL {
            assert_eq!(
                SeatActionErrorKind::from_tag(kind.key()),
                Some(SeatActionErrorKind::Seat(kind))
            );
            assert_eq!(SeatActionErrorKind::Seat(kind).seat(), Some(kind));
        }
        for kind in PagesErrorKind::ALL {
            let read = SeatActionErrorKind::from_tag(kind.key()).expect("a pages tag reads");
            assert_eq!(read.key(), kind.key());
            assert_eq!(read.pages(), kind);
        }
        for kind in RollTapErrorKind::ALL {
            assert_eq!(
                SeatActionErrorKind::from_tag(kind.key()),
                Some(SeatActionErrorKind::Roll(kind))
            );
        }
        assert_eq!(SeatActionErrorKind::from_tag("no_such_refusal"), None);
    }
}
