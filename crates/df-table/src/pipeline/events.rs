//! The typed events of the pipeline and of pending resolutions that are not already a variant of
//! `TableEvent` (O.6).
//!
//! It owns `PipelineEvent`, carried by `TableEvent::Pipeline` so the pipeline's vocabulary has one
//! home and `event.rs` keeps to its size: a clarification asked for, dice drawn, narration
//! attached, a pending opened, answered, resolved, nudged, held or reacquired, and a ruling
//! stored, reused or revoked. Every field is an `EventField` (ids, counts, closed keys), so no
//! player text, narration or secret can reach one. Every variant is class A and files under its
//! own audit area, and the rendering is an exhaustive `match`, so a variant added without its
//! line does not compile. It never decides what to record: the stage that changes state does.

use std::fmt::{self, Write};

use serde::{Deserialize, Serialize};

use crate::envelope::{CancelCause, PendingId};
use crate::event::{EventClass, write_context};
use crate::event_field::LineWriter;
use crate::identity::{Correlation, SeatId, TableScope};
use crate::listed::listed_enum;
use crate::outcome::PendingKind;
use crate::pending::RulingSource;
use crate::ports::{NonActionKind, RulingKey, RulingTier};

listed_enum! {
    /// Why the table asks a seat to pick instead of acting (C.5, C.7).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ClarifyCause {
        /// The reading fits more than one legal choice.
        Ambiguous,
        /// The interpreter could not tell what was meant.
        Unclear,
        /// The interpreter failed twice, so the buttons stand in for it and the draft is kept.
        Unreadable,
        /// A plan stopped at a step the rules owner declined after the steps before it were
        /// applied: the one question about the first step that failed (C.2, RES26).
        StepDeclined,
    }
}

impl ClarifyCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::Ambiguous => "ambiguous",
            Self::Unclear => "unclear",
            Self::Unreadable => "unreadable",
            Self::StepDeclined => "step_declined",
        }
    }
}

listed_enum! {
    /// Which part of the views a turn could not build (PROJECT).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ViewPart {
        /// A readout, a notice, the story or the activity was refused by the attachments.
        Publish,
        /// A seat's buttons were refused by the attachments.
        Offers,
        /// An audience's projection could not be built.
        Projection,
        /// What the running machine says for itself (its line, its standing buttons) could not be
        /// read: its saved state is corrupt or the registry does not hold it.
        Machine,
    }
}

impl ViewPart {
    pub fn key(self) -> &'static str {
        match self {
            Self::Publish => "publish",
            Self::Offers => "offers",
            Self::Projection => "projection",
            Self::Machine => "machine",
        }
    }
}

