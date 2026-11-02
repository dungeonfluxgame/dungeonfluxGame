//! Typed settings: each tunable number is one `SettingSpec` with its default stated once.
//!
//! It owns the specs (name, owner, default, range, unit, cost effect: O.5, RES50, J.4), the typed
//! `Settings` the rest of the crate reads, and load-time validation: a missing, malformed or
//! out-of-range value falls back to that setting's default and comes back as a
//! `TableEvent::SettingRejected`, never as a panic or a silent change (coding-style section 8,
//! pattern 9). It never reads a file: the host parses the central configuration file and hands the
//! values in. Every default is the J.4 number.
//!
//! Three names are C0 working names awaiting owner review, because J.4 describes those settings in
//! words only ("per-seat pending 3", "rate limit per seat 3 a second burst, 1 sustained"):
//! `seat_pending_limit`, `seat_rate_burst` and `seat_rate_sustained_per_s`. J.4 gives no ranges
//! either, so every maximum is the large `LARGE_SETTING_MAXIMUM` to be scoped down later (owner,
//! 2026-11-02). The one cross-setting rule is the one J.4 states: `narration_late_s` is the H.5
//! budget plus `narration_batch_s`, so it is never below it.
//!
//! Adding a setting is one `SettingId` variant, its spec arm, and a typed getter; the exhaustive
//! `match`es and the generated `SettingId::ALL` list stop it compiling half-added.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::element::name_type;
use crate::event::TableEvent;
use crate::listed::listed_enum;

name_type!(
    /// The name of one setting, as it appears in the configuration file.
    SettingName
);

/// Who may change a setting (RES50). The per-seat preference owner joins when the first per-seat
/// setting does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingOwner {
    /// The host's launcher Settings.
    LauncherSettings,
}

/// The unit of a setting's value; the name of the setting carries the same unit (RES50).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingUnit {
    Count,
    Seconds,
    PerSecond,
}

/// What raising a setting does to cost (the last column of J.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostEffect {
    None,
    /// A larger value means fewer narration calls.
    FewerNarrationCalls,
    /// A larger value allows more model calls.
    BoundsModelCalls,
}

/// The values a setting accepts, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingRange {
    pub minimum: u32,
    pub maximum: u32,
}

impl SettingRange {
    pub fn contains(self, value: u32) -> bool {
        (self.minimum..=self.maximum).contains(&value)
    }
}

/// The definition of one setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingSpec {
    pub name: &'static str,
    pub owner: SettingOwner,
    pub default: u32,
    pub range: SettingRange,
    pub unit: SettingUnit,
    pub cost_effect: CostEffect,
}

/// Why a configuration value was not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingFault {
    Missing,
    NotAWholeNumber,
    OutOfRange,
    UnknownSetting,
    /// The value contradicts a related setting, so both were put back to their defaults.
    #[serde(rename = "inconsistent_with_related_setting")]
    InconsistentWith,
}

impl SettingFault {
    pub fn key(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::NotAWholeNumber => "not_a_whole_number",
            Self::OutOfRange => "out_of_range",
            Self::UnknownSetting => "unknown_setting",
            Self::InconsistentWith => "inconsistent_with_related_setting",
        }
    }
}

impl SettingSpec {
    fn check(&self, value: &Value) -> Result<u32, SettingFault> {
        let whole_number = value.as_u64().ok_or(SettingFault::NotAWholeNumber)?;
        let value = u32::try_from(whole_number).map_err(|_| SettingFault::OutOfRange)?;
        if !self.range.contains(value) {
            return Err(SettingFault::OutOfRange);
        }
        Ok(value)
    }
}

/// The upper end of every range for now. Deliberately huge so it never limits play; scope each
/// setting down on its own later (owner, 2026-11-02). The plan gives defaults and no ranges.
pub const LARGE_SETTING_MAXIMUM: u32 = 1_000_000;

const NARRATION_LATE_S: SettingSpec = SettingSpec {
    name: "narration_late_s",
    owner: SettingOwner::LauncherSettings,
    default: 8,
    range: SettingRange {
        minimum: 1,
        maximum: LARGE_SETTING_MAXIMUM,
    },
    unit: SettingUnit::Seconds,
    cost_effect: CostEffect::None,
};

const NARRATION_BATCH_S: SettingSpec = SettingSpec {
    name: "narration_batch_s",
    owner: SettingOwner::LauncherSettings,
    default: 4,
    range: SettingRange {
        minimum: 1,
        maximum: LARGE_SETTING_MAXIMUM,
    },
    unit: SettingUnit::Seconds,
    cost_effect: CostEffect::FewerNarrationCalls,
};

const SEAT_PENDING_LIMIT: SettingSpec = SettingSpec {
    name: "seat_pending_limit",
    owner: SettingOwner::LauncherSettings,
    default: 3,
    range: SettingRange {
        minimum: 1,
        maximum: LARGE_SETTING_MAXIMUM,
    },
    unit: SettingUnit::Count,
    cost_effect: CostEffect::BoundsModelCalls,
};

const SEAT_RATE_BURST: SettingSpec = SettingSpec {
    name: "seat_rate_burst",
    owner: SettingOwner::LauncherSettings,
    default: 3,
    range: SettingRange {
        minimum: 1,
        maximum: LARGE_SETTING_MAXIMUM,
    },
    unit: SettingUnit::Count,
    cost_effect: CostEffect::BoundsModelCalls,
};

