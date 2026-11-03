//! The scene builder's workbench: the real pipeline over fake providers on a manual clock, behind
//! the dev verbs `scene build`, `scene show`, `plates` and `buffer show` (coding style section 8,
//! pattern 8).
//!
//! Each `scene build <place>` has the dev party enter a new scene (a new generation) and builds it
//! through the broker: the stage requests go to a fake worker whose latency and answers are
//! scripted, and time is the workbench's own counter, stepped a tenth of a second at a time, so a
//! build that takes a minute of provider time takes none of ours and a run is the same every time.
//! The answer lists the audit lines of the run in order, with the time each was recorded, so the
//! order of the stages, the broker's requests and the card falling due can be read off it. `fail=`
//! makes a stage's provider refuse (a refusal is never retried) and `slow=` makes a stage take
//! longer than the budget, to see the establishing card (K.2) and the fallbacks.
//!
//! `plates run <seconds>` lets the clock tick: each interval a creature moves (a visible change a
//! plate must show), or with `quiet` nothing changes and no plate is paid for (RES9). The buffer
//! shows plates in order at the rhythm of K.3. It holds no secret and calls no real provider.

use std::collections::VecDeque;
use std::fmt::Display;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use df_table::identity::{SceneId, TableId};
use df_table::lookahead::{PlanView, Take};
use df_table::lookahead_settings::LookaheadSettings;
use df_table::plate_buffer::{StageMoment, Step, StepAction};
use df_table::plate_timing::{PlateTiming, SceneBuildKind};
use df_table::ports::{Clock, PurposeName};
use df_table::scene_plan_brief::SceneBrief;
use df_table::table_state::DisplayName;

use super::builder::SceneBuilder;
use super::dev_world::{
    DevWorld, HERO_ROW_Y, HEROES, ManualClock, WorldError, dev_brief, dev_script,
};
use super::events::{SceneBuilderAudit, SceneBuilderEvent, end_key};
use super::fakes::{FakeStageWorker, StageScript};
use super::inputs::StageInputs;
use super::lookahead::request::{LookaheadReply, LookaheadRequest};
use super::lookahead::{BenchContext, LookaheadBench};
use super::pacer::{PacerError, ScenePacer};
use super::purposes::{BuildPriority, StagePurposes, cost_view, scene_purpose_table};
use super::report::{LocateReport, SceneReport};
use super::request::{
    BufferShow, BuildOutcome, BuildReply, BuildRequest, DevStage, PlatesReport, RunReply,
    RunRequest, SceneError, SceneShow, SlowStage, TimedLine,
};
use crate::broker::providers::ScriptedCall;
use crate::broker::request::{ProviderFault, WorkerId};
use crate::broker::scheduler::BrokerLimits;
use crate::broker::{BrokerAudit, BrokerEvent, BrokerHub};
use crate::event_log::EventLog;

/// How far the workbench's clock moves between looks at the pipeline, in milliseconds: a tenth of
/// a second, fine enough that a stage's time is read to the tenth.
pub const DEV_STEP_MS: u64 = 100;
/// The longest a build may take in the workbench's own time before it stops waiting: an hour, far
/// past every deadline in the broker, so only a pipeline that cannot finish reaches it.
pub const MAXIMUM_DEV_VIRTUAL_MS: u64 = 3_600_000;
/// The reading time the dev screen assumes for the lines over a plate, in milliseconds: a plate
/// stays up at least this long before the next one replaces it (K.5: cards of three seconds).
pub const DEV_READING_MS: u64 = 3_000;
/// Most audit events one run keeps for its answer; the oldest are forgotten first. Deliberately huge.
const MAXIMUM_KEPT_EVENTS: usize = 1_000_000;
/// How far a creature steps along the row when the workbench makes a visible change, in canvas
/// units, and how many steps it takes before it starts again.
const VISIBLE_STEP_UNITS: u32 = 4;
const VISIBLE_STEPS: u64 = 7;
/// The hero the workbench moves to make a visible change.
const MOVER: usize = 0;
const MILLISECONDS_PER_SECOND: u64 = 1_000;

/// One recorded change of the pipeline, typed.
enum Recorded {
    Scene(SceneBuilderEvent),
    Broker(BrokerEvent),
}

