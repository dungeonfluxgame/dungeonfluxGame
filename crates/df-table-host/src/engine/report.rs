//! Everything that crosses the dev engine routes, in one place: the answers the owner gives and
//! the errors it refuses with.
//!
//! The routes and the command line read and write these types, so a rename breaks the build
//! instead of a running command. A closed enum of the pure crate that already serializes (a lane,
//! an outcome, a decline code, a failure cause) is used as it is; a key that does not is its
//! `key()` text. Nothing here holds player text: a turn report says how long a line was, never
//! what it said.

pub mod observe;
pub mod reach;

use std::fmt;

use axum::http::StatusCode;
use df_table::config::SettingCause;
use df_table::event::FailureCause;
use df_table::outcome::{DeclineCode, OutcomeKind};
use df_table::owner_queue::{Lane, NotDeclinedCause};
use df_table::pipeline::PipelineRecord;
use df_table::ports::{PortError, PortName};
use df_table::private_data::LengthClass;
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::observe::SinkHealth;

use self::observe::NotBuiltVerb;
use super::commit::CommitError;
use super::setup::StartCause;
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

/// Why the queue refused an input: a decline code the player is shown, or one of the queue's own
/// causes they are not. It is written as the bare key of either (`"table_busy"`, `"duplicate"`);
/// the two lists share no key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RefusalCause {
    Declined(DeclineCode),
    NotDeclined(NotDeclinedCause),
}

impl RefusalCause {
    /// The serde name of the code or cause.
    pub fn key(self) -> &'static str {
        match self {
            Self::Declined(code) => code.key(),
            Self::NotDeclined(cause) => cause.key(),
        }
    }
}

/// Why an input was not admitted, or how the queue answered a duplicate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusalReport {
    pub cause: RefusalCause,
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
    /// The batch it belonged to failed to commit, so nothing it did was kept. Written as the
    /// error's own fields beside the tag (`"cause": "unavailable"`).
    Failed(CommitError),
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

/// What `force` could not do. The closed half of `EngineError::ForceRefused`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceCause {
    /// The machine refused to be entered with the table's scene and seats.
    EntryRefused,
    /// A group machine needs an active scene and the table has none.
    NoActiveScene,
    /// A seat lane is not forced.
    SeatLane,
    /// The table refused to start the machine.
    StartRefused,
    /// The commit of the forced table failed, so the table is as it was.
    CommitFailed,
}

impl ForceCause {
    pub const ALL: [Self; 5] = [
        Self::EntryRefused,
        Self::NoActiveScene,
        Self::SeatLane,
        Self::StartRefused,
        Self::CommitFailed,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::EntryRefused => "entry_refused",
            Self::NoActiveScene => "no_active_scene",
            Self::SeatLane => "seat_lane",
            Self::StartRefused => "start_refused",
            Self::CommitFailed => "commit_failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

/// Which step of making a view failed. The closed half of `EngineError::ViewUnavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewCause {
    /// The table's projection (or a machine's own) refused.
    Projection,
    /// The wire's guard refused the `MachineView` that was built.
    WireCheck,
    /// An inspect report could not be written as JSON.
    Report,
}

impl ViewCause {
    pub const ALL: [Self; 3] = [Self::Projection, Self::WireCheck, Self::Report];

    pub fn key(self) -> &'static str {
        match self {
            Self::Projection => "projection",
            Self::WireCheck => "wire_check",
            Self::Report => "report",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

/// Which step of answering a wire intent failed. The closed half of `EngineError::WireIntent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireIntentCause {
    /// The intent was refused at the edge (`admit_machine_intent`): the caller's mistake.
    RefusedAtTheEdge,
    /// The receipt the owner built broke a wire rule.
    ReceiptInvalid,
    /// The queue refused for a cause the wire has no answer for.
    UnknownRefusal,
    /// A declined turn carries no decline code.
    MissingDeclineCode,
    /// The line a decline carries cannot make a decline.
    DeclineInvalid,
}

impl WireIntentCause {
    pub const ALL: [Self; 5] = [
        Self::RefusedAtTheEdge,
        Self::ReceiptInvalid,
        Self::UnknownRefusal,
        Self::MissingDeclineCode,
        Self::DeclineInvalid,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::RefusedAtTheEdge => "refused_at_the_edge",
            Self::ReceiptInvalid => "receipt_invalid",
            Self::UnknownRefusal => "unknown_refusal",
            Self::MissingDeclineCode => "missing_decline_code",
            Self::DeclineInvalid => "decline_invalid",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

/// A dev control the table was built without (a hosted table has none of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevControl {
    Clock,
    Dice,
    CommitStore,
    InterpreterSwitch,
}

impl DevControl {
    pub const ALL: [Self; 4] = [
        Self::Clock,
        Self::Dice,
        Self::CommitStore,
        Self::InterpreterSwitch,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Dice => "dice",
            Self::CommitStore => "commit_store",
            Self::InterpreterSwitch => "interpreter_switch",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|control| control.key() == key)
    }

