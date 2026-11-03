//! Everything that crosses the dev engine routes, in one place: the answers the owner gives and
//! the errors it refuses with.
//!
//! The routes and the command line read and write these types, so a rename breaks the build
//! instead of a running command. A closed enum of the pure crate that already serializes (a lane,
//! an outcome, a decline code, a failure cause) is used as it is; a key that does not is its
//! `key()` text. Nothing here holds player text: a turn report says how long a line was, never
//! what it said.

use std::fmt;

use df_table::outcome::{DeclineCode, OutcomeKind};
use df_table::owner_queue::Lane;
use df_table::pipeline::PipelineRecord;
use df_table::private_data::LengthClass;
use serde::{Deserialize, Serialize};

use crate::broker::report::{LedgerReport, QueuesReport};
use crate::observe::SinkHealth;

use super::report_observe::NotBuiltVerb;
use super::verb_set::VerbLine;

/// The table's shared facts, as `dump` shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableDump {
    pub seat_capacity: usize,
    pub seats: Vec<u64>,
    pub active_scene: Option<u64>,
    pub active_beat: Option<String>,
    pub last_hand_off: Option<HandOffDump>,
    /// Every open scene, in scene order.
    pub scenes: Vec<u64>,
    pub cast: usize,
    pub world_time_s: u64,
    pub status: String,
    /// The table's digest as 16 hexadecimal digits, which changes with any fact or remembered
    /// operation: two checkpoints of one world carry one digest.
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandOffDump {
    pub from: String,
    pub to: String,
    pub carried: usize,
}

/// The committed state of the table: what `dump` prints and what a commit writes as its
/// checkpoint. `revision` is the commit sequence and `commit` the id of the batch that wrote it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineDump {
    pub revision: u64,
    pub commit: u64,
    pub machine: String,
    pub state: String,
    pub schema_version: u32,
    pub slot: serde_json::Value,
    pub table: TableDump,
}

/// What a checkpoint holds: the table as of the commit, and everything the pipeline holds beside
/// the table's facts (its open pendings, rulings, requests that are out, and the results of the
/// operations committed so far, which answer a duplicate after a restart, H.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckpointDocument {
    pub dump: EngineDump,
    pub record: PipelineRecord,
}

/// Why an input was not admitted, or how the queue answered a duplicate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusalReport {
    pub cause: String,
    pub detail: String,
    pub retry_after_ms: Option<u64>,
    /// For a duplicate: where the first copy stands.
    pub first_status: Option<String>,
}

/// What the queue did with one submitted input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmissionReport {
    pub operation: String,
    pub sender: String,
    pub kind: String,
    pub lane: Option<Lane>,
    pub refusal: Option<RefusalReport>,
}

/// What became of an input's commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "commit", rename_all = "snake_case")]
pub enum CommitDisposition {
    /// The batch it belonged to was committed with this id.
    Committed { id: u64 },
    /// Nothing in its batch needed saving.
    NothingToCommit,
    /// The batch it belonged to failed to commit, so nothing it did was kept.
    Failed { cause: String },
}

/// One input as the owner served it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnReport {
    pub turn_seq: u64,
    pub operation: String,
    pub sender: String,
    pub kind: String,
    pub lane: Lane,
    pub waited_ms: u64,
    pub outcome: OutcomeKind,
    pub decline: Option<DeclineCode>,
    /// The structured line the table shows at once for this input (H.15): never blank, and made
    /// only of typed fields, never of player words.
    pub readout: String,
    pub action: Option<String>,
    /// The input was a repeat of an operation committed before, so nothing was drawn or run.
    pub replayed: bool,
    /// The pending the input opened or answered, if any.
    pub pending: Option<u64>,
    pub machine_before: String,
    pub state_before: String,
    pub machine_after: String,
    pub state_after: String,
    pub effects: usize,
    pub commit: CommitDisposition,
    pub revision: Option<u64>,
    pub narration: Option<LengthClass>,
    /// What the sending seat is shown after this input, as of this input and not of the end of
    /// its batch, released only after the batch's commit. `None` for an engine input, for a batch
    /// that failed to commit, and in the recent-turns list of `queue show`.
    pub view: Option<serde_json::Value>,
}

