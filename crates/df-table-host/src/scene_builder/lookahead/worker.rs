//! The look-ahead worker: builds the next scene's description and background through the broker on
//! the lowest queue class while the current scene plays, so the transition finds it Ready (SB-LA,
//! idea 1 of "Ideas to come").
//!
//! The pure planner (`df_table::lookahead`) decides which scene to build, when to start, when to
//! give up and what the transition found; this carries out its actions. A build is two stored
//! stages: Describe (`lookahead_describe`, skipped when the brief already holds an authored
//! description) and Background (`lookahead_background`), each one broker request of the
//! look-ahead class, so any request of the current scene (live or plate class) is claimed first and
//! may preempt a look-ahead call that is out. A stage starts only from a stored output, as in the
//! scene builder. The worker owns no thread and no timer: the owner calls `advance` often and it
//! never waits for a provider.
//!
//! Lease and epoch: a build's requests are for the broker epoch after the current one (the scene the
//! table has not entered). When the planner cancels a build (the story branched, the facts changed,
//! the deadline passed, the table entered the scene before it was done) the worker cancels what the
//! build has out, and a result that arrives for a cancelled build is refused by its token. If the
//! broker's epoch has moved past a build's base without the owner telling the worker (`enter` is the
//! owner's call at every transition), the build is discarded and nothing of it is kept. A build's
//! output is held in the planner's book and goes to no projection and no client: the scene
//! builder's `adopt` receives it only at the transition.
//!
//! Every change is one `SceneBuilderEvent` under the area `LOOKAHEAD`; the stage requests are the
//! broker's own `Requested` events with the look-ahead purposes, so the cost view and the broker
//! ledger show what was spent on scenes that were used and on scenes that were not.

use std::collections::BTreeMap;
use std::num::NonZeroU128;
use std::sync::Arc;

use df_table::element::NameError;
use df_table::event::FailureCause;
use df_table::identity::{Correlation, OperationId, SceneId, TurnSeq};
use df_table::lookahead::{
    Action, BuildOutcome, BuildToken, Expected, LookaheadError, LookaheadPlanner, LookaheadReport,
    PlanView, Take, lookahead_candidates,
};
use df_table::lookahead_settings::LookaheadSettings;
use df_table::lookahead_stamp::WorldStamp;
use df_table::plate_digest::PlateDigest;
use df_table::ports::{PurposeName, ResourceHandle};
use df_table::scene_plan::{SceneStage, StageOutput, StageOutputs};
use df_table::scene_plan_brief::SceneBrief;
use df_table::table_state::{SceneGeneration, TableSnapshot};

use crate::broker::BrokerHub;
use crate::broker::request::{
    Epoch, NewRequest, RequestError, RequestStatus, ResourceResult, ResultError,
};
use crate::scene_builder::builder::SCENE_LEVEL_JOB;
use crate::scene_builder::events::{SceneBuilderAudit, SceneBuilderEvent, StageSource};
use crate::scene_builder::inputs::{StageInput, StageInputs};
use crate::scene_builder::job_run::output_of;
use crate::scene_builder::purposes::{BuildPriority, StagePurposes};

/// One build of the next scene in progress.
struct BuildRun {
    token: BuildToken,
    brief: SceneBrief,
    /// The broker epoch when the build started: the table has not entered the scene while the
    /// broker's epoch is still this.
    base_epoch: Epoch,
    started_ms: u64,
    stage_started_ms: u64,
    /// The stage whose request is out, and its handle.
    open: Option<(SceneStage, ResourceHandle)>,
    outputs: StageOutputs,
}

impl BuildRun {
    /// The epoch the build's requests are for: the next scene's.
    fn request_epoch(&self) -> Epoch {
        Epoch::new(self.base_epoch.get().saturating_add(1))
    }
}

/// The look-ahead worker of one table.
pub struct LookaheadWorker {
    hub: Arc<BrokerHub>,
    audit: Arc<dyn SceneBuilderAudit>,
    inputs: Arc<StageInputs>,
    describe: PurposeName,
    background: PurposeName,
    planner: LookaheadPlanner,
    runs: BTreeMap<SceneId, BuildRun>,
}

