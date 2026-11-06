//! The owner's event and settings verbs: `history`, `trace`, `replay` and `config`.
//!
//! `history` and `trace` read what the host's event sink kept (`crate::observe`), `trace` adds what
//! the owner itself remembers of the input (its submission and turns), `replay` asks the seam
//! (`replay`), and `config` shows and changes the settings with their specs (J.4). Every call is
//! one audited line, and none of them holds a request's own text in it: a setting is named by its
//! closed spec name or not at all, and a log name is never echoed.
//!
//! A setting the pipeline reads for every batch (`NextInput`) can change while the owner runs; one
//! the owner reads once at start (the three seat limits of the queue) is refused with a typed error
//! and not accepted and ignored, because a value shown as set that does nothing would be a lie.

use df_table::config::{CostEffect, SettingId, SettingOwner, SettingUnit, Settings};
use df_table::event::TableEvent;
use df_table::identity::OperationId;
use serde_json::{Map, Value};

use crate::engine::audit::EngineAudit;
use crate::engine::report::EngineError;
use crate::engine::report::observe::{
    Applies, ConfigReport, EventView, HistoryReport, ReplayReport, RetentionView, SettingChange,
    SettingView, TraceReport,
};
use crate::engine::request::{ClassFilter, ConfigControl, HistoryRequest, ReplayRequest};
use crate::observe::{
    DEFAULT_HISTORY_RECORDS, HistoryFilter, MAXIMUM_RECORDS_PER_HISTORY_ANSWER, RecordClass,
    Retention,
};

use super::Owner;

fn class_of(filter: ClassFilter) -> RecordClass {
    match filter {
        ClassFilter::Always => RecordClass::Always,
        ClassFilter::Verbose => RecordClass::Verbose,
    }
}

fn retention_view(retention: Retention) -> RetentionView {
    RetentionView {
        retained: retention.retained,
        evicted: retention.evicted,
        oldest_audit_id: retention.oldest_audit_id,
        newest_audit_id: retention.newest_audit_id,
    }
}

/// When a setting's change takes effect. Exhaustive, so a setting added to the spec list is not
/// shown until it is placed here.
fn applies_of(id: SettingId) -> Applies {
    match id {
        SettingId::NarrationLateS
        | SettingId::NarrationBatchS
        | SettingId::HeldNudgeS
        | SettingId::RulingNudgeS
        | SettingId::DecideDeadlineS => Applies::NextInput,
        SettingId::SeatPendingLimit
        | SettingId::SeatRateBurst
        | SettingId::SeatRateSustainedPerS => Applies::Restart,
    }
}

fn unit_label(unit: SettingUnit) -> &'static str {
    match unit {
        SettingUnit::Count => "count",
        SettingUnit::Seconds => "seconds",
        SettingUnit::PerSecond => "per_second",
    }
}

fn owner_label(owner: SettingOwner) -> &'static str {
    match owner {
        SettingOwner::LauncherSettings => "launcher_settings",
    }
}

fn cost_label(cost: CostEffect) -> &'static str {
    match cost {
        CostEffect::None => "none",
        CostEffect::FewerNarrationCalls => "fewer_narration_calls",
        CostEffect::BoundsModelCalls => "bounds_model_calls",
    }
}

fn setting_by_name(name: &str) -> Option<SettingId> {
    SettingId::ALL
        .iter()
        .copied()
        .find(|id| id.spec().name == name)
}

impl Owner {
    pub(super) fn history(&self, request: &HistoryRequest) -> HistoryReport {
        let limit = request
            .limit
            .unwrap_or(DEFAULT_HISTORY_RECORDS)
            .min(MAXIMUM_RECORDS_PER_HISTORY_ANSWER);
        let filter = HistoryFilter {
            area: request.area.clone(),
            class: request.class.map(class_of),
            operation: None,
        };
        let page = self.observer.query(&filter, limit);
        let report = HistoryReport {
            retention: retention_view(page.retention),
            matched: page.matched,
            events: page.records.iter().map(EventView::from).collect(),
            stream: self.observer.health(),
        };
        self.audit(EngineAudit::History {
            count: report.events.len(),
        });
        report
    }

