use crate::effects::TableEffect;
use crate::event::FailureCause;
use crate::identity::SceneId;
use crate::limits::LARGE_LOOKAHEAD_MAXIMUM;
use crate::lookahead::settings::{DEFAULT_LOOKAHEAD_DEPTH, LookaheadSetting, LookaheadSettings};
use crate::lookahead::stamp::{DAY_PART_SECONDS, StampChange, WorldStamp};
use crate::lookahead::{
    Action, BookState, BuildOutcome, BuildToken, CERTAIN_PERMILLE, CancelReason, Candidate,
    Expectation, Expected, FailedBuild, LookaheadError, LookaheadPlanner, PlanView, Take,
    lookahead_candidates,
};
use crate::plate_fixtures_tests::{World, display_name, market, picture, scene};
use crate::private_data::Narration;
use crate::scene_plan::brief::SceneBrief;
use crate::scene_plan::{SceneStage, StageOutputs};

const CURRENT: u64 = 10;
const NEXT: u64 = 11;
const OTHER: u64 = 12;
const START_MS: u64 = 10_000;
const DEADLINE_MS: u64 = 120_000;
const BOOK_AGE_MS: u64 = 900_000;

fn brief_of(number: u64, place: &str) -> SceneBrief {
    SceneBrief::new(scene(number), display_name(place))
}

fn expected(number: u64, expectation: Expectation, permille: u16) -> Expected {
    Expected {
        brief: brief_of(number, &format!("Place {number}")),
        expectation,
        likelihood_permille: permille,
    }
}

fn candidates(world: &World, list: Vec<Expected>) -> Vec<Candidate> {
    lookahead_candidates(&world.snapshot(), Some(scene(CURRENT)), list).expect("candidates")
}

fn planner() -> LookaheadPlanner {
    LookaheadPlanner::new(LookaheadSettings::default())
}

fn outputs() -> StageOutputs {
    StageOutputs {
        description: Some(Narration::new("A quiet place.")),
        background: Some(picture(7)),
        ..StageOutputs::default()
    }
}

fn started(actions: &[Action]) -> Vec<(SceneId, BuildToken)> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Start { token, brief, .. } => Some((brief.scene(), *token)),
            _ => None,
        })
        .collect()
}

fn cancelled(actions: &[Action]) -> Vec<(SceneId, CancelReason)> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Cancel { scene, reason, .. } => Some((*scene, *reason)),
            _ => None,
        })
        .collect()
}

/// A planner that expects `NEXT` for certain and has started its build.
fn building(world: &World) -> (LookaheadPlanner, BuildToken) {
    let mut planner = planner();
    planner
        .set_expected(candidates(
            world,
            vec![expected(NEXT, Expectation::AuthoredBeat, CERTAIN_PERMILLE)],
        ))
        .expect("expected");
    let actions = planner.plan(0, PlanView::default());
    let (_, token) = started(&actions)[0];
    (planner, token)
}

fn ready(world: &World, ready_ms: u64) -> LookaheadPlanner {
    let (mut planner, token) = building(world);
    planner
        .build_finished(ready_ms, scene(NEXT), token, BuildOutcome::Ready(outputs()))
        .expect("finished");
    planner
}

fn stamp_now(world: &World, number: u64) -> WorldStamp {
    WorldStamp::of(
        &world.snapshot(),
        &brief_of(number, &format!("Place {number}")),
    )
}

#[test]
fn candidates_are_ranked_certain_first_and_leave_out_the_current_scene() {
    let world = market();
    let ranked = candidates(
        &world,
        vec![
            expected(OTHER, Expectation::ExitHint, 700),
            expected(CURRENT, Expectation::TravelLeg, CERTAIN_PERMILLE),
            expected(NEXT, Expectation::TravelLeg, 0),
            expected(13, Expectation::ExitHint, 900),
        ],
    );
    let order: Vec<u64> = ranked
        .iter()
        .map(|candidate| candidate.scene().get())
        .collect();
    assert_eq!(
        order,
        vec![NEXT, 13, OTHER],
        "certain, then likelihood; no current scene"
    );
    assert_eq!(
        ranked[0].likelihood_permille(),
        CERTAIN_PERMILLE,
        "a certain scene is certain"
    );
}

