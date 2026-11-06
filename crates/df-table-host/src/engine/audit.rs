//! What the owner actor records besides the table's own events: the broker's audit and the
//! `EngineAudit` lines for every dev call and every commit.
//!
//! The engine's `TableEvent`s go to the host's event sink (`crate::observe::HostEventSink`), which
//! renders each as the line and the structured record; the broker's events go to `LogBrokerAudit`,
//! which appends their line to the same bounded log the canvas uses, so `df-table dev events`
//! shows all of them. A dev call and a commit are not table events (the pure crate's enum is
//! closed and has no variant for them), so they are `EngineAudit`: a closed enum whose `ENGINE
//! verb key=value` and `COMMIT verb key=value` lines hold ids, closed keys and counts only, never
//! player text, and which `audit_fields` also gives to the sink as structured records. Every
//! field is a validated type (`MachineId`, `PurposeName`, `OperationId`, `Sender`) or a closed
//! key, so a string from a request, which could hold a newline and forge a line, cannot be passed
//! in; the one place a request's name is refused is recorded as the fixed key `invalid_name`.

use std::fmt::Write;
use std::sync::Arc;

use df_table::envelope::Sender;
use df_table::identity::{MachineId, OperationId};
use df_table::inspect::InspectAudience;
use df_table::machine::StateName;
use df_table::outcome::OutcomeKind;
use df_table::ports::{Audience, PurposeName};

use crate::broker::{BrokerAudit, BrokerEvent};
use crate::event_log::EventLog;

use super::commit::CommitError;
use super::labels::sender_label;
use super::projection_adapter::audience_label;
use super::request::{InspectName, inspect_audience_word};

/// Appends every broker event to the host's log.
pub struct LogBrokerAudit {
    log: Arc<EventLog>,
}

impl LogBrokerAudit {
    pub fn new(log: Arc<EventLog>) -> Self {
        Self { log }
    }
}

impl BrokerAudit for LogBrokerAudit {
    fn record(&self, event: &BrokerEvent) {
        self.log.push(event.render_audit_line());
    }
}

/// How an input that was refused before the queue is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitResult {
    Admitted,
    Refused,
}

impl SubmitResult {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Refused => "refused",
        }
    }
}

/// What a `dump` printed: the active machine with the table, the table alone, or one machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DumpOf {
    Active,
    Table,
    Machine(MachineId),
}

impl DumpOf {
    pub(super) fn label(&self) -> String {
        match self {
            Self::Active => "active".to_owned(),
            Self::Table => "table".to_owned(),
            Self::Machine(machine) => machine.to_string(),
        }
    }
}

/// How a wire intent was answered: a receipt that took it, a receipt that declined it, or no
/// receipt at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireIntentOutcome {
    Accepted,
    Declined,
    Refused,
}

impl WireIntentOutcome {
    pub fn key(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Refused => "refused",
        }
    }
}

