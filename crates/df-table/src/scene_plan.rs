//! The five stages of the scene builder and the plan that walks one plate job through them (D12
//! to D15, K.2).
//!
//! The stages are Describe, Background, Plate, Locate and Ready. A stage asks for one kind of
//! request, its output is stored, and the next stage starts only from a stored output: the plan
//! refuses `begin` until the stage before it has a stored output, so a picture is never made from
//! a description that was not kept, and Locate never segments a plate nobody holds. Ready is the
//! last step and asks for nothing: it hands the stored plate and its anchors to the playout
//! buffer.
//!
//! Every step asks the table whether the job is still current (`judge_result`, D.2, RES42), so a
//! result for a scene that has moved on is refused and never stored, and a stale job never starts
//! a request it would pay for. A paused table holds a result back (`StaleCause::TablePaused`)
//! instead of discarding it, and the caller tries again on resume. The plan keeps no clock and
//! makes no request: the host does, and reports each outcome here.

use std::fmt;

use crate::event::FailureCause;
use crate::listed::listed_enum;
use crate::plate_job::PlateJob;
use crate::private_data::Narration;
use crate::scene_plan_locate::LocateOutcome;
use crate::table_state::{StaleResult, TableSnapshot};

listed_enum! {
    /// A step of the scene builder, in order.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum SceneStage {
        Describe,
        Background,
        Plate,
        Locate,
        Ready,
    }
}

impl SceneStage {
    pub fn key(self) -> &'static str {
        match self {
            Self::Describe => "describe",
            Self::Background => "background",
            Self::Plate => "plate",
            Self::Locate => "locate",
            Self::Ready => "ready",
        }
    }

    /// The stage whose stored output this one starts from: none for the first.
    pub fn needs(self) -> Option<Self> {
        match self {
            Self::Describe => None,
            Self::Background => Some(Self::Describe),
            Self::Plate => Some(Self::Background),
            Self::Locate => Some(Self::Plate),
            Self::Ready => Some(Self::Locate),
        }
    }
}

/// A picture the broker made, as the plan keeps it: the bytes live in the media store, never
/// here. `handle` is the broker request that made it, which is unique, so it also names the
/// picture for the next stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredPicture {
    pub handle: u64,
    pub width: u32,
    pub height: u32,
}

/// What a stage produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageOutput {
    Description(Narration),
    Background(StoredPicture),
    Plate(StoredPicture),
    Locate(LocateOutcome),
}

impl StageOutput {
    pub fn stage(&self) -> SceneStage {
        match self {
            Self::Description(_) => SceneStage::Describe,
            Self::Background(_) => SceneStage::Background,
            Self::Plate(_) => SceneStage::Plate,
            Self::Locate(_) => SceneStage::Locate,
        }
    }
}

/// The outputs stored so far. The description and the background belong to the scene and are
/// carried into every job of it; the plate and the anchors belong to one job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageOutputs {
    pub description: Option<Narration>,
    pub background: Option<StoredPicture>,
    pub plate: Option<StoredPicture>,
    pub locate: Option<LocateOutcome>,
}

impl StageOutputs {
    pub fn has(&self, stage: SceneStage) -> bool {
        match stage {
            SceneStage::Describe => self.description.is_some(),
            SceneStage::Background => self.background.is_some(),
            SceneStage::Plate => self.plate.is_some(),
            SceneStage::Locate => self.locate.is_some(),
            SceneStage::Ready => false,
        }
    }

    /// The part a scene keeps for its next job: its description and its background.
    pub fn carried_forward(&self) -> Self {
        Self {
            description: self.description.clone(),
            background: self.background,
            plate: None,
            locate: None,
        }
    }
}

/// A stage that failed, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageFailure {
    pub stage: SceneStage,
    pub cause: FailureCause,
}

/// A finished plate: the picture, where everything is in it, and when the clock asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyPlate {
    pub job: PlateJob,
    pub plate: StoredPicture,
    pub locate: LocateOutcome,
    pub ready_ms: u64,
}

impl ReadyPlate {
    /// How long the plate took from the tick that asked for it.
    pub fn build_ms(&self) -> u64 {
        self.ready_ms.saturating_sub(self.job.tick_ms())
    }
}

/// Why the plan refused a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// Another stage is still running.
    StageRunning {
        running: SceneStage,
    },
    /// The stage before this one has no stored output.
    MissingInput {
        stage: SceneStage,
        needs: SceneStage,
    },
    AlreadyStored {
        stage: SceneStage,
    },
    /// The output, or the failure, is for a stage that is not the one running.
    NotRunning {
        stage: SceneStage,
    },
    /// The job already ended in a failure, or already finished.
    Ended,
    /// The job is no longer current (D.2); nothing was stored or started.
    Stale(StaleResult),
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StageRunning { running } => {
                write!(formatter, "the {} stage is still running", running.key())
            }
            Self::MissingInput { stage, needs } => write!(
                formatter,
                "the {} stage starts from a stored {} output",
                stage.key(),
                needs.key()
            ),
            Self::AlreadyStored { stage } => {
                write!(formatter, "the {} output is already stored", stage.key())
            }
            Self::NotRunning { stage } => {
                write!(formatter, "the {} stage is not running", stage.key())
            }
            Self::Ended => formatter.write_str("the job has ended"),
            Self::Stale(stale) => write!(formatter, "the job is stale ({})", stale.cause.key()),
        }
    }
}

