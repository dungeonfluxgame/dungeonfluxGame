use crate::plate_buffer::{
    Delivery, GenerationRegression, PlayoutBuffer, RejectReason, StageMoment, StepAction,
    WaitReason,
};
use crate::plate_fixtures_tests::{SCENE, World, job, market, ready_plate, scene, timing};
use crate::plate_timing::PLATE_DISSOLVE_MINIMUM_MS;

const INTERVAL_MS: u64 = 15_000;
const READING_MS: u64 = 3_000;

fn buffer(world: &World) -> PlayoutBuffer {
    PlayoutBuffer::new(scene(SCENE), world.generation(), timing())
}

/// A job taken at `tick_ms`, expected by the buffer and finished at `ready_ms`.
fn expected(buffer: &mut PlayoutBuffer, world: &World, id: u64, tick_ms: u64) {
    buffer
        .expect(&job(world, id, tick_ms))
        .expect("the buffer expects it");
}

fn deliver(buffer: &mut PlayoutBuffer, world: &World, id: u64, tick_ms: u64, ready_ms: u64) {
    let plate = ready_plate(job(world, id, tick_ms), ready_ms);
    assert_eq!(buffer.deliver(plate), Delivery::Accepted);
}

fn swapped_to(action: StepAction) -> u64 {
    match action {
        StepAction::Swapped(swap) => swap.to,
        other => panic!("expected a swap, got {other:?}"),
    }
}

#[test]
fn the_first_plate_dissolves_in_only_at_a_line_boundary() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 9_000);
    let mid_line = buffer.step(9_000, StageMoment::MidLine, READING_MS);
    assert_eq!(
        mid_line.action,
        StepAction::Waiting {
            reason: WaitReason::Moment(StageMoment::MidLine),
            waited_ms: 0
        }
    );
    assert!(buffer.current().is_none());
    let boundary = buffer.step(9_400, StageMoment::LineBoundary, READING_MS);
    let StepAction::Swapped(swap) = boundary.action else {
        panic!("expected a swap, got {:?}", boundary.action);
    };
    assert_eq!(swap.to, 1);
    assert_eq!(swap.from, None);
    assert!(swap.dissolve_ms >= PLATE_DISSOLVE_MINIMUM_MS);
    assert_eq!(
        swap.content_age_ms, 9_400,
        "the picture is as old as its tick"
    );
}

#[test]
fn a_plate_never_swaps_during_a_dice_card_a_result_card_or_a_reveal_beat() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 1_000);
    for moment in [StageMoment::DiceOrResultCard, StageMoment::RevealBeat] {
        let step = buffer.step(2_000, moment, READING_MS);
        assert!(
            matches!(step.action, StepAction::Waiting { .. }),
            "{moment:?} must hold the plate"
        );
    }
    assert!(buffer.current().is_none());
}

#[test]
fn a_plate_that_waits_past_plate_swap_wait_s_is_reported_overdue_once_and_never_forced() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 1_000);
    let first = buffer.step(1_000, StageMoment::MidLine, READING_MS);
    assert!(!first.swap_overdue);
    let almost = buffer.step(6_999, StageMoment::MidLine, READING_MS);
    assert!(!almost.swap_overdue);
    let overdue = buffer.step(7_000, StageMoment::MidLine, READING_MS);
    assert!(overdue.swap_overdue, "6 s of waiting for a boundary");
    assert_eq!(
        overdue.action,
        StepAction::Waiting {
            reason: WaitReason::Moment(StageMoment::MidLine),
            waited_ms: 6_000
        }
    );
    let again = buffer.step(8_000, StageMoment::DiceOrResultCard, READING_MS);
    assert!(!again.swap_overdue, "reported once");
    assert!(buffer.current().is_none(), "still not forced in");
    assert_eq!(buffer.stats().swaps_overdue, 1);
}

