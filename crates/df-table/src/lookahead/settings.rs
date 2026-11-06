//! The settings of the look-ahead worker (SB-LA, idea 1 of "Ideas to come").
//!
//! It owns `LookaheadSettings`: `lookahead_enabled`, `lookahead_depth` (1: only the certain next
//! scene), `lookahead_start_s` (how long after the current scene is Ready a merely likely scene
//! waits before it is built, so it never competes with the current scene's own plate),
//! `lookahead_deadline_s` (a build still out after this is cancelled, K.2: art is never a gate) and
//! `lookahead_book_age_s` (how long a built scene nobody took is kept for reuse). Each value
//! outside its range is refused with a typed error that names the setting, so the loader can fall
//! back to the default and record why (coding style section 8, pattern 9). The maximum is the
//! large number every setting range uses until a measurement scopes it down (owner, 2026-11-02).
//!
//! The plan's per-hour cost ceiling is not here: the owner's rule is that spend is recorded and
//! never blocks (2026-10-24), so look-ahead requests carry their own purposes and the cost view
//! shows what was spent on scenes that were not used, and nothing caps it.
//!
//! These settings are not yet entries of `config::SettingId`, like the five of `PlateTiming`.

use std::fmt;

use crate::limits::LARGE_LOOKAHEAD_MAXIMUM;
use crate::listed::listed_enum;
use crate::units::MILLISECONDS_PER_SECOND;

/// `lookahead_enabled` default: on.
pub(crate) const DEFAULT_LOOKAHEAD_ENABLED: bool = true;
/// `lookahead_depth` default: only the one certain next scene is built (the first version).
pub(crate) const DEFAULT_LOOKAHEAD_DEPTH: u32 = 1;
/// `lookahead_start_s` default: a likely scene waits this long after the current scene is Ready.
pub(crate) const DEFAULT_LOOKAHEAD_START_S: u32 = 10;
/// `lookahead_deadline_s` default: four times the establishing-card budget of a first scene, a
/// candidate to be tuned from the K.2 timings in the logs (the plan names none).
pub(crate) const DEFAULT_LOOKAHEAD_DEADLINE_S: u32 = 120;
/// `lookahead_book_age_s` default: a quarter of an hour.
pub(crate) const DEFAULT_LOOKAHEAD_BOOK_AGE_S: u32 = 900;

listed_enum! {
    /// The numeric settings `LookaheadSettings` holds.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum LookaheadSetting {
        Depth,
        StartS,
        DeadlineS,
        BookAgeS,
    }
}

impl LookaheadSetting {
    /// The name in the configuration file.
    pub fn key(self) -> &'static str {
        match self {
            Self::Depth => "lookahead_depth",
            Self::StartS => "lookahead_start_s",
            Self::DeadlineS => "lookahead_deadline_s",
            Self::BookAgeS => "lookahead_book_age_s",
        }
    }

    pub fn default_value(self) -> u32 {
        match self {
            Self::Depth => DEFAULT_LOOKAHEAD_DEPTH,
            Self::StartS => DEFAULT_LOOKAHEAD_START_S,
            Self::DeadlineS => DEFAULT_LOOKAHEAD_DEADLINE_S,
            Self::BookAgeS => DEFAULT_LOOKAHEAD_BOOK_AGE_S,
        }
    }

    /// The smallest value: a depth of zero builds nothing (turn it off instead), and a zero
    /// deadline or age would discard every build at once. A start of zero is allowed: it means
    /// "at once".
    fn minimum(self) -> u32 {
        match self {
            Self::StartS => 0,
            Self::Depth | Self::DeadlineS | Self::BookAgeS => 1,
        }
    }
}

/// Why a setting was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookaheadSettingError {
    pub setting: LookaheadSetting,
    pub value: u64,
}

impl fmt::Display for LookaheadSettingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} is {} to {LARGE_LOOKAHEAD_MAXIMUM}, not {}",
            self.setting.key(),
            self.setting.minimum(),
            self.value
        )
    }
}

impl std::error::Error for LookaheadSettingError {}

/// The look-ahead worker's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookaheadSettings {
    enabled: bool,
    depth: u32,
    start_s: u32,
    deadline_s: u32,
    book_age_s: u32,
}

impl Default for LookaheadSettings {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_LOOKAHEAD_ENABLED,
            depth: DEFAULT_LOOKAHEAD_DEPTH,
            start_s: DEFAULT_LOOKAHEAD_START_S,
            deadline_s: DEFAULT_LOOKAHEAD_DEADLINE_S,
            book_age_s: DEFAULT_LOOKAHEAD_BOOK_AGE_S,
        }
    }
}

impl LookaheadSettings {
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// The same settings with one number changed, or the error that names it.
    pub fn with(
        self,
        setting: LookaheadSetting,
        value: u64,
    ) -> Result<Self, LookaheadSettingError> {
        let checked = u32::try_from(value)
            .ok()
            .filter(|checked| (setting.minimum()..=LARGE_LOOKAHEAD_MAXIMUM).contains(checked))
            .ok_or(LookaheadSettingError { setting, value })?;
        let mut changed = self;
        match setting {
            LookaheadSetting::Depth => changed.depth = checked,
            LookaheadSetting::StartS => changed.start_s = checked,
            LookaheadSetting::DeadlineS => changed.deadline_s = checked,
            LookaheadSetting::BookAgeS => changed.book_age_s = checked,
        }
        Ok(changed)
    }

    pub fn enabled(self) -> bool {
        self.enabled
    }

    pub fn depth(self) -> usize {
        usize::try_from(self.depth).unwrap_or(usize::MAX)
    }

    pub fn start_ms(self) -> u64 {
        u64::from(self.start_s) * MILLISECONDS_PER_SECOND
    }

    pub fn deadline_ms(self) -> u64 {
        u64::from(self.deadline_s) * MILLISECONDS_PER_SECOND
    }

    pub fn book_age_ms(self) -> u64 {
        u64::from(self.book_age_s) * MILLISECONDS_PER_SECOND
    }

    /// The value of a numeric setting, in its own unit.
    pub fn value_of(self, setting: LookaheadSetting) -> u32 {
        match setting {
            LookaheadSetting::Depth => self.depth,
            LookaheadSetting::StartS => self.start_s,
            LookaheadSetting::DeadlineS => self.deadline_s,
            LookaheadSetting::BookAgeS => self.book_age_s,
        }
    }
}
