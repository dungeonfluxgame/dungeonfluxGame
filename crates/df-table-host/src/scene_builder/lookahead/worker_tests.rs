//! The look-ahead worker over the real broker and the scripted fake providers, on a manual clock:
//! the acceptance cases of SB-LA (hit, miss, stale, preempt, cancel). No live provider is called and
//! nothing is spent; time is an explicit number of milliseconds and no test sleeps.

use std::sync::Arc;

use df_table::effects::TableEffect;
use df_table::element::ElementId;
use df_table::identity::{SceneId, TableId};
use df_table::lookahead::{Expectation, Expected, PlanView, Take};
use df_table::lookahead_settings::LookaheadSettings;
use df_table::plate_timing::{PlateTiming, SceneBuildKind};
use df_table::ports::{Clock, PurposeName};
use df_table::private_data::Narration;
use df_table::scene_plan_brief::SceneBrief;

use super::worker::LookaheadWorker;
use crate::broker::providers::ScriptedCall;
use crate::broker::request::{ProviderFault, WorkerId};
use crate::broker::scheduler::BrokerLimits;
use crate::broker::{BrokerAudit, BrokerEvent, BrokerHub};
use crate::scene_builder::builder::{BuilderOutput, SceneBuilder};
use crate::scene_builder::dev_world::{DevWorld, ManualClock, dev_brief, dev_script};
use crate::scene_builder::events::{SceneBuilderAudit, SceneBuilderEvent};
use crate::scene_builder::fakes::{FakeStageWorker, StageScript};
use crate::scene_builder::inputs::StageInputs;
use crate::scene_builder::purposes::{BuildPriority, scene_purpose_table};
use crate::scene_builder::tests_support::{RecordingAudit, plate_of, requested_stages};

const STEP_MS: u64 = 100;
const CURRENT: u64 = 1;
const NEXT: u64 = 2;
const OTHER: u64 = 3;
/// Describe takes 400 ms and the picture 2,000 ms by default.
const BUILD_MS: u64 = 2_500;

fn purpose(name: &str) -> PurposeName {
    PurposeName::new(name).expect("a valid purpose name")
}

struct Rig {
    clock: Arc<ManualClock>,
    hub: Arc<BrokerHub>,
    audit: Arc<RecordingAudit>,
    providers: FakeStageWorker,
    builder: SceneBuilder,
    world: DevWorld,
    lookahead: LookaheadWorker,
}

fn rig_with(script: StageScript, limits: BrokerLimits, settings: LookaheadSettings) -> Rig {
    let clock = Arc::new(ManualClock::default());
    let audit = Arc::new(RecordingAudit::default());
    let table = scene_purpose_table().expect("the purposes are valid");
    let hub = Arc::new(BrokerHub::new(
        table.clone(),
        limits,
        Arc::clone(&audit) as Arc<dyn BrokerAudit>,
        Arc::clone(&clock) as Arc<dyn Clock>,
    ));
    let inputs = Arc::new(StageInputs::default());
    let providers = FakeStageWorker::new(
        Arc::clone(&hub),
        Arc::clone(&inputs),
        WorkerId::new(1),
        script,
    );
    let lookahead = LookaheadWorker::new(
        Arc::clone(&hub),
        Arc::clone(&audit) as Arc<dyn SceneBuilderAudit>,
        Arc::clone(&inputs),
        settings,
    )
    .expect("the worker");
    let builder = SceneBuilder::new(
        TableId::new(1),
        table,
        Arc::clone(&hub),
        Arc::clone(&audit) as Arc<dyn SceneBuilderAudit>,
        inputs,
        PlateTiming::defaults(),
    );
    let mut world = DevWorld::new().expect("the dev table");
    world.enter_scene(SceneId::new(CURRENT)).expect("enters");
    world.people_of(SceneId::new(CURRENT)).expect("the party");
    Rig {
        clock,
        hub,
        audit,
        providers,
        builder,
        world,
        lookahead,
    }
}

