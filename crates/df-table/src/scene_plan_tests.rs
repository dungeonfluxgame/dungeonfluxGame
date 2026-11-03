use crate::event::FailureCause;
use crate::plate_fixtures_tests::{SCENE, market, picture, plan};
use crate::private_data::Narration;
use crate::scene_plan::{PlanError, SceneStage, StageOutput, StageOutputs, StagePlan};
use crate::scene_plan_locate::{LocateOutcome, UnlocatedReason};
use crate::table_state::StaleCause;

fn description() -> StageOutput {
    StageOutput::Description(Narration::new("A wet quay under paper lanterns."))
}

fn empty_locate() -> StageOutput {
    StageOutput::Locate(
        LocateOutcome::unlocated(&[], &[], UnlocatedReason::NoInstance).expect("an outcome"),
    )
}

#[test]
fn the_stages_run_in_order_and_each_starts_from_the_one_before() {
    assert_eq!(
        SceneStage::ALL
            .iter()
            .map(|stage| stage.key())
            .collect::<Vec<_>>(),
        vec!["describe", "background", "plate", "locate", "ready"]
    );
    assert_eq!(SceneStage::Describe.needs(), None);
    for pair in SceneStage::ALL.windows(2) {
        assert_eq!(pair[1].needs(), Some(pair[0]));
    }
}

#[test]
fn a_stage_cannot_begin_until_the_stage_before_it_has_a_stored_output() {
    let world = market();
    let snapshot = world.snapshot();
    let mut plan = plan(&world, 1, 0);
    assert_eq!(plan.next_stage(), Some(SceneStage::Describe));
    assert_eq!(
        plan.begin(&snapshot, SceneStage::Background),
        Err(PlanError::MissingInput {
            stage: SceneStage::Background,
            needs: SceneStage::Describe,
        })
    );
    plan.begin(&snapshot, SceneStage::Describe).expect("begins");
    assert_eq!(plan.next_stage(), None, "one stage at a time");
    assert_eq!(
        plan.begin(&snapshot, SceneStage::Background),
        Err(PlanError::StageRunning {
            running: SceneStage::Describe
        })
    );
    plan.store(&snapshot, description()).expect("stores");
    assert_eq!(plan.next_stage(), Some(SceneStage::Background));
    plan.begin(&snapshot, SceneStage::Background)
        .expect("starts from the stored description");
}

#[test]
fn a_job_walks_every_stage_and_finishes_with_the_plate_and_its_anchors() {
    let world = market();
    let snapshot = world.snapshot();
    let mut plan = plan(&world, 1, 2_000);
    let walk = [
        (SceneStage::Describe, description()),
        (SceneStage::Background, StageOutput::Background(picture(1))),
        (SceneStage::Plate, StageOutput::Plate(picture(2))),
        (SceneStage::Locate, empty_locate()),
    ];
    for (stage, output) in walk {
        assert_eq!(plan.next_stage(), Some(stage));
        plan.begin(&snapshot, stage).expect("begins");
        plan.store(&snapshot, output).expect("stores");
    }
    assert_eq!(plan.next_stage(), Some(SceneStage::Ready));
    let ready = plan.finish(&snapshot, 9_500).expect("finishes");
    assert_eq!(ready.plate, picture(2));
    assert_eq!(
        ready.build_ms(),
        7_500,
        "measured from the tick, not from the first request"
    );
    assert!(plan.is_finished());
    assert_eq!(plan.next_stage(), None);
    assert_eq!(plan.finish(&snapshot, 9_600), Err(PlanError::Ended));
}

#[test]
fn a_job_after_the_first_starts_at_plate_with_the_scene_s_stored_description_and_background() {
    let world = market();
    let snapshot = world.snapshot();
    let carried = StageOutputs {
        description: Some(Narration::new("A wet quay.")),
        background: Some(picture(1)),
        plate: Some(picture(2)),
        locate: Some(
            LocateOutcome::unlocated(&[], &[], UnlocatedReason::NoInstance).expect("an outcome"),
        ),
    };
    let job = crate::plate_fixtures_tests::job(&world, 2, 15_000);
    let mut plan = StagePlan::for_job(job, &carried);
    assert_eq!(
        plan.outputs().plate,
        None,
        "the earlier job's plate is not carried"
    );
    assert_eq!(plan.next_stage(), Some(SceneStage::Plate));
    plan.begin(&snapshot, SceneStage::Plate).expect("begins");
}

