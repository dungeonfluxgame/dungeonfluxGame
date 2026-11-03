//! The timings of the scene builder and the plate scheduler (J.4, K.2, K.3, RES10).
//!
//! It owns `PlateTiming`: `plate_interval_s` (15), `scene_build_budget_s` (8), the first-scene
//! hold (20), `plate_swap_wait_s` (6) and `image_stale_limit_s` (90), each a J.4 default stated
//! once. A value outside its range is refused with a typed error naming the setting, so the
//! loader can fall back to the default and record why (coding style section 8, pattern 9). The
//! one minimum is a rule of the domain (a clock that ticks every zero seconds is not a clock); the
//! maximum is the large number every setting range uses until a measurement scopes it down
//! (owner, 2026-11-02). It reads no file and no clock.
//!
//! These settings are not yet entries of `config::SettingId`: that list belongs to the settings
//! owner, and these five join it when the launcher Settings page lists them.

use std::fmt;

use crate::listed::listed_enum;

/// The J.4 default of `plate_interval_s`.
pub const DEFAULT_PLATE_INTERVAL_S: u32 = 15;
/// The J.4 default of `scene_build_budget_s` (K.2).
pub const DEFAULT_SCENE_BUILD_BUDGET_S: u32 = 8;
/// J.4's "first-scene hold 20 s": how long the first scene of a session or an authored arrival
/// beat may be held while narration fills the wait (K.2).
pub const DEFAULT_FIRST_SCENE_HOLD_S: u32 = 20;
/// The J.4 default of `plate_swap_wait_s` (K.3).
pub const DEFAULT_PLATE_SWAP_WAIT_S: u32 = 6;
/// The J.4 default of `image_stale_limit_s` (RES10).
pub const DEFAULT_IMAGE_STALE_LIMIT_S: u32 = 90;
/// J.4's "plate dissolve minimum 1.5 s": no plate swap dissolves in less.
pub const PLATE_DISSOLVE_MINIMUM_MS: u64 = 1_500;
/// The upper end of every timing for now. Deliberately huge, so it never limits play (owner,
/// 2026-11-02).
pub const LARGE_TIMING_MAXIMUM_S: u32 = 1_000_000;

const SECONDS_PER_HOUR: u32 = 3_600;
const MILLISECONDS_PER_SECOND: u64 = 1_000;

listed_enum! {
    /// The settings `PlateTiming` holds.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum TimingSetting {
        PlateIntervalS,
        SceneBuildBudgetS,
        FirstSceneHoldS,
        PlateSwapWaitS,
        ImageStaleLimitS,
    }
}

impl TimingSetting {
    /// The name in the configuration file.
    pub fn name(self) -> &'static str {
        match self {
            Self::PlateIntervalS => "plate_interval_s",
            Self::SceneBuildBudgetS => "scene_build_budget_s",
            Self::FirstSceneHoldS => "first_scene_hold_s",
            Self::PlateSwapWaitS => "plate_swap_wait_s",
            Self::ImageStaleLimitS => "image_stale_limit_s",
        }
    }

    pub fn default_s(self) -> u32 {
        match self {
            Self::PlateIntervalS => DEFAULT_PLATE_INTERVAL_S,
            Self::SceneBuildBudgetS => DEFAULT_SCENE_BUILD_BUDGET_S,
            Self::FirstSceneHoldS => DEFAULT_FIRST_SCENE_HOLD_S,
            Self::PlateSwapWaitS => DEFAULT_PLATE_SWAP_WAIT_S,
            Self::ImageStaleLimitS => DEFAULT_IMAGE_STALE_LIMIT_S,
        }
    }
}

/// How a scene is being built, which sets how long the table may wait for its art (K.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneBuildKind {
    /// Any scene after the first: play continues on the establishing card after the budget.
    Standard,
    /// The first scene of a session or an authored arrival beat, which may be held up to the
    /// first-scene hold.
    FirstScene,
}

impl SceneBuildKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::FirstScene => "first_scene",
        }
    }
}

/// Why a timing was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingError {
    pub setting: TimingSetting,
    pub value: u64,
}

impl fmt::Display for TimingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} is 1 to {LARGE_TIMING_MAXIMUM_S} seconds, not {}",
            self.setting.name(),
            self.value
        )
    }
}

impl std::error::Error for TimingError {}

/// The timings, each in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlateTiming {
    plate_interval_s: u32,
    scene_build_budget_s: u32,
    first_scene_hold_s: u32,
    plate_swap_wait_s: u32,
    image_stale_limit_s: u32,
}

impl Default for PlateTiming {
    fn default() -> Self {
        Self::defaults()
    }
}

impl PlateTiming {
    /// Every J.4 default.
    pub fn defaults() -> Self {
        Self {
            plate_interval_s: DEFAULT_PLATE_INTERVAL_S,
            scene_build_budget_s: DEFAULT_SCENE_BUILD_BUDGET_S,
            first_scene_hold_s: DEFAULT_FIRST_SCENE_HOLD_S,
            plate_swap_wait_s: DEFAULT_PLATE_SWAP_WAIT_S,
            image_stale_limit_s: DEFAULT_IMAGE_STALE_LIMIT_S,
        }
    }

    /// The same timings with one setting changed, or the error that names it.
    pub fn with(self, setting: TimingSetting, value: u64) -> Result<Self, TimingError> {
        let seconds = u32::try_from(value)
            .ok()
            .filter(|seconds| (1..=LARGE_TIMING_MAXIMUM_S).contains(seconds))
            .ok_or(TimingError { setting, value })?;
        let mut changed = self;
        match setting {
            TimingSetting::PlateIntervalS => changed.plate_interval_s = seconds,
            TimingSetting::SceneBuildBudgetS => changed.scene_build_budget_s = seconds,
            TimingSetting::FirstSceneHoldS => changed.first_scene_hold_s = seconds,
            TimingSetting::PlateSwapWaitS => changed.plate_swap_wait_s = seconds,
            TimingSetting::ImageStaleLimitS => changed.image_stale_limit_s = seconds,
        }
        Ok(changed)
    }

    pub fn plate_interval_s(self) -> u32 {
        self.plate_interval_s
    }

    pub fn plate_interval_ms(self) -> u64 {
        u64::from(self.plate_interval_s) * MILLISECONDS_PER_SECOND
    }

    /// How long the table waits for a scene's art before it plays on the establishing card. The
    /// first scene may be held longer, but never less than the standard budget.
    pub fn build_budget_ms(self, kind: SceneBuildKind) -> u64 {
        let seconds = match kind {
            SceneBuildKind::Standard => self.scene_build_budget_s,
            SceneBuildKind::FirstScene => self.first_scene_hold_s.max(self.scene_build_budget_s),
        };
        u64::from(seconds) * MILLISECONDS_PER_SECOND
    }

    pub fn swap_wait_ms(self) -> u64 {
        u64::from(self.plate_swap_wait_s) * MILLISECONDS_PER_SECOND
    }

    pub fn image_stale_limit_ms(self) -> u64 {
        u64::from(self.image_stale_limit_s) * MILLISECONDS_PER_SECOND
    }

    /// The most plates an hour can need at this interval, rounded up (RES9: the cost figure is an
    /// upper bound, because a quiet table pays for none).
    pub fn plates_per_hour_at_most(self) -> u32 {
        SECONDS_PER_HOUR.div_ceil(self.plate_interval_s)
    }
}
