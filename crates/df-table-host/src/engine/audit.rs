//! What the owner actor records: the sinks that carry typed events into the host's log, and the
//! `EngineAudit` lines for every dev call and every commit.
//!
//! The engine's `TableEvent`s go to `AuditSink`, which renders each with the one renderer and
//! appends it to the same bounded log the canvas uses, so `df-table dev events` shows both. The
//! broker's events go to `LogBrokerAudit` the same way. A dev call and a commit are not table
//! events (the pure crate's enum is closed and has no variant for them), so they are
//! `EngineAudit`: a closed enum whose `ENGINE verb key=value` and `COMMIT verb key=value` lines
//! hold ids, closed keys and counts only, never player text. Every field is a validated type
//! (`MachineId`, `PurposeName`, `OperationId`, `Sender`) or a closed key, so a string from a
//! request, which could hold a newline and forge a line, cannot be passed in; the one place a
//! request's name is refused is recorded as the fixed key `invalid_name`.

use std::fmt::Write;
use std::sync::Arc;

use df_table::envelope::Sender;
use df_table::event::TableEvent;
use df_table::identity::{MachineId, OperationId};
use df_table::machine::StateName;
use df_table::ports::{Audience, EventSink, PurposeName};

use crate::broker::{BrokerAudit, BrokerEvent};
use crate::event_log::EventLog;

use super::commit::CommitError;
use super::labels::sender_label;
use super::projection_adapter::audience_label;

/// Appends every engine event, of both classes, to the host's log. The dev engine's events are
/// few, so the verbose class is kept.
pub struct AuditSink {
    log: Arc<EventLog>,
}

impl AuditSink {
    pub fn new(log: Arc<EventLog>) -> Self {
        Self { log }
    }
}

impl EventSink for AuditSink {
    fn record(&self, event: &TableEvent) {
        self.log.push(event.render_audit_line());
    }
}

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
    fn key(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
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
    },
    Viable {
        seat: u64,
        count: usize,
    },
    Dump {
        revision: u64,
        commit: u64,
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
            } => write!(
                out,
                "input sender={} kind={kind} operation_id={operation} result={}",
                sender_label(*sender),
                result.key()
            ),
            Self::Viable { seat, count } => write!(out, "viable seat={seat} count={count}"),
            Self::Dump { revision, commit } => {
                write!(out, "dump revision={revision} commit={commit}")
            }
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
                "applied commit={commit} revision={revision} inputs={inputs} bytes={bytes} ms={ms} policy={policy}"
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
        }
    }
}
