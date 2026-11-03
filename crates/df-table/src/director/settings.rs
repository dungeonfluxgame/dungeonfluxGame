//! The Director's numbers, in data: holds, spacing and the sizes it will accept (O.14, G1).

use crate::director_error::DirectorError;
use crate::director_log::DEFAULT_MAXIMUM_LOG_ENTRIES;
use crate::director_reason::{DEFAULT_WIDE_MS, MAXIMUM_CAMERA_MS, ReasonTable};

/// The default of `focus_hold_s` (8), in milliseconds.
pub const DEFAULT_FOCUS_HOLD_MS: u64 = 8_000;
/// At most one new move in this many milliseconds (O.14 camera rule 5).
pub const DEFAULT_SPACING_MS: u64 = 800;
/// A focus whose reason nobody ends returns after this long, so a zoom is never stuck: ten minutes.
pub const UNENDED_FOCUS_LIMIT_MS: u64 = 600_000;
/// The default of most elements one request may name (an area). Deliberately large.
pub const DEFAULT_MAXIMUM_TARGETS: usize = 100_000;
/// The default of most requests that may wait for the next host tick. Deliberately large.
pub const DEFAULT_MAXIMUM_WAITING: usize = 100_000;
/// The default of most element names that may wait in all, over every waiting request: the bound
/// on memory, which the two caps above do not give together. Deliberately large.
pub const DEFAULT_MAXIMUM_WAITING_TARGETS: usize = 1_000_000;

/// The Director's numbers, in data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectorSettings {
    pub reasons: ReasonTable,
    /// `focus_hold_s` in milliseconds: how long a moment-reason focus holds.
    pub focus_hold_ms: u64,
    pub spacing_ms: u64,
    /// How long a focus nobody ended may hold.
    pub unended_limit_ms: u64,
    /// How long a pull-back takes.
    pub wide_ms: u32,
    /// Most elements one request may name.
    pub maximum_targets: usize,
    /// Most requests that may wait for the next host tick.
    pub maximum_waiting: usize,
    /// Most element names that may wait in all.
    pub maximum_waiting_targets: usize,
    /// Most entries the log keeps; the oldest are forgotten first.
    pub maximum_log_entries: usize,
}

impl DirectorSettings {
    pub fn defaults() -> Self {
        Self {
            reasons: ReasonTable::defaults(),
            focus_hold_ms: DEFAULT_FOCUS_HOLD_MS,
            spacing_ms: DEFAULT_SPACING_MS,
            unended_limit_ms: UNENDED_FOCUS_LIMIT_MS,
            wide_ms: DEFAULT_WIDE_MS,
            maximum_targets: DEFAULT_MAXIMUM_TARGETS,
            maximum_waiting: DEFAULT_MAXIMUM_WAITING,
            maximum_waiting_targets: DEFAULT_MAXIMUM_WAITING_TARGETS,
            maximum_log_entries: DEFAULT_MAXIMUM_LOG_ENTRIES,
        }
    }

    pub(super) fn check(&self) -> Result<(), DirectorError> {
        let limit = u64::from(MAXIMUM_CAMERA_MS);
        for (name, value) in [
            ("focus_hold_ms", self.focus_hold_ms),
            ("unended_limit_ms", self.unended_limit_ms),
        ] {
            if !(1..=limit).contains(&value) {
                return Err(DirectorError::BadSetting { name, value });
            }
        }
        if self.spacing_ms > limit {
            return Err(DirectorError::BadSetting {
                name: "spacing_ms",
                value: self.spacing_ms,
            });
        }
        if self.wide_ms > MAXIMUM_CAMERA_MS {
            return Err(DirectorError::BadSetting {
                name: "wide_ms",
                value: u64::from(self.wide_ms),
            });
        }
        for (name, value) in [
            ("maximum_targets", self.maximum_targets),
            ("maximum_waiting", self.maximum_waiting),
            ("maximum_waiting_targets", self.maximum_waiting_targets),
            ("maximum_log_entries", self.maximum_log_entries),
        ] {
            if value == 0 {
                return Err(DirectorError::BadSetting { name, value: 0 });
            }
        }
        Ok(())
    }
}
