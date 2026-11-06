use std::sync::Arc;

use df_table::plate::buffer::{StageMoment, StepAction, Swap, WaitReason};
use df_table::plate::scheduler::JobEnd;
use df_table::plate::timing::{PlateTiming, SceneBuildKind};

use super::dev_world::dev_script;
use super::events::{SceneBuilderAudit, SceneBuilderEvent};
use super::fakes::StageScript;
use super::fixtures::{Rig, STEP_MS};
use super::pacer::{PacerError, ScenePacer};
use crate::broker::providers::ScriptedCall;
use crate::broker::request::ProviderFault;

const READING_MS: u64 = 3_000;
const BEAT_MS: u64 = 15_000;

/// The pacer, the builder and the fake worker on one clock.
struct PacerRig {
    rig: Rig,
    pacer: ScenePacer,
    swaps: Vec<(u64, Swap)>,
}

impl PacerRig {
    fn new(script: StageScript) -> Self {
        let mut rig = Rig::unopened(script);
        let mut pacer = ScenePacer::new(
            rig.scene,
            PlateTiming::default(),
            Arc::clone(&rig.audit) as Arc<dyn SceneBuilderAudit>,
        );
        let snapshot = rig.world.snapshot();
        pacer
            .enter_scene(
                0,
                &snapshot,
                &rig.brief,
                SceneBuildKind::Standard,
                &mut rig.builder,
            )
            .expect("enters");
        Self {
            rig,
            pacer,
            swaps: Vec::new(),
        }
    }

    fn standard() -> Self {
        Self::new(dev_script().expect("the dev script"))
    }

    /// One tenth of a second.
    fn pump(&mut self, moment: StageMoment) -> StepAction {
        self.rig.clock.advance_ms(STEP_MS);
        let now = self.rig.now_ms();
        self.rig.worker.pump(now);
        let snapshot = self.rig.world.snapshot();
        let pumped = self
            .pacer
            .pump(
                now,
                &snapshot,
                &self.rig.brief,
                &mut self.rig.builder,
                moment,
                READING_MS,
            )
            .expect("pumps");
        if let StepAction::Swapped(swap) = pumped.step.action {
            self.swaps.push((now, swap));
        }
        pumped.step.action
    }

    /// Runs `ms` of time; when `changes` is set, a creature moves just before each tick is due.
    fn run(&mut self, ms: u64, moment: StageMoment, changes: bool) {
        let until = self.rig.now_ms() + ms;
        while self.rig.now_ms() < until {
            let due = self.pacer.scheduler().next_tick_ms();
            if changes && due.is_some_and(|due| self.rig.now_ms() + STEP_MS >= due) {
                let step = u32::try_from(self.rig.now_ms() / 1_000 % 7).expect("small");
                self.rig
                    .world
                    .move_cast("pip", 30 + 4 * step, 20)
                    .expect("a visible change");
            }
            self.pump(moment);
        }
    }

    fn lines_with(&self, verb: &str) -> usize {
        self.rig
            .audit
            .scene_lines()
            .iter()
            .filter(|line| line.contains(&format!(" {verb} ")))
            .count()
    }
}

#[test]
fn the_first_plate_reaches_the_screen_only_at_a_line_boundary() {
    let mut bench = PacerRig::standard();
    let mut waited_mid_line = false;
    for _ in 0..600 {
        match bench.pump(StageMoment::MidLine) {
            StepAction::Waiting { reason, .. } => {
                assert_eq!(reason, WaitReason::Moment(StageMoment::MidLine));
                waited_mid_line = true;
                break;
            }
            StepAction::Swapped(_) => panic!("a plate swapped in mid-line"),
            StepAction::Quiet | StepAction::Holding => {}
        }
    }
    assert!(
        waited_mid_line,
        "the plate finished and waited for a boundary"
    );
    assert!(bench.pacer.playout().buffer().current().is_none());
    let action = bench.pump(StageMoment::LineBoundary);
    let StepAction::Swapped(swap) = action else {
        panic!("expected a swap at the boundary, got {action:?}");
    };
    assert_eq!(swap.from, None);
    assert!(swap.dissolve_ms >= 1_500);
    let card = bench
        .rig
        .builder
        .card(
            bench.rig.scene,
            bench.rig.now_ms(),
            &bench.rig.world.snapshot(),
            true,
        )
        .expect("a card");
    assert_eq!(
        card.state.key(),
        "retired",
        "the plate on screen retires the card"
    );
}