#[test]
fn a_scene_named_twice_keeps_its_more_certain_entry_and_a_wild_likelihood_is_refused() {
    let world = market();
    let ranked = candidates(
        &world,
        vec![
            expected(NEXT, Expectation::ExitHint, 600),
            expected(NEXT, Expectation::AuthoredBeat, 0),
        ],
    );
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].expectation(), Expectation::AuthoredBeat);
    let refused = lookahead_candidates(
        &world.snapshot(),
        None,
        vec![expected(NEXT, Expectation::ExitHint, 1_001)],
    );
    assert_eq!(
        refused.unwrap_err(),
        LookaheadError::LikelihoodOutOfRange {
            scene: scene(NEXT),
            permille: 1_001
        }
    );
}

#[test]
fn a_certain_next_scene_starts_at_once_and_only_once() {
    let world = market();
    let mut planner = planner();
    planner
        .set_expected(candidates(
            &world,
            vec![expected(NEXT, Expectation::AuthoredBeat, CERTAIN_PERMILLE)],
        ))
        .expect("expected");
    let first = planner.plan(0, PlanView::default());
    assert_eq!(started(&first).len(), 1);
    assert!(
        planner.plan(1, PlanView::default()).is_empty(),
        "no second build of the scene"
    );
    assert_eq!(planner.counts().started, 1);
}

#[test]
fn a_likely_scene_waits_for_the_current_scene_to_be_ready_and_the_start_delay() {
    let world = market();
    let mut planner = LookaheadPlanner::new(
        LookaheadSettings::default()
            .with(LookaheadSetting::Depth, 2)
            .expect("depth"),
    );
    planner
        .set_expected(candidates(
            &world,
            vec![expected(NEXT, Expectation::ExitHint, 800)],
        ))
        .expect("expected");
    assert!(
        planner.plan(50_000, PlanView::default()).is_empty(),
        "the scene is not ready yet"
    );
    let ready_at = PlanView {
        current_ready_ms: Some(1_000),
        foreground_busy: false,
    };
    assert!(
        planner.plan(1_000 + START_MS - 1, ready_at).is_empty(),
        "inside the delay"
    );
    assert_eq!(started(&planner.plan(1_000 + START_MS, ready_at)).len(), 1);
}

#[test]
fn depth_limits_how_many_scenes_are_built_at_once() {
    let world = market();
    let list = vec![
        expected(NEXT, Expectation::AuthoredBeat, CERTAIN_PERMILLE),
        expected(OTHER, Expectation::ExitHint, 900),
        expected(13, Expectation::ExitHint, 800),
    ];
    let view = PlanView {
        current_ready_ms: Some(0),
        foreground_busy: false,
    };
    let mut one = planner();
    assert_eq!(DEFAULT_LOOKAHEAD_DEPTH, 1);
    one.set_expected(candidates(&world, list.clone()))
        .expect("expected");
    let first: Vec<u64> = started(&one.plan(60_000, view))
        .iter()
        .map(|(scene, _)| scene.get())
        .collect();
    assert_eq!(first, vec![NEXT], "depth 1 builds the certain scene only");
    let mut three = LookaheadPlanner::new(
        LookaheadSettings::default()
            .with(LookaheadSetting::Depth, 3)
            .expect("depth"),
    );
    three
        .set_expected(candidates(&world, list))
        .expect("expected");
    let order: Vec<u64> = started(&three.plan(60_000, view))
        .iter()
        .map(|(scene, _)| scene.get())
        .collect();
    assert_eq!(order, vec![NEXT, OTHER, 13], "in rank order");
}

#[test]
fn nothing_new_starts_while_the_current_scene_needs_the_lane_or_the_worker_is_paused() {
    let world = market();
    let mut planner = planner();
    planner
        .set_expected(candidates(
            &world,
            vec![expected(NEXT, Expectation::AuthoredBeat, CERTAIN_PERMILLE)],
        ))
        .expect("expected");
    let busy = PlanView {
        current_ready_ms: Some(0),
        foreground_busy: true,
    };
    assert!(
        planner.plan(0, busy).is_empty(),
        "the current plate has the lane"
    );
    planner.pause();
    assert!(planner.plan(1, PlanView::default()).is_empty(), "paused");
    planner.resume();
    assert_eq!(
        started(&planner.plan(2, PlanView::default())).len(),
        1,
        "resumed"
    );
}