fn rig() -> Rig {
    rig_with(
        dev_script().expect("the dev script"),
        BrokerLimits::default(),
        LookaheadSettings::defaults(),
    )
}

fn certain(number: u64, place: &str) -> Expected {
    Expected {
        brief: dev_brief(SceneId::new(number), place).expect("a brief"),
        expectation: Expectation::AuthoredBeat,
        likelihood_permille: 1_000,
    }
}

impl Rig {
    fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    fn expect(&mut self, list: Vec<Expected>) {
        let now = self.now();
        self.lookahead
            .set_expected(
                now,
                &self.world.snapshot(),
                Some(SceneId::new(CURRENT)),
                list,
            )
            .expect("expected");
    }

    fn step(&mut self, view: PlanView) -> Vec<BuilderOutput> {
        self.clock.advance_ms(STEP_MS);
        let now = self.now();
        self.providers.pump(now);
        let snapshot = self.world.snapshot();
        self.lookahead.advance(now, &snapshot, view);
        self.builder.advance(now, &snapshot)
    }

    fn run(&mut self, ms: u64, view: PlanView) {
        let until = self.now() + ms;
        while self.now() < until {
            self.step(view);
        }
    }

    /// The table enters scene `number` (a place called `place`): the world changes, the look-ahead is
    /// asked, and the scene is opened in the builder with what it built, as the pacer does.
    fn transition(&mut self, number: u64, place: &str) -> Take {
        self.transition_after(number, place, |_| {})
    }

    /// As `transition`, with `change` made to the world after the party has entered the scene.
    fn transition_after(
        &mut self,
        number: u64,
        place: &str,
        change: impl FnOnce(&mut DevWorld),
    ) -> Take {
        let scene = SceneId::new(number);
        let generation = self.world.enter_scene(scene).expect("enters");
        self.world.people_of(scene).expect("the party");
        change(&mut self.world);
        let snapshot = self.world.snapshot();
        let brief = dev_brief(scene, place).expect("a brief");
        let now = self.now();
        let take = self.lookahead.enter(now, scene, &snapshot, &brief);
        self.builder
            .open_scene(
                brief,
                generation,
                SceneBuildKind::Standard,
                BuildPriority::Current,
                now,
            )
            .expect("opens");
        if let Take::Hit(booked) | Take::Redress(booked) = &take {
            self.builder.adopt(scene, &booked.outputs).expect("adopts");
        }
        take
    }

    /// Takes a plate job of scene `number` and runs it to its plate; the time it took.
    fn build_first_plate(&mut self, number: u64) -> Option<u64> {
        let scene = SceneId::new(number);
        let brief = dev_brief(scene, "any").expect("a brief");
        let job = df_table::plate_job::PlateJob::capture(
            df_table::plate_job::PlateJobId::new(1),
            df_table::plate_job::JobCause::SceneEntered,
            self.now(),
            &self.world.snapshot(),
            &brief,
        )
        .expect("a job");
        self.builder.submit(job).expect("submits");
        let started = self.now();
        for _ in 0..2_000 {
            let outputs = self.step(PlanView::default());
            if plate_of(&outputs).is_some() {
                return Some(self.now() - started);
            }
        }
        None
    }

    fn lines_with(&self, needle: &str) -> usize {
        self.audit
            .scene_lines()
            .iter()
            .filter(|line| line.contains(needle))
            .count()
    }

    fn broker_purposes(&self) -> Vec<String> {
        self.audit.requested_purposes()
    }
}

