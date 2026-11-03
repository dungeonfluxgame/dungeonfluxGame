//! The plate pacer: the owner's one call that runs a scene's clock, builder and screen together
//! (D16 to D18, K.3).
//!
//! `pump` is what the owner calls on its wall clock, as often as it likes, and it never waits. It
//! lets the plate scheduler tick if a tick is due, hands the jobs the scheduler queued to the scene
//! builder in order (telling the playout buffer to expect each, with the tick time lateness is
//! measured from), lets the builder move every job as far as the broker's answers allow, gives the
//! finished plates to the buffer and tells it which jobs gave up, and then looks at the buffer: a
//! plate swaps in only where K.3 allows. The clock requests; the buffer decides when to show.
//!
//! The owner calls `enter_scene` when the table enters a scene: the builder opens its record under
//! the scene's new generation, the broker's epoch moves up so requests of the old scene are
//! superseded, the buffer empties (the old picture is never shown under the new scene), and the
//! scene's first plate is queued at once (K.2). It owns no thread and no timer and reads no clock.

use std::fmt;
use std::sync::Arc;

use df_table::identity::SceneId;
use df_table::plate_buffer::{Delivery, GenerationRegression, StageMoment, Step};
use df_table::plate_job::{JobError, PlateJob};
use df_table::plate_scheduler::{JobEnd, PlateScheduler, Tick, TickOutcome};
use df_table::plate_timing::{PlateTiming, SceneBuildKind};
use df_table::scene_plan::StageOutputs;
use df_table::scene_plan_brief::SceneBrief;
use df_table::table_state::{SceneGeneration, TableSnapshot};

use super::builder::{BuilderError, BuilderOutput, JobOutcome, SceneBuilder};
use super::events::{SceneBuilderAudit, SceneBuilderEvent};
use super::playout::Playout;
use super::purposes::BuildPriority;

/// Why the pacer refused a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacerError {
    Job(JobError),
    Builder(BuilderError),
    /// The scene's generation is not after the one the screen already moved to (D.2).
    Generation(GenerationRegression),
}

impl fmt::Display for PacerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Job(error) => write!(formatter, "{error}"),
            Self::Builder(error) => write!(formatter, "{error}"),
            Self::Generation(regression) => write!(
                formatter,
                "generation {} is not after the screen's generation {}",
                regression.requested, regression.current
            ),
        }
    }
}

impl std::error::Error for PacerError {}

impl From<JobError> for PacerError {
    fn from(error: JobError) -> Self {
        Self::Job(error)
    }
}

impl From<BuilderError> for PacerError {
    fn from(error: BuilderError) -> Self {
        Self::Builder(error)
    }
}

impl From<GenerationRegression> for PacerError {
    fn from(error: GenerationRegression) -> Self {
        Self::Generation(error)
    }
}

/// What one pump found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pumped {
    pub step: Step,
    /// Jobs that ended without a plate during this pump.
    pub ended: Vec<JobOutcome>,
}

/// One scene's clock and screen.
pub struct ScenePacer {
    scene: SceneId,
    scheduler: PlateScheduler,
    playout: Playout,
    audit: Arc<dyn SceneBuilderAudit>,
}

impl ScenePacer {
    /// The pacer of `scene`, whose screen has shown nothing yet: its buffer starts before every
    /// generation, so entering the scene moves it forward.
    pub fn new(scene: SceneId, timing: PlateTiming, audit: Arc<dyn SceneBuilderAudit>) -> Self {
        Self {
            scene,
            scheduler: PlateScheduler::new(scene, timing),
            playout: Playout::new(scene, SceneGeneration::new(0), timing, Arc::clone(&audit)),
            audit,
        }
    }

    pub fn scheduler(&self) -> &PlateScheduler {
        &self.scheduler
    }

    pub fn playout(&self) -> &Playout {
        &self.playout
    }

    /// The table entered the scene: opens its build, empties the screen's buffer and queues the
    /// first plate at once. The jobs of an earlier build of the scene that were still out are
    /// returned as ended.
    pub fn enter_scene(
        &mut self,
        now_ms: u64,
        snapshot: &TableSnapshot,
        brief: &SceneBrief,
        kind: SceneBuildKind,
        builder: &mut SceneBuilder,
    ) -> Result<Vec<JobOutcome>, PacerError> {
        self.enter_scene_with(now_ms, snapshot, brief, kind, builder, None)
    }