/// One audited dev call or commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineAudit {
    Input {
        sender: Sender,
        kind: &'static str,
        operation: OperationId,
        result: SubmitResult,
        /// How many inputs wait in the lane it was admitted to, itself included; `None` when it
        /// was refused.
        lane_depth: Option<usize>,
    },
    /// One input ran to its end: how long it waited in its lane, how long its step took on the
    /// owner's clock, and how it ended once its batch was saved or failed.
    Served {
        operation: OperationId,
        waited_ms: u64,
        step_ms: u64,
        outcome: OutcomeKind,
    },
    /// One wire intent and its answer. `operation` is absent when the intent was refused before
    /// its id was read; `code` is the decline code or the refusal's key, a closed key either way.
    WireIntent {
        seat: u64,
        operation: Option<OperationId>,
        outcome: WireIntentOutcome,
        code: Option<&'static str>,
    },
    /// A `MachineView` served to a TV or a phone, and its size on the wire.
    WireView {
        audience: Audience,
        revision: u64,
        bytes: usize,
    },
    /// An inspect report was built for an audience.
    Inspect {
        report: InspectName,
        audience: InspectAudience,
    },
    /// `chaos` with no words: how many faults are in force.
    ChaosShow {
        in_force: usize,
    },
    Viable {
        seat: u64,
        count: usize,
    },
    Dump {
        revision: u64,
        commit: u64,
        of: DumpOf,
    },
    Force {
        machine: MachineId,
        state: StateName,
        revision: u64,
    },
    /// A forced change that was refused. `machine` is absent when the name given was not a valid
    /// name, which is all that may be said about it.
    ForceRefused {
        machine: Option<MachineId>,
        cause: &'static str,
    },
    Projection {
        audience: Audience,
        revision: u64,
    },
    Queue {
        verb: &'static str,
        served: usize,
    },
    Describe,
    Health,
    Clock {
        verb: &'static str,
        now_ms: u64,
    },
    Dice {
        verb: &'static str,
        values: usize,
    },
    Chaos {
        dependency: &'static str,
        fault: &'static str,
        count: u32,
    },
    /// The owner could not serve: the queue is not able to number another input.
    Stalled {
        cause: &'static str,
    },
    /// The owner ended in an orderly way, with this many admitted inputs never served.
    Stopped {
        dropped_inputs: usize,
    },
    /// The owner thread panicked. Nothing of the panic's message is recorded: it may hold text.
    Panicked,
    Broker {
        verb: &'static str,
        purpose: Option<PurposeName>,
    },
    Committed {
        commit: u64,
        revision: u64,
        inputs: usize,
        bytes: usize,
        ms: u64,
        policy: &'static str,
    },
    NothingToCommit {
        inputs: usize,
    },
    CommitFailed {
        inputs: usize,
        error: CommitError,
        discarded_events: usize,
    },
    /// New input was refused `table_busy` while the owner backs off from a failed commit.
    BackingOff {
        operation: OperationId,
        retry_after_ms: u64,
    },
    /// `history` was read: how many records it showed.
    History {
        count: usize,
    },
    /// `trace` was read: the operation and how many events of it were retained.
    Trace {
        operation: OperationId,
        events: usize,
    },
    /// `config show` or `config set`. `setting` and `value` are present for a change, and the
    /// setting is always one of the closed list of names, never a name from the request.
    Config {
        verb: &'static str,
        setting: Option<&'static str>,
        value: Option<u32>,
    },
    /// A `config set` that was refused. `setting` is absent when the name given is not a setting.
    ConfigRefused {
        setting: Option<&'static str>,
        cause: &'static str,
    },
    /// `replay` ran and replayed this many turns.
    Replayed {
        turns: u64,
    },
    /// `replay` was asked for and refused with `cause`, a closed key (`not_supported_yet` until
    /// C7's replay is behind the seam).
    ReplayRefused {
        cause: &'static str,
    },
}

