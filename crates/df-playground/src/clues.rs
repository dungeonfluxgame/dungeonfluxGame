//! The clue board on the shared screen: every clue found so far, as small notes the table can read
//! at a glance.
//!
//! A note names a thing the party found, says what was found of it (what a close look shows, or,
//! once it is worked out, what it was worked out to mean) and may carry a picture of it from the
//! media library. The host sets the whole board; the page renders it and decides nothing about
//! what a clue means. This module holds the data, its range checks and how the board is laid out
//! (which note is the newest, which are compressed); `view::clues` draws it.

use serde::{Deserialize, Serialize};

use crate::bubble_place::Rect;
use crate::checks;
use crate::limits::{CLUE_NOTES_BEFORE_COMPACT, MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_CLUE_NOTES};
use crate::scene::SceneError;

/// One clue on the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClueNote {
    /// Names the clue across changes of the board, so the page can tell a new note from one it
    /// has shown already.
    pub id: String,
    /// A few words naming the thing.
    pub name: String,
    /// What was found of it: what a close look shows, or, once worked out, what it means.
    pub detail: String,
    /// Whether the party has worked the clue out.
    #[serde(default)]
    pub worked_out: bool,
    /// A picture of the thing from the media library, usually its close-up.
    #[serde(default)]
    pub picture: Option<String>,
}

/// The clues found so far, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClueBoard {
    pub notes: Vec<ClueNote>,
}

/// How much of a note is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteLook {
    /// The newest clue: picture, name, status and what was found.
    Newest,
    /// An older clue: picture, name and status.
    Regular,
    /// An older clue on a crowded board: a smaller picture, name and status.
    Compact,
}

/// A note and how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedNote<'a> {
    pub note: &'a ClueNote,
    pub look: NoteLook,
}

impl ClueBoard {
    /// The board's notes top to bottom: the newest first, then the older ones, compressed once the
    /// board holds more than `CLUE_NOTES_BEFORE_COMPACT`.
    pub fn placed(&self) -> Vec<PlacedNote<'_>> {
        let crowded = self.notes.len() > CLUE_NOTES_BEFORE_COMPACT;
        self.notes
            .iter()
            .enumerate()
            .rev()
            .map(|(index, note)| {
                let look = if index + 1 == self.notes.len() {
                    NoteLook::Newest
                } else if crowded {
                    NoteLook::Compact
                } else {
                    NoteLook::Regular
                };
                PlacedNote { note, look }
            })
            .collect()
    }

    /// The board's timeline id: it changes when a note comes, goes, is worked out or is worded
    /// differently, and never carries the words.
    pub fn board_identity(&self) -> String {
        let mut hash = 0x811c_9dc5_u32;
        for note in &self.notes {
            let picture = note.picture.as_deref().unwrap_or("");
            let parts = [
                note.id.as_bytes(),
                note.name.as_bytes(),
                note.detail.as_bytes(),
                picture.as_bytes(),
                &[u8::from(note.worked_out)],
            ];
            for part in parts {
                for byte in part.iter().chain(&[0xff]) {
                    hash = (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
                }
            }
        }
        format!("clues-{}-{hash:08x}", self.notes.len())
    }

    /// Checks the board the way a live change and a scene file are checked.
    pub fn check(&self) -> Result<(), SceneError> {
        if self.notes.is_empty() {
            return Err(SceneError::Empty("the clue board"));
        }
        if self.notes.len() > MAXIMUM_CLUE_NOTES {
            return Err(SceneError::TooMany {
                what: "the clue board",
                limit: MAXIMUM_CLUE_NOTES,
            });
        }
        for (index, note) in self.notes.iter().enumerate() {
            checks::labelled("a clue's id", &note.id)?;
            checks::labelled("a clue's name", &note.name)?;
            if note.detail.chars().count() > MAXIMUM_CAPTION_CHARACTERS {
                return Err(SceneError::TooLong {
                    what: "a clue's detail",
                    limit: MAXIMUM_CAPTION_CHARACTERS,
                });
            }
            if let Some(picture) = &note.picture {
                checks::labelled("a media name", picture)?;
            }
            if self.notes[..index].iter().any(|other| other.id == note.id) {
                return Err(SceneError::DuplicateClue(note.id.clone()));
            }
        }
        Ok(())
    }
}

/// Whether the board is open (large, for a while after it changes) or shut (a slim strip).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Open,
    Shut,
}

impl Phase {
    /// The end of the timeline id of the record that follows the board in this phase.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Shut => "shut",
        }
    }
}

/// What a look at the board found compared with the one before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// There was no board and there is none.
    Absent,
    /// The board went away.
    Cleared,
    /// The same board is drawn again: its phase and its clock carry on.
    Continuing,
    /// A new board or a changed one: it opens, and its clock starts.
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    key: String,
    since_ms: u64,
    phase: Phase,
}

/// What the page remembers of the board between draws, so a redraw that did not change it does not
/// open it again or restart its clock.
#[derive(Debug, Default)]
pub struct BoardState {
    shown: Option<Shown>,
}

impl BoardState {
    /// Notes the board now drawn (`key` is `ClueBoard::board_identity`, or `None` for no board).
    pub fn observe(&mut self, key: Option<String>, now_ms: u64) -> Observed {
        match (key, &self.shown) {
            (None, None) => Observed::Absent,
            (None, Some(_)) => {
                self.shown = None;
                Observed::Cleared
            }
            (Some(key), Some(shown)) if shown.key == key => Observed::Continuing,
            (Some(key), _) => {
                self.shown = Some(Shown {
                    key,
                    since_ms: now_ms,
                    phase: Phase::Open,
                });
                Observed::Changed
            }
        }
    }

    pub fn phase(&self) -> Option<Phase> {
        self.shown.as_ref().map(|shown| shown.phase)
    }

    /// The key of the board shown.
    pub fn key(&self) -> Option<&str> {
        self.shown.as_ref().map(|shown| shown.key.as_str())
    }

    /// How long the board has been open at `now_ms`, or 0 when it is shut or absent.
    pub fn open_for_ms(&self, now_ms: u64) -> u64 {
        match &self.shown {
            Some(shown) if shown.phase == Phase::Open => now_ms.saturating_sub(shown.since_ms),
            _ => 0,
        }
    }

    /// Shuts an open board. Returns whether it was open.
    pub fn shut(&mut self) -> bool {
        match &mut self.shown {
            Some(shown) if shown.phase == Phase::Open => {
                shown.phase = Phase::Shut;
                true
            }
            _ => false,
        }
    }
}

/// Whether the board, at `board` on the stage, covers any of `faces` (the people of the plate) or
/// `boxes` (the narration, the caption bar, the speech bubbles): then it fades out instead
/// of hiding them.
pub fn board_covers(board: &Rect, faces: &[Rect], boxes: &[Rect]) -> bool {
    faces
        .iter()
        .chain(boxes)
        .any(|other| board.overlap_area(other) > 0.0)
}