fn correlation_of(
    scene: SceneId,
    token: BuildToken,
    stage: SceneStage,
    snapshot: &TableSnapshot,
) -> Correlation {
    let mut digest = PlateDigest::new();
    digest.text("lookahead");
    digest.number(scene.get());
    digest.number(token.get());
    digest.text(stage.key());
    let operation = NonZeroU128::MIN.saturating_add(u128::from(digest.finish()));
    let revision = snapshot.revision();
    Correlation {
        operation_id: OperationId::from_nonzero_u128(operation),
        turn_seq: TurnSeq::new(revision.sequence()),
        revision,
    }
}

impl LookaheadWorker {
    pub fn new(
        hub: Arc<BrokerHub>,
        audit: Arc<dyn SceneBuilderAudit>,
        inputs: Arc<StageInputs>,
        settings: LookaheadSettings,
    ) -> Result<Self, NameError> {
        let purposes = StagePurposes::for_priority(BuildPriority::Lookahead)?;
        Ok(Self {
            hub,
            audit,
            inputs,
            describe: purposes.describe,
            background: purposes.background,
            planner: LookaheadPlanner::new(settings),
            runs: BTreeMap::new(),
        })
    }

    pub fn planner(&self) -> &LookaheadPlanner {
        &self.planner
    }

    pub fn settings(&self) -> LookaheadSettings {
        self.planner.settings()
    }

    pub fn set_settings(&mut self, settings: LookaheadSettings) {
        self.planner.set_settings(settings);
    }

    /// How many builds have requests out or are about to.
    pub fn running(&self) -> usize {
        self.runs.len()
    }

    pub fn report(&self, now_ms: u64) -> LookaheadReport {
        self.planner.report(now_ms)
    }

    fn record(&self, event: SceneBuilderEvent) {
        self.audit.record(&event);
    }

    /// Stops new builds from starting; builds already out run on at their low class.
    pub fn pause(&mut self) {
        self.planner.pause();
        self.record(SceneBuilderEvent::LookaheadPaused { paused: true });
    }

    pub fn resume(&mut self) {
        self.planner.resume();
        self.record(SceneBuilderEvent::LookaheadPaused { paused: false });
    }

    /// The story now expects these scenes next, `current` being the one on screen. Builds of scenes
    /// it no longer expects are cancelled.
    pub fn set_expected(
        &mut self,
        now_ms: u64,
        snapshot: &TableSnapshot,
        current: Option<SceneId>,
        expected: Vec<Expected>,
    ) -> Result<(), LookaheadError> {
        let candidates = lookahead_candidates(snapshot, current, expected)?;
        self.record(SceneBuilderEvent::LookaheadTargets {
            scenes: candidates.len(),
        });
        let actions = self.planner.set_expected(candidates);
        self.apply(actions, now_ms, snapshot);
        Ok(())
    }

    /// One look at the clock: moves every build as far as the broker's answers allow, then lets the
    /// planner age out, cancel and start. It never waits.
    pub fn advance(&mut self, now_ms: u64, snapshot: &TableSnapshot, view: PlanView) {
        let scenes: Vec<SceneId> = self.runs.keys().copied().collect();
        for scene in scenes {
            if let Some(run) = self.runs.remove(&scene) {
                self.drive(scene, run, now_ms, snapshot);
            }
        }
        let actions = self.planner.plan(now_ms, view);
        self.apply(actions, now_ms, snapshot);
    }

    /// The table is entering `scene`, whose brief is `brief`: what the look-ahead has for it. Call it
    /// before the broker's epoch moves to the scene, so a build that is still out is cancelled as
    /// late and the table builds the scene the normal way. A hit or a cast-only change returns the
    /// stages built, for `SceneBuilder::adopt`.
    pub fn enter(
        &mut self,
        now_ms: u64,
        scene: SceneId,
        snapshot: &TableSnapshot,
        brief: &SceneBrief,
    ) -> Take {
        let stamp = WorldStamp::of(snapshot, brief);
        let entered = self.planner.enter(scene, &stamp);
        self.apply(entered.actions, now_ms, snapshot);
        let ready_for_ms = match &entered.take {
            Take::Hit(booked) | Take::Redress(booked) => {
                Some(now_ms.saturating_sub(booked.ready_ms))
            }
            _ => None,
        };
        self.record(SceneBuilderEvent::LookaheadTaken {
            scene,
            outcome: entered.take.key(),
            ready_for_ms,
        });
        entered.take
    }

