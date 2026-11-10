//! The inventory panel: its rows as the kit's list, how each row looks (struck at zero, a last-one
//! tint, a flash for a move), how it is checked, how a live change sets and clears it and what the
//! page redraws when it changes.

use std::collections::BTreeSet;
use std::rc::Rc;

use crate::bubble_place::Rect;
use crate::clues::board_covers;
use crate::controls::{Controls, FrameMoment, FrameReason, PlateFrame, PlateTimer};
use crate::draw_plan::{DrawPlan, Part, Snapshot};
use crate::fixture::{Fixture, PlateKind};
use crate::inventory::{
    Flash, FlashClock, Flashing, InventoryPanel, InventoryRow, MoveKind, RowLook, RowMove, RowUse,
    picture_is_up,
};
use crate::kit::ledger::LedgerKind;
use crate::scene::{SceneChange, SceneError, SceneOverlay};

const FIXTURE: &str = r#"{
    "name": "Base",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn base() -> Fixture {
    Fixture::parse(FIXTURE).expect("the fixture reads")
}

fn row(id: &str, count: u64, usage: RowUse) -> InventoryRow {
    InventoryRow {
        id: id.to_owned(),
        name: format!("the {id}"),
        count,
        usage,
    }
}

fn moved(row: &str, before: u64, after: u64, kind: MoveKind) -> RowMove {
    RowMove {
        row: row.to_owned(),
        before,
        after,
        kind,
        at_ms: 1_000,
    }
}

/// The flashes on at `now_ms` for a page that first looks at `panel` then.
fn flashing_at(panel: &InventoryPanel, now_ms: u64) -> Vec<Flashing> {
    FlashClock::default().flashing(panel, now_ms, true)
}

fn panel() -> InventoryPanel {
    InventoryPanel {
        owner: "danny".to_owned(),
        title: "Danny".to_owned(),
        rows: vec![
            row("ochre", 4, RowUse::Consumed),
            row("rags", 1, RowUse::Consumed),
            row("boards", 0, RowUse::Consumed),
            row("easel", 1, RowUse::Kept),
        ],
        serial: 7,
        moves: Vec::new(),
    }
}

fn drawn(overlay: &SceneOverlay) -> Fixture {
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture
}

fn snapshot(fixture: &Fixture) -> Snapshot {
    Snapshot {
        names: Vec::new(),
        controls: Controls::default(),
        loaded_name: None,
        shown: Some(Rc::new(Ok(fixture.clone()))),
        connection_problem: None,
        live_changes: false,
        audio_unlocked: false,
    }
}

#[test]
fn a_scene_has_no_inventory_panel_unless_one_is_set() {
    assert_eq!(base().tv.inventory, None);
    assert_eq!(SceneOverlay::default().inventory.set_value(), None);
}

#[test]
fn the_rows_are_the_kits_loot_list_with_a_count_for_each() {
    let list = panel().ledger();
    assert_eq!(list.kind, LedgerKind::Loot);
    assert!(list.framed);
    assert_eq!(list.title(), Some("Danny"));
    let rows: Vec<(&str, Option<&str>)> = list
        .rows()
        .iter()
        .map(|row| (row.item.id.as_str(), row.figure))
        .collect();
    assert_eq!(
        rows,
        [
            ("ochre", Some("x4")),
            ("rags", Some("x1")),
            ("boards", Some("x0")),
            ("easel", Some("x1")),
        ],
        "a row at zero stays listed, in its place"
    );
}

#[test]
fn a_row_at_zero_is_struck_and_a_last_supply_is_tinted_but_a_last_tool_is_not() {
    let panel = panel();
    assert_eq!(panel.look_of("ochre", &[]), RowLook::default());
    let rags = panel.look_of("rags", &[]);
    assert!(rags.last && !rags.out);
    assert_eq!(rags.classes(), "inv-last");
    let boards = panel.look_of("boards", &[]);
    assert!(boards.out && !boards.last);
    assert_eq!(boards.classes(), "inv-out");
    assert_eq!(
        panel.look_of("easel", &[]),
        RowLook::default(),
        "one easel is not the last of a supply"
    );
    assert_eq!(panel.look_of("no-such-row", &[]), RowLook::default());
}

