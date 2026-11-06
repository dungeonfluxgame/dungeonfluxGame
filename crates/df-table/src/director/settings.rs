//! The Director's numbers, in data: holds, spacing and the sizes it will accept (O.14, G1).

use serde::{Deserialize, Serialize};

use crate::director::error::DirectorError;
use crate::director::reason::{DEFAULT_WIDE_MS, MAXIMUM_CAMERA_MS, ReasonTable};
use crate::limits::{
    DEFAULT_MAXIMUM_LOG_ENTRIES, DEFAULT_MAXIMUM_TARGETS, DEFAULT_MAXIMUM_WAITING,
    DEFAULT_MAXIMUM_WAITING_TARGETS,
};
use crate::listed::listed_enum;

/// The default of `focus_hold_s` (8), in milliseconds.
pub(crate) const DEFAULT_FOCUS_HOLD_MS: u64 = 8_000;
/// At most one new move in this many milliseconds (O.14 camera rule 5).
pub(crate) const DEFAULT_SPACING_MS: u64 = 800;
/// A focus whose reason nobody ends returns after this long, so a zoom is never stuck: ten minutes.
pub(crate) const UNENDED_FOCUS_LIMIT_MS: u64 = 600_000;

listed_enum! { keyed
    /// The numbers `DirectorSettings` holds, named as `with` takes them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum DirectorSetting {
        /// `focus_hold_s` in milliseconds: how long a moment-reason focus holds.
        FocusHoldMs,
        /// At most one new move in this many milliseconds.
        SpacingMs,
        /// How long a focus nobody ended may hold.
        UnendedLimitMs,
        /// How long a pull-back takes.
        WideMs,
        /// Most elements one request may name.
        MaximumTargets,
        /// Most requests that may wait for the next host tick.
        MaximumWaiting,
        /// Most element names that may wait in all.
        MaximumWaitingTargets,
        /// Most entries the log keeps; the oldest are forgotten first.
        MaximumLogEntries,
    }
}

impl DirectorSetting {
    pub fn key(self) -> &'static str {
        match self {
            Self::FocusHoldMs => "focus_hold_ms",
            Self::SpacingMs => "spacing_ms",
            Self::UnendedLimitMs => "unended_limit_ms",
            Self::WideMs => "wide_ms",
            Self::MaximumTargets => "maximum_targets",
            Self::MaximumWaiting => "maximum_waiting",
            Self::MaximumWaitingTargets => "maximum_waiting_targets",
            Self::MaximumLogEntries => "maximum_log_entries",
        }
    }

    /// The values the setting accepts, both ends included. A hold of no time would end every focus
    /// at once and a size of zero would refuse everything; a spacing or a pull-back of zero is a
    /// cut.
    pub fn range(self) -> (u64, u64) {
        match self {
            Self::FocusHoldMs | Self::UnendedLimitMs => (1, MAXIMUM_CAMERA_MS),
            Self::SpacingMs | Self::WideMs => (0, MAXIMUM_CAMERA_MS),
            Self::MaximumTargets
            | Self::MaximumWaiting
            | Self::MaximumWaitingTargets
            | Self::MaximumLogEntries => (1, u64::MAX),
        }
    }
}

/// The Director's numbers, in data. Built from `Default` and changed one checked setting at a
/// time with `with`, so a value outside its range cannot be held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectorSettings {
    pub(super) reasons: ReasonTable,
    pub(super) focus_hold_ms: u64,
    pub(super) spacing_ms: u64,
    pub(super) unended_limit_ms: u64,
    pub(super) wide_ms: u64,
    pub(super) maximum_targets: usize,
    pub(super) maximum_waiting: usize,
    pub(super) maximum_waiting_targets: usize,
    pub(super) maximum_log_entries: usize,
}

impl Default for DirectorSettings {
    fn default() -> Self {
        Self {
            reasons: ReasonTable::default(),
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
}

impl DirectorSettings {
    /// The same settings with one number changed, or the error that names it.
    pub fn with(mut self, setting: DirectorSetting, value: u64) -> Result<Self, DirectorError> {
        let (minimum, maximum) = setting.range();
        if !(minimum..=maximum).contains(&value) {
            return Err(DirectorError::InvalidSetting { setting, value });
        }
        let size =
            || usize::try_from(value).map_err(|_| DirectorError::InvalidSetting { setting, value });
        match setting {
            DirectorSetting::FocusHoldMs => self.focus_hold_ms = value,
            DirectorSetting::SpacingMs => self.spacing_ms = value,
            DirectorSetting::UnendedLimitMs => self.unended_limit_ms = value,
            DirectorSetting::WideMs => self.wide_ms = value,
            DirectorSetting::MaximumTargets => self.maximum_targets = size()?,
            DirectorSetting::MaximumWaiting => self.maximum_waiting = size()?,
            DirectorSetting::MaximumWaitingTargets => self.maximum_waiting_targets = size()?,
            DirectorSetting::MaximumLogEntries => self.maximum_log_entries = size()?,
        }
        Ok(self)
    }

    /// The same settings with another table of reasons; every spec in it is checked already.
    pub fn with_reasons(mut self, reasons: ReasonTable) -> Self {
        self.reasons = reasons;
        self
    }
}
