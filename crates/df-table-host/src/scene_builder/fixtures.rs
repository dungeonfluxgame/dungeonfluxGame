//! Shared fixtures for the scene builder's tests: a recording audit and a rig that wires the real
//! builder, broker hub and pacer to the fake worker on a manual clock. Time is always an explicit
//! number of milliseconds, never a sleep.

use std::sync::{Arc, Mutex, PoisonError};

use df_table::identity::{SceneId, TableId};
use df_table::plate::job::{JobReason, PlateJob, PlateJobId};
use df_table::plate::timing::{PlateTiming, SceneBuildKind};
use df_table::ports::{Clock, PurposeName};
use df_table::scene_plan::brief::SceneBrief;
use df_table::table_state::{DisplayName, SceneGeneration};

use super::builder::{BuilderOutput, SceneBuilder};
use super::dev_world::{DevWorld, ManualClock, dev_brief, dev_script};
use super::events::{SceneBuilderAudit, SceneBuilderEvent};
use super::fakes::{FakeStageWorker, StageScript};
use super::inputs::StageInputs;
use super::purposes::{BuildPriority, scene_purpose_table};
use super::request::{SceneBuilderError, SceneReply};
use crate::broker::request::WorkerId;
use crate::broker::scheduler::BrokerLimits;
use crate::broker::{BrokerAudit, BrokerEvent, BrokerHub};
use crate::dev_http::DevReply;

/// Keeps every event of the scene builder and the broker, in order.
#[derive(Default)]
pub struct RecordingAudit {
    scene: Mutex<Vec<SceneBuilderEvent>>,
    broker: Mutex<Vec<BrokerEvent>>,
}