#[test]
fn the_next_scene_is_ready_when_the_transition_comes_and_its_first_plate_skips_the_dress_step() {
    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(BUILD_MS + 500, PlanView::default());
    assert_eq!(
        rig.broker_purposes(),
        vec!["lookahead_describe", "lookahead_background"],
        "only the look-ahead class was asked"
    );
    assert_eq!(rig.lines_with("LOOKAHEAD ready scene=2"), 1);
    assert_eq!(rig.lookahead.planner().counts().built, 1);

    let take = rig.transition(NEXT, "Quay");
    assert!(matches!(take, Take::Hit(_)), "{take:?}");
    assert_eq!(rig.lines_with("LOOKAHEAD taken scene=2 outcome=hit"), 1);
    assert_eq!(
        rig.lines_with("stage_stored scene=2 generation=2 job=0 stage=describe source=lookahead"),
        1
    );
    assert_eq!(
        rig.lines_with("stage_stored scene=2 generation=2 job=0 stage=background source=lookahead"),
        1
    );
    let with_look_ahead = rig.build_first_plate(NEXT).expect("a plate");
    assert_eq!(
        requested_stages(&rig.audit),
        vec!["plate", "locate"],
        "the scene's own build starts at the Plate stage"
    );

    let mut plain = rig_with(
        dev_script().expect("the dev script"),
        BrokerLimits::default(),
        LookaheadSettings::defaults().with_enabled(false),
    );
    let _ = plain.transition(NEXT, "Quay");
    let without = plain.build_first_plate(NEXT).expect("a plate");
    assert!(
        without >= with_look_ahead + BUILD_MS - STEP_MS,
        "the transition waited {without} ms without look-ahead and {with_look_ahead} ms with it"
    );
}

#[test]
fn a_changed_story_discards_the_build_in_flight_and_builds_the_new_target() {
    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(1_000, PlanView::default());
    assert_eq!(rig.lookahead.running(), 1, "the picture is still out");

    rig.expect(vec![certain(OTHER, "Keep")]);
    assert_eq!(
        rig.lines_with("LOOKAHEAD cancelled scene=2 token=1 reason=retargeted"),
        1
    );
    assert!(
        rig.audit
            .broker_events()
            .iter()
            .any(|event| matches!(event, BrokerEvent::Cancelled { purpose, .. } if purpose.as_str() == "lookahead_background")),
        "the broker request of the stale build is cancelled"
    );
    rig.run(BUILD_MS + 500, PlanView::default());
    let counts = rig.lookahead.planner().counts();
    assert_eq!((counts.started, counts.built, counts.cancelled), (2, 1, 1));

    assert_eq!(
        rig.transition(NEXT, "Quay"),
        Take::Miss,
        "nothing of the stale build is kept"
    );
    assert_eq!(
        rig.lines_with("source=lookahead"),
        0,
        "and nothing of it was adopted"
    );
}

#[test]
fn a_build_made_for_other_facts_is_stale_at_the_transition_and_is_not_adopted() {
    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(BUILD_MS + 500, PlanView::default());
    let take = rig.transition(NEXT, "Another place");
    assert_eq!(take.key(), "stale", "{take:?}");
    assert_eq!(rig.lines_with("LOOKAHEAD taken scene=2 outcome=stale"), 1);
    assert_eq!(
        rig.lines_with("source=lookahead"),
        0,
        "the stale picture is not handed to the scene"
    );
    let counts = rig.lookahead.planner().counts();
    assert_eq!((counts.stale, counts.hit), (1, 0));
}

#[test]
fn a_slow_provider_never_delays_the_transition_and_the_normal_build_runs_unchanged() {
    let slow = StageScript::default()
        .with_people(Vec::new())
        .with_latency(&purpose("lookahead_background"), 90_000);
    let mut rig = rig_with(slow, BrokerLimits::default(), LookaheadSettings::defaults());
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(5_000, PlanView::default());
    assert_eq!(rig.lookahead.running(), 1, "the picture is still not made");

    let before = rig.now();
    let take = rig.transition(NEXT, "Quay");
    assert_eq!(take, Take::Late);
    assert_eq!(
        rig.now(),
        before,
        "the transition asked and waited for nothing"
    );
    assert_eq!(
        rig.lines_with("LOOKAHEAD cancelled scene=2 token=1 reason=late"),
        1
    );
    let fallback = rig.build_first_plate(NEXT).expect("a plate");

    let mut plain = rig_with(
        StageScript::default(),
        BrokerLimits::default(),
        LookaheadSettings::defaults().with_enabled(false),
    );
    let _ = plain.transition(NEXT, "Quay");
    let normal = plain.build_first_plate(NEXT).expect("a plate");
    assert_eq!(
        fallback, normal,
        "a late look-ahead leaves the scene's own build exactly as it was"
    );
    assert!(
        requested_stages(&rig.audit).starts_with(&["describe", "background", "plate"]),
        "the scene is built the normal way"
    );
}