impl std::error::Error for PlanError {}

/// One plate job's walk through the stages.
#[derive(Debug, Clone)]
pub struct StagePlan {
    job: PlateJob,
    outputs: StageOutputs,
    running: Option<SceneStage>,
    failure: Option<StageFailure>,
    finished: bool,
}

impl StagePlan {
    /// A plan for `job` that starts with what its scene already stored (`carried`): a job after
    /// the first begins at Plate.
    pub fn for_job(job: PlateJob, carried: &StageOutputs) -> Self {
        Self {
            job,
            outputs: carried.carried_forward(),
            running: None,
            failure: None,
            finished: false,
        }
    }

    pub fn job(&self) -> &PlateJob {
        &self.job
    }

    pub fn outputs(&self) -> &StageOutputs {
        &self.outputs
    }

    pub fn running(&self) -> Option<SceneStage> {
        self.running
    }

    pub fn failure(&self) -> Option<StageFailure> {
        self.failure
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Whether the job has no more to do: it failed or finished.
    pub fn has_ended(&self) -> bool {
        self.failure.is_some() || self.finished
    }

    /// The first stage with no stored output, when nothing is running and the job has not ended.
    pub fn next_stage(&self) -> Option<SceneStage> {
        if self.running.is_some() || self.has_ended() {
            return None;
        }
        SceneStage::ALL
            .iter()
            .copied()
            .find(|stage| !self.outputs.has(*stage))
    }

    fn judge(&self, snapshot: &TableSnapshot) -> Result<(), PlanError> {
        snapshot
            .judge_result(&self.job.stamp())
            .map_err(PlanError::Stale)
    }

    /// Starts `stage`, which must be the next one and must have its input stored. A stale job
    /// starts nothing.
    pub fn begin(&mut self, snapshot: &TableSnapshot, stage: SceneStage) -> Result<(), PlanError> {
        if self.has_ended() {
            return Err(PlanError::Ended);
        }
        if let Some(running) = self.running {
            return Err(PlanError::StageRunning { running });
        }
        if self.outputs.has(stage) {
            return Err(PlanError::AlreadyStored { stage });
        }
        if let Some(needs) = stage.needs()
            && !self.outputs.has(needs)
        {
            return Err(PlanError::MissingInput { stage, needs });
        }
        self.judge(snapshot)?;
        self.running = Some(stage);
        Ok(())
    }

    /// Stores the running stage's output, unless the job is stale (D.2): then nothing is stored.
    pub fn store(
        &mut self,
        snapshot: &TableSnapshot,
        output: StageOutput,
    ) -> Result<(), PlanError> {
        let stage = output.stage();
        if self.running != Some(stage) {
            return Err(PlanError::NotRunning { stage });
        }
        self.judge(snapshot)?;
        match output {
            StageOutput::Description(text) => self.outputs.description = Some(text),
            StageOutput::Background(picture) => self.outputs.background = Some(picture),
            StageOutput::Plate(picture) => self.outputs.plate = Some(picture),
            StageOutput::Locate(outcome) => self.outputs.locate = Some(outcome),
        }
        self.running = None;
        Ok(())
    }

    /// Ends the job: the running stage failed for `cause`.
    pub fn fail(
        &mut self,
        stage: SceneStage,
        cause: FailureCause,
    ) -> Result<StageFailure, PlanError> {
        if self.running != Some(stage) {
            return Err(PlanError::NotRunning { stage });
        }
        let failure = StageFailure { stage, cause };
        self.running = None;
        self.failure = Some(failure);
        Ok(failure)
    }

    /// The last step: the stored plate and its anchors, as a finished plate for the buffer.
    pub fn finish(
        &mut self,
        snapshot: &TableSnapshot,
        now_ms: u64,
    ) -> Result<ReadyPlate, PlanError> {
        if self.has_ended() {
            return Err(PlanError::Ended);
        }
        if let Some(running) = self.running {
            return Err(PlanError::StageRunning { running });
        }
        let (Some(plate), Some(locate)) = (self.outputs.plate, self.outputs.locate.clone()) else {
            let needs = if self.outputs.plate.is_some() {
                SceneStage::Locate
            } else {
                SceneStage::Plate
            };
            return Err(PlanError::MissingInput {
                stage: SceneStage::Ready,
                needs,
            });
        };
        self.judge(snapshot)?;
        self.finished = true;
        Ok(ReadyPlate {
            job: self.job.clone(),
            plate,
            locate,
            ready_ms: now_ms,
        })
    }
}