    /// `enter_scene` for a scene the look-ahead worker built ahead (SB-LA): `built` holds the
    /// stages it stored, which the builder adopts before the first plate's job is handed to it, so
    /// that job starts at the Plate stage.
    pub fn enter_scene_with(
        &mut self,
        now_ms: u64,
        snapshot: &TableSnapshot,
        brief: &SceneBrief,
        kind: SceneBuildKind,
        builder: &mut SceneBuilder,
        built: Option<&StageOutputs>,
    ) -> Result<Vec<JobOutcome>, PacerError> {
        let generation = snapshot
            .scene(self.scene)
            .ok_or(JobError::SceneNotOpen { scene: self.scene })?
            .generation();
        self.playout.focus_changed(self.scene, generation)?;
        let outputs = builder.open_scene(
            brief.clone(),
            generation,
            kind,
            BuildPriority::Current,
            now_ms,
        )?;
        let mut ended = Vec::new();
        self.settle(outputs, &mut ended);
        if let Some(built) = built {
            builder.adopt(self.scene, built)?;
        }
        let tick = self.scheduler.enter_scene(now_ms, snapshot, brief)?;
        self.record_tick(&tick, snapshot);
        self.hand_jobs_to(builder, &mut ended);
        Ok(ended)
    }

    /// One call of the owner's clock. It never waits for a picture.
    pub fn pump(
        &mut self,
        now_ms: u64,
        snapshot: &TableSnapshot,
        brief: &SceneBrief,
        builder: &mut SceneBuilder,
        moment: StageMoment,
        minimum_on_screen_ms: u64,
    ) -> Result<Pumped, PacerError> {
        let mut ended = Vec::new();
        let tick = self.scheduler.plate_tick(now_ms, snapshot, brief)?;
        self.record_tick(&tick, snapshot);
        self.hand_jobs_to(builder, &mut ended);
        let outputs = builder.advance(now_ms, snapshot);
        self.settle(outputs, &mut ended);
        let step = self.playout.step(now_ms, moment, minimum_on_screen_ms);
        Ok(Pumped { step, ended })
    }

    fn hand_jobs_to(&mut self, builder: &mut SceneBuilder, ended: &mut Vec<JobOutcome>) {
        while let Some(job) = self.scheduler.take_next() {
            self.hand_over(builder, job, ended);
        }
    }

    fn hand_over(
        &mut self,
        builder: &mut SceneBuilder,
        job: PlateJob,
        ended: &mut Vec<JobOutcome>,
    ) {
        self.playout.expect(&job);
        let id = job.id();
        if builder.submit(job).is_err() {
            // The scene is not open under the job's generation: the job is stale and never ran.
            self.playout.forget(id);
            self.scheduler.job_ended(id, JobEnd::Stale);
            ended.push(JobOutcome {
                job: id,
                scene: self.scene,
                end: JobEnd::Stale,
                failure: None,
            });
        }
    }

    /// Gives finished plates to the screen and tells the clock and the screen which jobs gave up.
    fn settle(&mut self, outputs: Vec<BuilderOutput>, ended: &mut Vec<JobOutcome>) {
        for output in outputs {
            match output {
                BuilderOutput::PlateReady(plate) => {
                    let id = plate.job.id();
                    let end = match self.playout.deliver(plate) {
                        Delivery::Accepted => JobEnd::Ready,
                        Delivery::Rejected(_) => JobEnd::Stale,
                    };
                    self.scheduler.job_ended(id, end);
                }
                BuilderOutput::JobEnded(outcome) => {
                    self.playout.forget(outcome.job);
                    self.scheduler.job_ended(outcome.job, outcome.end);
                    if outcome.end == JobEnd::Dropped {
                        self.audit.record(&SceneBuilderEvent::DroppedStale {
                            scene: outcome.scene,
                            job: outcome.job,
                        });
                    }
                    ended.push(outcome);
                }
            }
        }
    }

    fn record_tick(&self, tick: &Tick, snapshot: &TableSnapshot) {
        if !tick.discarded.is_empty() {
            self.audit.record(&SceneBuilderEvent::TickDiscarded {
                scene: self.scene,
                jobs: tick.discarded.len(),
            });
        }
        let generation = snapshot.scene(self.scene).map(|record| record.generation());
        match (tick.outcome, generation) {
            (
                TickOutcome::Queued {
                    job,
                    cause,
                    dropped,
                },
                Some(generation),
            ) => {
                self.audit.record(&SceneBuilderEvent::TickQueued {
                    scene: self.scene,
                    generation,
                    job,
                    cause,
                });
                if let Some(oldest) = dropped {
                    self.audit.record(&SceneBuilderEvent::DroppedStale {
                        scene: self.scene,
                        job: oldest,
                    });
                }
            }
            (TickOutcome::Skipped { reason }, _) => {
                self.audit.record(&SceneBuilderEvent::TickSkipped {
                    scene: self.scene,
                    reason,
                });
            }
            (TickOutcome::NotDue { .. } | TickOutcome::Queued { .. }, _) => {}
        }
    }
}