impl Recorded {
    fn line(&self) -> String {
        match self {
            Self::Scene(event) => event.render_audit_line(),
            Self::Broker(event) => event.render_audit_line(),
        }
    }
}

/// Records every event of the scene builder and the broker with the workbench's time, and passes
/// the lines on to the host's log.
struct TimedAudit {
    clock: Arc<ManualClock>,
    log: Option<Arc<EventLog>>,
    kept: Mutex<VecDeque<(u64, Recorded)>>,
}

impl TimedAudit {
    fn kept(&self) -> MutexGuard<'_, VecDeque<(u64, Recorded)>> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn keep(&self, recorded: Recorded) {
        if let Some(log) = &self.log {
            log.push(recorded.line());
        }
        let mut kept = self.kept();
        kept.push_back((self.clock.now_ms(), recorded));
        while kept.len() > MAXIMUM_KEPT_EVENTS {
            kept.pop_front();
        }
    }

    /// Takes every event recorded since the last take.
    fn take(&self) -> Vec<(u64, Recorded)> {
        self.kept().drain(..).collect()
    }
}

impl SceneBuilderAudit for TimedAudit {
    fn record(&self, event: &SceneBuilderEvent) {
        self.keep(Recorded::Scene(event.clone()));
    }
}

impl BrokerAudit for TimedAudit {
    fn record(&self, event: &BrokerEvent) {
        self.keep(Recorded::Broker(event.clone()));
    }
}

/// What a stretch of the pipeline left behind, read off its events.
#[derive(Default)]
struct Taken {
    lines: Vec<TimedLine>,
    card_due_after_ms: Option<u64>,
    plates_finished: u64,
    finished_build_ms: u64,
    last_end: Option<BuildOutcome>,
}

fn summarise(events: Vec<(u64, Recorded)>) -> Taken {
    let mut taken = Taken::default();
    for (at_ms, recorded) in events {
        if let Recorded::Scene(event) = &recorded {
            match event {
                SceneBuilderEvent::CardDue { after_ms, .. } => {
                    taken.card_due_after_ms = Some(*after_ms);
                }
                SceneBuilderEvent::JobReady { ms, .. } => {
                    taken.plates_finished += 1;
                    taken.finished_build_ms = taken.finished_build_ms.saturating_add(*ms);
                }
                SceneBuilderEvent::StageFailed { stage, cause, .. } => {
                    taken.last_end = Some(BuildOutcome::Ended {
                        end: "failed".to_owned(),
                        stage: Some(stage.key().to_owned()),
                        cause: Some(cause.key().to_owned()),
                    });
                }
                SceneBuilderEvent::JobEnded { end, .. } if taken.last_end.is_none() => {
                    taken.last_end = Some(BuildOutcome::Ended {
                        end: end_key(*end).to_owned(),
                        stage: None,
                        cause: None,
                    });
                }
                _ => {}
            }
        }
        taken.lines.push(TimedLine {
            at_ms,
            line: recorded.line(),
        });
    }
    taken
}

/// The scene the screen shows.
struct Active {
    scene: SceneId,
    brief: SceneBrief,
    pacer: ScenePacer,
    /// When the first plate of the scene was on screen: the scene is Ready.
    ready_ms: Option<u64>,
}

/// The pipeline, the fake providers and the dev table, together.
pub struct Workbench {
    clock: Arc<ManualClock>,
    audit: Arc<TimedAudit>,
    world: DevWorld,
    worker: FakeStageWorker,
    builder: SceneBuilder,
    lookahead: LookaheadBench,
    timing: PlateTiming,
    next_scene: u64,
    active: Option<Active>,
    plates_finished: u64,
    finished_build_ms: u64,
}

fn workbench_error(reason: impl Display) -> SceneError {
    SceneError::Workbench {
        reason: reason.to_string(),
    }
}

impl From<WorldError> for SceneError {
    fn from(error: WorldError) -> Self {
        workbench_error(error)
    }
}

impl From<PacerError> for SceneError {
    fn from(error: PacerError) -> Self {
        workbench_error(error)
    }
}

/// The purpose a dev stage asks the broker for, for the table's own scene.
fn purpose_of(stage: DevStage) -> Result<PurposeName, SceneError> {
    let purposes = StagePurposes::for_priority(BuildPriority::Current).map_err(workbench_error)?;
    purposes
        .of(stage.scene_stage())
        .cloned()
        .ok_or_else(|| workbench_error("a dev stage has no purpose"))
}

