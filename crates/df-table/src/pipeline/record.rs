//! What a turn leaves behind: the `TurnRecord` and the structured `Readout` (O.6, C.7).
//!
//! `TurnRecord` is the replayable summary of one input: who sent it, how it ended, the dice drawn
//! in order, the effects committed and the state digests before and after. It carries no commit
//! identity: the host that groups inputs into one checkpoint commit stamps the batch commit id
//! when it releases the record (S1 decision, O.16). `Readout` is the line the table shows at once
//! after an outcome, built only from typed fields and the rules owner's decline text, never from
//! model text or player words, and it is never blank (H.15).
//!
//! Neither type decides anything: the stages fill them in. The readout is the fallback the
//! storyteller's flavour is appended to and never replaces (C.7).

use crate::effects::TableEffect;
use crate::envelope::{InputKind, PendingId, Sender};
use crate::identity::{Correlation, SeatId};
use crate::outcome::{DeclineCode, OutcomeKind};
use crate::pending::StoredDraw;
use crate::ports::{ActionId, NonActionKind, ResultFact};

/// What is shown when a readout would otherwise be blank. Only an action with no name could
/// reach it, which the registry forbids.
const BLANK_READOUT_FALLBACK: &str = "Nothing changed.";

/// The structured line for an outcome. It cannot be blank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readout(String);

impl Readout {
    pub(crate) fn new(text: String) -> Self {
        if text.trim().is_empty() {
            return Self(BLANK_READOUT_FALLBACK.to_owned());
        }
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How a seat's action reads in a sentence: the registry name with its separators as spaces.
pub(crate) fn action_words(action: &ActionId) -> String {
    action.as_str().replace(['_', '-'], " ")
}

/// The line for an applied outcome (C.7): who did what, then the typed facts of the result, from
/// typed fields only ("Seat 2: attack innkeeper done (slashing damage 7).").
pub(crate) fn applied_line(seat: SeatId, action: &ActionId, facts: &[ResultFact]) -> String {
    let mut line = format!("Seat {seat}: {} done", action_words(action));
    if let Some((last, earlier)) = facts.split_last() {
        line.push_str(" (");
        for fact in earlier {
            line.push_str(&format!(
                "{} {}, ",
                action_words_of(fact.label.as_str()),
                fact.amount
            ));
        }
        line.push_str(&format!(
            "{} {})",
            action_words_of(last.label.as_str()),
            last.amount
        ));
    }
    line.push('.');
    line
}

fn action_words_of(name: &str) -> String {
    name.replace(['_', '-'], " ")
}

/// The line for a line of text that is not an action and was routed to its handler (C.8).
pub(crate) fn routed_line(kind: NonActionKind) -> &'static str {
    match kind {
        NonActionKind::Question => "That is a question, so nothing was run. It goes to the story.",
        NonActionKind::Social => "That is conversation, so nothing was run. It goes to them.",
        NonActionKind::PlanOnly => "Noted for the table. Nothing was run.",
        NonActionKind::Meta => "That is a rules question, so nothing was run.",
        NonActionKind::Joke => "That is a joke, so nothing was run.",
    }
}

/// The replayable summary of one input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub correlation: Correlation,
    pub sender: Sender,
    pub kind: InputKind,
    pub outcome: OutcomeKind,
    /// The decline code when the input ended declined.
    pub declined: Option<DeclineCode>,
    /// The pending the input opened, or the pending it answered when that is still open.
    pub pending: Option<PendingId>,
    /// Every die drawn for this input, in order. A replay never draws again.
    pub draws: Vec<StoredDraw>,
    /// The effects committed, in order.
    pub effects: Vec<TableEffect>,
    pub state_hash_before: u64,
    pub state_hash_after: u64,
    /// The input had opened a pending before (a restart replays it) and nothing was drawn or run.
    pub replayed: bool,
}
