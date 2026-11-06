use df_table::event::FailureCause;
use df_table::plate::cost::{CostBasis, StagePrice};
use df_table::plate::timing::PlateTiming;

use super::fixtures::name;
use super::limits::MAXIMUM_MINIMUM_ON_SCREEN_MS;
use super::request::{
    BuildOutcome, BuildRequest, DevMoment, DevStage, PlateTimingSetting, RunRequest,
    SceneBuilderError, SlowStage, TimingRequest,
};
use super::workbench::{DEV_READING_MS, Workbench};

fn bench() -> Workbench {
    Workbench::new(PlateTiming::default(), None).expect("a workbench")
}

fn request(place: &str) -> BuildRequest {
    BuildRequest {
        place: name(place),
        fail: None,
        slow: None,
    }
}

fn run(seconds: u64, quiet: bool, moment: DevMoment) -> RunRequest {
    RunRequest {
        seconds,
        quiet,
        moment,
    }
}

fn lines(reply: &super::request::BuildReply) -> Vec<&str> {
    reply
        .lines
        .iter()
        .map(|timed| timed.line.as_str())
        .collect()
}

#[test]
fn a_build_goes_through_every_stage_in_order_and_puts_the_first_plate_on_screen() {
    let mut bench = bench();
    let reply = bench.build(&request("Harbor market")).expect("builds");
    assert!(
        matches!(reply.outcome, BuildOutcome::PlateOnScreen { .. }),
        "{:?}",
        reply.outcome
    );
    let stages: Vec<&str> = lines(&reply)
        .into_iter()
        .filter(|line| line.starts_with("SCENE stage_requested"))
        .filter_map(|line| line.split("stage=").nth(1)?.split(' ').next())
        .collect();
    assert_eq!(stages, vec!["describe", "background", "plate", "locate"]);
    let stored: Vec<&str> = lines(&reply)
        .into_iter()
        .filter(|line| line.starts_with("SCENE stage_stored"))
        .filter_map(|line| line.split("stage=").nth(1)?.split(' ').next())
        .collect();
    assert_eq!(stored, vec!["describe", "background", "plate", "locate"]);
    assert!(
        lines(&reply)
            .iter()
            .any(|line| line.starts_with("RESOURCE request "))
    );
    assert!(
        lines(&reply)
            .iter()
            .any(|line| line.starts_with("PLATE swapped"))
    );
    assert_eq!(
        reply.card_due_after_ms, None,
        "the first scene is held for 20 s, so no card"
    );
}

#[test]
fn the_times_are_the_workbench_s_own_and_the_run_is_the_same_every_time() {
    let first = bench().build(&request("Harbor market")).expect("builds");
    let second = bench().build(&request("Harbor market")).expect("builds");
    assert_eq!(first.lines, second.lines);
    assert_eq!(first.elapsed_ms, second.elapsed_ms);
    assert!(
        first.elapsed_ms > 8_000,
        "providers took seconds that cost no time here"
    );
}

#[test]
fn the_scene_report_names_what_is_stored_and_where_locate_put_each_target() {
    let mut bench = bench();
    bench.build(&request("Harbor market")).expect("builds");
    let show = bench.show();
    assert_eq!(show.scenes.len(), 1);
    let scene = &show.scenes[0];
    assert!(scene.description_stored && scene.background_stored && scene.first_plate_ready);
    assert_eq!(scene.card.state, "retired");
    assert_eq!(scene.card.party, vec!["Mara", "Pip"]);
    let located = show.on_screen.expect("the plate on screen was located");
    assert_eq!(located.figures.len(), 3);
    let unlocated: Vec<&str> = located
        .figures
        .iter()
        .filter(|figure| figure.anchor == "unlocated")
        .map(|figure| figure.id.as_str())
        .collect();
    assert_eq!(
        unlocated,
        vec!["local"],
        "never silently dropped: it has a fallback point"
    );
    let urn = located
        .items
        .iter()
        .find(|item| item.id == "urn")
        .expect("the urn");
    assert_eq!(
        (urn.anchor.as_str(), urn.phrases_tried),
        ("own_picture", Some(3))
    );
}

