//! Everything that crosses the scene builder's dev routes, in one place: the requests the verbs
//! send, the answers the workbench gives and the errors it refuses with.
//!
//! The routes and the command line read and write these types, so a rename breaks the build instead
//! of a running command. Every value is typed: a closed enum of the pure crate that already
//! serializes is used as it is, and one that does not has a wire twin here (`BuildEnd`,
//! `LookaheadTake`). Inbound bodies refuse unknown fields. Nothing here holds player text: a line
//! is an audit line, which holds ids, keys and counts only, and a place is a short display name.

use std::fmt;

use axum::http::StatusCode;
use df_table::event::FailureCause;
use df_table::lookahead::settings::LookaheadSettingError;
use df_table::lookahead::{CERTAIN_PERMILLE, Expectation, Take};
use df_table::plate::buffer::{BufferReport, StageMoment};
use df_table::plate::cost::CostView;
use df_table::plate::scheduler::{JobEnd, SchedulerReport};
use df_table::plate::timing::{TimingError, TimingSetting};
use df_table::scene_plan::SceneStage;
use df_table::table_state::DisplayName;
use serde::{Deserialize, Serialize};

use super::limits::MAXIMUM_MINIMUM_ON_SCREEN_MS;
use super::lookahead::request::LookaheadSettingName;
use super::report::{CardReport, JobReport, LocateReport, SceneReport};
use crate::broker::report::QueuesReport;
use crate::dev_http::{BodyError, DevReply, RouteError};

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

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.key() == key)
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

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|moment| moment.key() == key)
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
    pub place: DisplayName,
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

/// How a plate job ended, on the wire: the twin of `df_table`'s `JobEnd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildEnd {
    Ready,
    Failed,
    Dropped,
    Stale,
}

impl BuildEnd {
    pub const ALL: [Self; 4] = [Self::Ready, Self::Failed, Self::Dropped, Self::Stale];

    pub fn key(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Dropped => "dropped",
            Self::Stale => "stale",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|end| end.key() == key)
    }
}

impl From<JobEnd> for BuildEnd {
    fn from(end: JobEnd) -> Self {
        match end {
            JobEnd::Ready => Self::Ready,
            JobEnd::Failed => Self::Failed,
            JobEnd::Dropped => Self::Dropped,
            JobEnd::Stale => Self::Stale,
        }
    }
}

/// What the look-ahead had built for a scene when the table entered it, on the wire: the twin of
/// `df_table`'s `Take` without the details, and without `miss`, which is no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookaheadTake {
    Hit,
    Redress,
    Stale,
    Late,
    Failed,
}

impl LookaheadTake {
    pub const ALL: [Self; 5] = [
        Self::Hit,
        Self::Redress,
        Self::Stale,
        Self::Late,
        Self::Failed,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Redress => "redress",
            Self::Stale => "stale",
            Self::Late => "late",
            Self::Failed => "failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|take| take.key() == key)
    }

    /// What `take` found, or `None` when nothing was built for the scene.
    pub fn of(take: &Take) -> Option<Self> {
        match take {
            Take::Hit(_) => Some(Self::Hit),
            Take::Redress(_) => Some(Self::Redress),
            Take::Stale { .. } => Some(Self::Stale),
            Take::Late => Some(Self::Late),
            Take::Failed { .. } => Some(Self::Failed),
            Take::Miss => None,
        }
    }
}

/// How a `scene build` ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BuildOutcome {
    /// The first plate is on screen.
    PlateOnScreen { at_ms: u64 },
    /// The job ended without a plate: the stage that failed and the cause, or why it was dropped.
    Ended {
        end: BuildEnd,
        stage: Option<SceneStage>,
        cause: Option<FailureCause>,
    },
    /// The workbench stopped waiting: nothing was left to run and no plate came.
    NoPlate,
}

/// The answer to `scene build`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildReply {
    pub place: DisplayName,
    pub scene: u64,
    pub generation: u64,
    pub outcome: BuildOutcome,
    /// Workbench time spent, in milliseconds.
    pub elapsed_ms: u64,
    /// When play went on to the establishing card, after the budget (K.2).
    pub card_due_after_ms: Option<u64>,
    pub lines: Vec<TimedLine>,
    pub scene_report: Option<SceneReport>,
    /// What the look-ahead found for the scene; none when nothing was built ahead for it.
    #[serde(default)]
    pub lookahead: Option<LookaheadTake>,
}

