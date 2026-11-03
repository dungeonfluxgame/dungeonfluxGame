use df_table::plate_cost::{CostBasis, StagePrice};
use df_table::plate_timing::PlateTiming;

use super::request::{
    BuildOutcome, BuildRequest, DevMoment, DevStage, RunRequest, SceneError, SlowStage,
};
use super::workbench::Workbench;

fn bench() -> Workbench {
    Workbench::new(PlateTiming::defaults(), None).expect("a workbench")
}

fn request(place: &str) -> BuildRequest {
    BuildRequest {
        place: place.to_owned(),
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
        place: "Quiet quay".to_owned(),
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
                place: "Harbor market".to_owned(),
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
                assert_eq!(failed, stage.key());
                assert_eq!(cause, "refused");
            }
            other => panic!("{stage:?} gave {other:?}"),
        }
    }
}

#[test]
fn a_place_that_is_not_a_name_is_refused_and_a_run_before_any_build_says_so() {
    let mut bench = bench();
    assert_eq!(
        bench.build(&request("  ")),
        Err(SceneError::InvalidName {
            what: "the place".to_owned()
        })
    );
    assert_eq!(
        bench.run(&run(10, false, DevMoment::LineBoundary)).err(),
        Some(SceneError::NothingBuilt)
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
    assert_eq!(run.moment, "mid_line");
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
    let mut bench =
        Workbench::new(PlateTiming::defaults(), Some(log.clone())).expect("a workbench");
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
