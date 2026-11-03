//! The scene builder: a scene's description, background, plate, anchors and ready state, built
//! through the broker one stored stage at a time (D12 to D15, K.2, O.16 SB).
//!
//! The builder holds one record for each open scene (its brief, its generation, the description and
//! background stored so far, and its jobs) and one `JobRun` for each plate job in flight. The owner
//! opens a scene when the table enters it, submits the jobs the plate scheduler queues, and calls
//! `advance` often (it never waits): each call moves every job as far as the broker's answers allow
//! and returns the plates that finished and the jobs that ended. A job after a scene's first starts
//! at the Plate stage because the scene already stored its description and background.
//!
//! The build takes a `BuildPriority`. `Current` builds the table's own scene at the live and plate
//! classes; `Lookahead` builds at the lowest class, the seam SB-LA uses to build the next scene
//! early with the same pipeline (that worker is not built here: it would call `open_scene` with
//! `Lookahead` and the certain next scene's brief, and `submit` a job for it).
//!
//! Waiting for art is never a gate (K.2): after the build's budget with no plate on screen the
//! builder reports the card due and the projection shows the establishing card, while the build goes
//! on to its deadlines. A scene that moved on is retired with its open requests cancelled, and a
//! result for it is never stored (D.2).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use df_table::element::NameError;
use df_table::identity::{SceneId, TableId, TableScope};
use df_table::plate_job::{PlateJob, PlateJobId};
use df_table::plate_scheduler::JobEnd;
use df_table::plate_timing::{PlateTiming, SceneBuildKind};
use df_table::scene_plan::{ReadyPlate, StageFailure, StageOutputs, StagePlan};
use df_table::scene_plan_brief::SceneBrief;
use df_table::scene_plan_card::{CardState, CardWatch, EstablishingCard};
use df_table::table_state::{SceneGeneration, TableSnapshot};

use super::events::{SceneBuilderAudit, SceneBuilderEvent, StageSource};
use super::inputs::StageInputs;
use super::job_run::{Driven, Driver, JobRun, cancel_open};
use super::purposes::{BuildPriority, StagePurposes};
use crate::broker::BrokerHub;
use crate::broker::purpose::PurposeTable;
use crate::broker::request::Epoch;

/// Job id 0 stands for the scene itself in events about what the scene stored before any job.
pub const SCENE_LEVEL_JOB: PlateJobId = PlateJobId::new(0);

/// What came out of a pass over the jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuilderOutput {
    /// A plate finished, with its anchors: for the playout buffer.
    PlateReady(ReadyPlate),
    JobEnded(JobOutcome),
}

/// A job that ended without a plate, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOutcome {
    pub job: PlateJobId,
    pub scene: SceneId,
    pub end: JobEnd,
    pub failure: Option<StageFailure>,
}

/// Why the builder refused a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuilderError {
    UnknownScene {
        scene: SceneId,
    },
    /// A scene's generation only goes up (D.2).
    GenerationNotForward {
        current: SceneGeneration,
        requested: SceneGeneration,
    },
    /// The job was taken under another generation than the one the scene is built for.
    JobForOtherGeneration {
        scene: SceneGeneration,
        job: SceneGeneration,
    },
    /// A purpose name is not valid.
    Name(NameError),
}

impl fmt::Display for BuilderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownScene { scene } => write!(formatter, "scene {scene} was never opened"),
            Self::GenerationNotForward { current, requested } => write!(
                formatter,
                "generation {requested} is not after the scene's generation {current}"
            ),
            Self::JobForOtherGeneration { scene, job } => write!(
                formatter,
                "the job is for generation {job}, and the scene is built for {scene}"
            ),
            Self::Name(error) => write!(formatter, "a purpose name is invalid: {error}"),
        }
    }
}

impl std::error::Error for BuilderError {}

impl From<NameError> for BuilderError {
    fn from(error: NameError) -> Self {
        Self::Name(error)
    }
}

/// One open scene.
pub(super) struct SceneRecord {
    pub(super) brief: SceneBrief,
    pub(super) generation: SceneGeneration,
    pub(super) kind: SceneBuildKind,
    pub(super) priority: BuildPriority,
    pub(super) purposes: StagePurposes,
    pub(super) started_ms: u64,
    /// What the scene stored: its description and its background, carried into every job.
    pub(super) outputs: StageOutputs,
    pub(super) watch: CardWatch,
    pub(super) jobs: Vec<JobRun>,
    pub(super) first_plate_ready: bool,
    card_reported: bool,
}

