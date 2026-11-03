//! Tests of the live changes to the lobby, the loading screen, the roleplay card and the top bar:
//! what each change does, what it refuses, that a refusal changes nothing, and what the audit
//! line may carry.

use crate::fixture::Fixture;
use crate::lobby_change::{
    BeginEdit, ChipEdit, JoinEdit, LoadingEdit, LobbyLine, LobbyPart, MAXIMUM_LOBBY_SEATS,
    PartyEdit, PillEdit, PillPart, RoleplayEdit, ScreenEdit, ScreenError, SeatEdit,
};
use crate::lobby_model::{BubbleTone, LogoStyle, PartyCount, SeatState, SummaryIcon};
use crate::overlay::Glyph;
use crate::scene::{SceneChange, SceneError, SceneOverlay};

const FIXTURE: &str = r##"{
    "name": "Base",
    "tv": {
        "place": "Here", "status": {"kind": "ok"}, "elements": [], "caption": null,
        "pills": {"session": "The Ember Roads", "balance": "$28.40", "rate": "$3.90/hr"}
    },
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"##;

fn base() -> Fixture {
    Fixture::parse(FIXTURE).expect("the fixture reads")
}

fn make(edit: ScreenEdit) -> (SceneOverlay, Result<(), SceneError>) {
    let mut overlay = SceneOverlay::default();
    let result = overlay.apply(SceneChange::Screen { edit }, &base());
    (overlay, result)
}

fn apply_all(edits: Vec<ScreenEdit>) -> Fixture {
    let mut overlay = SceneOverlay::default();
    for edit in edits {
        overlay
            .apply(SceneChange::Screen { edit }, &base())
            .expect("the change applies");
    }
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture
}

fn seat(number: usize, state: SeatState, name: &str) -> ScreenEdit {
    ScreenEdit::LobbySeat {
        number,
        seat: SeatEdit {
            state: Some(state),
            name: Some(name.to_owned()),
            ..SeatEdit::default()
        },
    }
}

#[test]
fn a_seat_past_the_end_adds_open_seats_up_to_it() {
    let fixture = apply_all(vec![seat(3, SeatState::LockedIn, "Cam")]);
    let lobby = fixture.tv.lobby.expect("a lobby was made");
    assert_eq!(lobby.seats.len(), 3);
    assert_eq!(lobby.seats[0].state, SeatState::Open);
    assert_eq!(
        (lobby.seats[2].state, lobby.seats[2].name.as_deref()),
        (SeatState::LockedIn, Some("Cam"))
    );
}

#[test]
fn opening_a_seat_empties_it_and_a_field_left_out_stays() {
    let fixture = apply_all(vec![
        seat(1, SeatState::LockedIn, "Cam"),
        ScreenEdit::LobbySeat {
            number: 1,
            seat: SeatEdit {
                class: Some("Paladin".to_owned()),
                host: Some(true),
                ..SeatEdit::default()
            },
        },
    ]);
    let cam = &fixture.tv.lobby.as_ref().expect("lobby").seats[0];
    assert_eq!(
        (cam.name.as_deref(), cam.class.as_deref(), cam.host),
        (Some("Cam"), Some("Paladin"), true)
    );
    let emptied = apply_all(vec![
        seat(1, SeatState::LockedIn, "Cam"),
        ScreenEdit::LobbySeat {
            number: 1,
            seat: SeatEdit {
                state: Some(SeatState::Open),
                ..SeatEdit::default()
            },
        },
    ]);
    let seat = &emptied.tv.lobby.expect("lobby").seats[0];
    assert_eq!(
        (seat.state, seat.name.clone(), seat.host),
        (SeatState::Open, None, false)
    );
}

#[test]
fn an_empty_text_takes_the_field_away() {
    let fixture = apply_all(vec![
        seat(1, SeatState::LockedIn, "Cam"),
        ScreenEdit::LobbySeat {
            number: 1,
            seat: SeatEdit {
                name: Some(String::new()),
                ..SeatEdit::default()
            },
        },
    ]);
    assert_eq!(fixture.tv.lobby.expect("lobby").seats[0].name, None);
}