#[test]
fn a_fall_flashes_spent_and_a_rise_flashes_gained_with_their_tags() {
    let mut panel = panel();
    panel.moves = vec![
        moved("ochre", 5, 4, MoveKind::Consume),
        moved("rags", 1, 4, MoveKind::Add),
        moved("boards", 1, 0, MoveKind::Consume),
    ];
    let on = flashing_at(&panel, 500);
    let ochre = panel.look_of("ochre", &on).flash.expect("flashes");
    assert_eq!(ochre, Flash::Spent(1));
    assert_eq!((ochre.tag().as_str(), ochre.class()), ("-1", "inv-spent"));
    let rags = panel.look_of("rags", &on).flash.expect("flashes");
    assert_eq!(rags, Flash::Gained(3));
    assert_eq!((rags.tag().as_str(), rags.class()), ("+3", "inv-gained"));
    let boards = panel.look_of("boards", &on);
    assert_eq!(boards.flash, Some(Flash::Spent(1)));
    assert_eq!(
        boards.classes(),
        "inv-out inv-spent",
        "the row that ran out is struck and flashes"
    );
}

#[test]
fn a_move_that_changed_nothing_flashes_nothing() {
    let mut panel = panel();
    panel.moves = vec![moved("ochre", 4, 4, MoveKind::Add)];
    assert!(flashing_at(&panel, 0).is_empty());
}

fn two_pushes() -> (InventoryPanel, InventoryPanel) {
    let mut first = panel();
    first.moves = vec![RowMove {
        at_ms: 1_000,
        ..moved("ochre", 5, 4, MoveKind::Consume)
    }];
    let mut second = first.clone();
    second.serial = 1_600;
    second.moves.push(RowMove {
        at_ms: 1_600,
        ..moved("rags", 1, 4, MoveKind::Add)
    });
    (first, second)
}

#[test]
fn two_quick_pushes_flash_both_moves_and_the_later_does_not_clear_the_earlier() {
    let (first, second) = two_pushes();
    let mut clock = FlashClock::default();
    let on = clock.flashing(&first, 10_000, true);
    assert_eq!(on.len(), 1);
    // The second push arrives 600 ms later and replaces the panel: the earlier flash carries on.
    let on = clock.flashing(&second, 10_600, true);
    let rows: Vec<(&str, u64)> = on.iter().map(|on| (on.row.as_str(), on.age_ms)).collect();
    assert_eq!(rows, [("ochre", 600), ("rags", 0)]);
}

#[test]
fn a_flash_lasts_four_seconds_from_when_the_page_first_showed_it_and_never_restarts() {
    let (first, _) = two_pushes();
    let mut clock = FlashClock::default();
    clock.flashing(&first, 50_000, true);
    for (now, ages) in [
        (50_000, Some(0)),
        (52_500, Some(2_500)),
        (53_999, Some(3_999)),
    ] {
        let on = clock.flashing(&first, now, true);
        assert_eq!(on.first().map(|on| on.age_ms), ages, "at {now}");
    }
    assert!(
        clock.flashing(&first, 54_000, true).is_empty(),
        "over after FLASH_MS"
    );
    assert!(
        clock.flashing(&first, 54_100, true).is_empty(),
        "a panel drawn again after that does not flash it again"
    );
}