const SEAT_RATE_SUSTAINED_PER_S: SettingSpec = SettingSpec {
    name: "seat_rate_sustained_per_s",
    owner: SettingOwner::LauncherSettings,
    default: 1,
    range: SettingRange {
        minimum: 1,
        maximum: LARGE_SETTING_MAXIMUM,
    },
    unit: SettingUnit::PerSecond,
    cost_effect: CostEffect::BoundsModelCalls,
};

listed_enum! {
    /// The settings the first steps of the build read (O.9 C0 to C3).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum SettingId {
        NarrationLateS,
        NarrationBatchS,
        SeatPendingLimit,
        SeatRateBurst,
        SeatRateSustainedPerS,
    }
}

impl SettingId {
    pub fn spec(self) -> &'static SettingSpec {
        match self {
            Self::NarrationLateS => &NARRATION_LATE_S,
            Self::NarrationBatchS => &NARRATION_BATCH_S,
            Self::SeatPendingLimit => &SEAT_PENDING_LIMIT,
            Self::SeatRateBurst => &SEAT_RATE_BURST,
            Self::SeatRateSustainedPerS => &SEAT_RATE_SUSTAINED_PER_S,
        }
    }

    fn name(self) -> Result<SettingName, ConfigError> {
        SettingName::new(self.spec().name).map_err(|_| ConfigError::InvalidSpecName(self))
    }

    /// The place of this setting in `ALL` and in `Settings::values`: variants are fieldless and
    /// `ALL` lists them in declaration order, so the discriminant is always below `ALL.len()`.
    fn index(self) -> usize {
        self as usize
    }
}

/// The settings in force, and what loading them said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSettings {
    pub settings: Settings,
    /// One event for every value that was not used, in the order the specs list them, then one
    /// for every unknown name in the file, then one for the keys that are not even valid names.
    pub events: Vec<TableEvent>,
}

/// Loading could not even start: a spec of this crate has a name the name rule refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    InvalidSpecName(SettingId),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpecName(id) => {
                write!(formatter, "the setting {id:?} has a name that is not valid")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Every setting as a typed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    values: [u32; SettingId::ALL.len()],
}

impl Settings {
    /// Every setting at its default.
    pub fn defaults() -> Self {
        let mut values = [0; SettingId::ALL.len()];
        for id in SettingId::ALL {
            values[id.index()] = id.spec().default;
        }
        Self { values }
    }

    /// Reads the settings from a parsed configuration object. Every setting whose value is
    /// missing, not a whole number or outside its range keeps its default and is reported. If
    /// `narration_late_s` ends below `narration_batch_s` (J.4 defines it as that budget plus the
    /// batch window), both go back to their defaults and both are reported.
    pub fn load(configuration: &Map<String, Value>) -> Result<LoadedSettings, ConfigError> {
        let mut settings = Self::defaults();
        let mut events = Vec::new();
        for id in SettingId::ALL {
            let spec = id.spec();
            let checked = match configuration.get(spec.name) {
                Some(value) => spec.check(value),
                None => Err(SettingFault::Missing),
            };
            match checked {
                Ok(value) => settings.values[id.index()] = value,
                Err(cause) => events.push(TableEvent::SettingRejected {
                    name: id.name()?,
                    cause,
                }),
            }
        }
        settings.enforce_late_is_not_below_batch(&mut events)?;
        let mut unnameable_keys = 0;
        for key in configuration.keys() {
            if SettingId::ALL.iter().any(|id| id.spec().name == key) {
                continue;
            }
            match SettingName::new(key) {
                Ok(name) => events.push(TableEvent::SettingRejected {
                    name,
                    cause: SettingFault::UnknownSetting,
                }),
                Err(_) => unnameable_keys += 1,
            }
        }
        if unnameable_keys > 0 {
            events.push(TableEvent::SettingKeysIgnored {
                count: unnameable_keys,
            });
        }
        Ok(LoadedSettings { settings, events })
    }

    fn enforce_late_is_not_below_batch(
        &mut self,
        events: &mut Vec<TableEvent>,
    ) -> Result<(), ConfigError> {
        if self.narration_late_s() >= self.narration_batch_s() {
            return Ok(());
        }
        for id in [SettingId::NarrationLateS, SettingId::NarrationBatchS] {
            self.values[id.index()] = id.spec().default;
            events.push(TableEvent::SettingRejected {
                name: id.name()?,
                cause: SettingFault::InconsistentWith,
            });
        }
        Ok(())
    }

    pub fn get(&self, id: SettingId) -> u32 {
        self.values[id.index()]
    }

    /// How long narration may run late before the structured readout stands alone, in seconds.
    pub fn narration_late_s(&self) -> u32 {
        self.get(SettingId::NarrationLateS)
    }

    /// How long inputs from different seats share one narration call, in seconds.
    pub fn narration_batch_s(&self) -> u32 {
        self.get(SettingId::NarrationBatchS)
    }

    /// How many inputs one seat may have waiting before `table_busy`.
    pub fn seat_pending_limit(&self) -> u32 {
        self.get(SettingId::SeatPendingLimit)
    }

    /// How many inputs one seat may send in a burst.
    pub fn seat_rate_burst(&self) -> u32 {
        self.get(SettingId::SeatRateBurst)
    }

    /// How many inputs per second one seat may sustain.
    pub fn seat_rate_sustained_per_s(&self) -> u32 {
        self.get(SettingId::SeatRateSustainedPerS)
    }
}
