//! The structured rendering of an `EngineAudit` line: the same call, the same fields, as typed
//! attributes for the host's event sink.
//!
//! `EngineAudit` writes its `ENGINE verb key=value` line in `audit.rs`; this file writes the same
//! record's attributes from the same value, with an exhaustive `match`, so a variant added to the
//! audit without its attributes does not compile and the line and the record cannot disagree. The
//! line is never parsed. A line about one input names its operation (`df.operation`), which is how
//! `trace` finds the host's own record of an input beside the table's events for it; a commit
//! covers a batch and names none.

use crate::observe::{AttributeValue, HostEventSink, RecordClass, RecordDraft, RecordSeverity};

use super::audit::EngineAudit;
use super::labels::sender_label;
use super::projection_adapter::audience_label;

pub(super) fn text(value: &str) -> AttributeValue {
    AttributeValue::Text(value.to_owned())
}

pub(super) fn whole(value: impl TryInto<i64>) -> AttributeValue {
    AttributeValue::Whole(value.try_into().unwrap_or(i64::MAX))
}

/// What one audit variant contributes to its record, beyond its line.
struct Parts {
    /// The OTEL event name.
    name: &'static str,
    /// The verb as `df.event` carries it.
    kind: &'static str,
    severity: RecordSeverity,
    operation: Option<String>,
    fields: Vec<(&'static str, AttributeValue)>,
}

impl Parts {
    fn new(name: &'static str, kind: &'static str) -> Self {
        Self {
            name,
            kind,
            severity: RecordSeverity::Info,
            operation: None,
            fields: Vec::new(),
        }
    }

    fn field(mut self, name: &'static str, value: AttributeValue) -> Self {
        self.fields.push((name, value));
        self
    }

    fn severity(mut self, severity: RecordSeverity) -> Self {
        self.severity = severity;
        self
    }

    fn operation(mut self, operation: String) -> Self {
        self.operation = Some(operation);
        self
    }