    /// The table closed: every build is cancelled and the book emptied.
    pub fn close(&mut self, now_ms: u64, snapshot: &TableSnapshot) {
        let actions = self.planner.close();
        self.apply(actions, now_ms, snapshot);
        for (_, run) in std::mem::take(&mut self.runs) {
            self.cancel_open(&run);
        }
    }

    fn cancel_open(&self, run: &BuildRun) {
        if let Some((_, handle)) = run.open {
            self.hub.cancel(handle);
        }
    }

    fn apply(&mut self, actions: Vec<Action>, now_ms: u64, snapshot: &TableSnapshot) {
        for action in actions {
            match action {
                Action::Start {
                    token,
                    brief,
                    expectation,
                    likelihood_permille,
                } => {
                    let scene = brief.scene();
                    let base_epoch = self.hub.epoch();
                    let run = BuildRun {
                        token,
                        brief,
                        base_epoch,
                        started_ms: now_ms,
                        stage_started_ms: now_ms,
                        open: None,
                        outputs: StageOutputs::default(),
                    };
                    self.record(SceneBuilderEvent::LookaheadStarted {
                        scene,
                        token: token.get(),
                        expectation,
                        likelihood_permille,
                        epoch: run.request_epoch().get(),
                    });
                    self.drive(scene, run, now_ms, snapshot);
                }
                Action::Cancel {
                    scene,
                    token,
                    reason,
                } => {
                    if self.runs.get(&scene).is_some_and(|run| run.token == token)
                        && let Some(run) = self.runs.remove(&scene)
                    {
                        self.cancel_open(&run);
                    }
                    self.record(SceneBuilderEvent::LookaheadCancelled {
                        scene,
                        token: token.get(),
                        reason,
                    });
                }
                Action::Expire { scene } => {
                    self.record(SceneBuilderEvent::LookaheadExpired { scene });
                }
            }
        }
    }

    /// Moves one build as far as it goes without waiting, then keeps it, or reports how it ended.
    fn drive(&mut self, scene: SceneId, mut run: BuildRun, now_ms: u64, snapshot: &TableSnapshot) {
        if !self.planner.is_building(scene, run.token) {
            // The planner cancelled or replaced it: nothing of it is wanted.
            self.cancel_open(&run);
            return;
        }
        if self.hub.epoch() > run.base_epoch {
            // The table entered a scene without telling the worker: the build is not for it.
            let stage = run.open.map_or(SceneStage::Describe, |(stage, _)| stage);
            self.cancel_open(&run);
            self.record(SceneBuilderEvent::LookaheadDiscarded {
                scene,
                token: run.token.get(),
                stage,
            });
            self.finish(
                scene,
                &run,
                now_ms,
                BuildOutcome::Failed {
                    stage,
                    cause: FailureCause::Stale,
                },
            );
            return;
        }
        match self.step(scene, &mut run, now_ms, snapshot) {
            None => {
                self.runs.insert(scene, run);
            }
            Some(outcome) => self.finish(scene, &run, now_ms, outcome),
        }
    }

    fn finish(&mut self, scene: SceneId, run: &BuildRun, now_ms: u64, outcome: BuildOutcome) {
        let event = match &outcome {
            BuildOutcome::Ready(_) => SceneBuilderEvent::LookaheadReady {
                scene,
                token: run.token.get(),
                ms: now_ms.saturating_sub(run.started_ms),
            },
            BuildOutcome::Failed { stage, cause } => SceneBuilderEvent::LookaheadFailed {
                scene,
                token: run.token.get(),
                stage: *stage,
                cause: *cause,
            },
        };
        if self
            .planner
            .build_finished(now_ms, scene, run.token, outcome)
            .is_ok()
        {
            self.record(event);
        } else {
            // A result for a build the planner no longer wants: refused, nothing stored.
            self.cancel_open(run);
        }
    }