#[test]
fn a_second_build_is_a_new_generation_built_inside_the_standard_budget_with_a_slow_stage_showing_the_card()
 {
    let mut bench = bench();
    let first = bench.build(&request("Harbor market")).expect("builds");
    let slow = BuildRequest {
        place: name("Quiet quay"),
        fail: None,
        slow: Some(SlowStage {
            stage: DevStage::Background,
            seconds: 12,
        }),
    };
    let second = bench.build(&slow).expect("builds");
    assert!(second.generation > first.generation);
    assert!(matches!(second.outcome, BuildOutcome::PlateOnScreen { .. }));
    assert_eq!(
        second.card_due_after_ms,
        Some(8_000),
        "play went on to the card at the 8 s budget"
    );
    assert!(second.elapsed_ms > 12_000, "and the build went on past it");
}

#[test]
fn a_stage_made_to_fail_ends_the_build_with_the_stage_and_the_cause() {
    let mut bench = bench();
    for stage in DevStage::ALL {
        let reply = bench
            .build(&BuildRequest {
                place: name("Harbor market"),
                fail: Some(stage),
                slow: None,
            })
            .expect("builds");
        match (stage, &reply.outcome) {
            (DevStage::Locate, BuildOutcome::PlateOnScreen { .. }) => {
                let located = bench.show().on_screen.expect("a plate");
                assert!(
                    located
                        .figures
                        .iter()
                        .all(|figure| figure.anchor == "unlocated"),
                    "a failed Locate leaves every person unlocated"
                );
            }
            (
                _,
                BuildOutcome::Ended {
                    stage: Some(failed),
                    cause: Some(cause),
                    ..
                },
            ) => {
                assert_eq!(*failed, stage.scene_stage());
                assert_eq!(*cause, FailureCause::Refused);
            }
            other => panic!("{stage:?} gave {other:?}"),
        }
    }
}

#[test]
fn a_place_that_is_not_a_name_is_refused_and_a_run_before_any_build_says_so() {
    let body = serde_json::json!({"place": "  "});
    assert!(
        serde_json::from_value::<BuildRequest>(body).is_err(),
        "a blank place is not a body of scene build"
    );
    let mut bench = bench();
    assert_eq!(
        bench.run(&run(10, false, DevMoment::LineBoundary)).err(),
        Some(SceneBuilderError::NothingBuilt)
    );
    assert!(bench.buffer_show().buffer.is_none());
    assert!(bench.show().scenes.is_empty());
}

#[test]
fn running_the_clock_makes_a_plate_each_beat_and_a_quiet_table_none() {
    let mut bench = bench();
    bench.build(&request("Harbor market")).expect("builds");
    let busy = bench
        .run(&run(80, false, DevMoment::LineBoundary))
        .expect("runs");
    assert!(busy.buffer.stats.swaps >= 4, "{:?}", busy.buffer.stats);
    let before = bench.plates();
    let quiet = bench
        .run(&run(60, true, DevMoment::LineBoundary))
        .expect("runs");
    let after = bench.plates();
    assert_eq!(
        after.plates_finished, before.plates_finished,
        "no plate was paid for while nothing changed"
    );
    assert!(
        quiet
            .lines
            .iter()
            .any(|timed| timed.line.contains("reason=nothing_changed"))
    );
}

#[test]
fn a_screen_that_is_never_between_lines_holds_every_plate_and_reports_the_wait() {
    let mut bench = bench();
    let reply = bench.build(&request("Harbor market")).expect("builds");
    assert!(matches!(reply.outcome, BuildOutcome::PlateOnScreen { .. }));
    let run = bench
        .run(&run(60, false, DevMoment::MidLine))
        .expect("runs");
    assert_eq!(run.moment, DevMoment::MidLine);
    assert_eq!(
        run.buffer.stats.swaps, 1,
        "nothing swapped in after the first plate"
    );
    assert!(run.buffer.stats.swaps_overdue >= 1);
    assert!(
        !run.buffer.ready.is_empty(),
        "plates are ready and waiting for a boundary"
    );
    assert!(
        run.lines
            .iter()
            .any(|timed| timed.line.starts_with("PLATE swap_overdue"))
    );
}