#[test]
fn a_paused_worker_lets_a_build_that_is_out_run_on() {
    let world = market();
    let (mut planner, token) = building(&world);
    planner.pause();
    assert!(planner.plan(5, PlanView::default()).is_empty());
    assert!(
        planner.is_building(scene(NEXT), token),
        "pausing cancels nothing"
    );
}

#[test]
fn the_scene_is_a_hit_when_the_transition_finds_it_ready_and_the_world_unchanged() {
    let world = market();
    let mut planner = ready(&world, 2_400);
    let entered = planner.enter(scene(NEXT), &stamp_now(&world, NEXT));
    match entered.take {
        Take::Hit(booked) => {
            assert!(booked.outputs.description.is_some() && booked.outputs.background.is_some());
            assert_eq!(booked.ready_ms, 2_400);
        }
        other => panic!("expected a hit, got {other:?}"),
    }
    assert!(entered.actions.is_empty());
    assert_eq!(planner.counts().hit, 1);
    assert_eq!(planner.booked(), 0, "a taken scene leaves the book");
}

#[test]
fn a_changed_cast_keeps_the_picture_and_drops_the_description() {
    let mut world = market();
    let mut planner = ready(&world, 2_400);
    world.apply(&[TableEffect::CastLeft {
        cast: crate::plate_fixtures_tests::element("mara"),
    }]);
    match planner.enter(scene(NEXT), &stamp_now(&world, NEXT)).take {
        Take::Redress(booked) => {
            assert!(
                booked.outputs.description.is_none(),
                "the dress step runs again"
            );
            assert!(booked.outputs.background.is_some(), "the picture is kept");
        }
        other => panic!("expected a redress, got {other:?}"),
    }
    assert_eq!(planner.counts().redressed, 1);
}

#[test]
fn a_changed_place_or_part_of_the_day_makes_the_build_stale() {
    let mut world = market();
    let mut by_place = ready(&world, 2_400);
    let other_place = WorldStamp::of(&world.snapshot(), &brief_of(NEXT, "Another place"));
    assert_eq!(
        by_place.enter(scene(NEXT), &other_place).take,
        Take::Stale {
            change: StampChange::PlaceOrTime
        }
    );
    let mut by_time = ready(&world, 2_400);
    world.apply(&[TableEffect::TimeAdvanced {
        advance_s: DAY_PART_SECONDS,
    }]);
    assert_eq!(
        by_time.enter(scene(NEXT), &stamp_now(&world, NEXT)).take,
        Take::Stale {
            change: StampChange::PlaceOrTime
        }
    );
    assert_eq!(by_time.counts().stale, 1);
}

#[test]
fn entering_before_the_build_is_done_cancels_it_and_the_table_builds_the_normal_way() {
    let world = market();
    let (mut planner, token) = building(&world);
    let entered = planner.enter(scene(NEXT), &stamp_now(&world, NEXT));
    assert_eq!(entered.take, Take::Late);
    assert_eq!(
        cancelled(&entered.actions),
        vec![(scene(NEXT), CancelReason::Late)]
    );
    let refused = planner.build_finished(9_000, scene(NEXT), token, BuildOutcome::Ready(outputs()));
    assert_eq!(
        refused,
        Err(LookaheadError::NoBuildOf { scene: scene(NEXT) })
    );
    assert_eq!(planner.counts().late, 1);
}

#[test]
fn a_scene_nobody_built_is_a_miss_and_a_failed_build_says_why() {
    let world = market();
    let mut planner = planner();
    assert_eq!(
        planner.enter(scene(NEXT), &stamp_now(&world, NEXT)).take,
        Take::Miss
    );
    let (mut failing, token) = building(&world);
    failing
        .build_finished(
            400,
            scene(NEXT),
            token,
            BuildOutcome::Failed {
                stage: SceneStage::Background,
                cause: FailureCause::Refused,
            },
        )
        .expect("recorded");
    let failure = FailedBuild {
        stage: Some(SceneStage::Background),
        cause: FailureCause::Refused,
    };
    let report = failing.report(400);
    assert_eq!(report.book[0].state, BookState::Failed);
    assert_eq!(report.book[0].failure, Some(failure));
    assert_eq!(failure.to_string(), "background refused");
    assert_eq!(
        serde_json::to_value(&report.book[0]).expect("a book entry serializes")["failure"],
        serde_json::json!({"stage": "background", "cause": "refused"})
    );
    assert_eq!(
        failing.enter(scene(NEXT), &stamp_now(&world, NEXT)).take,
        Take::Failed {
            stage: Some(SceneStage::Background),
            cause: FailureCause::Refused
        }
    );
}