#[test]
fn a_push_while_the_panel_is_hidden_flashes_for_the_whole_time_once_it_returns() {
    let (first, second) = two_pushes();
    let mut clock = FlashClock::default();
    clock.flashing(&first, 10_000, true);
    // A picture goes up, and the second push comes while the panel is faded out.
    assert_eq!(
        clock.flashing(&second, 11_000, false).len(),
        1,
        "only the move already shown carries on; the new one waits"
    );
    // The room returns 6 s later: the first flash is over, the second one starts now.
    let on = clock.flashing(&second, 17_000, true);
    let rows: Vec<(&str, u64)> = on.iter().map(|on| (on.row.as_str(), on.age_ms)).collect();
    assert_eq!(rows, [("rags", 0)]);
    // Back within the time, the first one is still on.
    let mut clock = FlashClock::default();
    clock.flashing(&first, 10_000, true);
    clock.flashing(&second, 11_000, false);
    let on = clock.flashing(&second, 12_000, true);
    let rows: Vec<(&str, u64)> = on.iter().map(|on| (on.row.as_str(), on.age_ms)).collect();
    assert_eq!(rows, [("ochre", 2_000), ("rags", 0)]);
}

#[test]
fn a_move_the_host_no_longer_carries_is_forgotten() {
    let (first, second) = two_pushes();
    let mut clock = FlashClock::default();
    clock.flashing(&first, 10_000, true);
    clock.flashing(&panel(), 10_500, true);
    let on = clock.flashing(&second, 11_000, true);
    assert_eq!(on.len(), 2, "both start now: the first was forgotten");
    assert!(on.iter().all(|on| on.age_ms == 0));
}

#[test]
fn a_panel_is_checked_at_the_edge() {
    assert_eq!(panel().check(), Ok(()));
    let mut blank_title = panel();
    blank_title.title = " ".to_owned();
    assert_eq!(
        blank_title.check(),
        Err(SceneError::Empty("the inventory's title"))
    );
    let mut twice = panel();
    twice.rows.push(row("ochre", 1, RowUse::Consumed));
    assert_eq!(twice.check(), Err(SceneError::DuplicateRow("ochre".into())));
    let mut stray = panel();
    stray.moves = vec![moved("gone", 2, 1, MoveKind::Consume)];
    assert_eq!(stray.check(), Err(SceneError::UnknownRow("gone".into())));
}

