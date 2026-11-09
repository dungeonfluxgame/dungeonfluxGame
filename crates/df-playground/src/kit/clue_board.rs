//! The clue board restyled (component 21): the board's outer pane is the only glass that blurs;
//! the notes are unblurred tiles inside it. The newest note gets a gold rim and a halo pulse, a
//! worked-out note a verdigris rim and a check.
//!
//! The board's data, its note order and its open, shut and reopen driver stay with `clues` and
//! `view::clues`; this is the look.

use serde::{Deserialize, Serialize};

use crate::clues::{ClueBoard, NoteLook, PlacedNote};

/// The board and whether it is open (a shut board is a thin strip).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KitClues {
    pub board: ClueBoard,
    #[serde(default = "open")]
    pub open: bool,
}

fn open() -> bool {
    true
}

impl KitClues {
    /// The notes top to bottom, as `ClueBoard::placed` orders and sizes them; a shut board shows
    /// only the newest.
    pub fn notes(&self) -> Vec<PlacedNote<'_>> {
        let placed = self.board.placed();
        if self.open {
            return placed;
        }
        placed
            .into_iter()
            .filter(|placed| placed.look == NoteLook::Newest)
            .collect()
    }
}

/// The word that ends a note's class (`kit-note-newest`).
pub fn note_marker(look: NoteLook) -> &'static str {
    match look {
        NoteLook::Newest => "newest",
        NoteLook::Regular => "regular",
        NoteLook::Compact => "compact",
    }
}