#[test]
fn an_output_for_a_stage_that_is_not_running_is_refused() {
    let world = market();
    let snapshot = world.snapshot();
    let mut plan = plan(&world, 1, 0);
    assert_eq!(
        plan.store(&snapshot, description()),
        Err(PlanError::NotRunning {
            stage: SceneStage::Describe
        })
    );
    plan.begin(&snapshot, SceneStage::Describe).expect("begins");
    assert_eq!(
        plan.store(&snapshot, StageOutput::Background(picture(1))),
        Err(PlanError::NotRunning {
            stage: SceneStage::Background
        })
    );
}

#[test]
fn a_failed_stage_ends_the_job_and_nothing_starts_after_it() {
    let world = market();
    let snapshot = world.snapshot();
    let mut plan = plan(&world, 1, 0);
    plan.begin(&snapshot, SceneStage::Describe).expect("begins");
    let failure = plan
        .fail(SceneStage::Describe, FailureCause::TimedOut)
        .expect("fails");
    assert_eq!(failure.stage, SceneStage::Describe);
    assert_eq!(failure.cause, FailureCause::TimedOut);
    assert!(plan.has_ended());
    assert_eq!(plan.next_stage(), None);
    assert_eq!(
        plan.begin(&snapshot, SceneStage::Describe),
        Err(PlanError::Ended)
    );
}

#[test]
fn a_result_for_a_scene_that_was_entered_again_is_rejected_and_not_stored() {
    let mut world = market();
    let mut plan = plan(&world, 1, 0);
    plan.begin(&world.snapshot(), SceneStage::Describe)
        .expect("begins");
    world.enter_scene(SCENE);
    let refusal = plan
        .store(&world.snapshot(), description())
        .expect_err("the generation moved on");
    let PlanError::Stale(stale) = refusal else {
        panic!("expected a stale refusal, got {refusal:?}");
    };
    assert_eq!(stale.cause, StaleCause::GenerationMismatch);
    assert_eq!(plan.outputs().description, None, "nothing was stored");
}

#[test]
fn a_stale_job_starts_no_request_it_would_pay_for() {
    let mut world = market();
    let mut plan = plan(&world, 1, 0);
    world.enter_scene(11);
    let refusal = plan
        .begin(&world.snapshot(), SceneStage::Describe)
        .expect_err("the scene is closed");
    let PlanError::Stale(stale) = refusal else {
        panic!("expected a stale refusal, got {refusal:?}");
    };
    assert_eq!(stale.cause, StaleCause::SceneClosed);
    assert_eq!(plan.running(), None);
}

#[test]
fn a_paused_table_holds_a_result_back_and_a_resume_before_it_arrives_keeps_it_stale() {
    let mut world = market();
    let mut plan = plan(&world, 1, 0);
    plan.begin(&world.snapshot(), SceneStage::Describe)
        .expect("begins");
    world.pause();
    let held = plan
        .store(&world.snapshot(), description())
        .expect_err("the table is paused");
    let PlanError::Stale(stale) = held else {
        panic!("expected a stale refusal, got {held:?}");
    };
    assert_eq!(stale.cause, StaleCause::TablePaused);
    assert_eq!(
        plan.running(),
        Some(SceneStage::Describe),
        "the stage is still running"
    );
    world.resume();
    let after = plan
        .store(&world.snapshot(), description())
        .expect_err("it started before the pause");
    let PlanError::Stale(stale) = after else {
        panic!("expected a stale refusal, got {after:?}");
    };
    assert_eq!(stale.cause, StaleCause::BeforePause);
}

#[test]
fn ready_needs_a_stored_plate_and_stored_anchors() {
    let world = market();
    let snapshot = world.snapshot();
    let mut plan = plan(&world, 1, 0);
    assert!(matches!(
        plan.finish(&snapshot, 1),
        Err(PlanError::MissingInput {
            needs: SceneStage::Plate,
            ..
        })
    ));
}

#[test]
fn only_the_description_and_the_background_are_carried_into_the_next_job() {
    let outputs = StageOutputs {
        description: Some(Narration::new("A quay.")),
        background: Some(picture(1)),
        plate: Some(picture(2)),
        locate: None,
    };
    let carried = outputs.carried_forward();
    assert!(carried.has(SceneStage::Describe));
    assert!(carried.has(SceneStage::Background));
    assert!(!carried.has(SceneStage::Plate));
    assert!(!carried.has(SceneStage::Locate));
    assert!(!carried.has(SceneStage::Ready), "ready is never stored");
}
