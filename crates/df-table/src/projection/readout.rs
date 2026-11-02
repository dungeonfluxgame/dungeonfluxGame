//! The structured readout: the plain line a committed outcome gets before any narration (C.7).
//!
//! It owns `StructuredReadout`, a rendering of an outcome's typed fields (who, which action, how
//! it ended) into one plain sentence. It is a rendering of rules output and not an interpretation:
//! the words come from a fixed template and an exhaustive table of outcome phrases, never from
//! model text or from the player's own words, so it can stand alone when narration is late or
//! blank, and a flavour line that arrives later is appended after it and never replaces it. It
//! never carries a reason or a missing piece: a decline's text goes to the declined seat's own
//! view only.

use crate::outcome::OutcomeKind;
use crate::ports::ActionId;
use crate::projection::text::NonEmptyText;
use crate::table_state::DisplayName;

/// What one seat's committed action came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredReadout {
    who: DisplayName,
    action: ActionId,
    outcome: OutcomeKind,
}

impl StructuredReadout {
    pub fn new(who: DisplayName, action: ActionId, outcome: OutcomeKind) -> Self {
        Self {
            who,
            action,
            outcome,
        }
    }

    /// The sentence: "Pip chose attack: done." The action name has its underscores and hyphens
    /// read as spaces.
    pub fn text(&self) -> NonEmptyText {
        let action = self.action.as_str().replace(['_', '-'], " ");
        NonEmptyText::or_fallback(&format!(
            "{} chose {action}: {}.",
            self.who.as_str(),
            outcome_phrase(self.outcome)
        ))
    }
}

fn outcome_phrase(outcome: OutcomeKind) -> &'static str {
    match outcome {
        OutcomeKind::Applied => "done",
        OutcomeKind::NeedsClarification => "needs a clearer choice",
        OutcomeKind::Declined => "not possible right now",
        OutcomeKind::Pending => "waiting for an answer",
    }
}