#[test]
fn a_seat_number_outside_the_range_is_refused_and_changes_nothing() {
    for number in [0, MAXIMUM_LOBBY_SEATS + 1] {
        let (overlay, result) = make(seat(number, SeatState::Open, "x"));
        assert_eq!(
            result,
            Err(SceneError::Screen(ScreenError::BadSeatNumber { number }))
        );
        assert!(overlay.is_empty());
    }
}

#[test]
fn a_progress_over_one_hundred_and_a_step_past_the_three_are_refused() {
    let progress = ScreenEdit::LobbySeat {
        number: 1,
        seat: SeatEdit {
            progress_percent: Some(101),
            ..SeatEdit::default()
        },
    };
    let (overlay, result) = make(progress);
    assert_eq!(
        result,
        Err(SceneError::Screen(ScreenError::BadPercent {
            what: "a seat's progress",
            value: 101
        }))
    );
    assert!(overlay.is_empty());
    let step = ScreenEdit::LobbySeat {
        number: 1,
        seat: SeatEdit {
            step: Some(3),
            ..SeatEdit::default()
        },
    };
    assert_eq!(
        make(step).1,
        Err(SceneError::Screen(ScreenError::BadStep(3)))
    );
}

#[test]
fn the_party_counts_players_and_refuses_more_than_the_seats() {
    let edit = |joined, capacity| ScreenEdit::LobbyParty {
        party: PartyEdit {
            joined,
            capacity,
            icon: Some(SummaryIcon::People),
            ..PartyEdit::default()
        },
    };
    let fixture = apply_all(vec![edit(Some(3), Some(6))]);
    let party = fixture.tv.lobby.expect("lobby").party.expect("party");
    assert_eq!(
        (party.joined, party.capacity, party.icon),
        (3, 6, SummaryIcon::People)
    );
    assert_eq!(
        make(edit(Some(7), Some(6))).1,
        Err(SceneError::Screen(ScreenError::JoinedOverCapacity {
            joined: 7,
            capacity: 6
        }))
    );
}

#[test]
fn the_join_code_is_set_and_an_empty_text_takes_a_field_away() {
    let join = |game_code: &str, hint: &str| ScreenEdit::LobbyJoin {
        join: JoinEdit {
            game_code: Some(game_code.to_owned()),
            hint: Some(hint.to_owned()),
            ..JoinEdit::default()
        },
    };
    let fixture = apply_all(vec![join("AC7F-K8M2", "Game code")]);
    let code = fixture.tv.lobby.expect("lobby").join.expect("join");
    assert_eq!(code.game_code.as_deref(), Some("AC7F-K8M2"));
    assert_eq!(code.hint.as_deref(), Some("Game code"));
    let fixture = apply_all(vec![join("AC7F-K8M2", "Game code"), join("", "")]);
    let code = fixture.tv.lobby.expect("lobby").join.expect("join");
    assert_eq!((code.game_code, code.hint), (None, None));
}

#[test]
fn the_seat_count_adds_open_seats_and_takes_the_rest_away() {
    let fixture = apply_all(vec![
        seat(1, SeatState::LockedIn, "Cam"),
        ScreenEdit::LobbySeats { count: 4 },
    ]);
    let lobby = fixture.tv.lobby.expect("lobby");
    assert_eq!(lobby.seats.len(), 4);
    assert_eq!(lobby.seats[0].state, SeatState::LockedIn);
    assert_eq!(lobby.seats[3].state, SeatState::Open);
    let fixture = apply_all(vec![
        seat(5, SeatState::Open, "x"),
        ScreenEdit::LobbySeats { count: 2 },
    ]);
    assert_eq!(fixture.tv.lobby.expect("lobby").seats.len(), 2);
    assert_eq!(
        make(ScreenEdit::LobbySeats {
            count: MAXIMUM_LOBBY_SEATS + 1
        })
        .1,
        Err(SceneError::Screen(ScreenError::BadSeatNumber {
            number: MAXIMUM_LOBBY_SEATS + 1
        }))
    );
}

