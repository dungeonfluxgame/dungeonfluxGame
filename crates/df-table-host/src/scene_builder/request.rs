//! Everything that crosses the scene builder's dev routes, in one place: the requests the verbs
//! send, the answers the workbench gives and the errors it refuses with.
//!
//! The routes and the command line read and write these types, so a rename breaks the build instead
//! of a running command. A closed enum of the pure crate that already serializes is used as it is;
//! a key that does not is its `key()` text. Nothing here holds player text: a line is an audit line,
//! which holds ids, keys and counts only.

use std::fmt;

use df_table::plate_buffer::{BufferReport, StageMoment};
use df_table::plate_cost::CostView;
use df_table::plate_scheduler::SchedulerReport;
use df_table::scene_plan::SceneStage;
use serde::{Deserialize, Serialize};

use super::report::{CardReport, JobReport, LocateReport, SceneReport};

/// The stages a dev call can slow down or make fail: the four that ask the broker for something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevStage {
    Describe,
    Background,
    Plate,
    Locate,
}

impl DevStage {
    pub const ALL: [Self; 4] = [Self::Describe, Self::Background, Self::Plate, Self::Locate];

    pub fn key(self) -> &'static str {
        self.scene_stage().key()
    }

    pub fn scene_stage(self) -> SceneStage {
        match self {
            Self::Describe => SceneStage::Describe,
            Self::Background => SceneStage::Background,
            Self::Plate => SceneStage::Plate,
            Self::Locate => SceneStage::Locate,
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.key() == text)
    }
}

/// What the stage conductor says the screen is doing, for `plates run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevMoment {
    #[default]
    LineBoundary,
    MidLine,
    DiceOrResultCard,
    RevealBeat,
}

impl DevMoment {
    pub const ALL: [Self; 4] = [
        Self::LineBoundary,
        Self::MidLine,
        Self::DiceOrResultCard,
        Self::RevealBeat,
    ];

    pub fn stage_moment(self) -> StageMoment {
        match self {
            Self::LineBoundary => StageMoment::LineBoundary,
            Self::MidLine => StageMoment::MidLine,
            Self::DiceOrResultCard => StageMoment::DiceOrResultCard,
            Self::RevealBeat => StageMoment::RevealBeat,
        }
    }

    pub fn key(self) -> &'static str {
        self.stage_moment().key()
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|moment| moment.key() == text)
    }
}

/// A stage made slow: every call of it takes `seconds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlowStage {
    pub stage: DevStage,
    pub seconds: u64,
}

/// The body of `scene build`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRequest {
    pub place: String,
    /// The provider refuses this stage, so it fails at once (a refusal is never retried).
    #[serde(default)]
    pub fail: Option<DevStage>,
    #[serde(default)]
    pub slow: Option<SlowStage>,
}

/// The body of `plates run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub seconds: u64,
    /// Nothing visible changes, so the quiet table pays for no plate (RES9).
    #[serde(default)]
    pub quiet: bool,
    #[serde(default)]
    pub moment: DevMoment,
}

/// One audit line and when it was recorded, in the workbench's own time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedLine {
    pub at_ms: u64,
    pub line: String,
}

/// How a `scene build` ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BuildOutcome {
    /// The first plate is on screen.
    PlateOnScreen { at_ms: u64 },
    /// The job ended without a plate: the stage that failed and the cause, or why it was dropped.
    Ended {
        end: String,
        stage: Option<String>,
        cause: Option<String>,
    },
    /// The workbench stopped waiting: nothing was left to run and no plate came.
    NoPlate,
}

/// The answer to `scene build`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildReply {
    pub place: String,
    pub scene: u64,
    pub generation: u64,
    pub outcome: BuildOutcome,
    /// Workbench time spent, in milliseconds.
    pub elapsed_ms: u64,
    /// When play went on to the establishing card, after the budget (K.2).
    pub card_due_after_ms: Option<u64>,
    pub lines: Vec<TimedLine>,
    pub scene_report: Option<SceneReport>,
}

/// The answer to `plates run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunReply {
    pub seconds: u64,
    pub moment: String,
    pub quiet: bool,
    pub lines: Vec<TimedLine>,
    pub buffer: BufferReport,
}

/// The answer to `plates`: the clock, the queue, what is in flight and the price of the interval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatesReport {
    pub now_ms: u64,
    pub scheduler: Option<SchedulerReport>,
    pub in_flight: Vec<JobReport>,
    pub cost: Option<CostView>,
    /// The mean time plates took from their tick to ready, over the plates finished so far.
    pub mean_build_ms: Option<u64>,
    pub plates_finished: u64,
}

/// The answer to `scene show`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneShow {
    pub now_ms: u64,
    pub scenes: Vec<SceneReport>,
    /// What Locate decided for the plate on screen.
    pub on_screen: Option<LocateReport>,
}

/// The answer to `buffer show`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BufferShow {
    pub now_ms: u64,
    pub buffer: Option<BufferReport>,
    pub card: Option<CardReport>,
}

/// Why a scene dev call was refused. Each variant says what the caller does next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum SceneError {
    /// The place name is not a valid name.
    InvalidName { what: String },
    /// A request was made before any scene was built.
    NothingBuilt,
    /// The workbench could not start or could not change its table.
    Workbench { reason: String },
    /// The request body is wrong.
    BadRequest { reason: String },
    /// The routes have nothing behind them: the workbench never started.
    NotStarted { reason: String },
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName { what } => write!(formatter, "{what} is not a valid name"),
            Self::NothingBuilt => {
                formatter.write_str("no scene has been built yet: try scene build <place>")
            }
            Self::Workbench { reason } => write!(formatter, "the workbench failed: {reason}"),
            Self::BadRequest { reason } => write!(formatter, "the request is wrong: {reason}"),
            Self::NotStarted { reason } => {
                write!(formatter, "the scene workbench did not start: {reason}")
            }
        }
    }
}

impl std::error::Error for SceneError {}

/// The answer to every scene dev call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SceneReply<T> {
    Done { value: T },
    Rejected { error: SceneError, message: String },
}

impl<T> SceneReply<T> {
    pub fn from_result(result: Result<T, SceneError>) -> Self {
        match result {
            Ok(value) => Self::Done { value },
            Err(error) => {
                let message = error.to_string();
                Self::Rejected { error, message }
            }
        }
    }
}
