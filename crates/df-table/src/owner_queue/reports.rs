//! What the owner queue tells its caller: the values `admit`, `serve_next`, `in_progress`,
//! `reap_expired` and `position` return.
//!
//! It owns the plain data only (`OperationStatus`, `ServedInput`, `InProgressOperation`,
//! `QueuePosition`). It never changes queue state.

use crate::envelope::{InputEnvelope, Sender};
use crate::identity::{Correlation, OperationId, TurnSeq};
use crate::outcome::OutcomeKind;

use super::Lane;

/// Where an operation stands. A duplicate submit is answered with this, and only to its own
/// sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    Queued {
        lane: Lane,
    },
    /// Served and not finished: the input is in the pipeline, perhaps in INTERPRET.
    InProgress {
        turn_seq: TurnSeq,
    },
    /// Finished; `turn_seq` names the turn whose record holds the full result.
    Finished {
        turn_seq: TurnSeq,
        outcome: OutcomeKind,
    },
    /// Given up without an outcome: its lease ran out, its seat left or the caller abandoned it.
    /// The turn may still commit, so the id is not free: a `finish` that arrives later is
    /// recorded once and moves this to `Finished`, and until then a duplicate hears `Abandoned`.
    /// The commit step checks the status before it applies (C3): an input that is `Abandoned`
    /// must not commit.
    Abandoned {
        turn_seq: TurnSeq,
    },
}

/// An input that was admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admitted {
    pub lane: Lane,
}

/// An input the owner now runs, with the place it took in the total order. The caller must end it
/// with `finish` (or the watchdog with `abandon`), or its seat stays busy until the lease runs out.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a served input stays in progress until it is finished or abandoned"]
pub struct ServedInput {
    pub envelope: InputEnvelope,
    pub turn_seq: TurnSeq,
    pub lane: Lane,
    /// From admission to service, in milliseconds.
    pub waited_ms: u64,
}

impl ServedInput {
    pub fn correlation(&self) -> Correlation {
        Correlation {
            operation_id: self.envelope.operation_id(),
            turn_seq: self.turn_seq,
            revision: self.envelope.view().revision(),
        }
    }
}

/// An input that was served and has not ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InProgressOperation {
    pub sender: Sender,
    /// The seat's membership when the input was served (zero for the engine).
    pub membership: u64,
    pub operation_id: OperationId,
    pub turn_seq: TurnSeq,
    /// Since it was served, in milliseconds.
    pub age_ms: u64,
}

/// Where a waiting input stands in the total order (K.6: a waiting phone shows its place).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuePosition {
    pub lane: Lane,
    /// How many inputs will be served before it if nothing new arrives. Exact while no seat is held
    /// behind its own free-text line; with a held seat it can overstate and never understates.
    pub ahead: usize,
    /// The input waits behind a free-text line of its own seat that is still in INTERPRET, so
    /// `ahead` is the place it takes once that line ends, not a promise of when.
    pub held_behind_own_text: bool,
}