#[test]
fn a_branch_cancels_the_build_and_a_late_result_for_it_is_refused() {
    let world = market();
    let (mut planner, token) = building(&world);
    let actions = planner
        .set_expected(candidates(
            &world,
            vec![expected(OTHER, Expectation::AuthoredBeat, CERTAIN_PERMILLE)],
        ))
        .expect("expected");
    assert_eq!(
        cancelled(&actions),
        vec![(scene(NEXT), CancelReason::Retargeted)]
    );
    let late = planner.build_finished(3_000, scene(NEXT), token, BuildOutcome::Ready(outputs()));
    assert_eq!(late, Err(LookaheadError::NoBuildOf { scene: scene(NEXT) }));
    let restarted = planner.plan(3_001, PlanView::default());
    assert_eq!(
        started(&restarted)
            .iter()
            .map(|(scene, _)| scene.get())
            .collect::<Vec<_>>(),
        vec![OTHER],
        "the new target is built instead"
    );
}

#[test]
fn a_result_of_a_replaced_build_is_refused_by_its_token() {
    let mut world = market();
    let (mut planner, old_token) = building(&world);
    world.apply(&[TableEffect::CastLeft {
        cast: crate::plate_fixtures_tests::element("mara"),
    }]);
    let actions = planner
        .set_expected(candidates(
            &world,
            vec![expected(NEXT, Expectation::AuthoredBeat, CERTAIN_PERMILLE)],
        ))
        .expect("expected");
    assert_eq!(
        cancelled(&actions),
        vec![(scene(NEXT), CancelReason::StampChanged)]
    );
    let (_, new_token) = started(&planner.plan(10, PlanView::default()))[0];
    assert_ne!(old_token, new_token);
    assert_eq!(
        planner.build_finished(20, scene(NEXT), old_token, BuildOutcome::Ready(outputs())),
        Err(LookaheadError::StaleBuild { scene: scene(NEXT) })
    );
    assert_eq!(planner.counts().refused_results, 1);
    planner
        .build_finished(30, scene(NEXT), new_token, BuildOutcome::Ready(outputs()))
        .expect("the current build is kept");
}

#[test]
fn entering_another_scene_cancels_running_builds_and_keeps_built_ones_for_reuse() {
    let world = market();
    let mut planner = LookaheadPlanner::new(
        LookaheadSettings::default()
            .with(LookaheadSetting::Depth, 2)
            .expect("depth"),
    );
    planner
        .set_expected(candidates(
            &world,
            vec![
                expected(NEXT, Expectation::AuthoredBeat, CERTAIN_PERMILLE),
                expected(OTHER, Expectation::TravelLeg, CERTAIN_PERMILLE),
            ],
        ))
        .expect("expected");
    let tokens = started(&planner.plan(0, PlanView::default()));
    let next_token = tokens
        .iter()
        .find(|(scene, _)| scene.get() == NEXT)
        .expect("built")
        .1;
    planner
        .build_finished(500, scene(NEXT), next_token, BuildOutcome::Ready(outputs()))
        .expect("finished");
    let entered = planner.enter(scene(13), &stamp_now(&world, 13));
    assert_eq!(entered.take, Take::Miss);
    assert_eq!(
        cancelled(&entered.actions),
        vec![(scene(OTHER), CancelReason::Branched)]
    );
    assert_eq!(planner.booked(), 1, "the finished scene stays in the book");
    assert!(
        matches!(
            planner.enter(scene(NEXT), &stamp_now(&world, NEXT)).take,
            Take::Hit(_)
        ),
        "and is reused when the table goes there later"
    );
}

#[test]
fn a_build_still_out_at_the_deadline_is_cancelled_and_not_started_again() {
    let world = market();
    let (mut planner, token) = building(&world);
    assert!(
        planner
            .plan(DEADLINE_MS - 1, PlanView::default())
            .is_empty()
    );
    let actions = planner.plan(DEADLINE_MS, PlanView::default());
    assert_eq!(
        cancelled(&actions),
        vec![(scene(NEXT), CancelReason::Deadline)]
    );
    assert!(!planner.is_building(scene(NEXT), token));
    assert!(
        planner
            .plan(DEADLINE_MS + 1_000, PlanView::default())
            .is_empty(),
        "no retry storm for the same facts"
    );
    assert_eq!(
        planner.enter(scene(NEXT), &stamp_now(&world, NEXT)).take,
        Take::Failed {
            stage: None,
            cause: FailureCause::TimedOut
        }
    );
}