impl RecordingAudit {
    pub fn scene_events(&self) -> Vec<SceneBuilderEvent> {
        self.scene
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn broker_events(&self) -> Vec<BrokerEvent> {
        self.broker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn scene_lines(&self) -> Vec<String> {
        self.scene_events()
            .iter()
            .map(SceneBuilderEvent::render_audit_line)
            .collect()
    }

    pub fn count(&self, matches: impl Fn(&SceneBuilderEvent) -> bool) -> usize {
        self.scene_events()
            .iter()
            .filter(|event| matches(event))
            .count()
    }

    /// The purposes of the broker requests, in the order they were made.
    pub fn requested_purposes(&self) -> Vec<String> {
        self.broker_events()
            .into_iter()
            .filter_map(|event| match event {
                BrokerEvent::Requested { purpose, .. } => Some(purpose.to_string()),
                _ => None,
            })
            .collect()
    }
}

impl SceneBuilderAudit for RecordingAudit {
    fn record(&self, event: &SceneBuilderEvent) {
        self.scene
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
    }
}

impl BrokerAudit for RecordingAudit {
    fn record(&self, event: &BrokerEvent) {
        self.broker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
    }
}

pub const STEP_MS: u64 = 100;

/// The builder, the broker and the fake worker on one clock, with the dev party in scene 1.
pub struct Rig {
    pub clock: Arc<ManualClock>,
    pub hub: Arc<BrokerHub>,
    pub audit: Arc<RecordingAudit>,
    pub worker: FakeStageWorker,
    pub builder: SceneBuilder,
    pub world: DevWorld,
    pub brief: SceneBrief,
    pub scene: SceneId,
    pub generation: SceneGeneration,
    next_job: u64,
}

impl Rig {
    /// A rig whose scene is in the table but not yet opened in the builder, answered by `script`.
    pub fn unopened(script: StageScript) -> Self {
        let clock = Arc::new(ManualClock::default());
        let audit = Arc::new(RecordingAudit::default());
        let table = scene_purpose_table().expect("the purposes are valid");
        let hub = Arc::new(BrokerHub::new(
            table.clone(),
            BrokerLimits::default(),
            Arc::clone(&audit) as Arc<dyn BrokerAudit>,
            Arc::clone(&clock) as Arc<dyn Clock>,
        ));
        let inputs = Arc::new(StageInputs::default());
        let worker = FakeStageWorker::new(
            Arc::clone(&hub),
            Arc::clone(&inputs),
            WorkerId::new(1),
            script,
        );
        let builder = SceneBuilder::new(
            TableId::new(1),
            table,
            Arc::clone(&hub),
            Arc::clone(&audit) as Arc<dyn SceneBuilderAudit>,
            inputs,
            PlateTiming::default(),
        );
        let mut world = DevWorld::new().expect("the dev table");
        let scene = SceneId::new(1);
        let generation = world.enter_scene(scene).expect("enters");
        world.people_of(scene).expect("the party");
        let brief = dev_brief(scene, "Harbor market").expect("a brief");
        Self {
            clock,
            hub,
            audit,
            worker,
            builder,
            world,
            brief,
            scene,
            generation,
            next_job: 0,
        }
    }

    /// A rig whose scene is built at `priority` and `kind`, answered by `script`.
    pub fn with(script: StageScript, kind: SceneBuildKind, priority: BuildPriority) -> Self {
        let mut rig = Self::unopened(script);
        rig.builder
            .open_scene(rig.brief.clone(), rig.generation, kind, priority, 0)
            .expect("opens");
        rig
    }

    /// The usual rig: the first scene, at the current priority, with the dev script.
    pub fn new() -> Self {
        Self::with(
            dev_script().expect("the dev script"),
            SceneBuildKind::Standard,
            BuildPriority::Current,
        )
    }

    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// A job of the scene as the table is now, taken at the current time.
    pub fn take_job(&mut self) -> PlateJob {
        self.next_job += 1;
        PlateJob::capture(
            PlateJobId::new(self.next_job),
            JobReason::Tick,
            self.now_ms(),
            &self.world.snapshot(),
            &self.brief,
        )
        .expect("a job")
    }

    pub fn submit_new_job(&mut self) -> PlateJobId {
        let job = self.take_job();
        let id = job.id();
        self.builder.submit(job).expect("submits");
        id
    }

    /// One tenth of a second: the clock moves, the worker delivers what is due, the builder moves
    /// every job as far as it can.
    pub fn step(&mut self) -> Vec<BuilderOutput> {
        self.clock.advance_ms(STEP_MS);
        let now = self.now_ms();
        self.worker.pump(now);
        self.builder.advance(now, &self.world.snapshot())
    }

    /// Steps until `done` says the outputs so far are enough, or `limit_ms` of time has passed.
    pub fn run_until(
        &mut self,
        limit_ms: u64,
        done: impl Fn(&[BuilderOutput]) -> bool,
    ) -> Vec<BuilderOutput> {
        let mut outputs = Vec::new();
        let until = self.now_ms() + limit_ms;
        while self.now_ms() < until && !done(&outputs) {
            outputs.extend(self.step());
        }
        outputs
    }

    /// Runs one job to its end and returns what it produced.
    pub fn run_one_job(&mut self) -> Vec<BuilderOutput> {
        self.submit_new_job();
        self.run_until(600_000, |outputs| !outputs.is_empty())
    }

    pub fn purpose(name: &str) -> PurposeName {
        PurposeName::new(name).expect("a valid purpose name")
    }
}

/// The stages whose requests were made, in order.
pub fn requested_stages(audit: &RecordingAudit) -> Vec<&'static str> {
    audit
        .scene_events()
        .into_iter()
        .filter_map(|event| match event {
            SceneBuilderEvent::StageRequested { stage, .. } => Some(stage.key()),
            _ => None,
        })
        .collect()
}

/// The ready plate among the outputs, if one finished.
pub fn plate_of(outputs: &[BuilderOutput]) -> Option<&df_table::scene_plan::ReadyPlate> {
    outputs.iter().find_map(|output| match output {
        BuilderOutput::PlateReady(plate) => Some(plate),
        BuilderOutput::JobEnded(_) => None,
    })
}

/// The error of a refused answer (a raw HTTP exchange), read as the scene builder's reply.
pub fn refusal_of(answer: &str) -> SceneBuilderError {
    let body = answer.split("\r\n\r\n").nth(1).expect("a body");
    match serde_json::from_str::<SceneReply<serde_json::Value>>(body) {
        Ok(DevReply::Rejected { error, .. }) => error,
        other => panic!("not a refusal: {other:?}"),
    }
}

/// `text` as a place name.
pub fn name(text: &str) -> DisplayName {
    DisplayName::new(text).expect("a place name")
}