    /// The next thing the build needs, as far as the broker's answers allow. `None` while a request
    /// is out; the outcome when the build is done or cannot go on.
    fn step(
        &self,
        scene: SceneId,
        run: &mut BuildRun,
        now_ms: u64,
        snapshot: &TableSnapshot,
    ) -> Option<BuildOutcome> {
        loop {
            if let Some((stage, handle)) = run.open {
                match self.poll(scene, run, stage, handle, now_ms) {
                    Poll::Waiting => return None,
                    Poll::Stored => continue,
                    Poll::Ended(outcome) => return Some(outcome),
                }
            }
            let stage = if run.outputs.description.is_none() {
                SceneStage::Describe
            } else if run.outputs.background.is_none() {
                SceneStage::Background
            } else {
                return Some(BuildOutcome::Ready(run.outputs.clone()));
            };
            if stage == SceneStage::Describe
                && let Some(authored) = run.brief.description()
            {
                run.outputs.description = Some(authored.clone());
                self.stored(scene, run, stage, StageSource::SceneRecord, now_ms);
                continue;
            }
            let purpose = if stage == SceneStage::Describe {
                &self.describe
            } else {
                &self.background
            };
            run.stage_started_ms = now_ms;
            match self.request(scene, run, stage, purpose, snapshot) {
                Ok(handle) => run.open = Some((stage, handle)),
                Err(error) => {
                    let cause = match error {
                        RequestError::StaleEpoch { .. } => FailureCause::Stale,
                        _ => FailureCause::Unavailable,
                    };
                    return Some(BuildOutcome::Failed { stage, cause });
                }
            }
        }
    }

    /// One broker request for `stage` at the look-ahead class, for the next scene's epoch, with its
    /// input recorded first.
    fn request(
        &self,
        scene: SceneId,
        run: &BuildRun,
        stage: SceneStage,
        purpose: &PurposeName,
        snapshot: &TableSnapshot,
    ) -> Result<ResourceHandle, RequestError> {
        let epoch = run.request_epoch();
        let input = StageInput {
            scene,
            generation: SceneGeneration::new(epoch.get()),
            job: SCENE_LEVEL_JOB,
            stage,
            staging_digest: 0,
            source: None,
            prompt: None,
        };
        let key = input.key(purpose);
        self.inputs.record(key, input);
        self.hub.request_keyed(NewRequest {
            purpose: purpose.clone(),
            key,
            epoch,
            correlation: correlation_of(scene, run.token, stage, snapshot),
        })
    }

    fn poll(
        &self,
        scene: SceneId,
        run: &mut BuildRun,
        stage: SceneStage,
        handle: ResourceHandle,
        now_ms: u64,
    ) -> Poll {
        let failed = |cause| Poll::Ended(BuildOutcome::Failed { stage, cause });
        let Some(status) = self.hub.status(handle) else {
            return failed(FailureCause::Stale);
        };
        match status {
            RequestStatus::Queued { .. }
            | RequestStatus::BackingOff { .. }
            | RequestStatus::InFlight { .. } => Poll::Waiting,
            RequestStatus::Failed { cause } => failed(cause),
            RequestStatus::Cancelled | RequestStatus::Superseded | RequestStatus::Dropped => {
                failed(FailureCause::Stale)
            }
            RequestStatus::Ready => {
                let result = match self.hub.take_result(handle) {
                    Ok(result) => result,
                    Err(ResultError::NotSettled) => return Poll::Waiting,
                    Err(ResultError::Unknown | ResultError::NoResult { .. }) => {
                        return failed(FailureCause::InvalidAnswer);
                    }
                };
                match keep(run, stage, handle, result) {
                    Ok(()) => {
                        run.open = None;
                        self.stored(scene, run, stage, StageSource::Requested, now_ms);
                        Poll::Stored
                    }
                    Err(cause) => failed(cause),
                }
            }
        }
    }

    fn stored(
        &self,
        scene: SceneId,
        run: &BuildRun,
        stage: SceneStage,
        source: StageSource,
        now_ms: u64,
    ) {
        self.record(SceneBuilderEvent::LookaheadStored {
            scene,
            token: run.token.get(),
            stage,
            source,
            ms: now_ms.saturating_sub(run.stage_started_ms),
        });
    }
}

/// What polling an open request found.
enum Poll {
    Waiting,
    /// The stage's output is stored, so the next stage may start.
    Stored,
    Ended(BuildOutcome),
}

/// Stores a stage's result in the build, or says why it cannot be used.
fn keep(
    run: &mut BuildRun,
    stage: SceneStage,
    handle: ResourceHandle,
    result: ResourceResult,
) -> Result<(), FailureCause> {
    match output_of(stage, handle, result)? {
        StageOutput::Description(text) => run.outputs.description = Some(text),
        StageOutput::Background(picture) => run.outputs.background = Some(picture),
        StageOutput::Plate(_) | StageOutput::Locate(_) => return Err(FailureCause::InvalidAnswer),
    }
    Ok(())
}
