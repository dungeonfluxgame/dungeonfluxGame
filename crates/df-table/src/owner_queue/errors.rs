//! Why the owner queue refused something, as typed errors.
//!
//! One enum per call that can refuse (`admit`, `join_seat` and `leave_seat`, `serve_next`,
//! `finish`, building the queue), each with variants a caller can branch on and never parse. It
//! owns the mapping from an admission refusal to how it is recorded: a decline with its code
//! (C.1, K.6, RES40) or a refusal with its cause. It never decides when to refuse: `owner_queue`
//! does.

use std::fmt;

use crate::config::SettingId;
use crate::identity::SeatId;
use crate::outcome::DeclineCode;

use super::{Lane, NotDeclinedCause, OperationStatus};

/// Why `admit` refused an input. Each variant says what the caller does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitError {
    /// The seat is not at this table; the host's session and the table disagree.
    UnknownSeat { seat: SeatId },
    /// The same operation id was admitted before; nothing runs twice.
    Duplicate { first: OperationStatus },
    /// The seat already has `limit` inputs admitted and not finished (K.6, `table_busy`).
    SeatBusy { pending: usize, limit: usize },
    /// The seat's token bucket is empty; it holds a token again after `retry_after_ms`.
    RateLimited { retry_after_ms: u64 },
    /// An engine lane holds `limit` inputs; the producer retries later and nothing is declined.
    LaneStalled { lane: Lane, limit: usize },
}

/// How a refusal is recorded: as a decline with its code, or as a refusal with its cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReport {
    Declined(DeclineCode),
    NotDeclined(NotDeclinedCause),
}

impl AdmitError {
    /// The decline code a player is shown, where the plan has one: `table_busy` and `rate_limited`
    /// (C.1, K.6, RES40).
    pub fn decline_code(&self) -> Option<DeclineCode> {
        match self.report() {
            RefusalReport::Declined(code) => Some(code),
            RefusalReport::NotDeclined(_) => None,
        }
    }

    pub fn report(&self) -> RefusalReport {
        match self {
            Self::SeatBusy { .. } => RefusalReport::Declined(DeclineCode::TableBusy),
            Self::RateLimited { .. } => RefusalReport::Declined(DeclineCode::RateLimited),
            Self::UnknownSeat { .. } => RefusalReport::NotDeclined(NotDeclinedCause::UnknownSeat),
            Self::Duplicate { .. } => RefusalReport::NotDeclined(NotDeclinedCause::Duplicate),
            Self::LaneStalled { .. } => RefusalReport::NotDeclined(NotDeclinedCause::LaneStalled),
        }
    }
}

impl fmt::Display for AdmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSeat { seat } => write!(formatter, "seat {seat} is not at this table"),
            Self::Duplicate { .. } => formatter.write_str("that operation was already admitted"),
            Self::SeatBusy { pending, limit } => {
                write!(
                    formatter,
                    "the seat has {pending} of {limit} inputs pending"
                )
            }
            Self::RateLimited { retry_after_ms } => {
                write!(
                    formatter,
                    "the seat is sending too fast, retry in {retry_after_ms} ms"
                )
            }
            Self::LaneStalled { lane, limit } => {
                write!(formatter, "the {} lane holds {limit} inputs", lane.key())
            }
        }
    }
}

impl std::error::Error for AdmitError {}

/// Why the queue could not be built from the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueConfigError {
    /// A rate setting is zero, which would never refill or never admit.
    ZeroSetting(SettingId),
}

impl fmt::Display for QueueConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroSetting(setting) => write!(formatter, "the setting {setting:?} is zero"),
        }
    }
}

impl std::error::Error for QueueConfigError {}

/// Why a seat could not join or leave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatError {
    AlreadySeated { seat: SeatId },
    UnknownSeat { seat: SeatId },
    TooManySeats { limit: usize },
}

impl fmt::Display for SeatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadySeated { seat } => write!(formatter, "seat {seat} is already seated"),
            Self::UnknownSeat { seat } => write!(formatter, "seat {seat} is not at this table"),
            Self::TooManySeats { limit } => write!(formatter, "a table seats at most {limit}"),
        }
    }
}

impl std::error::Error for SeatError {}

/// Why `serve_next` could not serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeError {
    /// Every `turn_seq` of the table is used. The input stays queued.
    TurnSequenceExhausted,
}

impl fmt::Display for ServeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TurnSequenceExhausted => formatter.write_str("the turn sequence is exhausted"),
        }
    }
}

impl std::error::Error for ServeError {}

/// Why `finish` refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishError {
    /// No such operation: it was never admitted, was discarded, or is forgotten.
    UnknownOperation,
    /// The operation is queued or already finished, so it cannot finish now.
    NotInProgress { status: OperationStatus },
}

impl fmt::Display for FinishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOperation => formatter.write_str("that operation is not known"),
            Self::NotInProgress { .. } => formatter.write_str("that operation is not in progress"),
        }
    }
}

impl std::error::Error for FinishError {}