#[test]
fn plates_show_in_order_one_interval_apart_never_in_a_burst() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    deliver(&mut buffer, &world, 2, 15_000, 200);
    deliver(&mut buffer, &world, 3, 30_000, 300);
    assert_eq!(
        swapped_to(
            buffer
                .step(300, StageMoment::LineBoundary, READING_MS)
                .action
        ),
        1
    );
    let early = buffer.step(5_000, StageMoment::LineBoundary, READING_MS);
    assert_eq!(
        early.action,
        StepAction::Waiting {
            reason: WaitReason::SlotNotDue {
                due_in_ms: INTERVAL_MS - 4_700
            },
            waited_ms: 0
        }
    );
    assert_eq!(
        swapped_to(
            buffer
                .step(300 + INTERVAL_MS, StageMoment::LineBoundary, READING_MS)
                .action
        ),
        2
    );
    assert_eq!(
        swapped_to(
            buffer
                .step(300 + 2 * INTERVAL_MS, StageMoment::LineBoundary, READING_MS)
                .action
        ),
        3
    );
}

#[test]
fn a_pipeline_that_catches_up_is_paced_by_the_slot_not_by_when_the_plates_finished() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    swapped_to(
        buffer
            .step(100, StageMoment::LineBoundary, READING_MS)
            .action,
    );
    expected(&mut buffer, &world, 2, 15_000);
    expected(&mut buffer, &world, 3, 30_000);
    deliver(&mut buffer, &world, 2, 15_000, 100_000);
    deliver(&mut buffer, &world, 3, 30_000, 100_050);
    let late = buffer.step(100_100, StageMoment::LineBoundary, READING_MS);
    assert_eq!(swapped_to(late.action), 2);
    let StepAction::Swapped(swap) = late.action else {
        panic!("expected a swap");
    };
    assert_eq!(swap.slot_late_ms, 100_100 - (100 + INTERVAL_MS));
    assert_eq!(swap.content_age_ms, 100_100 - 15_000);
    let next = buffer.step(100_200, StageMoment::LineBoundary, READING_MS);
    assert!(
        matches!(
            next.action,
            StepAction::Waiting {
                reason: WaitReason::SlotNotDue { .. },
                ..
            }
        ),
        "the plate that finished at once waits its turn: no burst"
    );
    assert_eq!(
        swapped_to(
            buffer
                .step(100_100 + INTERVAL_MS, StageMoment::LineBoundary, READING_MS)
                .action
        ),
        3
    );
}

#[test]
fn when_the_next_plate_is_not_ready_the_current_one_stays_and_the_lateness_is_reported() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    swapped_to(
        buffer
            .step(100, StageMoment::LineBoundary, READING_MS)
            .action,
    );
    expected(&mut buffer, &world, 2, INTERVAL_MS);
    let step = buffer.step(20_100, StageMoment::LineBoundary, READING_MS);
    assert_eq!(step.action, StepAction::Holding);
    assert_eq!(step.lateness.slot_late_ms, 20_100 - (100 + INTERVAL_MS));
    assert_eq!(
        step.lateness.oldest_outstanding_age_ms,
        Some(20_100 - INTERVAL_MS)
    );
    assert_eq!(buffer.current().expect("still showing").job().get(), 1);
}

#[test]
fn a_quiet_table_has_no_lateness_and_the_slot_passes_on_the_current_plate() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    swapped_to(
        buffer
            .step(100, StageMoment::LineBoundary, READING_MS)
            .action,
    );
    let step = buffer.step(100_000, StageMoment::LineBoundary, READING_MS);
    assert_eq!(step.action, StepAction::Holding);
    assert_eq!(step.lateness.slot_late_ms, 0);
    assert_eq!(step.lateness.oldest_outstanding_age_ms, None);
    assert!(!step.catching_up);
}

#[test]
fn no_plate_yet_and_nothing_expected_leaves_the_establishing_card_alone() {
    let world = market();
    let mut buffer = buffer(&world);
    let step = buffer.step(5_000, StageMoment::LineBoundary, READING_MS);
    assert_eq!(step.action, StepAction::Quiet);
}

