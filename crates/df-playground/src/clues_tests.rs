//! The clue board: its checks, how it is laid out, how a live change sets and clears it, and what
//! the page redraws and the timeline records when it changes.

use std::collections::BTreeSet;
use std::rc::Rc;

use crate::clues::{ClueBoard, ClueNote, NoteLook};
use crate::controls::Controls;
use crate::copy::ReadingPace;
use crate::draw_plan::{DrawPlan, Part, Snapshot};
use crate::fixture::Fixture;
use crate::limits::{CLUE_NOTES_BEFORE_COMPACT, MAXIMUM_CLUE_NOTES};
use crate::overlay::Edit;
use crate::scene::{SceneChange, SceneError, SceneOverlay};
use crate::timeline::{EndReason, PageLog, Showing, TimelineKind};

const FIXTURE: &str = r#"{
    "name": "Base",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn base() -> Fixture {
    Fixture::parse(FIXTURE).expect("the fixture reads")
}

fn note(id: &str) -> ClueNote {
    ClueNote {
        id: id.to_owned(),
        name: format!("Clue {id}"),
        detail: "A close look shows scratches.".to_owned(),
        worked_out: false,
        picture: Some(format!("{id}.png")),
    }
}

fn notes(count: usize) -> Vec<ClueNote> {
    (0..count).map(|index| note(&format!("c{index}"))).collect()
}

fn drawn(overlay: &SceneOverlay) -> Fixture {
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture
}

