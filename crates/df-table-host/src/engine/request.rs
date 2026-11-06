//! What a dev caller sends: the input to submit, the machine to force, the audience to project for
//! and the control of the queue.
//!
//! These are the request bodies of the routes and what the command line builds. They carry names
//! and numbers as the caller typed them; the owner checks every one (a name is a valid name, a
//! seat is at the table) and refuses with a typed `EngineError`, so nothing here is trusted. A
//! free line is player text: it travels as a string to the owner, which wraps it in `PlayerText`
//! at once and never records it.

use std::fmt;
use std::num::NonZeroU128;

use df_table::envelope::ProposalOrigin;
use df_table::identity::{OperationId, SeatId};
use df_table::ports::Audience;
use serde::{Deserialize, Serialize};

/// What one submitted input is. A seat's inputs name the seat; an engine input names none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputBody {
    /// A tap on an offered choice.
    Tap { seat: u64, action: String },
    /// A line typed into the phone: it is interpreted into one of the seat's choices.
    Text { seat: u64, text: String },
    /// A line the hero speaks aloud.
    Say { seat: u64, text: String },
    /// A message to one other seat.
    Private { seat: u64, to: u64, text: String },
    /// The pause card.
    Pause { seat: u64 },
    /// A timer of the engine fired.
    Timer { name: String },
    /// A proposal of the engine.
    Proposal {
        origin: ProposalSource,
        proposer: String,
    },
}

/// Where an engine proposal comes from, as the dev wire writes it: `df_table`'s `ProposalOrigin`
/// with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalSource {
    Npc,
    World,
    Story,
    Encounter,
    Timer,
    Machine,
}

impl ProposalSource {
    pub const ALL: [Self; 6] = [
        Self::Npc,
        Self::World,
        Self::Story,
        Self::Encounter,
        Self::Timer,
        Self::Machine,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Npc => "npc",
            Self::World => "world",
            Self::Story => "story",
            Self::Encounter => "encounter",
            Self::Timer => "timer",
            Self::Machine => "machine",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.key() == key)
    }

    /// The pure crate's origin this source stands for.
    pub fn origin(self) -> ProposalOrigin {
        match self {
            Self::Npc => ProposalOrigin::Npc,
            Self::World => ProposalOrigin::World,
            Self::Story => ProposalOrigin::Story,
            Self::Encounter => ProposalOrigin::Encounter,
            Self::Timer => ProposalOrigin::Timer,
            Self::Machine => ProposalOrigin::Machine,
        }
    }
}

/// One input to submit. `operation` is the operation id the sender minted (a duplicate of it is a
/// retry); left out, the owner mints one. `operation_id` is the same for a sender that has all 128
/// bits (a wire intent, whose id is 16 bytes) and wins when both are given. `view` is the revision
/// the sender drew the input from; left out, it is the current one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRequest {
    pub input: InputBody,
    #[serde(default)]
    pub operation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<u128>,
    #[serde(default)]
    pub view: Option<u64>,
    /// The machine the sender drew the input from (a wire intent names it). When given and the
    /// table's focus is another machine, the input is declined `window_closed` (a stale offer)
    /// and not run. Left out, nothing is compared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// Moves the table to `machine` in `state`. `payload` is the machine's saved state, for a state
/// the machine does not start in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForceRequest {
    pub machine: String,
    pub state: String,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

/// Who a projection is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "audience", rename_all = "snake_case")]
pub enum AudienceSpec {
    Tv,
    Seat { seat: u64 },
}

impl AudienceSpec {
    pub fn audience(self) -> Audience {
        match self {
            Self::Tv => Audience::Public,
            Self::Seat { seat } => Audience::Seat(SeatId::new(seat)),
        }
    }

    /// The audience as a query and the command line write it: `tv` or a seat number.
    pub fn parameter(self) -> String {
        match self {
            Self::Tv => "tv".to_owned(),
            Self::Seat { seat } => seat.to_string(),
        }
    }

    /// The audience `parameter` wrote.
    pub fn from_parameter(text: &str) -> Result<Self, AudienceError> {
        match text {
            "tv" => Ok(Self::Tv),
            seat => seat
                .parse::<u64>()
                .map(|seat| Self::Seat { seat })
                .map_err(|_| AudienceError),
        }
    }
}

/// A text that is neither `tv` nor a seat number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudienceError;

impl fmt::Display for AudienceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an audience is tv or a seat number")
    }
}

impl std::error::Error for AudienceError {}

/// An audience in a query (`?audience=tv` or `?audience=2`): the text `AudienceSpec::parameter`
/// writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct AudienceParameter(pub AudienceSpec);