#[test]
fn the_focus_goes_to_the_begin_button_or_a_seat_and_nothing_else() {
    let focus = |id: &str| ScreenEdit::LobbyFocus {
        id: Some(id.to_owned()),
    };
    let fixture = apply_all(vec![focus("seat:2")]);
    assert_eq!(
        fixture.tv.lobby.expect("lobby").focus.as_deref(),
        Some("seat:2")
    );
    let fixture = apply_all(vec![focus("begin")]);
    assert_eq!(
        fixture.tv.lobby.expect("lobby").focus.as_deref(),
        Some("begin")
    );
    let fixture = apply_all(vec![focus("seat:2"), ScreenEdit::LobbyFocus { id: None }]);
    assert_eq!(fixture.tv.lobby.expect("lobby").focus, None);
    for bad in [
        "seat:0",
        "seat:1001",
        "seat:x",
        "nav:1",
        "Begin",
        "begin now",
    ] {
        assert_eq!(
            make(focus(bad)).1,
            Err(SceneError::Screen(ScreenError::BadFocus(bad.to_owned()))),
            "{bad}"
        );
    }
}

#[test]
fn a_part_can_be_taken_away_on_its_own() {
    let built = vec![
        ScreenEdit::LobbyLine {
            field: LobbyLine::Title,
            text: "The Ashen Citadel Awaits".to_owned(),
        },
        ScreenEdit::LobbySeats { count: 3 },
        ScreenEdit::LobbyJoin {
            join: JoinEdit {
                game_code: Some("AC7F-K8M2".to_owned()),
                ..JoinEdit::default()
            },
        },
        ScreenEdit::LobbyParty {
            party: PartyEdit {
                joined: Some(1),
                ..PartyEdit::default()
            },
        },
        ScreenEdit::LobbyBegin {
            begin: BeginEdit {
                enabled: Some(true),
                ..BeginEdit::default()
            },
        },
    ];
    for part in LobbyPart::ALL {
        let mut edits = built.clone();
        edits.push(ScreenEdit::LobbyRemove { part });
        let lobby = apply_all(edits).tv.lobby.expect("lobby");
        let gone = match part {
            LobbyPart::Title => lobby.title.is_empty(),
            LobbyPart::Seats => lobby.seats.is_empty(),
            LobbyPart::Join => lobby.join.is_none(),
            LobbyPart::Party => lobby.party.is_none(),
            LobbyPart::Begin => lobby.begin.is_none(),
        };
        assert!(gone, "{}", part.key());
        let kept = [
            !lobby.title.is_empty(),
            !lobby.seats.is_empty(),
            lobby.join.is_some(),
            lobby.party.is_some(),
            lobby.begin.is_some(),
        ];
        assert_eq!(
            kept.iter().filter(|kept| **kept).count(),
            4,
            "{}",
            part.key()
        );
    }
    assert_eq!(LobbyPart::from_key("seats"), Some(LobbyPart::Seats));
    assert_eq!(LobbyPart::from_key("legend"), None);
}

#[test]
fn the_begin_button_is_enabled_and_labelled_live() {
    let fixture = apply_all(vec![
        ScreenEdit::LobbyBegin {
            begin: BeginEdit {
                label: Some("Begin Adventure".to_owned()),
                note: Some("Wait for all players to be ready".to_owned()),
                ..BeginEdit::default()
            },
        },
        ScreenEdit::LobbyBegin {
            begin: BeginEdit {
                enabled: Some(true),
                ..BeginEdit::default()
            },
        },
    ]);
    let begin = fixture.tv.lobby.expect("lobby").begin.expect("begin");
    assert!(begin.enabled);
    assert_eq!(begin.label.as_deref(), Some("Begin Adventure"));
}

#[test]
fn the_lobby_lines_change_and_clear_takes_the_lobby_away() {
    let fixture = apply_all(vec![
        ScreenEdit::LobbyLine {
            field: LobbyLine::Title,
            text: "The Ashen Citadel Awaits".to_owned(),
        },
        ScreenEdit::LobbyLine {
            field: LobbyLine::Subtitle,
            text: "A lost stronghold.".to_owned(),
        },
    ]);
    let lobby = fixture.tv.lobby.expect("lobby");
    assert_eq!(
        (lobby.title.as_str(), lobby.subtitle.as_deref()),
        ("The Ashen Citadel Awaits", Some("A lost stronghold."))
    );
    let cleared = apply_all(vec![
        ScreenEdit::LobbyLine {
            field: LobbyLine::Title,
            text: "x".to_owned(),
        },
        ScreenEdit::LobbyClear {},
    ]);
    assert!(cleared.tv.lobby.is_none());
}

