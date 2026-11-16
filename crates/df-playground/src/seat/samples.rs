//! The join board of the phone gallery, a development tool only: one sample seat machine for each
//! join screen, driven there by the host answers a real phone would get. A reviewer opens it on
//! the phone page with `?phone_kit=join`. Sample data, not a scene the host plays.

use crate::path::query_value;
use crate::phone_kit::gallery::GALLERY_QUERY_KEY;
use crate::seat::wire::{
    FriendCard, HelloReply, HeroCard, JoinStatus, PlayerKind, RoomReply, RoomState, SeatCard,
    SeatErrorKind, SeatView, TablePlayer, TableStage,
};
use crate::seat::{SeatEvent, SeatMachine};

/// The value of the gallery's query parameter that names the join board.
pub const JOIN_BOARD: &str = "join";

/// Whether a page address's query (`?phone_kit=join`) names the join board.
pub fn names_join_board(query: &str) -> bool {
    query_value(query, GALLERY_QUERY_KEY) == Some(JOIN_BOARD)
}

fn room() -> RoomReply {
    RoomReply {
        adventure: Some("The Sunken Bell".to_owned()),
        picture: Some("lobby-cam.webp".to_owned()),
        players: vec!["Danny".to_owned(), "Lyra".to_owned()],
        state: RoomState::Gathering,
    }
}

fn seat() -> SeatCard {
    SeatCard {
        seat: 3,
        name: "Mara Vell".to_owned(),
        kind: PlayerKind::Guest,
        hero: None,
    }
}

/// The seat once its player chose a hero.
fn seat_with_hero() -> SeatCard {
    SeatCard {
        hero: Some(1),
        ..seat()
    }
}

fn friends() -> Vec<FriendCard> {
    ["Danny", "Rylan"]
        .into_iter()
        .zip(1..)
        .map(|(name, id)| FriendCard {
            id,
            name: name.to_owned(),
        })
        .collect()
}

fn driven(events: Vec<SeatEvent>) -> SeatMachine {
    let mut machine = SeatMachine::default();
    for event in events {
        machine.apply(event);
    }
    machine
}

fn at_who_are_you(returning: bool) -> Vec<SeatEvent> {
    let hello = if returning {
        HelloReply::Returning {
            seat: seat(),
            friends: friends(),
        }
    } else {
        HelloReply::Unknown { friends: friends() }
    };
    vec![
        SeatEvent::RoomLoaded(room()),
        SeatEvent::HelloAnswered(hello),
    ]
}

fn then(mut events: Vec<SeatEvent>, more: impl IntoIterator<Item = SeatEvent>) -> Vec<SeatEvent> {
    events.extend(more);
    events
}

fn seated_view(ready: bool) -> SeatView {
    let player = |name: &str, present: bool, ready: bool, you: bool| TablePlayer {
        name: name.to_owned(),
        present,
        ready,
        you,
    };
    SeatView {
        revision: 4,
        stage: TableStage::Gathering,
        ready,
        can_toggle_ready: true,
        players: vec![
            player("Danny", true, true, false),
            player("Lyra", false, false, false),
            player("Mara Vell", true, ready, true),
        ],
    }
}

/// Each join screen's title and a machine standing in it.
pub fn join_samples() -> Vec<(&'static str, SeatMachine)> {
    let joined = |events: Vec<SeatEvent>| {
        then(
            events,
            [
                SeatEvent::ChoseGuest,
                SeatEvent::JoinAnswered(JoinStatus::Seated { seat: seat() }),
            ],
        )
    };
    let hero = HeroCard {
        id: 1,
        name: "Vix Ashlantern".to_owned(),
        species: "Elf".to_owned(),
        class: "Fighter".to_owned(),
        portrait: None,
        last_played_ms: None,
    };
    vec![
        ("Room", driven(vec![SeatEvent::RoomLoaded(room())])),
        ("Who are you?", driven(at_who_are_you(false))),
        (
            "Who are you? (been here before)",
            driven(at_who_are_you(true)),
        ),
        (
            "Guest: your name, refused",
            driven(then(
                at_who_are_you(false),
                [
                    SeatEvent::ChoseGuest,
                    SeatEvent::Refused {
                        kind: Some(SeatErrorKind::InvalidName),
                        message: "A name is letters, with single spaces, apostrophes, hyphens \
                                  or periods between."
                            .to_owned(),
                    },
                ],
            )),
        ),
        (
            "Friend: pick your profile",
            driven(then(at_who_are_you(false), [SeatEvent::ChoseFriend])),
        ),
        (
            "Sign in: comes later",
            driven(then(
                at_who_are_you(false),
                [
                    SeatEvent::ChoseSignIn,
                    SeatEvent::Refused {
                        kind: Some(SeatErrorKind::AccountsNotAvailable),
                        message: "Signing in comes later on this table.".to_owned(),
                    },
                ],
            )),
        ),
        (
            "Waiting for approval",
            driven(then(
                at_who_are_you(false),
                [
                    SeatEvent::ChoseFriend,
                    SeatEvent::JoinAnswered(JoinStatus::AwaitingApproval { request: 2 }),
                ],
            )),
        ),
        (
            "Your hero",
            driven(then(
                joined(at_who_are_you(false)),
                [SeatEvent::HeroesLoaded(vec![hero])],
            )),
        ),
        (
            "Your hero: none yet",
            driven(then(
                joined(at_who_are_you(false)),
                [SeatEvent::HeroesLoaded(Vec::new())],
            )),
        ),
        (
            "Creation comes next",
            driven(then(
                joined(at_who_are_you(false)),
                [SeatEvent::HeroesLoaded(Vec::new()), SeatEvent::MakeHero],
            )),
        ),
        (
            "At the table",
            driven(then(
                at_who_are_you(true),
                [
                    SeatEvent::JoinAnswered(JoinStatus::Seated {
                        seat: seat_with_hero(),
                    }),
                    SeatEvent::ViewLoaded(seated_view(false)),
                ],
            )),
        ),
        (
            "At the table, ready",
            driven(then(
                at_who_are_you(true),
                [
                    SeatEvent::JoinAnswered(JoinStatus::Seated {
                        seat: seat_with_hero(),
                    }),
                    SeatEvent::ViewLoaded(seated_view(true)),
                ],
            )),
        ),
        (
            "Refused by the host",
            driven(then(
                at_who_are_you(false),
                [
                    SeatEvent::ChoseFriend,
                    SeatEvent::JoinAnswered(JoinStatus::AwaitingApproval { request: 2 }),
                    SeatEvent::JoinAnswered(JoinStatus::Refused),
                ],
            )),
        ),
        (
            "Room ended",
            driven(vec![SeatEvent::RoomLoaded(RoomReply {
                state: RoomState::Closed,
                ..room()
            })]),
        ),
    ]
}