#[test]
fn a_visible_change_each_beat_makes_one_plate_a_beat_shown_in_order_one_interval_apart() {
    let mut bench = PacerRig::standard();
    bench.run(80_000, StageMoment::LineBoundary, true);
    assert!(bench.swaps.len() >= 4, "{} swaps", bench.swaps.len());
    let jobs: Vec<u64> = bench.swaps.iter().map(|(_, swap)| swap.to).collect();
    let mut sorted = jobs.clone();
    sorted.sort_unstable();
    assert_eq!(jobs, sorted, "plates are shown in order");
    for pair in bench.swaps.windows(2) {
        assert!(
            pair[1].0 - pair[0].0 >= BEAT_MS,
            "swaps at {} and {} are closer than a beat",
            pair[0].0,
            pair[1].0
        );
    }
}

#[test]
fn a_quiet_table_after_the_first_plate_pays_for_no_more() {
    let mut bench = PacerRig::standard();
    bench.run(70_000, StageMoment::LineBoundary, false);
    assert_eq!(bench.swaps.len(), 1, "only the scene's first plate");
    let plates = bench
        .rig
        .audit
        .requested_purposes()
        .iter()
        .filter(|purpose| purpose.as_str() == "plate")
        .count();
    assert_eq!(plates, 1, "no plate was paid for after the first");
    assert!(
        bench.lines_with("tick_skipped") >= 3,
        "every quiet beat is logged as skipped"
    );
    assert_eq!(bench.lines_with("late"), 0, "a quiet table is never late");
    assert!(
        bench
            .rig
            .audit
            .scene_lines()
            .iter()
            .any(|line| line.contains("reason=nothing_changed"))
    );
}

#[test]
fn a_slow_pipeline_keeps_the_current_plate_and_reports_the_lateness_once_for_the_slot() {
    let plate = Rig::purpose("plate");
    let script = dev_script()
        .expect("the dev script")
        .with_calls(&plate, [ScriptedCall::succeeds_after(2_000)])
        .with_latency(&plate, 60_000);
    let mut bench = PacerRig::new(script);
    bench.run(100_000, StageMoment::LineBoundary, true);
    assert!(bench.swaps.len() >= 2);
    let (second_at, second) = bench.swaps[1];
    assert!(
        second.content_age_ms >= 60_000,
        "the second plate was {} ms old when shown",
        second.content_age_ms
    );
    assert!(
        second.slot_late_ms >= 40_000,
        "its slot was {} ms past",
        second.slot_late_ms
    );
    assert!(second_at > 60_000);
    let late: Vec<String> = bench
        .rig
        .audit
        .scene_lines()
        .into_iter()
        .filter(|line| line.starts_with("PLATE late "))
        .collect();
    assert_eq!(
        late.len(),
        1,
        "one late line for the slot, not one for every look: {late:?}"
    );
    let first_shown = bench.pacer.playout().buffer().stats().swaps;
    assert!(first_shown >= 2);
}

#[test]
fn a_swap_waits_out_a_dice_card_is_reported_overdue_after_six_seconds_and_comes_at_the_boundary() {
    let mut bench = PacerRig::standard();
    let mut overdue = 0;
    let mut waited_on_dice = false;
    for _ in 0..1_200 {
        let action = bench.pump(StageMoment::DiceOrResultCard);
        if let StepAction::Waiting { reason, waited_ms } = action {
            assert_eq!(reason, WaitReason::Moment(StageMoment::DiceOrResultCard));
            waited_on_dice = true;
            if waited_ms >= 6_000 {
                overdue = bench.lines_with("swap_overdue");
                if bench.lines_with("swap_overdue") == 1 && waited_ms >= 7_000 {
                    break;
                }
            }
        }
        assert!(
            !matches!(action, StepAction::Swapped(_)),
            "no swap over a dice card"
        );
    }
    assert!(waited_on_dice);
    assert_eq!(overdue, 1, "reported once");
    assert!(
        bench.pacer.playout().buffer().current().is_none(),
        "never forced in"
    );
    let action = bench.pump(StageMoment::LineBoundary);
    assert!(matches!(action, StepAction::Swapped(_)), "{action:?}");
}