#[test]
fn a_fixture_without_a_board_has_none_and_a_board_reads_with_defaults() {
    assert_eq!(base().tv.clues, None);
    let board: ClueBoard =
        serde_json::from_str(r#"{"notes":[{"id":"a","name":"Ring","detail":"Gold."}]}"#)
            .expect("reads");
    assert!(!board.notes[0].worked_out);
    assert_eq!(board.notes[0].picture, None);
    assert!(
        serde_json::from_str::<ClueBoard>(r#"{"notes":[],"extra":1}"#).is_err(),
        "an unknown key is refused"
    );
}

#[test]
fn the_newest_note_is_first_and_a_small_board_keeps_every_note_regular() {
    let board = ClueBoard {
        notes: notes(CLUE_NOTES_BEFORE_COMPACT),
    };
    let placed = board.placed();
    let ids: Vec<&str> = placed
        .iter()
        .map(|placed| placed.note.id.as_str())
        .collect();
    assert_eq!(ids, ["c4", "c3", "c2", "c1", "c0"]);
    assert_eq!(placed[0].look, NoteLook::Newest);
    assert!(
        placed[1..]
            .iter()
            .all(|placed| placed.look == NoteLook::Regular)
    );
}

#[test]
fn past_the_threshold_the_older_notes_compress_and_the_newest_stays_whole() {
    let board = ClueBoard {
        notes: notes(CLUE_NOTES_BEFORE_COMPACT + 1),
    };
    let placed = board.placed();
    assert_eq!(placed[0].look, NoteLook::Newest);
    assert!(
        placed[1..]
            .iter()
            .all(|placed| placed.look == NoteLook::Compact)
    );
}

#[test]
fn the_key_changes_with_the_notes_and_never_carries_their_words() {
    let first = ClueBoard { notes: notes(2) };
    let mut worked = first.clone();
    worked.notes[0].worked_out = true;
    let longer = ClueBoard { notes: notes(3) };
    assert_eq!(first.board_identity(), first.clone().board_identity());
    assert_ne!(first.board_identity(), worked.board_identity());
    assert_ne!(first.board_identity(), longer.board_identity());
    assert!(first.board_identity().starts_with("clues-2-"));
    assert!(!first.board_identity().contains("Clue"));
}

#[test]
fn a_board_is_checked_at_the_edge() {
    let mut overlay = SceneOverlay::default();
    let set = |notes: Vec<ClueNote>| SceneChange::Clues { notes };
    assert_eq!(
        overlay.apply(set(Vec::new()), &base()),
        Err(SceneError::Empty("the clue board"))
    );
    let mut blank = notes(1);
    blank[0].name = "  ".to_owned();
    assert_eq!(
        overlay.apply(set(blank), &base()),
        Err(SceneError::Empty("a clue's name"))
    );
    let mut twice = notes(2);
    twice[1].id = "c0".to_owned();
    assert_eq!(
        overlay.apply(set(twice), &base()),
        Err(SceneError::DuplicateClue("c0".to_owned()))
    );
    let mut nameless = notes(1);
    nameless[0].picture = Some(String::new());
    assert_eq!(
        overlay.apply(set(nameless), &base()),
        Err(SceneError::Empty("a media name"))
    );
    assert_eq!(
        overlay.apply(set(notes(MAXIMUM_CLUE_NOTES + 1)), &base()),
        Err(SceneError::TooMany {
            what: "the clue board",
            limit: MAXIMUM_CLUE_NOTES
        })
    );
    assert!(
        overlay.is_empty(),
        "a refused change leaves the overlay as it was"
    );
}

#[test]
fn a_board_is_set_replaced_and_cleared_and_a_reset_drops_it() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(SceneChange::Clues { notes: notes(1) }, &base())
        .expect("sets");
    assert_eq!(overlay.changed_slots(), ["clues"]);
    assert_eq!(
        drawn(&overlay).tv.clues,
        Some(ClueBoard { notes: notes(1) })
    );
    overlay
        .apply(SceneChange::Clues { notes: notes(3) }, &base())
        .expect("replaces");
    assert_eq!(
        drawn(&overlay).tv.clues,
        Some(ClueBoard { notes: notes(3) })
    );
    overlay
        .apply(SceneChange::ClearClues {}, &base())
        .expect("clears");
    assert_eq!(overlay.clues, Edit::Clear);
    assert_eq!(drawn(&overlay).tv.clues, None);
    overlay
        .apply(SceneChange::Clues { notes: notes(1) }, &base())
        .expect("sets again");
    overlay
        .apply(SceneChange::Reset {}, &base())
        .expect("resets");
    assert!(overlay.is_empty());
}

#[test]
fn a_change_round_trips_through_its_wire_form() {
    let change = SceneChange::Clues { notes: notes(2) };
    let written = serde_json::to_string(&change).expect("writes");
    assert!(written.starts_with(r#"{"change":"clues""#), "{written}");
    let read: SceneChange = serde_json::from_str(&written).expect("reads");
    assert_eq!(read, change);
    let clear = serde_json::to_string(&SceneChange::ClearClues {}).expect("writes");
    assert_eq!(clear, r#"{"change":"clear_clues"}"#);
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
fn only_the_board_slot_is_redrawn_when_the_board_changes() {
    let before = snapshot(&base());
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(SceneChange::Clues { notes: notes(2) }, &base())
        .expect("sets");
    let after = snapshot(&drawn(&overlay));
    let plan = DrawPlan::between(Some(&before), &after);
    assert_eq!(plan.parts, BTreeSet::from([Part::TvClues]));
    assert!(!plan.devices && !plan.stage);
    let same = DrawPlan::between(Some(&after), &snapshot(&drawn(&overlay)));
    assert!(same.is_empty(), "the same board draws nothing");
}

fn board_showing(count: usize) -> Showing {
    Showing {
        scene: Some("quest.json".to_owned()),
        clues: vec![
            ClueBoard {
                notes: notes(count),
            }
            .board_identity(),
        ],
        ..Showing::default()
    }
}

#[test]
fn a_changed_board_ends_the_old_record_and_opens_the_new_one() {
    let mut log = PageLog::default();
    log.observe(board_showing(1), 1_000);
    log.observe(board_showing(1), 2_000);
    log.observe(board_showing(2), 3_000);
    log.observe(
        Showing {
            scene: Some("quest.json".to_owned()),
            ..Showing::default()
        },
        4_000,
    );
    let records: Vec<_> = log
        .unreported()
        .0
        .into_iter()
        .filter(|record| record.kind == TimelineKind::Clues)
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].start_page_ms, 1_000);
    assert_eq!(records[0].end_page_ms, Some(3_000));
    assert_eq!(records[0].end_reason, Some(EndReason::Replaced));
    assert_eq!(records[1].end_page_ms, Some(4_000));
    assert_eq!(records[1].end_reason, Some(EndReason::Removed));
}

#[test]
fn the_showing_of_a_scene_with_a_board_names_it() {
    let mut fixture = base();
    let board = ClueBoard { notes: notes(2) };
    fixture.tv.clues = Some(board.clone());
    let showing = Showing::of(
        Some("quest.json"),
        Some(&fixture),
        None,
        ReadingPace::Normal,
        true,
    );
    assert_eq!(showing.clues, vec![board.board_identity()]);
}

#[test]
fn a_board_opens_when_it_changes_shuts_once_and_a_redraw_does_not_restart_it() {
    use crate::clues::{BoardState, Observed, Phase};
    let mut state = BoardState::default();
    assert_eq!(state.observe(None, 0), Observed::Absent);
    assert_eq!(state.observe(Some("a".into()), 1_000), Observed::Changed);
    assert_eq!(state.phase(), Some(Phase::Open));
    assert_eq!(state.observe(Some("a".into()), 4_000), Observed::Continuing);
    assert_eq!(state.open_for_ms(4_000), 3_000, "the clock did not restart");
    assert!(state.shut());
    assert!(!state.shut(), "it shuts once");
    assert_eq!(state.open_for_ms(9_000), 0);
    assert_eq!(state.observe(Some("a".into()), 9_000), Observed::Continuing);
    assert_eq!(state.phase(), Some(Phase::Shut), "a redraw leaves it shut");
    assert_eq!(state.observe(Some("b".into()), 12_000), Observed::Changed);
    assert_eq!(state.phase(), Some(Phase::Open), "a change opens it again");
    assert_eq!(state.open_for_ms(12_500), 500);
    assert_eq!(state.observe(None, 13_000), Observed::Cleared);
    assert_eq!(state.phase(), None);
    assert!(!state.shut());
}

#[test]
fn the_board_fades_only_when_it_covers_a_face_or_a_box() {
    use crate::bubble_place::Rect;
    use crate::clues::board_covers;
    let board = Rect::new(24.0, 135.0, 100.0, 120.0);
    assert!(!board_covers(&board, &[], &[]));
    let clear = Rect::new(300.0, 200.0, 60.0, 80.0);
    assert!(!board_covers(&board, &[clear], &[clear]));
    let beside = Rect::new(124.0, 135.0, 50.0, 50.0);
    assert!(
        !board_covers(&board, &[beside], &[]),
        "touching is not covering"
    );
    let face = Rect::new(100.0, 200.0, 60.0, 80.0);
    assert!(board_covers(&board, &[clear, face], &[]));
    assert!(board_covers(&board, &[clear], &[face]));
}

#[test]
fn only_the_story_slot_is_redrawn_when_the_story_mark_changes() {
    use crate::overlay::StoryMark;

    let before = snapshot(&base());
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::Story {
                mark: StoryMark {
                    title: "The Charter".to_owned(),
                    progress: None,
                },
            },
            &base(),
        )
        .expect("sets");
    assert_eq!(overlay.changed_slots(), ["story"]);
    let after = snapshot(&drawn(&overlay));
    let plan = DrawPlan::between(Some(&before), &after);
    assert_eq!(plan.parts, BTreeSet::from([Part::TvStory]));
    assert!(!plan.devices && !plan.stage);
}

#[test]
fn a_story_mark_outside_its_run_or_without_a_title_is_refused() {
    use crate::overlay::{Progress, StoryMark};

    let mark = |title: &str, index: u16| StoryMark {
        title: title.to_owned(),
        progress: Some(Progress {
            index,
            total: 3,
            title: None,
        }),
    };
    assert!(mark("The Charter", 3).check().is_ok());
    assert!(mark("The Charter", 0).check().is_err());
    assert!(mark("The Charter", 4).check().is_err());
    assert!(mark(" ", 1).check().is_err());
}