#[test]
fn the_lowest_class_never_starves_a_foreground_request() {
    let limits = BrokerLimits {
        image_slots: 1,
        ..BrokerLimits::default()
    };
    let script = dev_script()
        .expect("the dev script")
        .with_latency(&purpose("lookahead_background"), 30_000);
    let mut rig = rig_with(script, limits, LookaheadSettings::defaults());
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(1_000, PlanView::default());
    assert_eq!(
        rig.lookahead.running(),
        1,
        "the look-ahead picture holds the only image slot"
    );

    // The current scene needs its own picture now.
    let scene = SceneId::new(CURRENT);
    let brief = dev_brief(scene, "Harbor market").expect("a brief");
    let generation = rig
        .world
        .snapshot()
        .scene(scene)
        .expect("open")
        .generation();
    rig.builder
        .open_scene(
            brief,
            generation,
            SceneBuildKind::Standard,
            BuildPriority::Current,
            rig.now(),
        )
        .expect("opens");
    let foreground = rig.build_first_plate(CURRENT).expect("a plate");
    let mut alone = rig_with(
        dev_script().expect("the dev script"),
        limits,
        LookaheadSettings::defaults().with_enabled(false),
    );
    let scene = SceneId::new(CURRENT);
    let brief = dev_brief(scene, "Harbor market").expect("a brief");
    let generation = alone
        .world
        .snapshot()
        .scene(scene)
        .expect("open")
        .generation();
    alone
        .builder
        .open_scene(
            brief,
            generation,
            SceneBuildKind::Standard,
            BuildPriority::Current,
            0,
        )
        .expect("opens");
    let baseline = alone.build_first_plate(CURRENT).expect("a plate");
    assert_eq!(
        foreground, baseline,
        "the foreground build took {foreground} ms with a look-ahead call out and {baseline} ms alone"
    );
    assert!(
        rig.audit.broker_events().iter().any(|event| matches!(
            event,
            BrokerEvent::Preempted { purpose, .. } if purpose.as_str() == "lookahead_background"
        )),
        "the foreground picture preempted the look-ahead call"
    );

    rig.run(40_000, PlanView::default());
    assert_eq!(
        rig.lookahead.planner().counts().built,
        1,
        "the preempted look-ahead build was queued again and finished once the lane was free"
    );
}

#[test]
fn nothing_new_is_asked_while_the_current_scene_needs_the_lane() {
    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay")]);
    let busy = PlanView {
        current_ready_ms: None,
        foreground_busy: true,
    };
    rig.run(3_000, busy);
    assert!(
        rig.broker_purposes().is_empty(),
        "no look-ahead request while the plate has the lane"
    );
    rig.run(300, PlanView::default());
    assert_eq!(rig.broker_purposes(), vec!["lookahead_describe"]);
}

#[test]
fn a_paused_worker_starts_nothing_and_lets_the_build_that_is_out_finish() {
    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(300, PlanView::default());
    rig.lookahead.pause();
    rig.run(BUILD_MS, PlanView::default());
    assert_eq!(
        rig.lookahead.planner().counts().built,
        1,
        "the describe request was already out"
    );
    assert_eq!(rig.lines_with("LOOKAHEAD paused paused=true"), 1);
    rig.lookahead.resume();
    assert_eq!(rig.lines_with("LOOKAHEAD paused paused=false"), 1);
}

