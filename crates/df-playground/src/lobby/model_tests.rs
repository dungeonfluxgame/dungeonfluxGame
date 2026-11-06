//! Tests of the lobby, loading, roleplay and top-bar model: what a seat says in each state, how a
//! count and a code read, the placeholder QR cells and where a bubble's tail sits.

use crate::fixtures::assert_keys_are_serde_names;
use crate::lobby::model::{
    BEGIN_FOCUS_ID, BeginButton, Bubble, BubbleTone, CHARACTER_STEPS, LobbyScreen, LobbySeat,
    LogoStyle, PartyCount, PartySummary, QR_CELLS, SEATS_SHOWN, SeatState, SummaryIcon, count_text,
    party_text, qr_like_cells, seat_focus_id, seat_size_key, seat_view, session_text, summary_word,
    tail_of, title_size_key,
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
fn a_locked_in_seat_shows_class_and_level_and_the_ready_seal() {
    let mut cam = seat(SeatState::LockedIn);
    cam.name = Some("  Cam ".to_owned());
    cam.class = Some("Paladin".to_owned());
    cam.level = Some(5);
    let view = seat_view(&cam, 0);
    assert_eq!(view.name, "Cam");
    assert_eq!(view.sub_line.as_deref(), Some("Paladin \u{2022} Lv 5"));
    assert_eq!(view.badge.as_deref(), Some("Ready"));
}

#[test]
fn a_badge_word_from_the_scene_replaces_the_states_own() {
    let mut nyx = seat(SeatState::LockedIn);
    nyx.badge = Some("Locked in".to_owned());
    assert_eq!(seat_view(&nyx, 2).badge.as_deref(), Some("Locked in"));
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
fn counts_and_pills_read_the_way_the_renders_do() {
    assert_eq!(count_text(4, 6), "4 of 6");
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
    let mut party = PartySummary {
        joined: 3,
        capacity: 6,
        word: None,
        detail: None,
        icon: SummaryIcon::Check,
    };
    assert_eq!(summary_word(&party), "Joined");
    party.word = Some("Ready".to_owned());
    assert_eq!(summary_word(&party), "Ready");
}

fn lobby(seats: usize, begin: bool) -> LobbyScreen {
    LobbyScreen {
        eyebrow: None,
        title: "The Ashen Citadel Awaits".to_owned(),
        subtitle: None,
        seats: (0..seats).map(|_| seat(SeatState::Open)).collect(),
        join: None,
        party: None,
        begin: begin.then_some(BeginButton {
            label: None,
            enabled: true,
            note: None,
        }),
        focus: None,
    }
}

#[test]
fn the_focus_is_the_begin_button_unless_the_scene_says_otherwise() {
    let mut screen = lobby(4, true);
    assert_eq!(screen.focus_id().as_deref(), Some(BEGIN_FOCUS_ID));
    screen.focus = Some(seat_focus_id(2));
    assert_eq!(screen.focus_id().as_deref(), Some("seat:2"));
    assert_eq!(lobby(4, false).focus_id(), None);
}

#[test]
fn the_lobby_draws_at_most_six_seats_and_keeps_the_rest_in_its_data() {
    let screen = lobby(9, true);
    assert_eq!(screen.shown_seats().len(), SEATS_SHOWN);
    assert_eq!(screen.seats.len(), 9);
    assert!(lobby(0, true).shown_seats().is_empty());
}

#[test]
fn cards_are_wide_up_to_five_seats_and_narrow_at_six() {
    assert_eq!(seat_size_key(2), "wide");
    assert_eq!(seat_size_key(5), "wide");
    assert_eq!(seat_size_key(6), "narrow");
}

#[test]
fn a_long_title_steps_down_in_size() {
    assert_eq!(title_size_key("The Ashen Citadel Awaits"), "full");
    assert_eq!(title_size_key(&"x".repeat(35)), "middle");
    assert_eq!(title_size_key(&"x".repeat(90)), "long");
    assert_eq!(title_size_key("   "), "full");
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
        anchor: None,
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

#[test]
fn the_keys_of_a_bubble_tone_are_its_serde_names() {
    assert_keys_are_serde_names(&BubbleTone::ALL, BubbleTone::key, BubbleTone::from_key);
}

#[test]
fn the_keys_of_a_seat_state_a_summary_icon_and_a_logo_style_are_their_serde_names() {
    assert_keys_are_serde_names(&SeatState::ALL, SeatState::key, SeatState::from_key);
    assert_keys_are_serde_names(&SummaryIcon::ALL, SummaryIcon::key, SummaryIcon::from_key);
    assert_keys_are_serde_names(&LogoStyle::ALL, LogoStyle::key, LogoStyle::from_key);
}