impl TryFrom<String> for AudienceParameter {
    type Error = AudienceError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        AudienceSpec::from_parameter(&text).map(Self)
    }
}

/// The query of the projection and the wire view: who the view is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudienceQuery {
    pub audience: AudienceParameter,
}

/// The query of `viable`: the seat asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViableQuery {
    pub seat: u64,
}

/// An operation as a person writes it: the number `--op` took, or the 32 hexadecimal digits the
/// reports print. It is written back as the 32 digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct TraceOperation(OperationId);

impl TraceOperation {
    /// The width of an operation written as hexadecimal digits.
    const HEXADECIMAL_DIGITS: usize = 32;

    pub fn new(text: &str) -> Result<Self, TraceOperationError> {
        if text.len() == Self::HEXADECIMAL_DIGITS {
            return OperationId::from_hex(text)
                .map(Self)
                .map_err(|_| TraceOperationError);
        }
        text.parse::<u64>()
            .ok()
            .and_then(|number| NonZeroU128::new(u128::from(number)))
            .map(|number| Self(OperationId::from_nonzero_u128(number)))
            .ok_or(TraceOperationError)
    }

    pub fn id(self) -> OperationId {
        self.0
    }
}

impl From<OperationId> for TraceOperation {
    fn from(id: OperationId) -> Self {
        Self(id)
    }
}

impl From<TraceOperation> for String {
    fn from(operation: TraceOperation) -> Self {
        operation.0.to_string()
    }
}

impl TryFrom<String> for TraceOperation {
    type Error = TraceOperationError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

/// A text that is neither a number from 1 nor 32 hexadecimal digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceOperationError;

impl fmt::Display for TraceOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an operation is a number from 1 or 32 hexadecimal digits")
    }
}

impl std::error::Error for TraceOperationError {}

/// The query of `trace`: the operation to follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceQuery {
    pub operation: TraceOperation,
}

/// The body of `broker request`: the purpose to request a resource for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerRequest {
    pub purpose: String,
}

/// What to do with the owner's serving of the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueueControl {
    /// Admit inputs but serve none, so the order they wait in can be read.
    Hold,
    /// Serve everything waiting, and keep serving as inputs arrive.
    Release,
    /// Serve one batch of at most `count` inputs while held.
    Step { count: usize },
}

/// A control of the owner's clock (H.16 `clock now, advance or freeze`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClockControl {
    Now,
    /// Moves the clock forward by `ms` and keeps it moving.
    Advance {
        ms: u64,
    },
    /// Stops the clock where it is, so time passes only by `advance`.
    Freeze,
    /// Lets a frozen clock run again from where it stands.
    Resume,
}

/// A control of the dice (H.16 `dice seed or force`). A forced value is marked `driven=true` in
/// the audit line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiceControl {
    /// Restarts the seeded sequence.
    Seed { seed: u64 },
    /// Queues values to be rolled next, in order.
    Force { values: Vec<u16> },
}

/// Which way a commit is made to fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitFault {
    Unavailable,
    Refused,
    /// The write takes longer than the commit deadline.
    Deadline,
}

impl CommitFault {
    pub const ALL: [Self; 3] = [Self::Unavailable, Self::Refused, Self::Deadline];

    pub fn key(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Refused => "refused",
            Self::Deadline => "deadline",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|fault| fault.key() == key)
    }
}

/// A fault to put in force (H.16 `chaos <dependency> <fault>`): the next `count` commits fail, or
/// the interpreter is down until it is put back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "dependency", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChaosRequest {
    Commit { fault: CommitFault, count: u32 },
    Interpreter { down: bool },
}

/// Which class of event `history` is asked to show: the letter the plan uses or its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassFilter {
    /// Class A: always on and never sampled.
    #[serde(alias = "a")]
    Always,
    /// Class B: verbose detail.
    #[serde(alias = "b")]
    Verbose,
}

/// What `history` is asked for: the newest events the host kept, optionally of one area or class.
/// `limit` left out is `DEFAULT_HISTORY_RECORDS`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRequest {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub area: Option<String>,
    #[serde(default)]
    pub class: Option<ClassFilter>,
}

/// What to do with the settings in force (H.16 `config show|set`, J.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigControl {
    Show,
    /// Changes one setting. The owner checks the name and the value against the setting's spec
    /// and refuses with a typed error that names the cause.
    Set {
        name: String,
        value: u64,
    },
}

/// A turn log to replay (H.16 `replay <log> [--until seq] [--diff]`). `log` names the log; the
/// host does not read it, the replay behind the seam does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayRequest {
    pub log: String,
    #[serde(default)]
    pub until: Option<u64>,
    #[serde(default)]
    pub diff: bool,
}