/// The answer to `plates run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunReply {
    pub seconds: u64,
    pub moment: DevMoment,
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
    /// The plate timings the workbench runs with.
    pub timing: PlateTimingReport,
    /// The queues and running calls of the workbench's own broker.
    pub broker: QueuesReport,
}

/// A plate timing `scene timing set` changes. The dissolve of a swap is not one: `df-table`'s
/// playout buffer fixes it at its minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlateTimingSetting {
    /// Seconds between plate ticks (`plate_interval_s`).
    IntervalS,
    /// The reading time a plate stays up at least before the next replaces it.
    MinimumOnScreenMs,
}

impl PlateTimingSetting {
    pub const ALL: [Self; 2] = [Self::IntervalS, Self::MinimumOnScreenMs];

    pub fn key(self) -> &'static str {
        match self {
            Self::IntervalS => "interval_s",
            Self::MinimumOnScreenMs => "minimum_on_screen_ms",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|setting| setting.key() == key)
    }
}

/// The body of `scene timing set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingRequest {
    pub setting: PlateTimingSetting,
    pub value: u64,
}

/// The plate timings the workbench runs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateTimingReport {
    pub interval_s: u32,
    /// The interval the scene on screen swaps its plates at: the one it was entered with, which a
    /// later `scene timing set interval_s` leaves alone until the next scene. None with no scene.
    pub on_screen_interval_s: Option<u32>,
    pub minimum_on_screen_ms: u64,
    /// How long the newest swap dissolved; none before the first swap.
    pub dissolve_ms: Option<u64>,
    /// How old the oldest outstanding plate may get before the "picture catching up" mark shows
    /// (`image_stale_limit_s`, RES10).
    pub catch_up_ms: u64,
}

/// The answer to `scene timing set`: the timing before, the timings now and the audit line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingReply {
    pub setting: PlateTimingSetting,
    pub previous: u64,
    pub timing: PlateTimingReport,
    pub lines: Vec<TimedLine>,
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

/// Which part of the workbench failed: the dev table, the pacer, the purpose table or the
/// look-ahead planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbenchCause {
    World,
    Pacer,
    Purposes,
    Lookahead,
}

impl WorkbenchCause {
    pub const ALL: [Self; 4] = [Self::World, Self::Pacer, Self::Purposes, Self::Lookahead];

