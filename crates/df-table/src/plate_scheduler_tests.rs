use crate::plate::job::{JobError, JobReason, PlateJobId};
use crate::plate::scheduler::{
    JobEnd, MAXIMUM_QUEUED_JOBS, PlateScheduler, SkipReason, Tick, TickOutcome,
};
use crate::plate::timing::TimingSetting;
use crate::plate_fixtures_tests::{SCENE, World, brief, display_name, market, scene, timing};
use crate::scene_plan::brief::SceneBrief;
use crate::table_state::Visibility;

const INTERVAL_MS: u64 = 15_000;

fn scheduler() -> PlateScheduler {
    PlateScheduler::new(scene(SCENE), timing())
}

fn queued(tick: &Tick) -> PlateJobId {
    match tick.outcome {
        TickOutcome::Queued { job, .. } => job,
        other => panic!("expected a queued job, got {other:?}"),
    }
}

fn skipped_for(tick: &Tick) -> SkipReason {
    match tick.outcome {
        TickOutcome::Skipped { reason } => reason,
        other => panic!("expected a skipped tick, got {other:?}"),
    }
}

/// A scheduler that entered the scene at 0 and has had its first job taken.
fn running(world: &World) -> PlateScheduler {
    let mut scheduler = scheduler();
    scheduler
        .enter_scene(0, &world.snapshot(), &brief())
        .expect("enters");
    scheduler.take_next().expect("the first plate's job");
    scheduler
}

#[test]
fn entering_a_scene_queues_its_first_plate_and_starts_the_clock_one_interval_out() {
    let world = market();
    let mut scheduler = scheduler();
    let tick = scheduler
        .enter_scene(1_000, &world.snapshot(), &brief())
        .expect("enters");
    assert_eq!(queued(&tick), PlateJobId::new(1));
    assert_eq!(scheduler.next_tick_ms(), Some(1_000 + INTERVAL_MS));
    let job = scheduler.take_next().expect("a job");
    assert_eq!(job.reason(), JobReason::SceneEntered);
    assert_eq!(job.tick_ms(), 1_000, "a job keeps its tick time");
}

#[test]
fn a_tick_before_the_interval_has_passed_is_not_due_and_changes_nothing() {
    let world = market();
    let mut scheduler = running(&world);
    let tick = scheduler
        .plate_tick(INTERVAL_MS - 1, &world.snapshot(), &brief())
        .expect("ticks");
    assert_eq!(tick.outcome, TickOutcome::NotDue { due_in_ms: 1 });
    assert_eq!(scheduler.stats().ticks, 0);
}