impl SceneRecord {
    /// Keeps what a job stored that belongs to the scene, the first time it is stored.
    pub(super) fn keep_scene_outputs(&mut self, stored: &StageOutputs) {
        if self.outputs.description.is_none() {
            self.outputs.description.clone_from(&stored.description);
        }
        if self.outputs.background.is_none() {
            self.outputs.background = stored.background;
        }
    }
}

/// The establishing card and where its timing stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardView {
    pub state: CardState,
    pub card: EstablishingCard,
}

/// The scene builder of one table.
pub struct SceneBuilder {
    table: TableId,
    purposes: PurposeTable,
    hub: Arc<BrokerHub>,
    audit: Arc<dyn SceneBuilderAudit>,
    inputs: Arc<StageInputs>,
    timing: PlateTiming,
    scenes: BTreeMap<SceneId, SceneRecord>,
}

impl SceneBuilder {
    pub fn new(
        table: TableId,
        purposes: PurposeTable,
        hub: Arc<BrokerHub>,
        audit: Arc<dyn SceneBuilderAudit>,
        inputs: Arc<StageInputs>,
        timing: PlateTiming,
    ) -> Self {
        Self {
            table,
            purposes,
            hub,
            audit,
            inputs,
            timing,
            scenes: BTreeMap::new(),
        }
    }

    pub fn timing(&self) -> PlateTiming {
        self.timing
    }

    pub fn set_timing(&mut self, timing: PlateTiming) {
        self.timing = timing;
    }

    pub fn is_open(&self, scene: SceneId) -> bool {
        self.scenes.contains_key(&scene)
    }

    /// The purposes the scene's jobs ask the broker for.
    pub fn purposes_of(&self, scene: SceneId) -> Option<&StagePurposes> {
        self.scenes.get(&scene).map(|record| &record.purposes)
    }

    pub fn purpose_table(&self) -> &PurposeTable {
        &self.purposes
    }

    /// Jobs of `scene` that have not ended.
    pub fn jobs_in_flight(&self, scene: SceneId) -> usize {
        self.scenes
            .get(&scene)
            .map_or(0, |record| record.jobs.len())
    }

    /// Starts building a scene: the table entered it (or look-ahead chose it) under `generation`.
    /// A scene already open under an earlier generation is retired first, its open requests
    /// cancelled, and the jobs it ended are returned.
    pub fn open_scene(
        &mut self,
        brief: SceneBrief,
        generation: SceneGeneration,
        kind: SceneBuildKind,
        priority: BuildPriority,
        now_ms: u64,
    ) -> Result<Vec<BuilderOutput>, BuilderError> {
        let scene = brief.scene();
        let purposes = StagePurposes::for_priority(priority)?;
        let mut ended = Vec::new();
        if let Some(existing) = self.scenes.get(&scene) {
            if existing.generation >= generation {
                return Err(BuilderError::GenerationNotForward {
                    current: existing.generation,
                    requested: generation,
                });
            }
            ended = self.retire(scene);
        }
        if priority == BuildPriority::Current {
            self.move_epoch_to(generation);
        }
        self.audit.record(&SceneBuilderEvent::Opened {
            scene,
            generation,
            kind,
            priority,
        });
        let outputs = StageOutputs {
            description: brief.description().cloned(),
            ..StageOutputs::default()
        };
        if outputs.description.is_some() {
            self.audit.record(&SceneBuilderEvent::StageStored {
                scene,
                generation,
                job: SCENE_LEVEL_JOB,
                stage: df_table::scene_plan::SceneStage::Describe,
                source: StageSource::SceneRecord,
                ms: 0,
            });
        }
        self.scenes.insert(
            scene,
            SceneRecord {
                brief,
                generation,
                kind,
                priority,
                purposes,
                started_ms: now_ms,
                outputs,
                watch: CardWatch::new(now_ms, kind, self.timing),
                jobs: Vec::new(),
                first_plate_ready: false,
                card_reported: false,
            },
        );
        Ok(ended)
    }