#[test]
fn a_panel_reads_and_an_unknown_key_is_refused() {
    let text = serde_json::to_string(&panel()).expect("writes");
    let read: InventoryPanel = serde_json::from_str(&text).expect("reads");
    assert_eq!(read, panel());
    let with_extra = text.replacen('{', r#"{"extra":1,"#, 1);
    assert!(serde_json::from_str::<InventoryPanel>(&with_extra).is_err());
}

#[test]
fn a_live_change_sets_and_clears_the_panel_as_its_own_slot() {
    let base = base();
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(SceneChange::Inventory { panel: panel() }, &base)
        .expect("sets");
    assert_eq!(overlay.changed_slots(), ["inventory"]);
    assert_eq!(drawn(&overlay).tv.inventory, Some(panel()));
    overlay
        .apply(SceneChange::ClearInventory {}, &base)
        .expect("clears");
    assert_eq!(drawn(&overlay).tv.inventory, None);
    overlay
        .apply(SceneChange::Inventory { panel: panel() }, &base)
        .expect("sets");
    assert_eq!(overlay.cleared().inventory.set_value(), None);
}

#[test]
fn a_panel_with_a_row_twice_is_refused_by_the_live_change() {
    let mut twice = panel();
    twice.rows.push(row("ochre", 1, RowUse::Consumed));
    let mut overlay = SceneOverlay::default();
    assert_eq!(
        overlay.apply(SceneChange::Inventory { panel: twice }, &base()),
        Err(SceneError::DuplicateRow("ochre".to_owned()))
    );
    assert!(overlay.is_empty());
}

#[test]
fn only_the_inventory_slot_is_redrawn_when_the_panel_changes() {
    let before = snapshot(&base());
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(SceneChange::Inventory { panel: panel() }, &base())
        .expect("sets");
    let shown = snapshot(&drawn(&overlay));
    let plan = DrawPlan::between(Some(&before), &shown);
    assert_eq!(plan.parts, BTreeSet::from([Part::TvInventory]));
    assert!(!plan.devices && !plan.stage);
    assert!(
        DrawPlan::between(Some(&shown), &snapshot(&drawn(&overlay))).is_empty(),
        "the same panel draws nothing"
    );
    let mut next = panel();
    next.serial = 8;
    next.moves = vec![moved("ochre", 5, 4, MoveKind::Consume)];
    overlay
        .apply(SceneChange::Inventory { panel: next }, &base())
        .expect("sets");
    let plan = DrawPlan::between(Some(&shown), &snapshot(&drawn(&overlay)));
    assert_eq!(plan.parts, BTreeSet::from([Part::TvInventory]));
}

#[test]
fn the_panel_fades_while_it_would_cover_a_face_a_box_or_another_panel() {
    let lower_right_panel = Rect::new(700.0, 300.0, 250.0, 200.0);
    assert!(!board_covers(&lower_right_panel, &[], &[]));
    let face_clear = Rect::new(300.0, 150.0, 60.0, 80.0);
    assert!(!board_covers(&lower_right_panel, &[face_clear], &[]));
    let face_under = Rect::new(800.0, 350.0, 60.0, 80.0);
    assert!(board_covers(&lower_right_panel, &[face_under], &[]));
    let bubble = Rect::new(650.0, 280.0, 100.0, 60.0);
    assert!(board_covers(&lower_right_panel, &[], &[bubble]));
    let caption_bar = Rect::new(100.0, 480.0, 800.0, 60.0);
    assert!(board_covers(&lower_right_panel, &[], &[caption_bar]));
    let beside = Rect::new(950.0, 300.0, 40.0, 200.0);
    assert!(
        !board_covers(&lower_right_panel, &[], &[beside]),
        "touching is not covering"
    );
}

fn timer(last: Option<(FrameReason, FrameMoment)>) -> PlateTimer {
    PlateTimer {
        running: true,
        last: last.map(|(reason, moment)| PlateFrame {
            frame: "frame".to_owned(),
            reason,
            moment,
            at_ms: 1_000,
        }),
        ..PlateTimer::default()
    }
}

#[test]
fn a_reveal_or_a_close_up_insert_on_screen_fades_the_panel_and_the_room_brings_it_back() {
    let room = Some(PlateKind::Room);
    assert!(!picture_is_up(&timer(None), None), "no picture, no frame");
    for reason in [
        FrameReason::Reveal,
        FrameReason::Inspection,
        FrameReason::Focus,
    ] {
        assert!(
            picture_is_up(&timer(Some((reason, FrameMoment::Shown))), room),
            "{reason:?} on screen covers the picture"
        );
        assert!(
            !picture_is_up(&timer(Some((reason, FrameMoment::Ended))), room),
            "{reason:?} ended: the room is back"
        );
        assert!(
            !picture_is_up(&timer(Some((reason, FrameMoment::Started))), room),
            "{reason:?} still being made: the room is still up"
        );
    }
    assert!(
        picture_is_up(&timer(None), Some(PlateKind::CloseUp)),
        "a close-up backdrop covers the picture"
    );
    assert!(
        picture_is_up(&timer(None), Some(PlateKind::Show)),
        "the day's show plate covers the picture"
    );
    let ended = Some((FrameReason::Walk, FrameMoment::Shown));
    assert!(
        picture_is_up(&timer(ended), Some(PlateKind::Show)),
        "whatever frame is newest, the show plate is up"
    );
}

#[test]
fn walk_declared_line_and_scheduled_plates_keep_the_panel() {
    for reason in [
        FrameReason::Walk,
        FrameReason::Declared,
        FrameReason::Line,
        FrameReason::Schedule,
    ] {
        for backdrop in [PlateKind::Room, PlateKind::RoomKeptCast] {
            assert!(
                !picture_is_up(&timer(Some((reason, FrameMoment::Shown))), Some(backdrop)),
                "{reason:?} keeps the panel"
            );
        }
    }
}
