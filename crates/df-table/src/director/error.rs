//! Why the Director refused a request outright (O.14, G1).
//!
//! These are the caller's mistakes: a number outside its checked range, a request that names no
//! target, a clock that ran backwards. A request the Director understood and declined for a reason
//! of the table (a hidden target, a lower reason, a paused Director) is not an error: it is a
//! `Receipt::Dropped` and a line in the log.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::director::DirectorSetting;
use crate::director::reason::{
    FOCUS_MOVE_MAXIMUM_MS, FOCUS_MOVE_MINIMUM_MS, MAXIMUM_CAMERA_MS, MAXIMUM_ZOOM_PERCENT,
    MINIMUM_ZOOM_PERCENT,
};

/// A request or a setting the Director cannot take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectorError {
    ZoomOutOfRange {
        zoom_percent: u16,
    },
    /// A focus move that is neither a cut nor a time a person can follow.
    FocusMoveOutOfRange {
        over_ms: u64,
    },
    /// A pull-back longer than the camera's own limit.
    WideMoveOutOfRange {
        over_ms: u64,
    },
    /// A hold of no time, or longer than the clock can count.
    HoldOutOfRange {
        hold_ms: u64,
    },
    /// A focus that names no element.
    NoTargets,
    TooManyTargets {
        limit: usize,
    },
    /// The Director's list of waiting requests is full; it is emptied each host tick.
    TooManyWaiting {
        limit: usize,
    },
    /// The requests that wait name more elements in all than the Director will hold.
    TooManyWaitingTargets {
        limit: usize,
    },
    /// A call with a time earlier than one the Director already saw.
    ClockWentBack {
        now_ms: u64,
        last_ms: u64,
    },
    /// A setting outside its range.
    InvalidSetting {
        setting: DirectorSetting,
        value: u64,
    },
}

impl fmt::Display for DirectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZoomOutOfRange { zoom_percent } => write!(
                formatter,
                "a zoom is a whole number of percent from {MINIMUM_ZOOM_PERCENT} to \
                 {MAXIMUM_ZOOM_PERCENT} (200 is twice as large), not {zoom_percent}"
            ),
            Self::FocusMoveOutOfRange { over_ms } => write!(
                formatter,
                "a focus takes 0 (a cut) or {FOCUS_MOVE_MINIMUM_MS} to {FOCUS_MOVE_MAXIMUM_MS} ms \
                 so a person can follow it, not {over_ms}"
            ),
            Self::WideMoveOutOfRange { over_ms } => write!(
                formatter,
                "a pull-back takes 0 to {MAXIMUM_CAMERA_MS} ms, not {over_ms}"
            ),
            Self::HoldOutOfRange { hold_ms } => write!(
                formatter,
                "a hold is 1 to {MAXIMUM_CAMERA_MS} ms, not {hold_ms}"
            ),
            Self::NoTargets => formatter.write_str("a focus names at least one element"),
            Self::TooManyTargets { limit } => {
                write!(formatter, "one request names at most {limit} elements")
            }
            Self::TooManyWaiting { limit } => write!(
                formatter,
                "{limit} requests are already waiting for the next host tick"
            ),
            Self::TooManyWaitingTargets { limit } => write!(
                formatter,
                "the waiting requests already name {limit} elements, as many as are held"
            ),
            Self::ClockWentBack { now_ms, last_ms } => write!(
                formatter,
                "the clock read {now_ms} ms after it had read {last_ms} ms"
            ),
            Self::InvalidSetting { setting, value } => {
                let (minimum, maximum) = setting.range();
                write!(
                    formatter,
                    "the setting {} is {minimum} to {maximum}, not {value}",
                    setting.key()
                )
            }
        }
    }
}

impl std::error::Error for DirectorError {}