    /// The table entered a scene under `generation`, so the broker's epoch moves up to it and every
    /// request of an earlier scene is superseded (D.2). A look-ahead build never calls this: it is
    /// for a scene the table has not entered.
    fn move_epoch_to(&self, generation: SceneGeneration) {
        let epoch = Epoch::new(generation.get());
        if self.hub.epoch() < epoch {
            // The broker records the advance. It can only refuse an epoch that is not forward,
            // which the check above excludes.
            let _ = self.hub.advance_epoch(epoch);
        }
    }

    /// Retires a scene: cancels what its jobs have out and reports each as ended `Stale`.
    fn retire(&mut self, scene: SceneId) -> Vec<BuilderOutput> {
        let Some(record) = self.scenes.remove(&scene) else {
            return Vec::new();
        };
        record
            .jobs
            .into_iter()
            .map(|run| {
                cancel_open(&self.hub, &run);
                let job = run.plan().job();
                self.audit.record(&SceneBuilderEvent::JobEnded {
                    scene,
                    generation: record.generation,
                    job: job.id(),
                    end: JobEnd::Stale,
                });
                BuilderOutput::JobEnded(JobOutcome {
                    job: job.id(),
                    scene,
                    end: JobEnd::Stale,
                    failure: None,
                })
            })
            .collect()
    }

    /// Takes a plate job from the scheduler. It starts at the first stage its scene has no stored
    /// output for, on the next `advance`.
    pub fn submit(&mut self, job: PlateJob) -> Result<(), BuilderError> {
        let scene = job.scene();
        let record = self
            .scenes
            .get_mut(&scene)
            .ok_or(BuilderError::UnknownScene { scene })?;
        if job.stamp().generation != record.generation {
            return Err(BuilderError::JobForOtherGeneration {
                scene: record.generation,
                job: job.stamp().generation,
            });
        }
        record
            .jobs
            .push(JobRun::new(StagePlan::for_job(job, &record.outputs)));
        Ok(())
    }

    /// One pass over every job: moves each as far as the broker's answers allow, never waiting.
    /// Returns the plates that finished and the jobs that ended, in the order they did.
    pub fn advance(&mut self, now_ms: u64, snapshot: &TableSnapshot) -> Vec<BuilderOutput> {
        let mut outputs = Vec::new();
        let scenes: Vec<SceneId> = self.scenes.keys().copied().collect();
        for scene in scenes {
            let Some(mut record) = self.scenes.remove(&scene) else {
                continue;
            };
            let current = snapshot.scene(scene).map(|open| open.generation());
            let scope = TableScope::of_table(self.table).in_scene(scene);
            let driver = Driver {
                hub: &self.hub,
                audit: self.audit.as_ref(),
                inputs: &self.inputs,
                table: &self.purposes,
                scope: &scope,
                now_ms,
                snapshot,
            };
            for run in std::mem::take(&mut record.jobs) {
                match driver.drive(&mut record, run) {
                    Driven::Alive(run) => record.jobs.push(run),
                    Driven::Ended(output) => outputs.push(output),
                }
            }
            self.report_card_due(&mut record, now_ms);
            if current == Some(record.generation) || !record.jobs.is_empty() {
                self.scenes.insert(scene, record);
            }
        }
        outputs
    }

    /// Records, once, that the build passed its budget with no plate: play continues on the
    /// establishing card (K.2).
    fn report_card_due(&self, record: &mut SceneRecord, now_ms: u64) {
        if record.card_reported || record.first_plate_ready {
            return;
        }
        if matches!(record.watch.state(now_ms, false), CardState::Showing { .. }) {
            record.card_reported = true;
            self.audit.record(&SceneBuilderEvent::CardDue {
                scene: record.brief.scene(),
                generation: record.generation,
                after_ms: now_ms.saturating_sub(record.started_ms),
            });
        }
    }

    /// The establishing card of `scene` and its timing at `now_ms`. `plate_on_screen` is whether the
    /// playout buffer shows a plate of the scene, which retires the card.
    pub fn card(
        &self,
        scene: SceneId,
        now_ms: u64,
        snapshot: &TableSnapshot,
        plate_on_screen: bool,
    ) -> Option<CardView> {
        let record = self.scenes.get(&scene)?;
        Some(CardView {
            state: record.watch.state(now_ms, plate_on_screen),
            card: EstablishingCard::compose(&record.brief, &record.outputs, snapshot),
        })
    }

    pub(super) fn records(&self) -> impl Iterator<Item = (&SceneId, &SceneRecord)> {
        self.scenes.iter()
    }
}