#[test]
fn a_due_tick_with_a_visible_change_queues_a_job_and_returns_at_once() {
    let mut world = market();
    let mut scheduler = running(&world);
    world.move_to("pip", 33, 20);
    let tick = scheduler
        .plate_tick(INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    assert_eq!(queued(&tick), PlateJobId::new(2));
    assert_eq!(scheduler.queued(), 1);
    let job = scheduler.take_next().expect("a job");
    assert_eq!(job.tick_ms(), INTERVAL_MS);
    assert_eq!(job.stamp().generation, world.generation());
    assert_eq!(scheduler.next_tick_ms(), Some(2 * INTERVAL_MS));
}

#[test]
fn a_quiet_table_pays_for_no_plate() {
    let world = market();
    let mut scheduler = running(&world);
    let tick = scheduler
        .plate_tick(INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    assert_eq!(skipped_for(&tick), SkipReason::NothingChanged);
    assert_eq!(scheduler.queued(), 0);
    assert_eq!(scheduler.stats().skipped_nothing_changed, 1);
    assert_eq!(
        scheduler.next_tick_ms(),
        Some(2 * INTERVAL_MS),
        "the beat still arrives"
    );
}

#[test]
fn a_change_only_an_overlay_shows_needs_no_new_plate() {
    let mut world = market();
    let mut scheduler = running(&world);
    world.hurt("goblin", 3);
    let tick = scheduler
        .plate_tick(INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    assert_eq!(skipped_for(&tick), SkipReason::NothingChanged);
}

#[test]
fn a_creature_the_party_can_no_longer_see_changes_the_picture() {
    let mut world = market();
    let mut scheduler = running(&world);
    world.set_visibility("goblin", Visibility::Hidden);
    let tick = scheduler
        .plate_tick(INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    queued(&tick);
    let job = scheduler.take_next().expect("a job");
    let names: Vec<&str> = job.figures().iter().map(|f| f.id.as_str()).collect();
    assert_eq!(
        names,
        vec!["mara", "pip"],
        "a hidden creature is left out of the plate"
    );
}

#[test]
fn a_hidden_hero_is_still_drawn_but_a_sensed_threat_is_not() {
    let mut world = market();
    world.set_visibility("pip", Visibility::Hidden);
    world.set_visibility("goblin", Visibility::Sensed);
    let job = crate::plate_fixtures_tests::job(&world, 1, 0);
    let names: Vec<&str> = job.figures().iter().map(|f| f.id.as_str()).collect();
    assert_eq!(names, vec!["mara", "pip"]);
}

#[test]
fn a_paused_table_makes_no_plates() {
    let mut world = market();
    let mut scheduler = running(&world);
    world.pause();
    let tick = scheduler
        .plate_tick(INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    assert_eq!(skipped_for(&tick), SkipReason::TableNotActive);
}

#[test]
fn a_scene_with_no_viewer_makes_no_plates() {
    let mut world = market();
    world.lose_the_viewer_of_scene_eleven();
    let other = SceneBrief::new(scene(11), display_name("Back room"));
    let mut scheduler = PlateScheduler::new(scene(11), timing());
    let tick = scheduler
        .plate_tick(0, &world.snapshot(), &other)
        .expect("ticks");
    assert_eq!(skipped_for(&tick), SkipReason::NoViewer);
}

#[test]
fn a_stalled_clock_makes_one_tick_and_the_next_is_on_the_following_beat_never_a_burst() {
    let mut world = market();
    let mut scheduler = running(&world);
    world.move_to("pip", 33, 20);
    let tick = scheduler
        .plate_tick(100_000, &world.snapshot(), &brief())
        .expect("ticks");
    queued(&tick);
    assert_eq!(scheduler.stats().missed_beats, 5);
    assert_eq!(scheduler.next_tick_ms(), Some(105_000));
    let again = scheduler
        .plate_tick(100_001, &world.snapshot(), &brief())
        .expect("ticks");
    assert!(matches!(again.outcome, TickOutcome::NotDue { .. }));
}

#[test]
fn a_scene_entered_again_throws_away_the_queued_jobs_of_the_old_generation() {
    let mut world = market();
    let mut scheduler = running(&world);
    world.move_to("pip", 33, 20);
    let tick = scheduler
        .plate_tick(INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    let stale = queued(&tick);
    world.enter_scene(SCENE);
    let after = scheduler
        .plate_tick(2 * INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    assert_eq!(after.discarded, vec![stale]);
    assert_eq!(scheduler.stats().discarded_for_generation, 1);
    let fresh = queued(&after);
    let job = scheduler.take_next().expect("a job");
    assert_eq!(job.id(), fresh);
    assert_eq!(job.stamp().generation, world.generation());
}

#[test]
fn a_closed_scene_skips_and_discards_what_was_waiting() {
    let mut world = market();
    let mut scheduler = running(&world);
    world.move_to("pip", 33, 20);
    let tick = scheduler
        .plate_tick(INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    let waiting = queued(&tick);
    world.enter_scene(11);
    let after = scheduler
        .plate_tick(2 * INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    assert_eq!(skipped_for(&after), SkipReason::SceneClosed);
    assert_eq!(after.discarded, vec![waiting]);
}

#[test]
fn the_queue_holds_four_jobs_and_drops_the_oldest_past_it() {
    let mut world = market();
    let mut scheduler = scheduler();
    scheduler
        .enter_scene(0, &world.snapshot(), &brief())
        .expect("enters");
    let mut now = 0;
    let mut last = None;
    for step in 0..MAXIMUM_QUEUED_JOBS {
        now += INTERVAL_MS;
        world.move_to("pip", 31 + u32::try_from(step).expect("small"), 20);
        let tick = scheduler
            .plate_tick(now, &world.snapshot(), &brief())
            .expect("ticks");
        last = Some(tick);
    }
    let Some(Tick {
        outcome: TickOutcome::Queued { dropped, .. },
        ..
    }) = last
    else {
        panic!("the last tick queued");
    };
    assert_eq!(
        dropped,
        Some(PlateJobId::new(1)),
        "the queue held the first plate's job and three ticks; the fifth job pushed the oldest out"
    );
    assert_eq!(scheduler.queued(), MAXIMUM_QUEUED_JOBS);
    assert_eq!(scheduler.stats().dropped_stale, 1);
}

#[test]
fn a_job_that_failed_is_asked_for_again_even_though_nothing_changed() {
    let mut world = market();
    let mut scheduler = running(&world);
    world.move_to("pip", 33, 20);
    let tick = scheduler
        .plate_tick(INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    let failed = queued(&tick);
    scheduler.take_next().expect("the job runs");
    scheduler.job_ended(failed, JobEnd::Failed);
    let again = scheduler
        .plate_tick(2 * INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    queued(&again);
}

#[test]
fn a_job_that_produced_a_plate_keeps_the_next_quiet_tick_skipped() {
    let mut world = market();
    let mut scheduler = running(&world);
    world.move_to("pip", 33, 20);
    let tick = scheduler
        .plate_tick(INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    let done = queued(&tick);
    scheduler.take_next().expect("the job runs");
    scheduler.job_ended(done, JobEnd::Ready);
    let again = scheduler
        .plate_tick(2 * INTERVAL_MS, &world.snapshot(), &brief())
        .expect("ticks");
    assert_eq!(skipped_for(&again), SkipReason::NothingChanged);
}

#[test]
fn a_resume_asks_for_one_fresh_plate_at_once() {
    let world = market();
    let mut scheduler = running(&world);
    scheduler.resumed();
    let tick = scheduler
        .plate_tick(3_000, &world.snapshot(), &brief())
        .expect("ticks");
    queued(&tick);
}

#[test]
fn a_brief_for_another_scene_is_refused() {
    let world = market();
    let mut scheduler = scheduler();
    let other = SceneBrief::new(scene(99), display_name("Elsewhere"));
    assert_eq!(
        scheduler.plate_tick(0, &world.snapshot(), &other).err(),
        Some(JobError::BriefForOtherScene {
            asked: scene(SCENE),
            brief: scene(99),
        })
    );
}

#[test]
fn the_interval_is_a_setting_and_changes_the_beat() {
    let world = market();
    let mut scheduler = PlateScheduler::new(
        scene(SCENE),
        timing()
            .with(TimingSetting::PlateIntervalS, 5)
            .expect("in range"),
    );
    scheduler
        .enter_scene(0, &world.snapshot(), &brief())
        .expect("enters");
    assert_eq!(scheduler.next_tick_ms(), Some(5_000));
    assert_eq!(scheduler.report().plates_per_hour_at_most, 720);
}

#[test]
fn the_job_s_digest_ignores_overlay_facts_and_follows_the_set() {
    let mut world = market();
    let before = crate::plate_fixtures_tests::job(&world, 1, 0).staging_digest();
    world.hurt("pip", 1);
    let after_overlay = crate::plate_fixtures_tests::job(&world, 2, 0).staging_digest();
    assert_eq!(before, after_overlay);
    world.move_to("mara", 50, 20);
    let after_move = crate::plate_fixtures_tests::job(&world, 3, 0).staging_digest();
    assert_ne!(before, after_move);
}

#[test]
fn every_job_reason_key_is_its_serde_name_and_a_queued_job_reports_it_typed() {
    for reason in JobReason::ALL {
        assert_eq!(
            serde_json::to_value(reason).expect("a reason serializes"),
            serde_json::Value::from(reason.key())
        );
        assert_eq!(JobReason::from_key(reason.key()), Some(*reason));
    }
    assert_eq!(JobReason::from_key("entered"), None);
    let world = market();
    let mut scheduler = scheduler();
    scheduler
        .enter_scene(0, &world.snapshot(), &brief())
        .expect("enters");
    let report = scheduler.report();
    let queued = serde_json::to_value(&report.queued).expect("a report serializes");
    assert_eq!(queued[0]["reason"], "scene_entered", "{queued}");
}
