//! The settings' report: each setting with its value in force, default, range, unit, owner and
//! cost effect, generated from the setting specs so it cannot drift from them (O.5, J.4).
//!
//! Settings are the table's published configuration and hide nothing, so every audience sees them
//! all. The closed keys of the config enums are written as their names. It never changes a
//! setting.

use serde::Serialize;

use crate::config::{CostEffect, SettingId, SettingOwner, SettingUnit, Settings};
use crate::inspect::{Inspect, InspectView};

/// One setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingLine {
    pub name: &'static str,
    pub value: u32,
    pub default: u32,
    pub minimum: u32,
    pub maximum: u32,
    /// `count`, `seconds` or `per_second`.
    pub unit: &'static str,
    /// `launcher_settings`.
    pub owner: &'static str,
    /// `none`, `fewer_narration_calls` or `bounds_model_calls`.
    pub cost_effect: &'static str,
}

/// Every setting in the order the specs list them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsReport {
    pub settings: Vec<SettingLine>,
}

fn unit_key(unit: SettingUnit) -> &'static str {
    match unit {
        SettingUnit::Count => "count",
        SettingUnit::Seconds => "seconds",
        SettingUnit::PerSecond => "per_second",
    }
}

fn owner_key(owner: SettingOwner) -> &'static str {
    match owner {
        SettingOwner::LauncherSettings => "launcher_settings",
    }
}

fn cost_key(effect: CostEffect) -> &'static str {
    match effect {
        CostEffect::None => "none",
        CostEffect::FewerNarrationCalls => "fewer_narration_calls",
        CostEffect::BoundsModelCalls => "bounds_model_calls",
    }
}

impl Inspect for Settings {
    type Report = SettingsReport;

    fn inspect(&self, _view: InspectView) -> SettingsReport {
        SettingsReport {
            settings: SettingId::ALL
                .iter()
                .map(|id| {
                    let spec = id.spec();
                    SettingLine {
                        name: spec.name,
                        value: self.get(*id),
                        default: spec.default,
                        minimum: spec.range.minimum,
                        maximum: spec.range.maximum,
                        unit: unit_key(spec.unit),
                        owner: owner_key(spec.owner),
                        cost_effect: cost_key(spec.cost_effect),
                    }
                })
                .collect(),
        }
    }
}