#[test]
fn a_built_scene_nobody_took_ages_out_of_the_book() {
    let world = market();
    let mut planner = ready(&world, 1_000);
    planner.set_expected(Vec::new()).expect("expected");
    assert!(
        planner
            .plan(1_000 + BOOK_AGE_MS - 1, PlanView::default())
            .is_empty()
    );
    let actions = planner.plan(1_000 + BOOK_AGE_MS, PlanView::default());
    assert_eq!(actions, vec![Action::Expire { scene: scene(NEXT) }]);
    assert_eq!(planner.booked(), 0);
    assert_eq!(planner.counts().expired, 1);
}

#[test]
fn closing_the_table_cancels_every_build_and_turning_the_worker_off_does_too() {
    let world = market();
    let (mut closing, _) = building(&world);
    let actions = closing.close();
    assert_eq!(
        cancelled(&actions),
        vec![(scene(NEXT), CancelReason::Closed)]
    );
    assert_eq!(closing.booked(), 0);
    let (mut off, _) = building(&world);
    off.set_settings(LookaheadSettings::default().with_enabled(false));
    let actions = off.plan(1, PlanView::default());
    assert_eq!(
        cancelled(&actions),
        vec![(scene(NEXT), CancelReason::Disabled)]
    );
}

#[test]
fn settings_refuse_values_outside_their_range_and_name_the_setting() {
    let defaults = LookaheadSettings::default();
    let error = defaults.with(LookaheadSetting::Depth, 0).unwrap_err();
    assert_eq!(error.setting, LookaheadSetting::Depth);
    assert!(error.to_string().contains("lookahead_depth"));
    assert!(defaults.with(LookaheadSetting::DeadlineS, 0).is_err());
    assert!(
        defaults
            .with(
                LookaheadSetting::BookAgeS,
                u64::from(LARGE_LOOKAHEAD_MAXIMUM) + 1
            )
            .is_err()
    );
    assert_eq!(
        defaults
            .with(LookaheadSetting::StartS, 0)
            .expect("zero is at once")
            .start_ms(),
        0
    );
    for setting in LookaheadSetting::ALL {
        assert_eq!(defaults.value_of(*setting), setting.default_value());
    }
}

#[test]
fn the_report_lists_the_expected_scenes_the_book_and_the_counters() {
    let world = market();
    let (planner, _) = building(&world);
    let report = planner.report(2_000);
    assert!(report.enabled && !report.paused);
    assert_eq!(report.wanted.len(), 1);
    assert_eq!(report.wanted[0].expectation, Expectation::AuthoredBeat);
    assert!(report.wanted[0].certain);
    assert_eq!(report.book.len(), 1);
    assert_eq!(report.book[0].state, BookState::Building);
    assert_eq!(report.book[0].age_ms, 2_000);
    assert_eq!(report.counts.started, 1);
}

#[test]
fn a_stamp_tells_a_changed_cast_from_a_changed_place() {
    let mut world = market();
    let before = stamp_now(&world, NEXT);
    assert_eq!(
        before.change_to(&stamp_now(&world, NEXT)),
        StampChange::Unchanged
    );
    world.apply(&[TableEffect::CastLeft {
        cast: crate::plate_fixtures_tests::element("mara"),
    }]);
    assert_eq!(
        before.change_to(&stamp_now(&world, NEXT)),
        StampChange::CastOnly
    );
    let renamed = WorldStamp::of(&world.snapshot(), &brief_of(NEXT, "Elsewhere"));
    assert_eq!(before.change_to(&renamed), StampChange::PlaceOrTime);
}

fn gate_open() -> PlanView {
    PlanView {
        current_ready_ms: Some(0),
        foreground_busy: false,
    }
}

