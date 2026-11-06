use crate::limits::LARGE_TIMING_MAXIMUM_S;
use crate::plate::cost::{CostBasis, CostView, StagePrice, StageUse};
use crate::plate::timing::{PlateTiming, SceneBuildKind, TimingError, TimingSetting};
use crate::ports::PurposeName;

fn purpose(name: &str) -> PurposeName {
    PurposeName::new(name).expect("a valid purpose name")
}

fn locate_use(least: u32, most: u32) -> StageUse {
    StageUse {
        purpose: purpose("locate"),
        price: StagePrice::Priced {
            micro_dollars: 10_000,
        },
        calls_least_per_plate: least,
        calls_most_per_plate: most,
    }
}

fn plate_use() -> StageUse {
    StageUse {
        purpose: purpose("plate"),
        price: StagePrice::Unpriced,
        calls_least_per_plate: 1,
        calls_most_per_plate: 1,
    }
}

#[test]
fn the_defaults_are_the_numbers_of_j_4() {
    let timing = PlateTiming::default();
    assert_eq!(timing.plate_interval_s(), 15);
    assert_eq!(timing.build_budget_ms(SceneBuildKind::Standard), 8_000);
    assert_eq!(timing.build_budget_ms(SceneBuildKind::FirstScene), 20_000);
    assert_eq!(timing.swap_wait_ms(), 6_000);
    assert_eq!(timing.image_stale_limit_ms(), 90_000);
    assert_eq!(timing.plate_interval_ms(), 15_000);
}

#[test]
fn plates_per_hour_match_the_plan_s_table() {
    let per_hour = |interval: u64| {
        PlateTiming::default()
            .with(TimingSetting::PlateIntervalS, interval)
            .expect("in range")
            .plates_per_hour_at_most()
    };
    assert_eq!(per_hour(5), 720);
    assert_eq!(per_hour(10), 360);
    assert_eq!(per_hour(15), 240);
    assert_eq!(per_hour(30), 120);
    assert_eq!(per_hour(60), 60);
    assert_eq!(per_hour(7), 515, "rounded up: an upper bound");
}

#[test]
fn a_timing_out_of_range_is_refused_with_the_setting_named() {
    let timing = PlateTiming::default();
    for value in [0, u64::from(LARGE_TIMING_MAXIMUM_S) + 1, u64::MAX] {
        assert_eq!(
            timing.with(TimingSetting::PlateIntervalS, value),
            Err(TimingError {
                setting: TimingSetting::PlateIntervalS,
                value
            })
        );
    }
    let error = timing
        .with(TimingSetting::PlateSwapWaitS, 0)
        .expect_err("zero is refused");
    assert!(error.to_string().contains("plate_swap_wait_s"));
}

#[test]
fn every_setting_has_its_own_name_and_default() {
    let names: Vec<&str> = TimingSetting::ALL
        .iter()
        .map(|setting| setting.key())
        .collect();
    assert_eq!(
        names,
        vec![
            "plate_interval_s",
            "scene_build_budget_s",
            "first_scene_hold_s",
            "plate_swap_wait_s",
            "image_stale_limit_s"
        ]
    );
    let defaults: Vec<u32> = TimingSetting::ALL
        .iter()
        .map(|setting| setting.default_s())
        .collect();
    assert_eq!(defaults, vec![15, 8, 20, 6, 90]);
}

#[test]
fn locate_alone_at_fifteen_seconds_costs_two_dollars_forty_an_hour() {
    let view = CostView::estimate(PlateTiming::default(), &[locate_use(1, 1)]);
    assert_eq!(view.plates_per_hour_at_most, 240);
    assert_eq!(view.priced_micro_dollars_least_per_hour, 2_400_000);
    assert_eq!(view.basis, CostBasis::AllStagesPriced);
}

#[test]
fn an_unpriced_stage_makes_the_figure_partial_and_shows_its_calls_instead() {
    let view = CostView::estimate(PlateTiming::default(), &[plate_use(), locate_use(3, 7)]);
    assert_eq!(view.basis, CostBasis::SomeStagesUnpriced);
    let plate = &view.stages[0];
    assert_eq!(
        plate.micro_dollars_least_per_hour, None,
        "never a made-up dollar figure"
    );
    assert_eq!(plate.calls_least_per_hour, 240);
    let locate = &view.stages[1];
    assert_eq!(locate.calls_least_per_hour, 720);
    assert_eq!(locate.calls_most_per_hour, 1_680);
    assert_eq!(view.priced_micro_dollars_least_per_hour, 7_200_000);
    assert_eq!(view.priced_micro_dollars_most_per_hour, 16_800_000);
}

#[test]
fn a_shorter_interval_costs_more_and_a_longer_one_less() {
    let at = |interval: u64| {
        CostView::estimate(
            PlateTiming::default()
                .with(TimingSetting::PlateIntervalS, interval)
                .expect("in range"),
            &[locate_use(1, 1)],
        )
        .priced_micro_dollars_least_per_hour
    };
    assert!(at(5) > at(15));
    assert!(at(15) > at(60));
    assert_eq!(at(60), 600_000);
}
