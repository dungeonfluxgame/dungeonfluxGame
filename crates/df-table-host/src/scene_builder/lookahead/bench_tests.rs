//! The look-ahead through the workbench: the same pipeline the dev verbs drive, over scripted
//! providers on the workbench's own clock.

use df_table::plate_timing::PlateTiming;

use super::request::{LookaheadReply, LookaheadRequest};
use crate::scene_builder::request::{
    BuildReply, BuildRequest, DevMoment, DevStage, RunRequest, SceneError, SlowStage,
};
use crate::scene_builder::workbench::Workbench;

fn bench() -> Workbench {
    Workbench::new(PlateTiming::defaults(), None).expect("a workbench")
}

fn build(bench: &mut Workbench, place: &str) -> BuildReply {
    bench
        .build(&BuildRequest {
            place: place.to_owned(),
            fail: None,
            slow: None,
        })
        .expect("builds")
}

fn run(bench: &mut Workbench, seconds: u64) {
    bench
        .run(&RunRequest {
            seconds,
            quiet: true,
            moment: DevMoment::LineBoundary,
        })
        .expect("runs");
}

fn target(place: &str) -> LookaheadRequest {
    LookaheadRequest::Target {
        place: place.to_owned(),
        kind: df_table::lookahead::Expectation::AuthoredBeat,
        likelihood_permille: None,
        fail: None,
        slow: None,
    }
}

fn say(bench: &mut Workbench, request: &LookaheadRequest) -> LookaheadReply {
    bench.lookahead(Some(request)).expect("answers")
}

fn plate_ms(reply: &BuildReply) -> u64 {
    match reply.outcome {
        crate::scene_builder::request::BuildOutcome::PlateOnScreen { at_ms } => at_ms,
        ref other => panic!("no plate: {other:?}"),
    }
}

#[test]
fn a_target_is_built_while_the_clock_runs_and_the_next_scene_build_finds_it_ready() {
    let mut bench = bench();
    build(&mut bench, "Harbor market");
    let started = say(&mut bench, &target("Quay"));
    assert!(
        started
            .lines
            .iter()
            .any(|line| line.line.starts_with("LOOKAHEAD started scene=2")),
        "a certain scene starts at once: {:?}",
        started.lines
    );
    run(&mut bench, 5);
    let shown = bench.lookahead(None).expect("shows");
    assert_eq!(shown.report.book[0].state, "ready");
    assert_eq!(shown.report.counts.built, 1);

    let ahead = build(&mut bench, "Quay");
    assert_eq!(ahead.lookahead.as_deref(), Some("hit"));
    assert!(
        ahead
            .lines
            .iter()
            .any(|line| line.line.contains("source=lookahead")),
        "the scene adopted what was built"
    );

    let mut plain = self::bench();
    build(&mut plain, "Harbor market");
    let normal = build(&mut plain, "Quay");
    assert_eq!(normal.lookahead, None);
    assert!(
        plate_ms(&ahead) + 2_000 <= plate_ms(&normal),
        "{} ms with the look-ahead, {} ms without",
        plate_ms(&ahead),
        plate_ms(&normal)
    );
}

#[test]
fn a_slow_look_ahead_provider_leaves_the_build_of_the_scene_as_it_was() {
    let mut bench = bench();
    build(&mut bench, "Harbor market");
    say(
        &mut bench,
        &LookaheadRequest::Target {
            place: "Quay".to_owned(),
            kind: df_table::lookahead::Expectation::TravelLeg,
            likelihood_permille: None,
            fail: None,
            slow: Some(SlowStage {
                stage: DevStage::Background,
                seconds: 90,
            }),
        },
    );
    run(&mut bench, 5);
    let late = build(&mut bench, "Quay");
    assert_eq!(late.lookahead.as_deref(), Some("late"));
    let mut plain = self::bench();
    build(&mut plain, "Harbor market");
    assert_eq!(plate_ms(&late), plate_ms(&build(&mut plain, "Quay")));
}

#[test]
fn a_scene_built_for_another_place_is_stale_and_a_branch_to_a_new_target_cancels_the_first() {
    let mut bench = bench();
    build(&mut bench, "Harbor market");
    say(&mut bench, &target("Quay"));
    run(&mut bench, 5);
    assert_eq!(
        build(&mut bench, "Elsewhere").lookahead.as_deref(),
        Some("stale")
    );

    say(&mut bench, &target("Keep"));
    run(&mut bench, 1);
    let branched = say(&mut bench, &target("Tower"));
    assert!(
        branched
            .lines
            .iter()
            .any(|line| line.line.starts_with("LOOKAHEAD cancelled scene=3")
                && line.line.ends_with("reason=stamp_changed")),
        "{:?}",
        branched.lines
    );
    run(&mut bench, 5);
    assert_eq!(build(&mut bench, "Tower").lookahead.as_deref(), Some("hit"));
}

#[test]
fn pause_resume_and_clear_change_what_the_worker_does() {
    let mut bench = bench();
    build(&mut bench, "Harbor market");
    say(&mut bench, &LookaheadRequest::Pause);
    let paused = say(&mut bench, &target("Quay"));
    assert!(paused.report.paused);
    assert!(
        !paused
            .lines
            .iter()
            .any(|line| line.line.contains("started")),
        "a paused worker starts nothing"
    );
    let resumed = say(&mut bench, &LookaheadRequest::Resume);
    assert!(!resumed.report.paused);
    assert!(
        resumed
            .lines
            .iter()
            .any(|line| line.line.contains("LOOKAHEAD started"))
    );
    let cleared = say(&mut bench, &LookaheadRequest::Clear);
    assert!(cleared.report.wanted.is_empty());
    assert!(
        cleared
            .lines
            .iter()
            .any(|line| line.line.contains("reason=retargeted"))
    );
}

#[test]
fn only_describe_and_background_are_built_ahead() {
    let mut bench = bench();
    let refused = bench.lookahead(Some(&LookaheadRequest::Target {
        place: "Quay".to_owned(),
        kind: df_table::lookahead::Expectation::AuthoredBeat,
        likelihood_permille: None,
        fail: Some(DevStage::Plate),
        slow: None,
    }));
    assert!(
        matches!(refused, Err(SceneError::BadRequest { .. })),
        "{refused:?}"
    );
    let wild = bench.lookahead(Some(&LookaheadRequest::Target {
        place: "Quay".to_owned(),
        kind: df_table::lookahead::Expectation::ExitHint,
        likelihood_permille: Some(1_001),
        fail: None,
        slow: None,
    }));
    assert!(
        matches!(wild, Err(SceneError::Workbench { .. })),
        "{wild:?}"
    );
}

#[test]
fn a_likely_scene_waits_for_the_start_delay_after_the_current_scene_is_ready() {
    let mut bench = bench();
    build(&mut bench, "Harbor market");
    let waiting = say(
        &mut bench,
        &LookaheadRequest::Target {
            place: "Quay".to_owned(),
            kind: df_table::lookahead::Expectation::ExitHint,
            likelihood_permille: Some(700),
            fail: None,
            slow: None,
        },
    );
    assert!(
        waiting.report.book.is_empty(),
        "a likely scene does not start at once"
    );
    run(&mut bench, 5);
    assert_eq!(
        bench.lookahead(None).expect("shows").report.counts.started,
        0
    );
    run(&mut bench, 8);
    let shown = bench.lookahead(None).expect("shows");
    assert_eq!(
        shown.report.counts.started, 1,
        "ten seconds after the scene was ready"
    );
    assert_eq!(shown.report.wanted[0].likelihood_permille, 700);
}