#[test]
fn a_scene_entered_again_empties_the_screen_and_the_old_scenes_plates_never_show() {
    let mut bench = PacerRig::standard();
    bench.run(25_000, StageMoment::LineBoundary, false);
    assert_eq!(bench.swaps.len(), 1);
    bench
        .rig
        .world
        .move_cast("pip", 40, 20)
        .expect("a visible change");
    bench.run(BEAT_MS, StageMoment::LineBoundary, false);
    let old_generation = bench.rig.generation;
    let shown_before = bench.swaps.len();
    bench
        .rig
        .world
        .enter_scene(bench.rig.scene)
        .expect("enters again");
    let generation = bench
        .rig
        .world
        .snapshot()
        .scene(bench.rig.scene)
        .expect("open")
        .generation();
    assert!(generation > old_generation);
    let snapshot = bench.rig.world.snapshot();
    let now = bench.rig.now_ms();
    let ended = bench
        .pacer
        .enter_scene(
            now,
            &snapshot,
            &bench.rig.brief,
            SceneBuildKind::Standard,
            &mut bench.rig.builder,
        )
        .expect("enters again");
    assert!(
        ended.iter().all(|outcome| outcome.end == JobEnd::Stale),
        "{ended:?}"
    );
    assert!(
        bench.pacer.playout().buffer().current().is_none(),
        "the old picture is gone"
    );
    assert_eq!(bench.pacer.playout().buffer().generation(), generation);
    bench.run(40_000, StageMoment::LineBoundary, false);
    let new_swaps = &bench.swaps[shown_before..];
    assert_eq!(
        new_swaps.len(),
        1,
        "only the new scene's first plate was shown"
    );
}

#[test]
fn a_plate_that_failed_is_asked_for_again_on_the_next_beat_though_nothing_changed() {
    let plate = Rig::purpose("plate");
    let script = dev_script().expect("the dev script").with_calls(
        &plate,
        [
            ScriptedCall::succeeds_after(2_000),
            ScriptedCall::fails_after(0, ProviderFault::Refused),
        ],
    );
    let mut bench = PacerRig::new(script);
    bench.run(25_000, StageMoment::LineBoundary, false);
    bench
        .rig
        .world
        .move_cast("pip", 40, 20)
        .expect("one visible change");
    bench.run(60_000, StageMoment::LineBoundary, false);
    assert_eq!(
        bench.lines_with("tick_queued"),
        3,
        "the first plate, the changed beat, and the beat after its failure"
    );
    assert!(bench.swaps.len() >= 2, "the second asking made a plate");
}

#[test]
fn entering_a_scene_under_a_generation_that_is_not_forward_is_refused() {
    let mut bench = PacerRig::standard();
    let snapshot = bench.rig.world.snapshot();
    let now = bench.rig.now_ms();
    let result = bench.pacer.enter_scene(
        now,
        &snapshot,
        &bench.rig.brief,
        SceneBuildKind::Standard,
        &mut bench.rig.builder,
    );
    assert!(
        matches!(result, Err(PacerError::Generation(_))),
        "{result:?}"
    );
}

#[test]
fn the_events_of_a_run_name_the_tick_the_delivery_and_the_swap() {
    let mut bench = PacerRig::standard();
    bench.run(30_000, StageMoment::LineBoundary, false);
    let lines = bench.rig.audit.scene_lines();
    let verbs = ["tick_queued", "delivered", "swapped"];
    for verb in verbs {
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with(&format!("PLATE {verb} "))),
            "no {verb} line in {lines:?}"
        );
    }
    let swapped = lines
        .iter()
        .find(|line| line.starts_with("PLATE swapped "))
        .expect("a swap line");
    assert!(
        swapped.contains("from=none") && swapped.contains("dissolve_ms=1500"),
        "{swapped}"
    );
    assert!(
        bench
            .rig
            .audit
            .scene_events()
            .iter()
            .any(|event| matches!(event, SceneBuilderEvent::JobReady { .. }))
    );
}