    fn driven(self) -> Self {
        self.field("driven", AttributeValue::Flag(true))
    }
}

impl EngineAudit {
    fn parts(&self) -> Parts {
        match self {
            Self::Input {
                sender,
                kind,
                operation,
                result,
            } => Parts::new("engine.input", "input")
                .operation(operation.to_string())
                .field("sender", text(&sender_label(*sender)))
                .field("input_kind", text(kind))
                .field("result", text(result.key())),
            Self::Viable { seat, count } => Parts::new("engine.viable", "viable")
                .field("seat", whole(*seat))
                .field("count", whole(*count)),
            Self::Dump { revision, commit } => Parts::new("engine.dump", "dump")
                .field("revision", whole(*revision))
                .field("commit", whole(*commit)),
            Self::Force {
                machine,
                state,
                revision,
            } => Parts::new("engine.force", "force")
                .field("machine", text(&machine.to_string()))
                .field("state", text(&state.to_string()))
                .field("revision", whole(*revision))
                .driven(),
            Self::ForceRefused { machine, cause } => {
                Parts::new("engine.force_refused", "force_refused")
                    .severity(RecordSeverity::Warn)
                    .field(
                        "machine",
                        text(
                            &machine
                                .as_ref()
                                .map_or_else(|| "invalid_name".to_owned(), ToString::to_string),
                        ),
                    )
                    .field("cause", text(cause))
            }
            Self::Projection { audience, revision } => {
                Parts::new("engine.projection", "projection")
                    .field("audience", text(&audience_label(*audience)))
                    .field("revision", whole(*revision))
            }
            Self::Queue { verb, served } => Parts::new("engine.queue", "queue")
                .field("verb", text(verb))
                .field("served", whole(*served)),
            Self::Describe => Parts::new("engine.describe", "describe"),
            Self::Health => Parts::new("engine.health", "health"),
            Self::Clock { verb, now_ms } => Parts::new("engine.clock", "clock")
                .field("verb", text(verb))
                .field("now_ms", whole(*now_ms))
                .driven(),
            Self::Dice { verb, values } => Parts::new("engine.dice", "dice")
                .field("verb", text(verb))
                .field("values", whole(*values))
                .driven(),
            Self::Chaos {
                dependency,
                fault,
                count,
            } => Parts::new("engine.chaos", "chaos")
                .field("dependency", text(dependency))
                .field("fault", text(fault))
                .field("count", whole(*count))
                .driven(),
            Self::Stalled { cause } => Parts::new("engine.stalled", "stalled")
                .severity(RecordSeverity::Error)
                .field("cause", text(cause)),
            Self::Stopped { dropped_inputs } => Parts::new("engine.stopped", "stopped")
                .field("cause", text("closed"))
                .field("dropped_inputs", whole(*dropped_inputs)),
            Self::Panicked => Parts::new("engine.stopped", "stopped")
                .severity(RecordSeverity::Error)
                .field("cause", text("panic")),
            Self::Broker { verb, purpose } => {
                let parts = Parts::new("engine.broker", "broker").field("verb", text(verb));
                match purpose {
                    Some(purpose) => parts.field("purpose", text(&purpose.to_string())),
                    None => parts,
                }
            }
            Self::Committed {
                commit,
                revision,
                inputs,
                bytes,
                ms,
                policy,
            } => Parts::new("commit.applied", "applied")
                .field("commit", whole(*commit))
                .field("revision", whole(*revision))
                .field("inputs", whole(*inputs))
                .field("bytes", whole(*bytes))
                .field("ms", whole(*ms))
                .field("policy", text(policy)),
            Self::NothingToCommit { inputs } => {
                Parts::new("commit.none", "none").field("inputs", whole(*inputs))
            }
            Self::CommitFailed {
                inputs,
                error,
                discarded_events,
            } => Parts::new("commit.failed", "failed")
                .severity(RecordSeverity::Error)
                .field("inputs", whole(*inputs))
                .field("cause", text(error.key()))
                .field("discarded_events", whole(*discarded_events)),
            Self::BackingOff {
                operation,
                retry_after_ms,
            } => Parts::new("commit.refused", "refused")
                .severity(RecordSeverity::Warn)
                .operation(operation.to_string())
                .field("cause", text("backing_off"))
                .field("retry_after_ms", whole(*retry_after_ms)),
            Self::History { count } => {
                Parts::new("engine.history", "history").field("count", whole(*count))
            }
            Self::Trace { operation, events } => Parts::new("engine.trace", "trace")
                .operation(operation.to_string())
                .field("events", whole(*events)),
            Self::Config {
                verb,
                setting,
                value,
            } => {
                let parts = Parts::new("engine.config", "config").field("verb", text(verb));
                match (setting, value) {
                    (Some(setting), Some(value)) => parts
                        .field("setting", text(setting))
                        .field("value", whole(*value))
                        .driven(),
                    _ => parts,
                }
            }
            Self::ConfigRefused { setting, cause } => {
                let parts = Parts::new("engine.config_refused", "config_refused")
                    .severity(RecordSeverity::Warn)
                    .field("cause", text(cause));
                match setting {
                    Some(setting) => parts.field("setting", text(setting)),
                    None => parts,
                }
            }
            Self::Replayed { turns } => {
                Parts::new("engine.replay", "replay").field("turns", whole(*turns))
            }
            Self::ReplayRefused { cause } => Parts::new("engine.replay_refused", "replay_refused")
                .severity(RecordSeverity::Warn)
                .field("cause", text(cause)),
        }
    }

    /// The record of this line: always class A (a dev call or a commit is never sampled), with the
    /// same line the log gets.
    pub fn draft(&self) -> RecordDraft {
        let parts = self.parts();
        let mut draft = RecordDraft::new(
            RecordClass::Always,
            parts.severity,
            self.area(),
            parts.name,
            parts.kind,
            self.render_audit_line(),
        );
        if let Some(operation) = parts.operation {
            draft = draft.about_operation(operation);
        }
        for (name, value) in parts.fields {
            draft = draft.with_field(name, value);
        }
        draft
    }
}

/// Records one engine audit line through the sink: the line to the log and the record to every
/// outlet, from the one value.
pub fn record_engine(sink: &HostEventSink, audit: &EngineAudit) {
    sink.record_draft(audit.draft());
}