#[test]
fn a_build_whose_epoch_passed_without_the_owner_saying_so_is_discarded() {
    let slow = StageScript::default().with_latency(&purpose("lookahead_background"), 30_000);
    let mut rig = rig_with(slow, BrokerLimits::default(), LookaheadSettings::defaults());
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(1_000, PlanView::default());
    // The table enters a scene and the owner does not call `enter`: the broker's epoch moves.
    let scene = SceneId::new(OTHER);
    let generation = rig.world.enter_scene(scene).expect("enters");
    let brief = dev_brief(scene, "Keep").expect("a brief");
    rig.builder
        .open_scene(
            brief,
            generation,
            SceneBuildKind::Standard,
            BuildPriority::Current,
            rig.now(),
        )
        .expect("opens");
    rig.run(200, PlanView::default());
    assert_eq!(
        rig.lines_with("LOOKAHEAD discarded scene=2 token=1 stage=background reason=epoch_moved"),
        1
    );
    assert_eq!(rig.lookahead.running(), 0);
    assert_eq!(
        rig.lookahead.planner().counts().built,
        0,
        "nothing of it was kept"
    );
}

#[test]
fn closing_the_table_cancels_every_build_and_empties_the_book() {
    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay"), certain(OTHER, "Keep")]);
    rig.lookahead.set_settings(
        LookaheadSettings::defaults()
            .with(df_table::lookahead_settings::LookaheadSetting::Depth, 2)
            .expect("depth"),
    );
    rig.run(500, PlanView::default());
    assert_eq!(rig.lookahead.running(), 2);
    let now = rig.now();
    rig.lookahead.close(now, &rig.world.snapshot());
    assert_eq!(rig.lookahead.running(), 0);
    assert_eq!(rig.lookahead.planner().booked(), 0);
    assert_eq!(rig.lines_with("LOOKAHEAD cancelled"), 2);
    assert!(
        rig.audit
            .broker_events()
            .iter()
            .filter(|event| matches!(event, BrokerEvent::Cancelled { .. }))
            .count()
            >= 2,
        "the broker requests were cancelled"
    );
}

#[test]
fn a_scene_nobody_took_stays_in_the_book_and_is_reused_when_the_table_goes_there() {
    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(BUILD_MS + 500, PlanView::default());
    let requests_before = rig.broker_purposes().len();
    assert_eq!(
        rig.transition(OTHER, "Keep"),
        Take::Miss,
        "the story went another way"
    );
    assert_eq!(
        rig.lookahead.planner().booked(),
        1,
        "the built scene is kept"
    );
    rig.run(5_000, PlanView::default());
    assert_eq!(
        rig.broker_purposes().len(),
        requests_before,
        "and nothing more is asked"
    );
    // The table goes to the built scene later with the party in the same state.
    let take = rig.transition(NEXT, "Quay");
    assert!(matches!(take, Take::Hit(_)), "{take:?}");
}

#[test]
fn an_authored_description_is_not_asked_for_again() {
    let mut rig = rig();
    let authored = SceneBrief::new(
        SceneId::new(NEXT),
        df_table::table_state::DisplayName::new("Quay").expect("a name"),
    )
    .described(Narration::new("Gulls over a wet stone quay."));
    rig.expect(vec![Expected {
        brief: authored,
        expectation: Expectation::TravelLeg,
        likelihood_permille: 0,
    }]);
    rig.run(BUILD_MS, PlanView::default());
    assert_eq!(rig.broker_purposes(), vec!["lookahead_background"]);
    assert_eq!(
        rig.lines_with("LOOKAHEAD stage_stored scene=2 token=1 stage=describe source=scene_record"),
        1
    );
    assert_eq!(rig.lookahead.planner().counts().built, 1);
}