#[test]
fn pills_change_one_part_at_a_time_and_clear_takes_a_part_away() {
    let fixture = apply_all(vec![
        ScreenEdit::PillSet {
            pill: PillEdit {
                party: Some(PartyCount {
                    joined: 4,
                    capacity: 6,
                }),
                logo: Some(LogoStyle::Inline),
                ..PillEdit::default()
            },
        },
        ScreenEdit::PillClear {
            part: PillPart::Rate,
        },
    ]);
    let bar = fixture.tv.pills.expect("pills");
    assert_eq!(bar.session.as_deref(), Some("The Ember Roads"));
    assert_eq!(bar.balance.as_deref(), Some("$28.40"));
    assert_eq!(bar.rate, None);
    assert_eq!(
        bar.party,
        Some(PartyCount {
            joined: 4,
            capacity: 6
        })
    );
    assert_eq!(bar.logo, Some(LogoStyle::Inline));
}

#[test]
fn a_party_of_more_than_its_capacity_is_refused() {
    let edit = ScreenEdit::PillSet {
        pill: PillEdit {
            party: Some(PartyCount {
                joined: 7,
                capacity: 6,
            }),
            ..PillEdit::default()
        },
    };
    assert_eq!(
        make(edit).1,
        Err(SceneError::Screen(ScreenError::JoinedOverCapacity {
            joined: 7,
            capacity: 6
        }))
    );
}

#[test]
fn the_roleplay_card_gets_a_title_a_bubble_and_a_caption() {
    let fixture = apply_all(vec![ScreenEdit::RoleplaySet {
        roleplay: RoleplayEdit {
            title: Some("1. Arrive in Town".to_owned()),
            icon: Some(Glyph::Shield),
            speaker: Some("Kael".to_owned()),
            line: Some("So this is Valemere.".to_owned()),
            tone: Some(BubbleTone::Hero),
            tail_x_percent: Some(40),
            caption: Some("The party arrives.".to_owned()),
            ..RoleplayEdit::default()
        },
    }]);
    let card = fixture.tv.roleplay.expect("a card");
    assert_eq!(card.title.expect("title").glyph, Glyph::Shield);
    assert_eq!(card.caption.as_deref(), Some("The party arrives."));
    assert_eq!(card.bubbles.len(), 1);
    assert_eq!(
        (
            card.bubbles[0].speaker.as_str(),
            card.bubbles[0].tail_x_percent
        ),
        ("Kael", 40)
    );
}

#[test]
fn a_second_bubble_is_added_by_number_and_a_bad_number_or_percent_is_refused() {
    let second = ScreenEdit::RoleplaySet {
        roleplay: RoleplayEdit {
            bubble: Some(2),
            speaker: Some("Lyra".to_owned()),
            line: Some("Hm.".to_owned()),
            ..RoleplayEdit::default()
        },
    };
    assert_eq!(
        apply_all(vec![second])
            .tv
            .roleplay
            .expect("card")
            .bubbles
            .len(),
        2
    );
    let none = ScreenEdit::RoleplaySet {
        roleplay: RoleplayEdit {
            bubble: Some(0),
            line: Some("x".to_owned()),
            ..RoleplayEdit::default()
        },
    };
    assert_eq!(
        make(none).1,
        Err(SceneError::Screen(ScreenError::BadBubbleNumber {
            number: 0
        }))
    );
    let wide = ScreenEdit::RoleplaySet {
        roleplay: RoleplayEdit {
            width_percent: Some(200),
            line: Some("x".to_owned()),
            ..RoleplayEdit::default()
        },
    };
    assert!(matches!(
        make(wide).1,
        Err(SceneError::Screen(ScreenError::BadPercent { .. }))
    ));
}