#[test]
fn the_plates_report_prices_the_interval_from_the_stages_purposes() {
    let mut bench = bench();
    assert!(
        bench.plates().cost.is_none(),
        "nothing is priced before a scene is open"
    );
    bench.build(&request("Harbor market")).expect("builds");
    let report = bench.plates();
    let cost = report.cost.expect("a cost view");
    assert_eq!(cost.interval_s, 15);
    assert_eq!(cost.plates_per_hour_at_most, 240);
    assert_eq!(
        cost.basis,
        CostBasis::SomeStagesUnpriced,
        "the plate lane records no price"
    );
    let plate = &cost.stages[0];
    assert_eq!(plate.price, StagePrice::Unpriced);
    assert_eq!(plate.micro_dollars_least_per_hour, None);
    assert_eq!(plate.calls_least_per_hour, 240);
    let locate = &cost.stages[1];
    assert_eq!(
        locate.price,
        StagePrice::Priced {
            micro_dollars: 10_000
        }
    );
    assert_eq!(
        locate.calls_least_per_hour,
        240 * 4,
        "the people and one phrase for each of three items"
    );
    assert_eq!(
        locate.calls_most_per_hour,
        240 * 9,
        "the people and every phrase of every item"
    );
    assert_eq!(locate.micro_dollars_least_per_hour, Some(9_600_000));
    assert_eq!(report.plates_finished, 1);
    assert!(report.mean_build_ms.is_some());
}

#[test]
fn the_events_of_a_run_also_reach_the_hosts_log_without_any_text_of_the_scene() {
    let log = std::sync::Arc::new(crate::event_log::EventLog::default());
    let mut bench = Workbench::new(PlateTiming::default(), Some(log.clone())).expect("a workbench");
    bench.build(&request("Harbor market")).expect("builds");
    let logged = log.after_sequence(0, usize::MAX);
    assert!(
        logged
            .iter()
            .any(|line| line.text.starts_with("SCENE stage_stored"))
    );
    assert!(
        logged
            .iter()
            .any(|line| line.text.starts_with("RESOURCE ready"))
    );
    assert!(logged.iter().all(|line| !line.text.contains("Harbor")));
}

#[test]
fn every_line_the_workbench_logs_carries_its_own_time() {
    let log = std::sync::Arc::new(crate::event_log::EventLog::default());
    let mut bench = Workbench::new(PlateTiming::default(), Some(log.clone())).expect("a workbench");
    let reply = bench.build(&request("Harbor market")).expect("builds");
    let logged = log.after_sequence(0, usize::MAX);
    assert_eq!(logged.len(), reply.lines.len(), "one log line per event");
    for (line, timed) in logged.iter().zip(&reply.lines) {
        assert_eq!(
            line.text,
            format!("{} at_ms={}", timed.line, timed.at_ms),
            "the log line is the event's line with the workbench's time"
        );
    }
    let resource: Vec<&str> = logged
        .iter()
        .map(|line| line.text.as_str())
        .filter(|text| text.starts_with("RESOURCE "))
        .collect();
    assert!(!resource.is_empty());
    for text in resource {
        assert!(
            text.contains(" hub=workbench"),
            "the workbench's broker names itself: {text}"
        );
    }
}

#[test]
fn a_failed_stage_and_its_job_say_how_long_they_took() {
    let mut bench = bench();
    let reply = bench
        .build(&BuildRequest {
            place: name("Harbor market"),
            fail: None,
            slow: None,
        })
        .expect("builds");
    assert!(matches!(reply.outcome, BuildOutcome::PlateOnScreen { .. }));
    let failed = bench
        .build(&BuildRequest {
            place: name("Quiet quay"),
            fail: Some(DevStage::Plate),
            slow: None,
        })
        .expect("builds");
    let line_of = |prefix: &str| {
        failed
            .lines
            .iter()
            .find(|timed| timed.line.starts_with(prefix))
            .map(|timed| timed.line.clone())
            .unwrap_or_else(|| panic!("no {prefix} line in {:?}", failed.lines))
    };
    let stage_failed = line_of("SCENE stage_failed");
    assert!(
        stage_failed.contains("stage=plate cause=refused fallback=hold_last_plate duration_ms="),
        "{stage_failed}"
    );
    let job_ended = line_of("SCENE job_ended");
    let duration: u64 = job_ended
        .rsplit_once("duration_ms=")
        .and_then(|(_, value)| value.parse().ok())
        .expect("a duration");
    assert!(job_ended.contains("end=failed"), "{job_ended}");
    assert!(
        duration > 0,
        "the job ran through its stages first: {job_ended}"
    );
}