/// The script with the dev tweaks on it: a slower stage and a stage that refuses.
fn tweak_script(
    script: StageScript,
    brief: &SceneBrief,
    fail: Option<DevStage>,
    slow: Option<SlowStage>,
) -> Result<StageScript, SceneError> {
    let mut script = script;
    if let Some(SlowStage { stage, seconds }) = slow {
        script = script.with_latency(
            &purpose_of(stage)?,
            seconds.saturating_mul(MILLISECONDS_PER_SECOND),
        );
    }
    if let Some(stage) = fail {
        // A refusal is never retried, so each scripted call is one request that fails. Locate
        // makes at most one request for the people and one for every phrase of every item.
        let calls = if stage == DevStage::Locate {
            1 + brief
                .set()
                .iter()
                .map(|piece| piece.phrases().len())
                .sum::<usize>()
        } else {
            1
        };
        script = script.with_calls(
            &purpose_of(stage)?,
            std::iter::repeat_n(ScriptedCall::fails_after(0, ProviderFault::Refused), calls),
        );
    }
    Ok(script)
}

impl Workbench {
    /// A workbench whose events also go to `log`, when there is one.
    pub fn new(timing: PlateTiming, log: Option<Arc<EventLog>>) -> Result<Self, SceneError> {
        let clock = Arc::new(ManualClock::default());
        let audit = Arc::new(TimedAudit {
            clock: Arc::clone(&clock),
            log,
            kept: Mutex::new(VecDeque::new()),
        });
        let table = scene_purpose_table().map_err(workbench_error)?;
        let hub = Arc::new(BrokerHub::new(
            table.clone(),
            BrokerLimits::default(),
            Arc::clone(&audit) as Arc<dyn BrokerAudit>,
            Arc::clone(&clock) as Arc<dyn Clock>,
        ));
        let inputs = Arc::new(StageInputs::default());
        let lookahead = LookaheadBench::new(
            Arc::clone(&hub),
            Arc::clone(&audit) as Arc<dyn SceneBuilderAudit>,
            Arc::clone(&inputs),
            LookaheadSettings::defaults(),
        )?;
        let worker = FakeStageWorker::new(
            Arc::clone(&hub),
            Arc::clone(&inputs),
            WorkerId::new(1),
            StageScript::default(),
        );
        let builder = SceneBuilder::new(
            TableId::new(1),
            table,
            hub,
            Arc::clone(&audit) as Arc<dyn SceneBuilderAudit>,
            inputs,
            timing,
        );
        Ok(Self {
            clock,
            audit,
            world: DevWorld::new()?,
            worker,
            builder,
            lookahead,
            timing,
            next_scene: 1,
            active: None,
            plates_finished: 0,
            finished_build_ms: 0,
        })
    }

    fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Makes the visible change of the beat: the mover steps along the row.
    fn make_visible_change(&mut self, now_ms: u64) -> Result<(), SceneError> {
        let (id, _, _, home_x) = HEROES[MOVER];
        let step = now_ms / MILLISECONDS_PER_SECOND % VISIBLE_STEPS;
        let shift = u32::try_from(step).unwrap_or(0) * VISIBLE_STEP_UNITS;
        self.world.move_cast(id, home_x + shift, HERO_ROW_Y)?;
        Ok(())
    }

    /// What the current scene looks like to the look-ahead: when it became Ready, and whether its
    /// own plates are being built (they have the lane first).
    fn plan_view(&self) -> PlanView {
        PlanView {
            current_ready_ms: self.active.as_ref().and_then(|active| active.ready_ms),
            foreground_busy: self
                .active
                .as_ref()
                .is_some_and(|active| self.builder.jobs_in_flight(active.scene) > 0),
        }
    }

