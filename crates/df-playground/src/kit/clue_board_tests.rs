//! The restyled clue board's notes.

use crate::clues::NoteLook;
use crate::kit::clue_board::{KitClues, note_marker};

fn clues(open: bool) -> KitClues {
    let mut clues: KitClues = serde_json::from_str(
        r#"{"board":{"notes":[
            {"id":"a","name":"Seal","detail":"Cracked","worked_out":true},
            {"id":"b","name":"Map","detail":"Torn"}
        ]}}"#,
    )
    .expect("a board reads");
    clues.open = open;
    clues
}

#[test]
fn a_board_reads_open_and_lists_the_newest_note_first() {
    let read: KitClues = serde_json::from_str(r#"{"board":{"notes":[]}}"#).expect("a board reads");
    assert!(read.open);
    let board = clues(true);
    let notes = board.notes();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].note.id, "b");
    assert_eq!(notes[0].look, NoteLook::Newest);
}

#[test]
fn a_shut_board_shows_only_the_newest_note() {
    let board = clues(false);
    let notes = board.notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(note_marker(notes[0].look), "newest");
}