/// The answer to a submission: what the queue did, and the turns that ran because of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitReply {
    pub submission: SubmissionReport,
    pub turns: Vec<TurnReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneDepth {
    pub lane: Lane,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatPending {
    pub seat: u64,
    pub pending: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InProgressReport {
    pub sender: String,
    pub operation: String,
    pub turn_seq: u64,
    pub age_ms: u64,
}

/// The commit policy and how commits have gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitReport {
    pub policy: String,
    pub deadline_ms: u64,
    pub commits: u64,
    pub failures: u64,
    pub last_commit: Option<u64>,
    pub last_checkpoint_bytes: Option<usize>,
    /// How much longer the owner refuses input `table_busy` after a failed commit.
    pub backing_off_for_ms: Option<u64>,
}

/// `queue show`: the lanes, who is in progress, who has what pending, and what happened lately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueReport {
    pub serving: String,
    pub now_ms: u64,
    pub lanes: Vec<LaneDepth>,
    pub in_progress: Vec<InProgressReport>,
    pub seats: Vec<SeatPending>,
    pub remembered_operations: usize,
    pub last_turn_seq: u64,
    pub commit: CommitReport,
    pub submissions: Vec<SubmissionReport>,
    pub turns: Vec<TurnReport>,
}

/// `health`: whether the owner is serving and how its commits and queue stand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    pub owner: String,
    pub serving: String,
    pub waiting_inputs: usize,
    pub oldest_in_progress_ms: Option<u64>,
    pub commits: u64,
    pub failures: u64,
    pub backing_off_for_ms: Option<u64>,
    pub broker_epoch: u64,
    pub broker_closed: bool,
    pub now_ms: u64,
    /// The event stream: what was seen by class, what each outlet confirmed and lost, and how far
    /// behind ingestion is (`lag_ms`).
    pub events: SinkHealth,
}

/// `clock`: the owner's time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockReport {
    pub now_ms: u64,
    pub frozen: bool,
}

/// `dice`: how many forced values wait before the seeded sequence continues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiceReport {
    pub forced_waiting: usize,
    pub seed: Option<u64>,
}

/// `chaos`: what fault is now in force.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChaosReport {
    pub fault: String,
    pub in_force: String,
}

/// `viable`: the actions the machine would accept from the seat now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViableReport {
    pub seat: u64,
    pub machine: String,
    pub state: String,
    pub actions: Vec<String>,
}

/// `force`: where the table was moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForceReport {
    pub machine: String,
    pub state: String,
    pub revision: u64,
    pub commit: CommitDisposition,
}

/// `queue step` and `queue release`: the turns that ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServedReply {
    pub turns: Vec<TurnReport>,
}

/// One registered machine, as `describe` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineDescription {
    pub machine: String,
    pub scope: String,
    pub enabled: bool,
    pub active: bool,
    pub states: Vec<String>,
    pub hand_off_targets: Vec<String>,
    pub rules_actions: Vec<String>,
    pub actions: Vec<String>,
}

/// `describe`: generated from the registry and the verb table, never written by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Description {
    pub machines: Vec<MachineDescription>,
    pub lanes: Vec<Lane>,
    pub ports: Vec<String>,
    pub commit_policy: String,
    pub verbs: Vec<String>,
    /// C7's verb set, generated from `df_table::inspect`, with what the host has of each.
    pub verb_set: Vec<VerbLine>,
    /// The H.16 verbs the host does not have yet and what each waits for.
    pub not_built: Vec<NotBuiltVerb>,
}

/// `broker request`: the handle of the request that was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerRequested {
    pub handle: u64,
    pub purpose: String,
    pub readiness: String,
}

/// `broker queues` and `broker ledger`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum BrokerView {
    Queues(QueuesReport),
    Ledger(LedgerReport),
}

