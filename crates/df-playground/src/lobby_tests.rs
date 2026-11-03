//! Tests of the lobby, loading, roleplay and top-bar model: what a seat says in each state, how a
//! count and a code read, the placeholder QR cells and where a bubble's tail sits.

use crate::lobby_model::{
    Bubble, BubbleTone, CHARACTER_STEPS, Invite, LEGEND, LobbySeat, PartyCount, QR_CELLS,
    SeatState, SummaryIcon, count_text, party_text, qr_like_cells, seat_view, session_text,
    spaced_code, summary_word, tail_of,
};

fn seat(state: SeatState) -> LobbySeat {
    LobbySeat {
        state,
        name: None,
        class: None,
        glyph: None,
        level: None,
        portrait: None,
        host: false,
        progress_percent: None,
        step: None,
        stage: None,
        badge: None,
        controller: false,
    }
}

#[test]
fn an_open_seat_is_called_by_its_number_and_says_open() {
    let view = seat_view(&seat(SeatState::Open), 3);
    assert_eq!(
        (view.name.as_str(), view.sub_line.as_deref()),
        ("Seat 4", Some("Open"))
    );
    assert_eq!(
        (view.badge, view.progress_percent, view.step),
        (None, None, None)
    );
    assert_eq!(view.key, "open");
}

#[test]
fn a_closed_seat_says_closed_not_locked_in() {
    let view = seat_view(&seat(SeatState::Closed), 5);
    assert_eq!(view.sub_line.as_deref(), Some("Closed"));
    assert_eq!(view.badge, None);
}

#[test]
fn a_locked_in_seat_shows_class_and_level_and_the_locked_in_badge() {
    let mut cam = seat(SeatState::LockedIn);
    cam.name = Some("  Cam ".to_owned());
    cam.class = Some("Paladin".to_owned());
    cam.level = Some(5);
    let view = seat_view(&cam, 0);
    assert_eq!(view.name, "Cam");
    assert_eq!(view.sub_line.as_deref(), Some("Paladin \u{2022} Lv 5"));
    assert_eq!(view.badge.as_deref(), Some("Locked in"));
}

#[test]
fn a_badge_word_from_the_scene_replaces_the_states_own() {
    let mut nyx = seat(SeatState::LockedIn);
    nyx.badge = Some("Ready".to_owned());
    assert_eq!(seat_view(&nyx, 2).badge.as_deref(), Some("Ready"));
}

#[test]
fn a_creating_seat_has_no_badge_unless_the_scene_gives_one_and_holds_its_progress() {
    let mut maker = seat(SeatState::Creating);
    maker.progress_percent = Some(250);
    maker.step = Some(9);
    let view = seat_view(&maker, 1);
    assert_eq!(view.sub_line.as_deref(), Some("Creating character"));
    assert_eq!(view.badge, None);
    assert_eq!(view.progress_percent, Some(100));
    assert_eq!(view.step, Some(CHARACTER_STEPS.len() - 1));
    maker.badge = Some("In character menu".to_owned());
    assert_eq!(
        seat_view(&maker, 1).badge.as_deref(),
        Some("In character menu")
    );
}

#[test]
fn progress_and_step_belong_to_a_creating_seat_only() {
    let mut locked = seat(SeatState::LockedIn);
    locked.progress_percent = Some(40);
    locked.step = Some(1);
    let view = seat_view(&locked, 0);
    assert_eq!((view.progress_percent, view.step), (None, None));
}

#[test]
fn counts_codes_and_pills_read_the_way_the_renders_do() {
    assert_eq!(count_text(4, 6), "4 of 6");
    assert_eq!(spaced_code("AC7F-K8M2"), "A C 7 F \u{2013} K 8 M 2");
    assert_eq!(
        party_text(PartyCount {
            joined: 4,
            capacity: 6
        }),
        "Party: 4 / 6"
    );
    assert_eq!(
        session_text(" The Ember Roads "),
        "Session: The Ember Roads"
    );
}

#[test]
fn the_summary_word_defaults_to_joined() {
    let mut invite = Invite {
        heading: None,
        hint: None,
        join_link: None,
        game_code: None,
        joined: 3,
        capacity: 6,
        word: None,
        detail: None,
        icon: SummaryIcon::Check,
    };
    assert_eq!(summary_word(&invite), "Joined");
    invite.word = Some("Ready".to_owned());
    assert_eq!(summary_word(&invite), "Ready");
}

#[test]
fn the_legend_names_the_three_states_in_order() {
    let keys: Vec<&str> = LEGEND.iter().map(|entry| entry.key).collect();
    assert_eq!(keys, ["locked", "creating", "open"]);
}

#[test]
fn the_qr_like_block_is_square_deterministic_and_has_three_finder_squares() {
    let first = qr_like_cells("AC7F-K8M2");
    assert_eq!(first.len(), QR_CELLS * QR_CELLS);
    assert_eq!(first, qr_like_cells("AC7F-K8M2"));
    assert_ne!(first, qr_like_cells("ZZZZ-0000"));
    let at = |row: usize, column: usize| first[row * QR_CELLS + column];
    for (row, column) in [(0, 0), (0, QR_CELLS - 7), (QR_CELLS - 7, 0)] {
        assert!(at(row, column) && at(row + 3, column + 3) && !at(row + 1, column + 1));
    }
    assert!(!at(QR_CELLS - 1, QR_CELLS - 1) || at(QR_CELLS - 1, QR_CELLS - 1));
}

fn bubble(left: u8, top: u8, width: u8, tail_x: u8, tail_y: u8) -> Bubble {
    Bubble {
        speaker: "Kael".to_owned(),
        line: "So this is Valemere.".to_owned(),
        emphasis: None,
        glyph: None,
        portrait: None,
        tone: BubbleTone::Hero,
        left_percent: left,
        top_percent: top,
        width_percent: width,
        tail_x_percent: tail_x,
        tail_y_percent: tail_y,
    }
}

#[test]
fn a_tail_hangs_below_when_the_speaker_is_lower_and_rises_when_higher() {
    assert!(tail_of(&bubble(20, 10, 20, 30, 40)).below);
    assert!(!tail_of(&bubble(10, 60, 20, 22, 40)).below);
}

#[test]
fn a_tail_root_is_held_away_from_the_corners_and_its_tip_inside_the_box() {
    let near_left = tail_of(&bubble(20, 10, 20, 20, 40));
    assert_eq!((near_left.root_percent, near_left.tip_percent), (12, 0));
    let far_right = tail_of(&bubble(20, 10, 20, 90, 40));
    assert_eq!((far_right.root_percent, far_right.tip_percent), (88, 100));
    let middle = tail_of(&bubble(20, 10, 20, 30, 40));
    assert_eq!((middle.root_percent, middle.tip_percent), (50, 50));
}