    /// Moves the clock a tenth of a second and looks at the pipeline once.
    fn step(&mut self, moment: StageMoment, make_changes: bool) -> Result<Step, SceneError> {
        self.clock.advance_ms(DEV_STEP_MS);
        let now = self.now_ms();
        let tick_due = self
            .active
            .as_ref()
            .and_then(|active| active.pacer.scheduler().next_tick_ms())
            .is_some_and(|due| now >= due);
        if make_changes && tick_due {
            self.make_visible_change(now)?;
        }
        let snapshot = self.world.snapshot();
        self.worker.pump(now);
        let active = self.active.as_mut().ok_or(SceneError::NothingBuilt)?;
        let pumped = active.pacer.pump(
            now,
            &snapshot,
            &active.brief,
            &mut self.builder,
            moment,
            DEV_READING_MS,
        )?;
        if matches!(pumped.step.action, StepAction::Swapped(_)) && active.ready_ms.is_none() {
            active.ready_ms = Some(now);
        }
        let view = self.plan_view();
        self.lookahead.advance(now, &snapshot, view);
        Ok(pumped.step)
    }

    /// The dev party enters a new scene called `place` and the scene is built until its first plate
    /// is on screen, or the build ends without one.
    pub fn build(&mut self, request: &BuildRequest) -> Result<BuildReply, SceneError> {
        let place = DisplayName::new(&request.place).map_err(|_| SceneError::InvalidName {
            what: "the place".to_owned(),
        })?;
        self.audit.take();
        let scene = SceneId::new(self.next_scene);
        let kind = if self.next_scene == 1 {
            SceneBuildKind::FirstScene
        } else {
            SceneBuildKind::Standard
        };
        self.next_scene += 1;
        let generation = self.world.enter_scene(scene)?;
        self.world.people_of(scene)?;
        let snapshot = self.world.snapshot();
        let brief = dev_brief(scene, place.as_str())?;
        let started_ms = self.now_ms();
        // The look-ahead is asked before the broker's epoch moves to the scene: a build that is
        // not done is cancelled as late, and the scene is built the normal way.
        let take = self.lookahead.enter(started_ms, scene, &snapshot, &brief);
        self.worker.set_script(tweak_script(
            dev_script()?,
            &brief,
            request.fail,
            request.slow,
        )?);
        let mut pacer = ScenePacer::new(
            scene,
            self.timing,
            Arc::clone(&self.audit) as Arc<dyn SceneBuilderAudit>,
        );
        let built = match &take {
            Take::Hit(booked) | Take::Redress(booked) => Some(&booked.outputs),
            _ => None,
        };
        pacer.enter_scene_with(
            started_ms,
            &snapshot,
            &brief,
            kind,
            &mut self.builder,
            built,
        )?;
        self.active = Some(Active {
            scene,
            brief,
            pacer,
            ready_ms: None,
        });
        let on_screen_at = self.run_until_first_plate(started_ms)?;
        let taken = self.take_run();
        let outcome = match on_screen_at {
            Some(at_ms) => BuildOutcome::PlateOnScreen { at_ms },
            None => taken.last_end.clone().unwrap_or(BuildOutcome::NoPlate),
        };
        Ok(BuildReply {
            place: place.as_str().to_owned(),
            scene: scene.get(),
            generation: generation.get(),
            outcome,
            elapsed_ms: self.now_ms().saturating_sub(started_ms),
            card_due_after_ms: taken.card_due_after_ms,
            lines: taken.lines,
            scene_report: self
                .scene_reports()
                .into_iter()
                .find(|report| report.scene == scene.get()),
            lookahead: (!matches!(take, Take::Miss)).then(|| take.key().to_owned()),
        })
    }

    /// Steps until a plate is on screen, or nothing is left that could put one there. The time it
    /// went on screen, from `started_ms`, when it did.
    fn run_until_first_plate(&mut self, started_ms: u64) -> Result<Option<u64>, SceneError> {
        loop {
            let step = self.step(StageMoment::LineBoundary, false)?;
            let elapsed = self.now_ms().saturating_sub(started_ms);
            if matches!(step.action, StepAction::Swapped(_)) {
                return Ok(Some(elapsed));
            }
            let active = self.active.as_ref().ok_or(SceneError::NothingBuilt)?;
            let idle = self.builder.jobs_in_flight(active.scene) == 0
                && active.pacer.scheduler().queued() == 0
                && active.pacer.playout().buffer().outstanding() == 0;
            if idle || elapsed >= MAXIMUM_DEV_VIRTUAL_MS {
                return Ok(None);
            }
        }
    }

    /// Takes the events of the run since the last take and counts the plates it finished.
    fn take_run(&mut self) -> Taken {
        let taken = summarise(self.audit.take());
        self.plates_finished += taken.plates_finished;
        self.finished_build_ms = self
            .finished_build_ms
            .saturating_add(taken.finished_build_ms);
        taken
    }