#[test]
fn a_job_still_being_built_is_never_swapped_in_half_built() {
    let world = market();
    let mut buffer = buffer(&world);
    expected(&mut buffer, &world, 1, 0);
    let step = buffer.step(60_000, StageMoment::LineBoundary, READING_MS);
    assert_eq!(step.action, StepAction::Holding);
    assert!(buffer.current().is_none());
    assert_eq!(step.lateness.oldest_outstanding_age_ms, Some(60_000));
}

#[test]
fn a_plate_waits_for_the_reading_time_of_the_lines_over_the_one_on_screen() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    swapped_to(buffer.step(100, StageMoment::LineBoundary, 30_000).action);
    deliver(&mut buffer, &world, 2, INTERVAL_MS, INTERVAL_MS);
    let step = buffer.step(100 + INTERVAL_MS, StageMoment::LineBoundary, 30_000);
    assert_eq!(
        step.action,
        StepAction::Waiting {
            reason: WaitReason::MinimumOnScreen {
                remaining_ms: 30_000 - INTERVAL_MS
            },
            waited_ms: 0
        }
    );
    let later = buffer.step(100 + 30_000, StageMoment::LineBoundary, 30_000);
    assert_eq!(swapped_to(later.action), 2);
}

#[test]
fn the_picture_catching_up_mark_needs_a_stale_plate_that_disagrees_with_the_table() {
    let mut world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    swapped_to(
        buffer
            .step(100, StageMoment::LineBoundary, READING_MS)
            .action,
    );
    world.move_to("pip", 40, 20);
    expected(&mut buffer, &world, 2, INTERVAL_MS);
    let young = buffer.step(60_000, StageMoment::LineBoundary, READING_MS);
    assert!(
        !young.catching_up,
        "45 s behind is under image_stale_limit_s"
    );
    let stale = buffer.step(105_000, StageMoment::LineBoundary, READING_MS);
    assert!(stale.catching_up, "90 s behind and the cast moved");
}

#[test]
fn a_stale_plate_that_still_agrees_with_the_table_shows_no_mark() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    swapped_to(
        buffer
            .step(100, StageMoment::LineBoundary, READING_MS)
            .action,
    );
    expected(&mut buffer, &world, 2, INTERVAL_MS);
    let step = buffer.step(200_000, StageMoment::LineBoundary, READING_MS);
    assert!(!step.catching_up, "the pending job draws the same picture");
}

#[test]
fn a_plate_of_another_scene_is_refused_and_never_shown() {
    let mut world = market();
    let mut buffer = buffer(&world);
    world.enter_scene(11);
    let other = crate::plate_job::PlateJob::capture(
        crate::plate_job::PlateJobId::new(1),
        crate::plate_job::JobCause::Tick,
        0,
        &world.snapshot(),
        &crate::scene_plan_brief::SceneBrief::new(
            scene(11),
            crate::plate_fixtures_tests::display_name("Back room"),
        ),
    )
    .expect("a job");
    assert_eq!(
        buffer.deliver(ready_plate(other, 10)),
        Delivery::Rejected(RejectReason::WrongScene)
    );
    assert_eq!(buffer.stats().rejected_wrong_scene, 1);
    assert_eq!(
        buffer
            .step(20, StageMoment::LineBoundary, READING_MS)
            .action,
        StepAction::Quiet
    );
}

#[test]
fn a_plate_of_an_earlier_generation_is_stored_as_late_and_never_shown() {
    let mut world = market();
    let mut buffer = buffer(&world);
    let old = job(&world, 1, 0);
    world.enter_scene(SCENE);
    buffer
        .focus_changed(scene(SCENE), world.generation())
        .expect("moves forward");
    assert_eq!(
        buffer.deliver(ready_plate(old, 10)),
        Delivery::Rejected(RejectReason::StaleGeneration)
    );
    assert_eq!(buffer.stats().rejected_stale_generation, 1);
    let step = buffer.step(20, StageMoment::LineBoundary, READING_MS);
    assert_eq!(step.action, StepAction::Quiet);
    assert!(buffer.current().is_none());
}