impl EngineAudit {
    pub fn area(&self) -> &'static str {
        match self {
            Self::Committed { .. }
            | Self::NothingToCommit { .. }
            | Self::CommitFailed { .. }
            | Self::BackingOff { .. } => "COMMIT",
            _ => "ENGINE",
        }
    }

    /// The whole line, `AREA verb key=value ...`.
    pub fn render_audit_line(&self) -> String {
        let mut line = String::with_capacity(128);
        // Writing into a String cannot fail, so the result carries no information.
        let _ = self.write_line(&mut line);
        line
    }

    fn write_line(&self, out: &mut String) -> std::fmt::Result {
        write!(out, "{} ", self.area())?;
        match self {
            Self::Input {
                sender,
                kind,
                operation,
                result,
                lane_depth,
            } => {
                write!(
                    out,
                    "input sender={} kind={kind} operation_id={operation} result={} lane_depth=",
                    sender_label(*sender),
                    result.key()
                )?;
                write_or_dash(out, *lane_depth)
            }
            Self::Served {
                operation,
                waited_ms,
                step_ms,
                outcome,
            } => write!(
                out,
                "served operation={operation} waited_ms={waited_ms} step_ms={step_ms} outcome={}",
                outcome.key()
            ),
            Self::WireIntent {
                seat,
                operation,
                outcome,
                code,
            } => {
                write!(out, "wire_intent seat={seat} operation=")?;
                write_or_dash(out, operation.as_ref())?;
                write!(out, " outcome={} code=", outcome.key())?;
                write_or_dash(out, *code)
            }
            Self::WireView {
                audience,
                revision,
                bytes,
            } => write!(
                out,
                "wire_view audience={} revision={revision} bytes={bytes}",
                audience_label(*audience)
            ),
            Self::Inspect { report, audience } => write!(
                out,
                "inspect report={} audience={}",
                report.key(),
                inspect_audience_word(*audience)
            ),
            Self::ChaosShow { in_force } => write!(out, "chaos verb=show in_force={in_force}"),
            Self::Viable { seat, count } => write!(out, "viable seat={seat} count={count}"),
            Self::Dump {
                revision,
                commit,
                of,
            } => write!(
                out,
                "dump revision={revision} commit={commit} of={}",
                of.label()
            ),
            Self::Force {
                machine,
                state,
                revision,
            } => write!(
                out,
                "force machine={machine} state={state} driven=true revision={revision}"
            ),
            Self::ForceRefused { machine, cause } => {
                let machine = machine
                    .as_ref()
                    .map_or_else(|| "invalid_name".to_owned(), ToString::to_string);
                write!(out, "force_refused machine={machine} cause={cause}")
            }
            Self::Projection { audience, revision } => write!(
                out,
                "projection audience={} revision={revision}",
                audience_label(*audience)
            ),
            Self::Queue { verb, served } => write!(out, "queue verb={verb} served={served}"),
            Self::Describe => out.write_str("describe"),
            Self::Health => out.write_str("health"),
            Self::Clock { verb, now_ms } => {
                write!(out, "clock verb={verb} now_ms={now_ms} driven=true")
            }
            Self::Dice { verb, values } => {
                write!(out, "dice verb={verb} values={values} driven=true")
            }
            Self::Chaos {
                dependency,
                fault,
                count,
            } => write!(
                out,
                "chaos dependency={dependency} fault={fault} count={count} driven=true"
            ),
            Self::Stalled { cause } => write!(out, "stalled cause={cause}"),
            Self::Stopped { dropped_inputs } => {
                write!(out, "stopped cause=closed dropped_inputs={dropped_inputs}")
            }
            Self::Panicked => out.write_str("stopped cause=panic"),
            Self::Broker { verb, purpose } => {
                write!(out, "broker verb={verb}")?;
                if let Some(purpose) = purpose {
                    write!(out, " purpose={purpose}")?;
                }
                Ok(())
            }
            Self::Committed {
                commit,
                revision,
                inputs,
                bytes,
                ms,
                policy,
            } => write!(
                out,
                "applied commit={commit} revision={revision} inputs={inputs} bytes={bytes} ms={ms} \
                 policy={policy}"
            ),
            Self::NothingToCommit { inputs } => write!(out, "none inputs={inputs}"),
            Self::CommitFailed {
                inputs,
                error,
                discarded_events,
            } => {
                write!(
                    out,
                    "failed inputs={inputs} cause={} discarded_events={discarded_events}",
                    error.key()
                )?;
                if let CommitError::DeadlineExceeded { deadline_ms } = error {
                    write!(out, " deadline_ms={deadline_ms}")?;
                }
                Ok(())
            }
            Self::BackingOff {
                operation,
                retry_after_ms,
            } => write!(
                out,
                "refused operation_id={operation} cause=backing_off retry_after_ms={retry_after_ms}"
            ),
            Self::History { count } => write!(out, "history count={count}"),
            Self::Trace { operation, events } => {
                write!(out, "trace operation_id={operation} events={events}")
            }
            Self::Config {
                verb,
                setting,
                value,
            } => {
                write!(out, "config verb={verb}")?;
                if let (Some(setting), Some(value)) = (setting, value) {
                    write!(out, " setting={setting} value={value} driven=true")?;
                }
                Ok(())
            }
            Self::ConfigRefused { setting, cause } => {
                write!(out, "config_refused")?;
                if let Some(setting) = setting {
                    write!(out, " setting={setting}")?;
                }
                write!(out, " cause={cause}")
            }
            Self::Replayed { turns } => write!(out, "replay turns={turns}"),
            Self::ReplayRefused { cause } => write!(out, "replay_refused cause={cause}"),
        }
    }
}

/// Writes `value`, or `-` when there is none.
fn write_or_dash(out: &mut String, value: Option<impl std::fmt::Display>) -> std::fmt::Result {
    match value {
        Some(value) => write!(out, "{value}"),
        None => out.write_str("-"),
    }
}