#[test]
fn the_loading_bar_and_its_chips_change_live() {
    let fixture = apply_all(vec![
        ScreenEdit::LoadingSet {
            loading: LoadingEdit {
                title: Some("The Ashen Citadel".to_owned()),
                percent: Some(40),
                narrator_name: Some("Vaelin".to_owned()),
                ..LoadingEdit::default()
            },
        },
        ScreenEdit::LoadingSeat {
            number: 2,
            chip: ChipEdit {
                name: Some("Vyx".to_owned()),
                ready: Some(true),
                ..ChipEdit::default()
            },
        },
    ]);
    let loading = fixture.tv.loading.expect("loading");
    assert_eq!((loading.percent, loading.seats.len()), (40, 2));
    assert!(loading.seats[1].ready && loading.seats[0].name == "Seat 1");
    assert_eq!(loading.narrator.expect("narrator").name, "Vaelin");
    let over = ScreenEdit::LoadingSet {
        loading: LoadingEdit {
            percent: Some(150),
            ..LoadingEdit::default()
        },
    };
    assert!(matches!(
        make(over).1,
        Err(SceneError::Screen(ScreenError::BadPercent { .. }))
    ));
}

#[test]
fn reset_drops_the_screen_changes() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::Screen {
                edit: seat(1, SeatState::Open, "x"),
            },
            &base(),
        )
        .expect("applies");
    assert!(!overlay.is_empty());
    overlay
        .apply(SceneChange::Reset {}, &base())
        .expect("resets");
    assert!(overlay.is_empty());
}

#[test]
fn the_audit_fields_name_the_component_and_field_and_never_carry_free_text() {
    let edit = ScreenEdit::LobbySeat {
        number: 2,
        seat: SeatEdit {
            state: Some(SeatState::Creating),
            name: Some("Secret Name".to_owned()),
            portrait: Some("lobby cam.webp".to_owned()),
            progress_percent: Some(65),
            ..SeatEdit::default()
        },
    };
    let (verb, pairs) = edit.audit_fields();
    assert_eq!(verb, "screen");
    assert_eq!(pairs[0], ("component", "lobby".to_owned()));
    assert_eq!(pairs[1], ("field", "seat".to_owned()));
    assert!(pairs.contains(&("state", "creating".to_owned())));
    assert!(pairs.contains(&("progress", "65".to_owned())));
    assert!(pairs.contains(&("portrait", "lobby_cam.webp".to_owned())));
    assert!(pairs.iter().all(|(_, value)| !value.contains("Secret")));
}

#[test]
fn the_audit_lines_of_the_other_components_are_named() {
    let cases = [
        (ScreenEdit::LobbyClear {}, "lobby", "clear"),
        (ScreenEdit::LobbySeats { count: 4 }, "lobby", "seats"),
        (
            ScreenEdit::LobbyFocus {
                id: Some("begin".to_owned()),
            },
            "lobby",
            "focus",
        ),
        (
            ScreenEdit::LobbyRemove {
                part: LobbyPart::Join,
            },
            "lobby",
            "remove",
        ),
        (
            ScreenEdit::PillClear {
                part: PillPart::Balance,
            },
            "pill",
            "clear",
        ),
        (ScreenEdit::RoleplayClear {}, "roleplay", "clear"),
        (ScreenEdit::LoadingClear {}, "loading", "clear"),
        (
            ScreenEdit::LobbyLine {
                field: LobbyLine::Title,
                text: "Words here".to_owned(),
            },
            "lobby",
            "title",
        ),
    ];
    for (edit, component, field) in cases {
        let (_, pairs) = edit.audit_fields();
        assert_eq!(pairs[0].1, component);
        assert_eq!(pairs[1].1, field);
    }
}

#[test]
fn a_change_round_trips_through_json_as_the_wire_carries_it() {
    let change = SceneChange::Screen {
        edit: seat(2, SeatState::Creating, "Alyssa"),
    };
    let text = serde_json::to_string(&change).expect("serialises");
    assert!(
        text.contains("\"change\":\"screen\"") && text.contains("\"action\":\"lobby_seat\""),
        "{text}"
    );
    let back: SceneChange = serde_json::from_str(&text).expect("reads");
    assert_eq!(back, change);
}

#[test]
fn media_names_are_listed_for_the_host_to_check() {
    let edit = ScreenEdit::LobbySeat {
        number: 1,
        seat: SeatEdit {
            portrait: Some("lobby-cam.webp".to_owned()),
            ..SeatEdit::default()
        },
    };
    assert_eq!(edit.media_names(), vec!["lobby-cam.webp"]);
    assert!(ScreenEdit::LobbyClear {}.media_names().is_empty());
}