    pub(super) fn trace(&self, operation_id: OperationId) -> TraceReport {
        let hex = operation_id.to_string();
        let filter = HistoryFilter {
            operation: Some(hex.clone()),
            ..HistoryFilter::default()
        };
        let page = self
            .observer
            .query(&filter, MAXIMUM_RECORDS_PER_HISTORY_ANSWER);
        let report = TraceReport {
            retention: retention_view(page.retention),
            events: page.records.iter().map(EventView::from).collect(),
            submissions: self
                .recent_submissions
                .iter()
                .filter(|submission| submission.operation == hex)
                .cloned()
                .collect(),
            turns: self
                .recent_turns
                .iter()
                .filter(|turn| turn.operation == hex)
                .cloned()
                .collect(),
            operation: hex,
        };
        self.audit(EngineAudit::Trace {
            operation: operation_id,
            events: report.events.len(),
        });
        report
    }

    pub(super) fn replay(&self, request: &ReplayRequest) -> Result<ReplayReport, EngineError> {
        if request.log.trim().is_empty() {
            return Err(EngineError::BlankLog);
        }
        match self.replayer.replay(request) {
            Ok(report) => {
                self.audit(EngineAudit::Replayed {
                    turns: report.turns_replayed,
                });
                Ok(report)
            }
            Err(refusal) => {
                self.audit(EngineAudit::ReplayRefused {
                    cause: refusal.key(),
                });
                Err(refusal.into())
            }
        }
    }

    pub(super) fn config(&mut self, control: ConfigControl) -> Result<ConfigReport, EngineError> {
        match control {
            ConfigControl::Show => {
                self.audit(EngineAudit::Config {
                    verb: "show",
                    setting: None,
                    value: None,
                });
                Ok(self.config_report(None))
            }
            ConfigControl::Set { name, value } => self.set_setting(&name, value),
        }
    }

    fn config_report(&self, changed: Option<SettingChange>) -> ConfigReport {
        let settings = SettingId::ALL
            .iter()
            .map(|id| {
                let spec = id.spec();
                SettingView {
                    name: spec.name.to_owned(),
                    value: self.config.settings.get(*id),
                    default: spec.default,
                    minimum: spec.range.minimum,
                    maximum: spec.range.maximum,
                    unit: unit_label(spec.unit).to_owned(),
                    owner: owner_label(spec.owner).to_owned(),
                    cost_effect: cost_label(spec.cost_effect).to_owned(),
                    applies: applies_of(*id),
                }
            })
            .collect();
        ConfigReport { settings, changed }
    }

    fn refuse_setting(
        &self,
        setting: Option<&'static str>,
        cause: &'static str,
        error: EngineError,
    ) -> EngineError {
        self.audit(EngineAudit::ConfigRefused { setting, cause });
        error
    }

    fn set_setting(&mut self, name: &str, value: u64) -> Result<ConfigReport, EngineError> {
        let Some(id) = setting_by_name(name) else {
            return Err(self.refuse_setting(
                None,
                "unknown_setting",
                EngineError::UnknownSetting {
                    name: name.to_owned(),
                },
            ));
        };
        let spec = id.spec();
        if applies_of(id) == Applies::Restart {
            return Err(self.refuse_setting(
                Some(spec.name),
                "needs_restart",
                EngineError::SettingNeedsRestart {
                    name: spec.name.to_owned(),
                },
            ));
        }
        let before = self.config.settings.get(id);
        let mut proposed = Map::new();
        for other in SettingId::ALL {
            let number = if *other == id {
                Value::from(value)
            } else {
                Value::from(self.config.settings.get(*other))
            };
            proposed.insert(other.spec().name.to_owned(), number);
        }
        // The spec checks the value and the one rule between two settings; any complaint refuses
        // the whole change and leaves the settings as they were.
        let loaded = Settings::load(&proposed).map_err(|_| {
            self.refuse_setting(
                Some(spec.name),
                "spec_unreadable",
                EngineError::SettingSpecInvalid {
                    name: spec.name.to_owned(),
                },
            )
        })?;
        let refusal = loaded.events.iter().find_map(|event| match event {
            TableEvent::SettingRejected { cause, .. } => Some(*cause),
            _ => None,
        });
        if let Some(cause) = refusal {
            return Err(self.refuse_setting(
                Some(spec.name),
                cause.key(),
                EngineError::SettingRefused {
                    name: spec.name.to_owned(),
                    cause,
                },
            ));
        }
        self.config.settings = loaded.settings;
        let after = self.config.settings.get(id);
        self.audit(EngineAudit::Config {
            verb: "set",
            setting: Some(spec.name),
            value: Some(after),
        });
        Ok(self.config_report(Some(SettingChange {
            name: spec.name.to_owned(),
            from: before,
            to: after,
        })))
    }
}
