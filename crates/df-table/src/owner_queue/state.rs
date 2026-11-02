//! The records the owner queue keeps: what a waiting input is, what a seat holds, what an
//! operation has become, and where an input is placed.
//!
//! It owns plain data and the small steps that keep a seat's counts consistent. It never decides
//! admission or order: `owner_queue` does.

use crate::envelope::{EngineInput, InputEnvelope, PlayerInput, Sender};
use crate::identity::{OperationId, SeatId, SessionRevision};
use crate::outcome::OutcomeKind;

use super::counted::Fifo;
use super::{Lane, OperationStatus, TokenBucket};

/// Which seat membership an operation counts against, so an input that ends after its seat left
/// and came back does not change the new membership's count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SeatHold {
    pub(super) seat: SeatId,
    pub(super) membership: u64,
}

pub(super) struct Waiting {
    pub(super) envelope: InputEnvelope,
    pub(super) admitted_ms: u64,
    pub(super) holder: Option<SeatHold>,
    pub(super) free_text: bool,
}

pub(super) struct SeatState {
    pub(super) membership: u64,
    pub(super) lines: Fifo<Waiting>,
    pub(super) controls: Fifo<Waiting>,
    pub(super) lines_in_flight: usize,
    pub(super) free_text_in_flight: usize,
    pub(super) bucket: TokenBucket,
}

impl SeatState {
    /// What `seat_pending_limit` bounds: lines waiting plus lines in progress.
    pub(super) fn pending(&self) -> usize {
        self.lines.len().saturating_add(self.lines_in_flight)
    }

    /// A seat is served unless its next line is free text and another is still in INTERPRET.
    pub(super) fn lines_eligible(&self) -> bool {
        self.lines
            .front()
            .is_some_and(|next| !(next.free_text && self.free_text_in_flight > 0))
    }

    pub(super) fn take_line(&mut self) -> Option<Waiting> {
        let waiting = self.lines.pop_front()?;
        self.lines_in_flight = self.lines_in_flight.saturating_add(1);
        if waiting.free_text {
            self.free_text_in_flight = self.free_text_in_flight.saturating_add(1);
        }
        Some(waiting)
    }

    pub(super) fn release(&mut self, lane: Lane, free_text: bool) {
        if lane != Lane::Player {
            return;
        }
        self.lines_in_flight = self.lines_in_flight.saturating_sub(1);
        if free_text {
            self.free_text_in_flight = self.free_text_in_flight.saturating_sub(1);
        }
    }
}

pub(super) struct OperationRecord {
    pub(super) status: OperationStatus,
    pub(super) holder: Option<SeatHold>,
    pub(super) lane: Lane,
    pub(super) free_text: bool,
    pub(super) revision: SessionRevision,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct InProgressEntry {
    pub(super) key: OperationKey,
    pub(super) served_ms: u64,
    pub(super) revision: SessionRevision,
}

pub(super) struct FifoLane {
    pub(super) waiting: Fifo<Waiting>,
    pub(super) limit: usize,
}

impl FifoLane {
    pub(super) fn with_limit(limit: usize) -> Self {
        Self {
            waiting: Fifo::new(),
            limit,
        }
    }
}

/// What an operation is remembered under: its sender, the membership of that sender (zero for the
/// engine; a seat's membership changes each time it joins), and the id the sender gave it. The
/// membership keeps a seat that left and came back, or a different player who took its seat id,
/// from meeting the history of the one before.
pub(super) type OperationKey = (Sender, u64, OperationId);

/// How an in-progress operation ended.
#[derive(Debug, Clone, Copy)]
pub(super) enum Ending {
    Finished(OutcomeKind),
    Abandoned,
    /// Given up with nothing kept: the operation id is forgotten, so the same id may be sent again.
    Released,
}

/// Which engine lane an input joins.
#[derive(Debug, Clone, Copy)]
pub(super) enum EngineLane {
    Proposals,
    TimersAndResults,
}

impl EngineLane {
    pub(super) fn of(input: &EngineInput) -> Self {
        match input {
            EngineInput::Proposal { .. } | EngineInput::Control(_) => Self::Proposals,
            EngineInput::Timer { .. } | EngineInput::ResourceResult { .. } => {
                Self::TimersAndResults
            }
        }
    }

    pub(super) fn lane(self) -> Lane {
        match self {
            Self::Proposals => Lane::Engine,
            Self::TimersAndResults => Lane::TimersAndResults,
        }
    }
}

/// Where a seat's input goes.
#[derive(Debug, Clone, Copy)]
pub(super) enum PlayerRoute {
    Line { free_text: bool },
    Control,
}

impl PlayerRoute {
    pub(super) fn of(input: &PlayerInput) -> Self {
        match input {
            PlayerInput::SubmitText { .. } | PlayerInput::Say { .. } => {
                Self::Line { free_text: true }
            }
            PlayerInput::Tap { .. } | PlayerInput::PrivateMessage { .. } => {
                Self::Line { free_text: false }
            }
            PlayerInput::SafetyCard | PlayerInput::PendingAnswer { .. } => Self::Control,
        }
    }
}

/// Where an envelope is placed, decided before it is moved.
pub(super) enum Placement {
    Seat(SeatId, PlayerRoute),
    Engine(EngineLane),
}