    fn scene_reports(&self) -> Vec<SceneReport> {
        let on_screen_scene = self
            .active
            .as_ref()
            .filter(|active| active.pacer.playout().buffer().current().is_some())
            .map(|active| active.scene);
        self.builder
            .report(self.now_ms(), &self.world.snapshot(), |scene| {
                on_screen_scene == Some(scene)
            })
    }

    /// Lets the clock run for `seconds`: it ticks every plate interval, and the screen swaps
    /// plates in as the moment allows.
    pub fn run(&mut self, request: &RunRequest) -> Result<RunReply, SceneError> {
        if self.active.is_none() {
            return Err(SceneError::NothingBuilt);
        }
        self.audit.take();
        let moment = request.moment.stage_moment();
        let until = self
            .now_ms()
            .saturating_add(request.seconds.saturating_mul(MILLISECONDS_PER_SECOND));
        while self.now_ms() < until {
            self.step(moment, !request.quiet)?;
        }
        let taken = self.take_run();
        let active = self.active.as_ref().ok_or(SceneError::NothingBuilt)?;
        Ok(RunReply {
            seconds: request.seconds,
            moment: request.moment.key().to_owned(),
            quiet: request.quiet,
            lines: taken.lines,
            buffer: active.pacer.playout().buffer().report(self.now_ms()),
        })
    }

    /// The look-ahead worker: carries out `request` when there is one, and answers with the planner
    /// as it stands and the audit lines of the call.
    pub fn lookahead(
        &mut self,
        request: Option<&LookaheadRequest>,
    ) -> Result<LookaheadReply, SceneError> {
        self.audit.take();
        if let Some(request) = request {
            let now_ms = self.now_ms();
            let snapshot = self.world.snapshot();
            let context = BenchContext {
                now_ms,
                snapshot: &snapshot,
                current: self.active.as_ref().map(|active| active.scene),
                next_scene: self.next_scene,
                view: self.plan_view(),
                providers: &mut self.worker,
            };
            self.lookahead.handle(request, context)?;
        }
        let taken = self.take_run();
        Ok(LookaheadReply {
            now_ms: self.now_ms(),
            report: self.lookahead.worker().report(self.now_ms()),
            lines: taken.lines,
        })
    }

    /// What every open scene has stored and where its jobs stand.
    pub fn show(&self) -> SceneShow {
        let on_screen = self.active.as_ref().and_then(|active| {
            active
                .pacer
                .playout()
                .buffer()
                .current()
                .map(|shown| LocateReport::of(shown.locate()))
        });
        SceneShow {
            now_ms: self.now_ms(),
            scenes: self.scene_reports(),
            on_screen,
        }
    }

    /// The scene on screen's report: what is stored, what is in flight and its card.
    fn active_report(&self) -> Option<SceneReport> {
        let scene = self.active.as_ref()?.scene;
        self.scene_reports()
            .into_iter()
            .find(|report| report.scene == scene.get())
    }

    /// The clock, the queue, the jobs in flight and the cost of the interval.
    pub fn plates(&self) -> PlatesReport {
        let cost = self.active.as_ref().and_then(|active| {
            let purposes = self.builder.purposes_of(active.scene)?;
            let has_people = self.world.snapshot().heroes().next().is_some();
            Some(cost_view(
                self.timing,
                self.builder.purpose_table(),
                purposes,
                &active.brief,
                has_people,
            ))
        });
        PlatesReport {
            now_ms: self.now_ms(),
            scheduler: self
                .active
                .as_ref()
                .map(|active| active.pacer.scheduler().report()),
            in_flight: self
                .active_report()
                .map(|report| report.jobs)
                .unwrap_or_default(),
            cost,
            mean_build_ms: self.finished_build_ms.checked_div(self.plates_finished),
            plates_finished: self.plates_finished,
        }
    }

    /// The playout buffer and the establishing card.
    pub fn buffer_show(&self) -> BufferShow {
        let now = self.now_ms();
        BufferShow {
            now_ms: now,
            buffer: self
                .active
                .as_ref()
                .map(|active| active.pacer.playout().buffer().report(now)),
            card: self.active_report().map(|report| report.card),
        }
    }
}