/// One thing the pipeline did that explains a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "what", rename_all = "snake_case", deny_unknown_fields)]
pub enum PipelineEvent {
    /// An input drawn from a view older than the seat's last change of legal choices was checked
    /// again against the table as it is (H.2).
    InputRechecked {
        scope: TableScope,
        correlation: Correlation,
    },
    /// The seat is asked to pick one of `options`.
    InputClarified {
        scope: TableScope,
        correlation: Correlation,
        options: usize,
        cause: ClarifyCause,
    },
    /// A plan's first `applied` steps were committed and the step after them was not.
    PlanPartlyApplied {
        scope: TableScope,
        correlation: Correlation,
        applied: usize,
        total: usize,
    },
    /// One die was drawn at the one draw point (H.3): `draw` is its place among the draws of this
    /// input, which with the correlation is its id, then the die and the face it showed. Dice are
    /// public in play, so the value belongs in the log.
    DiceDrawn {
        scope: TableScope,
        correlation: Correlation,
        draw: usize,
        sides: u16,
        value: u16,
    },
    /// The storyteller's text passed qualification and was appended to the structured readout.
    NarrationAttached {
        scope: TableScope,
        correlation: Correlation,
        ms: u64,
    },
    PendingOpened {
        scope: TableScope,
        correlation: Correlation,
        pending: PendingId,
        kind: PendingKind,
        offers: usize,
    },
    PendingAnswered {
        scope: TableScope,
        correlation: Correlation,
        pending: PendingId,
        seat: SeatId,
    },
    /// A pending was resolved; `ms` is how long it waited (the answer latency of O.6).
    PendingResolved {
        scope: TableScope,
        correlation: Correlation,
        pending: PendingId,
        kind: PendingKind,
        ms: u64,
    },
    /// A pending has waited past its nudge time; nothing was resolved or spent.
    PendingNudged {
        scope: TableScope,
        correlation: Correlation,
        pending: PendingId,
        kind: PendingKind,
        ms: u64,
    },
    /// A seat the pending waits on dropped, so its turn is held and nothing is spent.
    PendingHeld {
        scope: TableScope,
        correlation: Correlation,
        pending: PendingId,
        seat: SeatId,
    },
    /// The seat came back and the pending's offer is shown again by id.
    PendingReacquired {
        scope: TableScope,
        correlation: Correlation,
        pending: PendingId,
        seat: SeatId,
    },
    RulingStored {
        scope: TableScope,
        correlation: Correlation,
        key: RulingKey,
        tier: RulingTier,
        source: RulingSource,
    },
    /// A stored ruling answered a later question with an equal key and scope.
    RulingReused {
        scope: TableScope,
        correlation: Correlation,
        key: RulingKey,
        tier: RulingTier,
    },
    RulingRevoked {
        scope: TableScope,
        correlation: Correlation,
        key: RulingKey,
    },
    /// A pending ended without an answer and without spending anything (C.4).
    PendingCancelled {
        scope: TableScope,
        correlation: Correlation,
        pending: PendingId,
        kind: PendingKind,
        cause: CancelCause,
    },
    /// A line that is not an action was sent to the handler of its kind and nothing was run (C.8).
    InputRouted {
        scope: TableScope,
        correlation: Correlation,
        kind: NonActionKind,
    },
    /// Part of the views could not be built, so the table shows the last ones it had.
    ViewFailed {
        scope: TableScope,
        correlation: Correlation,
        part: ViewPart,
        seat: Option<SeatId>,
    },
}

impl PipelineEvent {
    /// Every pipeline event is class A: always recorded and never sampled.
    pub fn class(&self) -> EventClass {
        match self {
            Self::InputRechecked { .. }
            | Self::InputClarified { .. }
            | Self::PlanPartlyApplied { .. }
            | Self::DiceDrawn { .. }
            | Self::NarrationAttached { .. }
            | Self::PendingOpened { .. }
            | Self::PendingAnswered { .. }
            | Self::PendingResolved { .. }
            | Self::PendingNudged { .. }
            | Self::PendingHeld { .. }
            | Self::PendingReacquired { .. }
            | Self::RulingStored { .. }
            | Self::RulingReused { .. }
            | Self::RulingRevoked { .. }
            | Self::PendingCancelled { .. }
            | Self::InputRouted { .. }
            | Self::ViewFailed { .. } => EventClass::Always,
        }
    }