#[test]
fn a_certain_scene_cancels_the_likely_build_that_holds_the_only_slot() {
    let world = market();
    let mut planner = planner();
    planner
        .set_expected(candidates(
            &world,
            vec![expected(OTHER, Expectation::ExitHint, 900)],
        ))
        .expect("expected");
    assert_eq!(started(&planner.plan(60_000, gate_open())).len(), 1);
    planner
        .set_expected(candidates(
            &world,
            vec![
                expected(NEXT, Expectation::AuthoredBeat, CERTAIN_PERMILLE),
                expected(OTHER, Expectation::ExitHint, 900),
            ],
        ))
        .expect("expected");
    let actions = planner.plan(60_001, gate_open());
    assert_eq!(
        started(&actions)
            .iter()
            .map(|(scene, _)| scene.get())
            .collect::<Vec<_>>(),
        vec![NEXT],
        "the certain scene is started at once"
    );
    assert_eq!(
        cancelled(&actions),
        vec![(scene(OTHER), CancelReason::Retargeted)],
        "the likely build gives its slot up"
    );
    assert!(
        planner.plan(60_002, gate_open()).is_empty(),
        "the likely scene does not start again behind the certain one"
    );
}

#[test]
fn a_certain_scene_is_not_held_back_by_a_built_likely_scene_either() {
    let world = market();
    let mut planner = planner();
    planner
        .set_expected(candidates(
            &world,
            vec![expected(OTHER, Expectation::ExitHint, 900)],
        ))
        .expect("expected");
    let (_, token) = started(&planner.plan(60_000, gate_open()))[0];
    planner
        .build_finished(60_100, scene(OTHER), token, BuildOutcome::Ready(outputs()))
        .expect("finished");
    planner
        .set_expected(candidates(
            &world,
            vec![
                expected(NEXT, Expectation::TravelLeg, CERTAIN_PERMILLE),
                expected(OTHER, Expectation::ExitHint, 900),
            ],
        ))
        .expect("expected");
    let actions = planner.plan(60_101, gate_open());
    assert_eq!(started(&actions).len(), 1, "the certain scene starts");
    assert!(cancelled(&actions).is_empty(), "the built scene is kept");
    assert_eq!(planner.booked(), 2);
}

#[test]
fn an_unused_built_scene_stays_in_the_book_without_using_the_depth() {
    let world = market();
    let mut planner = ready(&world, 1_000);
    planner
        .set_expected(candidates(
            &world,
            vec![expected(OTHER, Expectation::TravelLeg, CERTAIN_PERMILLE)],
        ))
        .expect("expected");
    let actions = planner.plan(1_001, PlanView::default());
    assert_eq!(
        started(&actions)
            .iter()
            .map(|(scene, _)| scene.get())
            .collect::<Vec<_>>(),
        vec![OTHER],
        "depth 1 still builds the certain scene"
    );
    let report = planner.report(1_001);
    assert!(
        report
            .book
            .iter()
            .any(|entry| entry.scene == NEXT && entry.state == BookState::Ready),
        "the unused scene is still in the book"
    );
}

#[test]
fn a_closed_planner_refuses_expectations_and_starts_nothing() {
    let world = market();
    let (mut planner, _) = building(&world);
    planner.close();
    let refused = planner.set_expected(candidates(
        &world,
        vec![expected(NEXT, Expectation::AuthoredBeat, CERTAIN_PERMILLE)],
    ));
    assert_eq!(refused, Err(LookaheadError::Closed));
    assert!(started(&planner.plan(5, PlanView::default())).is_empty());
    assert!(planner.wanted().is_empty());
    assert!(LookaheadError::Closed.to_string().contains("closed"));
}

#[test]
fn a_full_depth_tick_changes_nothing_however_many_scenes_are_expected() {
    let world = market();
    let mut planner = planner();
    let mut list = vec![expected(NEXT, Expectation::AuthoredBeat, CERTAIN_PERMILLE)];
    for number in 100..2_100 {
        list.push(expected(number, Expectation::ExitHint, 500));
    }
    planner
        .set_expected(candidates(&world, list))
        .expect("expected");
    assert_eq!(started(&planner.plan(0, gate_open())).len(), 1);
    let before = planner.wanted().to_vec();
    assert!(planner.plan(60_000, gate_open()).is_empty());
    assert!(planner.plan(60_001, gate_open()).is_empty());
    assert_eq!(planner.wanted(), before.as_slice());
    assert_eq!(planner.counts().started, 1);
}

#[test]
fn every_book_state_key_is_its_serde_name() {
    for state in BookState::ALL {
        assert_eq!(
            serde_json::to_value(state).expect("a book state serializes"),
            serde_json::Value::from(state.key())
        );
        assert_eq!(BookState::from_key(state.key()), Some(*state));
    }
    assert_eq!(BookState::from_key("built"), None);
}