    pub fn key(self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Pacer => "pacer",
            Self::Purposes => "purposes",
            Self::Lookahead => "lookahead",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

/// Why a scene dev call was refused. Each variant says what the caller does next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum SceneBuilderError {
    /// The request body is not what the route reads (a place that is not a name included).
    InvalidBody(BodyError),
    /// A request was made before any scene was built.
    NothingBuilt,
    /// `scene lookahead set` with a value outside the setting's range.
    InvalidSetting {
        setting: LookaheadSettingName,
        value: u64,
    },
    /// `scene timing set` with a value outside the setting's range.
    InvalidTiming {
        setting: PlateTimingSetting,
        value: u64,
    },
    /// `scene timing set` while the scene on screen has plate jobs queued or in flight: let
    /// them finish (`plates run <seconds> quiet`) and ask again.
    Building,
    /// A look-ahead target that slows or fails a stage the look-ahead does not build.
    InvalidLookaheadStage { stage: DevStage },
    /// A look-ahead target whose likelihood does not fit its kind: a certain kind takes none, a
    /// likely kind needs one of 0 to 1,000 thousandths.
    InvalidLikelihood {
        kind: Expectation,
        likelihood_permille: Option<u16>,
    },
    /// The workbench failed in one of its parts. `detail` is that part's sentence.
    ///
    /// Convention exception: the typed source (`WorkbenchError`) is not a wire value, because its
    /// sources in df-table and the broker carry no serde, so the reply keeps its closed `cause`
    /// and its sentence. `WorkbenchError::failed` is the one explicit place that makes it.
    Workbench {
        cause: WorkbenchCause,
        detail: String,
    },
    /// The routes have nothing behind them: the workbench never started. `detail` is why.
    NotStarted {
        cause: WorkbenchCause,
        detail: String,
    },
    /// The blocking task stopped without an answer (it panicked or the runtime is stopping).
    Stopped,
}

impl SceneBuilderError {
    /// The serde name of the variant, for audit lines.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::NothingBuilt => "nothing_built",
            Self::InvalidSetting { .. } => "invalid_setting",
            Self::InvalidTiming { .. } => "invalid_timing",
            Self::Building => "building",
            Self::InvalidLookaheadStage { .. } => "invalid_lookahead_stage",
            Self::InvalidLikelihood { .. } => "invalid_likelihood",
            Self::Workbench { .. } => "workbench",
            Self::NotStarted { .. } => "not_started",
            Self::Stopped => "stopped",
        }
    }

    fn write_likelihood(
        formatter: &mut fmt::Formatter<'_>,
        kind: Expectation,
        likelihood_permille: Option<u16>,
    ) -> fmt::Result {
        let kind_key = kind.key();
        match (kind.is_certain(), likelihood_permille) {
            (true, _) => write!(
                formatter,
                "a {kind_key} scene is certain and takes no likelihood"
            ),
            (false, None) => write!(
                formatter,
                "a {kind_key} scene needs a likelihood in thousandths"
            ),
            (false, Some(permille)) => write!(
                formatter,
                "a likelihood is 0 to {CERTAIN_PERMILLE} thousandths, not {permille}"
            ),
        }
    }
}

impl fmt::Display for SceneBuilderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::NothingBuilt => {
                formatter.write_str("no scene has been built yet: try scene build <place>")
            }
            Self::InvalidSetting { setting, value } => match setting.numeric() {
                Some(numeric) => write!(
                    formatter,
                    "{}",
                    LookaheadSettingError {
                        setting: numeric,
                        value: *value
                    }
                ),
                None => write!(formatter, "lookahead_enabled is 0 or 1, not {value}"),
            },
            Self::InvalidTiming { setting, value } => match setting {
                PlateTimingSetting::IntervalS => write!(
                    formatter,
                    "{}",
                    TimingError {
                        setting: TimingSetting::PlateIntervalS,
                        value: *value
                    }
                ),
                PlateTimingSetting::MinimumOnScreenMs => write!(
                    formatter,
                    "minimum_on_screen_ms is 0 to {MAXIMUM_MINIMUM_ON_SCREEN_MS} ms, not {value}"
                ),
            },
            Self::Building => formatter.write_str(
                "the scene on screen has plate jobs out: let them finish (plates run <seconds> \
                 quiet) and set the timing again",
            ),
            Self::InvalidLookaheadStage { stage } => write!(
                formatter,
                "only describe and background are built ahead, not {}",
                stage.key()
            ),
            Self::InvalidLikelihood {
                kind,
                likelihood_permille,
            } => Self::write_likelihood(formatter, *kind, *likelihood_permille),
            Self::Workbench { detail, .. } => {
                write!(formatter, "the workbench failed: {detail}")
            }
            Self::NotStarted { detail, .. } => {
                write!(formatter, "the scene workbench did not start: {detail}")
            }
            Self::Stopped => {
                formatter.write_str("the workbench stopped before it answered; ask again")
            }
        }
    }
}

impl std::error::Error for SceneBuilderError {}

impl RouteError for SceneBuilderError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::NothingBuilt
            | Self::InvalidSetting { .. }
            | Self::InvalidTiming { .. }
            | Self::InvalidLookaheadStage { .. }
            | Self::InvalidLikelihood { .. } => StatusCode::BAD_REQUEST,
            Self::Building => StatusCode::CONFLICT,
            Self::Workbench { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotStarted { .. } | Self::Stopped => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl From<BodyError> for SceneBuilderError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

/// The answer to every scene dev call: `done` with the value, or `rejected`.
pub type SceneReply<T> = DevReply<T, SceneBuilderError>;