    /// The control as a person names it.
    fn describe(self) -> &'static str {
        match self {
            Self::Clock => "controllable clock",
            Self::Dice => "controllable dice",
            Self::CommitStore => "scripted commit store",
            Self::InterpreterSwitch => "interpreter switch",
        }
    }
}

/// Why a request body could not be read at all, before its JSON was looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreadableBodyCause {
    /// The body is longer than the host's request limit.
    TooLarge,
    /// The connection failed or ended while the body was read.
    Interrupted,
}

impl UnreadableBodyCause {
    pub const ALL: [Self; 2] = [Self::TooLarge, Self::Interrupted];

    pub fn key(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

/// A query of an engine GET route that is missing a parameter or has one that does not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryError {
    /// The reader's own sentence, naming the parameter it stopped at.
    pub detail: String,
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the query is not valid: {}", self.detail)
    }
}

impl std::error::Error for QueryError {}

/// Why a dev call was refused. Each variant says what the caller does next.
///
/// A `detail` beside a closed cause is the source error's own sentence (an error of another crate
/// that does not cross the wire typed); it never holds player text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum EngineError {
    /// The owner's mailbox is full; try again.
    MailboxFull,
    /// The owner thread has stopped.
    Stopped,
    /// The owner took the command and ended without answering.
    NoAnswer,
    /// The owner thread panicked; the table is stopped and nothing it held was saved after its
    /// last commit.
    OwnerPanicked,
    /// The owner never started, so the routes have nobody to talk to.
    NotStarted {
        cause: StartCause,
        detail: String,
    },
    /// The request body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The request body could not be read at all: it is over the host's limit, or it broke off.
    UnreadableBody {
        cause: UnreadableBodyCause,
    },
    /// The query of a GET is missing a parameter or has one that does not read.
    InvalidQuery(QueryError),
    /// An operation id is zero; ids start at 1.
    ZeroOperation,
    /// `queue step 0`: a step serves at least one input.
    ZeroStep,
    InvalidName {
        what: String,
    },
    UnknownSeat {
        seat: u64,
    },
    UnknownMachine {
        machine: String,
    },
    /// The machine is registered but holds no place on the table now (`dump <machine>`).
    MachineNotOnTable {
        machine: String,
    },
    /// No inspect report has this name.
    UnknownReport {
        report: String,
    },
    UnknownState {
        machine: String,
        state: String,
    },
    UnknownPurpose {
        purpose: String,
    },
    /// The saved state given to `force` cannot be read by the machine.
    PayloadUnreadable {
        machine: String,
        detail: String,
    },
    /// The machine reached another state than the one `force` named: give the saved state as
    /// `field=value` words.
    ForcedStateMismatch {
        machine: String,
        named: String,
        reached: String,
    },
    /// `force` could not put the table in the machine; `detail` is the source's sentence when
    /// there is one.
    ForceRefused {
        cause: ForceCause,
        detail: Option<String>,
    },
    /// A port of the table (the broker) could not answer.
    Port {
        port: PortName,
        cause: FailureCause,
    },
    /// This table has no such control (a control of the dev table, not of a hosted one).
    NotAvailable {
        control: DevControl,
    },
    /// No setting has this name.
    UnknownSetting {
        name: String,
    },
    /// The setting's spec refused the value.
    SettingRefused {
        name: String,
        cause: SettingCause,
    },
    /// The setting specs themselves do not read (a defect of the build, not of the value).
    SettingSpecInvalid {
        name: String,
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
    /// A replay names no turn log.
    BlankLog,
    /// Too much, too fast (a rate limit or a seat with its inputs waiting): retry after the
    /// pause, when the queue gave one.
    TooFast {
        retry_after_ms: Option<u64>,
    },
    /// A lane is full: a stall and not a decline (RES22). Retry the same operation id later.
    Stalled,
    /// The table has no running machine, so there is no view to make.
    NoMachineRunning,
    /// A view could not be made: a fault of the host, never of the caller.
    ViewUnavailable {
        cause: ViewCause,
        detail: String,
    },
    /// A wire intent got no receipt for a reason that is not the queue's own answer.
    WireIntent {
        cause: WireIntentCause,
        detail: Option<String>,
    },
    /// The input is admitted and waits (the owner is holding, or it is queued behind others):
    /// nothing has run for it yet, so there is no answer. Retry the same operation id.
    NotServedYet,
}