#[test]
fn the_plates_report_shows_the_timings_and_the_workbench_broker() {
    let mut bench = bench();
    let before = bench.plates();
    assert_eq!(before.timing.interval_s, 15);
    assert_eq!(
        before.timing.on_screen_interval_s, None,
        "no scene on screen"
    );
    assert_eq!(before.timing.minimum_on_screen_ms, DEV_READING_MS);
    assert_eq!(before.timing.dissolve_ms, None, "no swap yet");
    assert_eq!(before.timing.catch_up_ms, 90_000);
    bench.build(&request("Harbor market")).expect("builds");
    let after = bench.plates();
    assert_eq!(
        after.timing.dissolve_ms,
        Some(1_500),
        "the first swap dissolved"
    );
    assert_eq!(after.timing.on_screen_interval_s, Some(15));
    assert_eq!(
        after.broker.epoch, 1,
        "the broker is at the scene's generation"
    );
    assert!(!after.broker.classes.is_empty());
}

#[test]
fn a_timing_set_changes_the_next_run_and_writes_one_driven_line() {
    let mut bench = bench();
    bench.build(&request("Harbor market")).expect("builds");
    let reply = bench
        .set_timing(&TimingRequest {
            setting: PlateTimingSetting::IntervalS,
            value: 5,
        })
        .expect("sets");
    assert_eq!(reply.previous, 15);
    assert_eq!(reply.timing.interval_s, 5);
    assert_eq!(
        reply.timing.on_screen_interval_s,
        Some(15),
        "the screen of the scene on screen swaps at its entry interval until the next scene"
    );
    let lines: Vec<&str> = reply
        .lines
        .iter()
        .map(|timed| timed.line.as_str())
        .collect();
    assert_eq!(
        lines,
        ["PLATE timing name=interval_s value=5 previous=15 driven=true"]
    );
    let run = bench
        .run(&run(30, true, DevMoment::LineBoundary))
        .expect("runs");
    let ticks = run
        .lines
        .iter()
        .filter(|timed| timed.line.starts_with("PLATE tick_skipped"))
        .count();
    assert!(
        ticks >= 5,
        "a tick every 5 s after the one already due: {:?}",
        run.lines
    );
    assert_eq!(
        bench.plates().scheduler.expect("a scene").interval_s,
        5,
        "the clock of the scene on screen runs at the new interval"
    );
    let reading = bench
        .set_timing(&TimingRequest {
            setting: PlateTimingSetting::MinimumOnScreenMs,
            value: 4_500,
        })
        .expect("sets");
    assert_eq!(reading.previous, DEV_READING_MS);
    assert_eq!(bench.plates().timing.minimum_on_screen_ms, 4_500);
}

#[test]
fn a_timing_out_of_range_or_while_plates_are_out_is_refused() {
    let mut bench = bench();
    assert_eq!(
        bench
            .set_timing(&TimingRequest {
                setting: PlateTimingSetting::IntervalS,
                value: 0,
            })
            .err(),
        Some(SceneBuilderError::InvalidTiming {
            setting: PlateTimingSetting::IntervalS,
            value: 0,
        })
    );
    assert_eq!(
        bench
            .set_timing(&TimingRequest {
                setting: PlateTimingSetting::MinimumOnScreenMs,
                value: MAXIMUM_MINIMUM_ON_SCREEN_MS + 1,
            })
            .err()
            .map(|error| error.key()),
        Some("invalid_timing")
    );
    bench.build(&request("Harbor market")).expect("builds");
    let plates = bench.plates();
    let next_tick_ms = plates
        .scheduler
        .and_then(|scheduler| scheduler.next_tick_ms)
        .expect("a tick is due");
    // One second past the tick: its plate takes provider seconds, so it is still being built.
    let seconds = next_tick_ms.saturating_sub(plates.now_ms) / 1_000 + 1;
    bench
        .run(&run(seconds, false, DevMoment::LineBoundary))
        .expect("runs");
    assert_eq!(
        bench
            .set_timing(&TimingRequest {
                setting: PlateTimingSetting::IntervalS,
                value: 5,
            })
            .err(),
        Some(SceneBuilderError::Building),
        "the plate of the tick is still being built"
    );
    assert_eq!(bench.plates().timing.interval_s, 15, "nothing changed");
}