    pub fn area(&self) -> &'static str {
        match self {
            Self::InputRechecked { .. }
            | Self::InputClarified { .. }
            | Self::PlanPartlyApplied { .. }
            | Self::InputRouted { .. } => "INPUT",
            Self::DiceDrawn { .. } => "DICE",
            Self::NarrationAttached { .. } => "NARRATION",
            Self::ViewFailed { .. } => "PROJECT",
            Self::PendingOpened { .. }
            | Self::PendingAnswered { .. }
            | Self::PendingResolved { .. }
            | Self::PendingNudged { .. }
            | Self::PendingHeld { .. }
            | Self::PendingReacquired { .. }
            | Self::PendingCancelled { .. } => "PENDING",
            Self::RulingStored { .. } | Self::RulingReused { .. } | Self::RulingRevoked { .. } => {
                "RULING"
            }
        }
    }

    pub(crate) fn write_audit_body<W: Write>(&self, out: &mut W) -> fmt::Result {
        match self {
            Self::InputRechecked { scope, correlation } => {
                let mut line = LineWriter::start(out, "rechecked")?;
                write_context(&mut line, scope, correlation)
            }
            Self::InputClarified {
                scope,
                correlation,
                options,
                cause,
            } => {
                let mut line = LineWriter::start(out, "clarified")?;
                write_context(&mut line, scope, correlation)?;
                line.field("options", options)?;
                line.field("cause", cause)
            }
            Self::PlanPartlyApplied {
                scope,
                correlation,
                applied,
                total,
            } => {
                let mut line = LineWriter::start(out, "plan_partly_applied")?;
                write_context(&mut line, scope, correlation)?;
                line.field("applied", applied)?;
                line.field("total", total)
            }
            Self::DiceDrawn {
                scope,
                correlation,
                draw,
                sides,
                value,
            } => {
                let mut line = LineWriter::start(out, "drawn")?;
                write_context(&mut line, scope, correlation)?;
                line.field("draw", draw)?;
                line.field("sides", &u64::from(*sides))?;
                line.field("value", &u64::from(*value))
            }
            Self::NarrationAttached {
                scope,
                correlation,
                ms,
            } => {
                let mut line = LineWriter::start(out, "attached")?;
                write_context(&mut line, scope, correlation)?;
                line.field("ms", ms)
            }
            Self::PendingOpened {
                scope,
                correlation,
                pending,
                kind,
                offers,
            } => {
                let mut line = LineWriter::start(out, "opened")?;
                write_context(&mut line, scope, correlation)?;
                line.field("pending", pending)?;
                line.field("kind", kind)?;
                line.field("offers", offers)
            }
            Self::PendingAnswered {
                scope,
                correlation,
                pending,
                seat,
            } => {
                let mut line = LineWriter::start(out, "answered")?;
                write_context(&mut line, scope, correlation)?;
                line.field("pending", pending)?;
                line.field("seat", seat)
            }
            Self::PendingResolved {
                scope,
                correlation,
                pending,
                kind,
                ms,
            } => {
                let mut line = LineWriter::start(out, "resolved")?;
                write_context(&mut line, scope, correlation)?;
                line.field("pending", pending)?;
                line.field("kind", kind)?;
                line.field("ms", ms)
            }
            Self::PendingNudged {
                scope,
                correlation,
                pending,
                kind,
                ms,
            } => {
                let mut line = LineWriter::start(out, "nudged")?;
                write_context(&mut line, scope, correlation)?;
                line.field("pending", pending)?;
                line.field("kind", kind)?;
                line.field("ms", ms)
            }
            Self::PendingHeld {
                scope,
                correlation,
                pending,
                seat,
            } => {
                let mut line = LineWriter::start(out, "held")?;
                write_context(&mut line, scope, correlation)?;
                line.field("pending", pending)?;
                line.field("seat", seat)
            }
            Self::PendingReacquired {
                scope,
                correlation,
                pending,
                seat,
            } => {
                let mut line = LineWriter::start(out, "reacquired")?;
                write_context(&mut line, scope, correlation)?;
                line.field("pending", pending)?;
                line.field("seat", seat)
            }
            Self::RulingStored {
                scope,
                correlation,
                key,
                tier,
                source,
            } => {
                let mut line = LineWriter::start(out, "stored")?;
                write_context(&mut line, scope, correlation)?;
                line.field("key", key)?;
                line.field("tier", tier)?;
                line.field("source", source)
            }
            Self::RulingReused {
                scope,
                correlation,
                key,
                tier,
            } => {
                let mut line = LineWriter::start(out, "reused")?;
                write_context(&mut line, scope, correlation)?;
                line.field("key", key)?;
                line.field("tier", tier)
            }
            Self::RulingRevoked {
                scope,
                correlation,
                key,
            } => {
                let mut line = LineWriter::start(out, "revoked")?;
                write_context(&mut line, scope, correlation)?;
                line.field("key", key)
            }
            Self::PendingCancelled {
                scope,
                correlation,
                pending,
                kind,
                cause,
            } => {
                let mut line = LineWriter::start(out, "cancelled")?;
                write_context(&mut line, scope, correlation)?;
                line.field("pending", pending)?;
                line.field("kind", kind)?;
                line.field("cause", cause)
            }
            Self::InputRouted {
                scope,
                correlation,
                kind,
            } => {
                let mut line = LineWriter::start(out, "routed")?;
                write_context(&mut line, scope, correlation)?;
                line.field("kind", kind)
            }
            Self::ViewFailed {
                scope,
                correlation,
                part,
                seat,
            } => {
                let mut line = LineWriter::start(out, "view_failed")?;
                write_context(&mut line, scope, correlation)?;
                line.field("part", part)?;
                line.field_if_present("seat", seat)
            }
        }
    }
}