impl EngineError {
    /// The serde tag of the variant, for audit lines.
    pub fn key(&self) -> &'static str {
        match self {
            Self::MailboxFull => "mailbox_full",
            Self::Stopped => "stopped",
            Self::NoAnswer => "no_answer",
            Self::OwnerPanicked => "owner_panicked",
            Self::NotStarted { .. } => "not_started",
            Self::InvalidBody(_) => "invalid_body",
            Self::UnreadableBody { .. } => "unreadable_body",
            Self::InvalidQuery(_) => "invalid_query",
            Self::ZeroOperation => "zero_operation",
            Self::ZeroStep => "zero_step",
            Self::InvalidName { .. } => "invalid_name",
            Self::UnknownSeat { .. } => "unknown_seat",
            Self::UnknownMachine { .. } => "unknown_machine",
            Self::MachineNotOnTable { .. } => "machine_not_on_table",
            Self::UnknownReport { .. } => "unknown_report",
            Self::UnknownState { .. } => "unknown_state",
            Self::UnknownPurpose { .. } => "unknown_purpose",
            Self::PayloadUnreadable { .. } => "payload_unreadable",
            Self::ForcedStateMismatch { .. } => "forced_state_mismatch",
            Self::ForceRefused { .. } => "force_refused",
            Self::Port { .. } => "port",
            Self::NotAvailable { .. } => "not_available",
            Self::UnknownSetting { .. } => "unknown_setting",
            Self::SettingRefused { .. } => "setting_refused",
            Self::SettingSpecInvalid { .. } => "setting_spec_invalid",
            Self::SettingNeedsRestart { .. } => "setting_needs_restart",
            Self::NotSupportedYet { .. } => "not_supported_yet",
            Self::UnknownLog { .. } => "unknown_log",
            Self::BlankLog => "blank_log",
            Self::TooFast { .. } => "too_fast",
            Self::Stalled => "stalled",
            Self::NoMachineRunning => "no_machine_running",
            Self::ViewUnavailable { .. } => "view_unavailable",
            Self::WireIntent { .. } => "wire_intent",
            Self::NotServedYet => "not_served_yet",
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MailboxFull => formatter.write_str("the owner is busy; try again"),
            Self::Stopped => formatter.write_str("the owner thread has stopped"),
            Self::NoAnswer => formatter.write_str("the owner ended without answering"),
            Self::OwnerPanicked => formatter.write_str(
                "the owner thread panicked and the table is stopped; see the log for the line",
            ),
            Self::NotStarted { cause, detail } => {
                write!(
                    formatter,
                    "the owner did not start ({}): {detail}",
                    cause.key()
                )
            }
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::UnreadableBody {
                cause: UnreadableBodyCause::TooLarge,
            } => formatter.write_str("the request body is over the host's limit"),
            Self::UnreadableBody {
                cause: UnreadableBodyCause::Interrupted,
            } => formatter.write_str("the request body broke off before it was read"),
            Self::InvalidQuery(error) => write!(formatter, "{error}"),
            Self::ZeroOperation => formatter.write_str("an operation id is not zero"),
            Self::ZeroStep => formatter.write_str("a step serves at least one input"),
            Self::InvalidName { what } => write!(formatter, "'{what}' is not a valid name"),
            Self::UnknownSeat { seat } => write!(formatter, "seat {seat} is not at this table"),
            Self::UnknownMachine { machine } => {
                write!(
                    formatter,
                    "no machine '{machine}' is registered and enabled"
                )
            }
            Self::MachineNotOnTable { machine } => {
                write!(formatter, "{machine} holds no place on the table now")
            }
            Self::UnknownReport { report } => write!(
                formatter,
                "there is no report '{report}'; see `inspect` for the list"
            ),
            Self::UnknownState { machine, state } => {
                write!(formatter, "{machine} has no state '{state}'")
            }
            Self::UnknownPurpose { purpose } => {
                write!(formatter, "the purpose '{purpose}' is not registered")
            }
            Self::PayloadUnreadable { machine, detail } => {
                write!(formatter, "that is not a state of {machine}: {detail}")
            }
            Self::ForcedStateMismatch {
                machine,
                named,
                reached,
            } => write!(
                formatter,
                "{machine} starts in '{reached}', not '{named}'; give the saved state as \
                 field=value words"
            ),
            Self::ForceRefused { cause, detail } => {
                let why = match cause {
                    ForceCause::EntryRefused => "the machine refused to be entered",
                    ForceCause::NoActiveScene => "no scene is active for a group machine",
                    ForceCause::SeatLane => "a seat lane is not forced",
                    ForceCause::StartRefused => "the table refused to start the machine",
                    ForceCause::CommitFailed => "the commit failed",
                };
                match detail {
                    Some(detail) => write!(formatter, "the change was refused: {why}: {detail}"),
                    None => write!(formatter, "the change was refused: {why}"),
                }
            }
            Self::Port { port, cause } => write!(
                formatter,
                "the {} port could not answer ({})",
                port.key(),
                cause.key()
            ),
            Self::NotAvailable { control } => {
                write!(formatter, "this table has no {}", control.describe())
            }
            Self::UnknownSetting { name } => write!(formatter, "there is no setting '{name}'"),
            Self::SettingRefused { name, cause } => {
                write!(formatter, "{name} was refused: {}", cause.key())
            }
            Self::SettingSpecInvalid { name } => write!(
                formatter,
                "{name} cannot be set because the setting specs do not read"
            ),
            Self::SettingNeedsRestart { name } => write!(
                formatter,
                "{name} is read once when the owner starts, so it cannot be changed while it runs"
            ),
            Self::NotSupportedYet { what, needs } => {
                write!(formatter, "{what} is not built yet: it needs {needs}")
            }
            Self::UnknownLog { log } => write!(formatter, "there is no turn log '{log}'"),
            Self::BlankLog => formatter.write_str("a replay names the turn log to replay"),
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
            Self::NoMachineRunning => {
                formatter.write_str("no view could be made: no machine is running")
            }
            Self::ViewUnavailable { cause, detail } => write!(
                formatter,
                "no view could be made ({}): {detail}",
                cause.key()
            ),
            Self::WireIntent {
                cause,
                detail: Some(detail),
            } => write!(
                formatter,
                "the wire intent got no receipt ({}): {detail}",
                cause.key()
            ),
            Self::WireIntent {
                cause,
                detail: None,
            } => write!(
                formatter,
                "the wire intent got no receipt ({})",
                cause.key()
            ),
            Self::NotServedYet => formatter.write_str(
                "the input waits in the queue and has not run; retry the same operation id",
            ),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<BodyError> for EngineError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<QueryError> for EngineError {
    fn from(error: QueryError) -> Self {
        Self::InvalidQuery(error)
    }
}

impl From<PortError> for EngineError {
    fn from(error: PortError) -> Self {
        Self::Port {
            port: error.port,
            cause: error.cause,
        }
    }
}

impl RouteError for EngineError {
    /// The owner's busy or stopped state (and a port that could not answer) is a 503, a rate
    /// limit a 429, a fault of the host a 500,
    /// a feature not built yet a 501, and any mistake of the caller a 400.
    fn status(&self) -> StatusCode {
        match self {
            Self::MailboxFull
            | Self::Stopped
            | Self::OwnerPanicked
            | Self::NotStarted { .. }
            | Self::Port { .. }
            | Self::Stalled
            | Self::NotServedYet
            | Self::ForceRefused {
                cause: ForceCause::CommitFailed,
                ..
            } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TooFast { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::UnreadableBody {
                cause: UnreadableBodyCause::TooLarge,
            } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::NoAnswer
            | Self::SettingSpecInvalid { .. }
            | Self::NoMachineRunning
            | Self::ViewUnavailable { .. }
            | Self::WireIntent {
                cause:
                    WireIntentCause::ReceiptInvalid
                    | WireIntentCause::UnknownRefusal
                    | WireIntentCause::MissingDeclineCode
                    | WireIntentCause::DeclineInvalid,
                ..
            } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotSupportedYet { .. } => StatusCode::NOT_IMPLEMENTED,
            Self::InvalidBody(_)
            | Self::UnreadableBody {
                cause: UnreadableBodyCause::Interrupted,
            }
            | Self::InvalidQuery(_)
            | Self::ZeroOperation
            | Self::ZeroStep
            | Self::InvalidName { .. }
            | Self::UnknownSeat { .. }
            | Self::UnknownMachine { .. }
            | Self::MachineNotOnTable { .. }
            | Self::UnknownReport { .. }
            | Self::UnknownState { .. }
            | Self::UnknownPurpose { .. }
            | Self::PayloadUnreadable { .. }
            | Self::ForcedStateMismatch { .. }
            | Self::ForceRefused {
                cause:
                    ForceCause::EntryRefused
                    | ForceCause::NoActiveScene
                    | ForceCause::SeatLane
                    | ForceCause::StartRefused,
                ..
            }
            | Self::NotAvailable { .. }
            | Self::UnknownSetting { .. }
            | Self::SettingRefused { .. }
            | Self::SettingNeedsRestart { .. }
            | Self::UnknownLog { .. }
            | Self::BlankLog
            | Self::WireIntent {
                cause: WireIntentCause::RefusedAtTheEdge,
                ..
            } => StatusCode::BAD_REQUEST,
        }
    }
}

/// The answer to every dev engine call: the host's one envelope with the engine's error.
pub type EngineReply<T> = DevReply<T, EngineError>;