#[test]
fn a_refused_stage_fails_the_build_once_and_the_transition_says_why() {
    let script = dev_script().expect("the dev script").with_calls(
        &purpose("lookahead_background"),
        [ScriptedCall::fails_after(0, ProviderFault::Refused)],
    );
    let mut rig = rig_with(
        script,
        BrokerLimits::default(),
        LookaheadSettings::defaults(),
    );
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(BUILD_MS + 5_000, PlanView::default());
    assert_eq!(
        rig.lines_with("LOOKAHEAD failed scene=2 token=1 stage=background cause=refused"),
        1
    );
    assert_eq!(
        rig.broker_purposes().len(),
        2,
        "no second try for the same facts"
    );
    let take = rig.transition(NEXT, "Quay");
    assert_eq!(take.key(), "failed");
    let _ = rig.hub.epoch();
}

#[test]
fn every_look_ahead_line_is_one_audit_line_with_ids_and_keys_and_no_player_text() {
    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(BUILD_MS + 500, PlanView::default());
    let _ = rig.transition(NEXT, "Quay");
    for event in rig.audit.scene_events() {
        if let SceneBuilderEvent::LookaheadStarted { .. }
        | SceneBuilderEvent::LookaheadStored { .. }
        | SceneBuilderEvent::LookaheadReady { .. }
        | SceneBuilderEvent::LookaheadTaken { .. }
        | SceneBuilderEvent::LookaheadTargets { .. } = &event
        {
            let line = event.render_audit_line();
            assert!(line.starts_with("LOOKAHEAD "), "{line}");
            assert!(!line.contains('\n'), "{line}");
            assert!(
                !line.contains("Quay") && !line.contains("quiet place"),
                "{line}"
            );
        }
    }
}

#[test]
fn a_build_that_settled_since_the_last_advance_is_a_hit_not_late() {
    let mut reference = rig();
    reference.expect(vec![certain(NEXT, "Quay")]);
    while reference.lookahead.planner().counts().built == 0 {
        reference.step(PlanView::default());
    }
    let settled_at = reference.now();

    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay")]);
    while rig.now() + STEP_MS < settled_at {
        rig.step(PlanView::default());
    }
    // The last request settles at `settled_at`; the owner has not called `advance` since.
    rig.clock.advance_ms(STEP_MS);
    let now = rig.now();
    rig.providers.pump(now);
    assert_eq!(rig.lookahead.planner().counts().built, 0, "not seen yet");
    let take = rig.transition(NEXT, "Quay");
    assert!(matches!(take, Take::Hit(_)), "{take:?}");
    assert_eq!(rig.lookahead.planner().counts().late, 0);
}

#[test]
fn a_changed_party_keeps_the_picture_and_asks_for_the_description_again() {
    let mut rig = rig();
    rig.expect(vec![certain(NEXT, "Quay")]);
    rig.run(BUILD_MS + 500, PlanView::default());
    let before = rig.broker_purposes().len();
    let take = rig.transition_after(NEXT, "Quay", |world| {
        world
            .apply(&[TableEffect::CastLeft {
                cast: ElementId::new("mara").expect("an id"),
            }])
            .expect("mara leaves");
    });
    assert_eq!(take.key(), "redress", "{take:?}");
    assert_eq!(rig.lines_with("LOOKAHEAD taken scene=2 outcome=redress"), 1);
    assert_eq!(
        rig.lines_with("stage_stored scene=2 generation=2 job=0 stage=background source=lookahead"),
        1,
        "the picture is adopted"
    );
    assert_eq!(
        rig.lines_with("stage_stored scene=2 generation=2 job=0 stage=describe source=lookahead"),
        0,
        "the description is not"
    );
    rig.build_first_plate(NEXT).expect("a plate");
    let asked: Vec<String> = rig.broker_purposes().split_off(before);
    let own = |stage: &str| {
        asked
            .iter()
            .filter(|purpose| !purpose.starts_with("lookahead_") && purpose.ends_with(stage))
            .count()
    };
    assert_eq!(own("describe"), 1, "the dress step runs again: {asked:?}");
    assert_eq!(
        own("background"),
        0,
        "the picture is not made again: {asked:?}"
    );
}