#[test]
fn a_scene_change_empties_the_buffer_so_the_old_scene_s_picture_is_never_under_the_new_one() {
    let mut world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    swapped_to(
        buffer
            .step(100, StageMoment::LineBoundary, READING_MS)
            .action,
    );
    deliver(&mut buffer, &world, 2, INTERVAL_MS, INTERVAL_MS);
    expected(&mut buffer, &world, 3, 2 * INTERVAL_MS);
    world.enter_scene(11);
    buffer
        .focus_changed(
            scene(11),
            world
                .snapshot()
                .scene(scene(11))
                .expect("open")
                .generation(),
        )
        .expect("moves forward");
    assert!(buffer.current().is_none());
    assert_eq!(buffer.outstanding(), 0);
    assert_eq!(buffer.scene(), scene(11));
}

#[test]
fn the_buffer_never_goes_back_to_an_older_generation() {
    let mut world = market();
    let mut buffer = buffer(&world);
    let first = world.generation();
    world.enter_scene(11);
    let second = world
        .snapshot()
        .scene(scene(11))
        .expect("open")
        .generation();
    buffer
        .focus_changed(scene(11), second)
        .expect("moves forward");
    assert_eq!(
        buffer.focus_changed(scene(SCENE), first),
        Err(GenerationRegression {
            current: second,
            requested: first
        })
    );
    assert_eq!(
        buffer.focus_changed(scene(11), second),
        Err(GenerationRegression {
            current: second,
            requested: second
        })
    );
    assert_eq!(buffer.scene(), scene(11));
}

#[test]
fn a_plate_older_than_the_one_on_screen_is_refused() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 2, INTERVAL_MS, 100);
    swapped_to(
        buffer
            .step(100, StageMoment::LineBoundary, READING_MS)
            .action,
    );
    let late = ready_plate(job(&world, 1, 0), 200);
    assert_eq!(
        buffer.deliver(late),
        Delivery::Rejected(RejectReason::OlderThanShown)
    );
    assert_eq!(buffer.stats().rejected_older_than_shown, 1);
    assert_eq!(buffer.current().expect("unchanged").job().get(), 2);
}

#[test]
fn a_job_that_gave_up_no_longer_makes_the_slot_late() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    swapped_to(
        buffer
            .step(100, StageMoment::LineBoundary, READING_MS)
            .action,
    );
    let failing = job(&world, 2, INTERVAL_MS);
    buffer.expect(&failing).expect("expected");
    let late = buffer.step(40_000, StageMoment::LineBoundary, READING_MS);
    assert!(late.lateness.slot_late_ms > 0);
    buffer.forget(failing.id());
    let quiet = buffer.step(40_100, StageMoment::LineBoundary, READING_MS);
    assert_eq!(quiet.lateness.slot_late_ms, 0);
    assert_eq!(buffer.stats().forgotten_jobs, 1);
}

#[test]
fn the_report_names_the_plate_on_screen_what_is_ready_and_what_is_pending() {
    let world = market();
    let mut buffer = buffer(&world);
    deliver(&mut buffer, &world, 1, 0, 100);
    swapped_to(
        buffer
            .step(100, StageMoment::LineBoundary, READING_MS)
            .action,
    );
    deliver(&mut buffer, &world, 2, INTERVAL_MS, 200);
    expected(&mut buffer, &world, 3, 2 * INTERVAL_MS);
    let report = buffer.report(1_000);
    assert_eq!(report.current.as_ref().map(|shown| shown.job), Some(1));
    assert_eq!(report.ready, vec![2]);
    assert_eq!(report.pending.len(), 1);
    assert_eq!(report.pending[0].job, 3);
    assert_eq!(report.next_slot_ms, Some(100 + INTERVAL_MS));
}
