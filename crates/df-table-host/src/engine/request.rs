//! What a dev caller sends: the input to submit, the machine to force, the audience to project for
//! and the control of the queue.
//!
//! These are the request bodies of the routes and what the command line builds. They carry names
//! and numbers as the caller typed them; the owner checks every one (a name is a valid name, a
//! seat is at the table) and refuses with a typed `EngineError`, so nothing here is trusted. A
//! free line is player text: it travels as a string to the owner, which wraps it in `PlayerText`
//! at once and never records it.

use df_table::identity::SeatId;
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
    /// A proposal of the engine (`npc`, `world`, `story`, `encounter`, `timer` or `machine`).
    Proposal { origin: String, proposer: String },
}

/// One input to submit. `operation` is the operation id the sender minted (a duplicate of it is a
/// retry); left out, the owner mints one. `view` is the revision the sender drew the input from;
/// left out, it is the current one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRequest {
    pub input: InputBody,
    #[serde(default)]
    pub operation: Option<u64>,
    #[serde(default)]
    pub view: Option<u64>,
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
}

/// What to do with the owner's serving of the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case")]
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
#[serde(tag = "control", rename_all = "snake_case")]
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
#[serde(tag = "control", rename_all = "snake_case")]
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

/// A fault to put in force (H.16 `chaos <dependency> <fault>`): the next `count` commits fail, or
/// the interpreter is down until it is put back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "dependency", rename_all = "snake_case")]
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
#[serde(tag = "control", rename_all = "snake_case")]
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