/// Why a dev call was refused. Each variant says what the caller does next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum EngineError {
    /// The owner's mailbox is full; try again.
    MailboxFull,
    /// The owner thread has stopped.
    Stopped,
    /// The owner took the command and ended without answering.
    NoAnswer,
    InvalidName {
        what: String,
    },
    UnknownSeat {
        seat: u64,
    },
    UnknownMachine {
        machine: String,
    },
    UnknownState {
        machine: String,
        state: String,
    },
    /// The payload given to `force` is not a state of the machine.
    BadPayload {
        machine: String,
        reason: String,
    },
    /// The machine could not be entered or the commit of the change failed.
    ForceRefused {
        reason: String,
    },
    UnknownPurpose {
        purpose: String,
    },
    /// The request body is wrong: a zero operation id, an unknown audience.
    BadRequest {
        reason: String,
    },
    /// A result could not be written.
    Unwritable {
        what: String,
    },
    /// The owner never started, so the routes have nobody to talk to.
    NotStarted {
        reason: String,
    },
    /// The owner thread panicked; the table is stopped and nothing it held was saved after its
    /// last commit.
    OwnerPanicked,
    /// This table has no such control (a control of the dev table, not of a hosted one).
    NotAvailable {
        what: String,
    },
    /// No setting has this name.
    UnknownSetting {
        name: String,
    },
    /// The setting's spec refused the value; `cause` is the closed key of the fault.
    SettingRefused {
        name: String,
        cause: String,
    },
    /// The setting is read once when the owner starts, so it cannot change while the owner runs.
    SettingNeedsRestart {
        name: String,
    },
    /// A feature that is half built declines and says what it waits for (RES31).
    NotSupportedYet {
        what: String,
        needs: String,
    },
    /// There is no turn log of that name.
    UnknownLog {
        log: String,
    },
    /// The replay ran and failed.
    ReplayFailed {
        reason: String,
    },
    /// Too much, too fast (a rate limit or a seat with its inputs waiting): retry after the
    /// pause, when the queue gave one.
    TooFast {
        retry_after_ms: Option<u64>,
    },
    /// A lane is full: a stall and not a decline (RES22). Retry the same operation id later.
    Stalled,
    /// A view could not be made: the table has no running machine, or its projection or its wire
    /// mapping failed. A fault of the host, never of the caller.
    ViewUnavailable {
        what: String,
    },
    /// The input is admitted and waits (the owner is holding, or it is queued behind others):
    /// nothing has run for it yet, so there is no answer. Retry the same operation id.
    NotServedYet,
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MailboxFull => formatter.write_str("the owner is busy; try again"),
            Self::Stopped => formatter.write_str("the owner thread has stopped"),
            Self::NoAnswer => formatter.write_str("the owner ended without answering"),
            Self::InvalidName { what } => write!(formatter, "'{what}' is not a valid name"),
            Self::UnknownSeat { seat } => write!(formatter, "seat {seat} is not at this table"),
            Self::UnknownMachine { machine } => {
                write!(
                    formatter,
                    "no machine '{machine}' is registered and enabled"
                )
            }
            Self::UnknownState { machine, state } => {
                write!(formatter, "{machine} has no state '{state}'")
            }
            Self::BadPayload { machine, reason } => {
                write!(formatter, "that is not a state of {machine}: {reason}")
            }
            Self::ForceRefused { reason } => write!(formatter, "the change was refused: {reason}"),
            Self::UnknownPurpose { purpose } => {
                write!(formatter, "the purpose '{purpose}' is not registered")
            }
            Self::BadRequest { reason } => write!(formatter, "bad request: {reason}"),
            Self::Unwritable { what } => write!(formatter, "could not write the {what}"),
            Self::NotStarted { reason } => {
                write!(formatter, "the owner did not start: {reason}")
            }
            Self::OwnerPanicked => formatter.write_str(
                "the owner thread panicked and the table is stopped; see the log for the line",
            ),
            Self::NotAvailable { what } => write!(formatter, "this table has no {what}"),
            Self::UnknownSetting { name } => write!(formatter, "there is no setting '{name}'"),
            Self::SettingRefused { name, cause } => {
                write!(formatter, "{name} was refused: {cause}")
            }
            Self::SettingNeedsRestart { name } => write!(
                formatter,
                "{name} is read once when the owner starts, so it cannot be changed while it runs"
            ),
            Self::NotSupportedYet { what, needs } => {
                write!(formatter, "{what} is not built yet: it needs {needs}")
            }
            Self::UnknownLog { log } => write!(formatter, "there is no turn log '{log}'"),
            Self::ReplayFailed { reason } => write!(formatter, "the replay failed: {reason}"),
            Self::TooFast {
                retry_after_ms: Some(retry_after_ms),
            } => write!(
                formatter,
                "too many inputs too fast; retry in {retry_after_ms} ms"
            ),
            Self::TooFast {
                retry_after_ms: None,
            } => formatter.write_str("too many inputs too fast"),
            Self::Stalled => formatter.write_str("a lane is full; retry the same operation later"),
            Self::ViewUnavailable { what } => write!(formatter, "no view could be made: {what}"),
            Self::NotServedYet => formatter.write_str(
                "the input waits in the queue and has not run; retry the same operation id",
            ),
        }
    }
}

impl std::error::Error for EngineError {}

/// The answer to every dev engine call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EngineReply<T> {
    Done { value: T },
    Rejected { error: EngineError, message: String },
}

impl<T> EngineReply<T> {
    pub fn rejected(error: EngineError) -> Self {
        let message = error.to_string();
        Self::Rejected { error, message }
    }

    pub fn from_result(result: Result<T, EngineError>) -> Self {
        match result {
            Ok(value) => Self::Done { value },
            Err(error) => Self::rejected(error),
        }
    }
}
