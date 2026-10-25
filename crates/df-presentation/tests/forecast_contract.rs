mod support;

use df_model::checkpoint::{
    AudienceScope, ContentDigest, ExecutionMode, LogicalTime, PrefetchPolicy,
};
use df_presentation::forecast_contract::{
    Calibration, ForecastCandidate, ForecastError, ForecastLimits, HeuristicRank, qualify_forecast,
};
use df_presentation::moment_selection::{MomentAlternative, MomentDisposition, select_moment_plan};
use support::{Fixture, label, limits, now, record};

fn policy(fixture: &Fixture) -> PrefetchPolicy {
    PrefetchPolicy {
        definition: fixture.demands[0].policy.clone(),
        maximum_candidates: 8,
        maximum_branches: 8,
        maximum_bytes: 1024,
        maximum_duration_ticks: 100,
    }
}

fn forecast_limits() -> ForecastLimits {
    ForecastLimits {
        maximum_candidates: 8,
        maximum_items: 1024,
        maximum_reference_bytes: 1024,
        maximum_demand_bytes: 1024,
        maximum_duration_ticks: 100,
    }
}

fn build_candidate(fixture: &Fixture, heuristic_rank: u32) -> ForecastCandidate<'_> {
    ForecastCandidate {
        demand: &fixture.demands[0],
        branch: 0,
        duration_ticks: 5,
        rank: HeuristicRank(heuristic_rank),
    }
}

#[test]
fn ordinal_extremes_ties_and_unavailable_calibration_preserve_borrowed_order() {
    let fixture = Fixture::new();
    let original = fixture.demands.clone();
    let mut other = fixture.demands[0].clone();
    other.id = record(33);
    for rank in [0, 1, u32::MAX] {
        for calibration in [Calibration::Uncalibrated, Calibration::Unavailable] {
            let candidates = [
                build_candidate(&fixture, rank),
                ForecastCandidate {
                    demand: &other,
                    ..build_candidate(&fixture, rank)
                },
            ];
            let result = qualify_forecast(
                fixture.context(),
                now(),
                Some(fixture.permitted()),
                &policy(&fixture),
                &candidates,
                calibration,
                forecast_limits(),
            )
            .unwrap();
            assert!(std::ptr::eq(result.candidates(), candidates.as_slice()));
            assert!(std::ptr::eq(
                result.candidates()[0].demand,
                &fixture.demands[0]
            ));
            assert_eq!(result.calibration(), calibration);
            assert_eq!(result.candidates()[0].rank, result.candidates()[1].rank);
        }
    }
    assert_eq!(fixture.demands, original);
}

#[test]
fn canonical_policy_and_current_demand_bindings_refuse_stale_inputs() {
    for change in 0..9 {
        let mut fixture = Fixture::new();
        let selected_policy = policy(&fixture);
        match change {
            0 => {
                fixture.demands[0].basis.revision =
                    fixture.demands[0].basis.revision.next_sequence().unwrap()
            }
            1 => fixture.demands[0].key.source = ContentDigest([99; 32]),
            2 => fixture.demands[0].key.moment = record(99),
            3 => fixture.demands[0].key.identity = label("old-identity"),
            4 => fixture.demands[0].key.audience = AudienceScope::Host,
            5 => fixture.demands[0].mode = ExecutionMode::Live,
            6 => fixture.demands[0].policy.entry = label("other-policy"),
            7 => {
                fixture.presentation.source_revision = fixture
                    .presentation
                    .source_revision
                    .next_sequence()
                    .unwrap()
            }
            _ => fixture.presentation.audience = AudienceScope::Host,
        }
        let candidates = [build_candidate(&fixture, u32::MAX)];
        assert!(
            matches!(
                qualify_forecast(
                    fixture.context(),
                    now(),
                    Some(fixture.permitted()),
                    &selected_policy,
                    &candidates,
                    Calibration::Uncalibrated,
                    forecast_limits(),
                ),
                Err(ForecastError::StaleContext)
            ),
            "change {change}"
        );
    }
    let fixture = Fixture::new();
    let mut old = Fixture::new();
    old.pins.content.package_digest = ContentDigest([99; 32]);
    let candidates = [build_candidate(&fixture, 0)];
    assert!(matches!(
        qualify_forecast(
            fixture.context(),
            now(),
            Some(old.permitted()),
            &policy(&fixture),
            &candidates,
            Calibration::Uncalibrated,
            forecast_limits(),
        ),
        Err(ForecastError::StaleContext)
    ));
}

#[test]
fn exact_caps_fit_and_excess_duplicate_and_checked_overflow_refuse_whole_input() {
    let fixture = Fixture::new();
    let mut selected_policy = policy(&fixture);
    selected_policy.maximum_candidates = 1;
    selected_policy.maximum_branches = 1;
    selected_policy.maximum_bytes = 64;
    selected_policy.maximum_duration_ticks = 5;
    let exact = ForecastLimits {
        maximum_candidates: 1,
        maximum_items: 16,
        maximum_reference_bytes: 4,
        maximum_demand_bytes: 64,
        maximum_duration_ticks: 5,
    };
    let candidates = [build_candidate(&fixture, 0)];
    assert!(
        qualify_forecast(
            fixture.context(),
            now(),
            Some(fixture.permitted()),
            &selected_policy,
            &candidates,
            Calibration::Uncalibrated,
            exact,
        )
        .is_ok()
    );
    for change in 0..5 {
        let mut tight = exact;
        match change {
            0 => tight.maximum_items -= 1,
            1 => tight.maximum_reference_bytes -= 1,
            2 => tight.maximum_demand_bytes -= 1,
            3 => tight.maximum_duration_ticks -= 1,
            _ => selected_policy.maximum_bytes -= 1,
        }
        assert!(matches!(
            qualify_forecast(
                fixture.context(),
                now(),
                Some(fixture.permitted()),
                &selected_policy,
                &candidates,
                Calibration::Uncalibrated,
                tight,
            ),
            Err(ForecastError::Capacity)
        ));
    }
    let duplicate = [build_candidate(&fixture, 0), build_candidate(&fixture, 1)];
    assert!(matches!(
        qualify_forecast(
            fixture.context(),
            now(),
            Some(fixture.permitted()),
            &policy(&fixture),
            &duplicate,
            Calibration::Uncalibrated,
            forecast_limits(),
        ),
        Err(ForecastError::DuplicateDemand)
    ));
    for dimension in 0..3 {
        let mut huge = fixture.demands[0].clone();
        huge.id = record(34);
        if dimension == 0 {
            huge.maximum_bytes = u64::MAX;
        }
        if dimension == 1 {
            huge.key.references[0].byte_length = u64::MAX;
        }
        let candidates = [
            ForecastCandidate {
                demand: &huge,
                duration_ticks: if dimension == 2 { u64::MAX } else { 5 },
                ..build_candidate(&fixture, 0)
            },
            build_candidate(&fixture, 0),
        ];
        let mut unlimited = forecast_limits();
        unlimited.maximum_demand_bytes = u64::MAX;
        unlimited.maximum_reference_bytes = u64::MAX;
        unlimited.maximum_duration_ticks = u64::MAX;
        let mut unlimited_policy = policy(&fixture);
        unlimited_policy.maximum_bytes = u64::MAX;
        unlimited_policy.maximum_duration_ticks = u64::MAX;
        assert!(
            matches!(
                qualify_forecast(
                    fixture.context(),
                    now(),
                    Some(fixture.permitted()),
                    &unlimited_policy,
                    &candidates,
                    Calibration::Uncalibrated,
                    unlimited,
                ),
                Err(ForecastError::Capacity)
            ),
            "dimension {dimension}"
        );
    }
    let mut branch = build_candidate(&fixture, 0);
    branch.branch = policy(&fixture).maximum_branches;
    assert!(matches!(
        qualify_forecast(
            fixture.context(),
            now(),
            Some(fixture.permitted()),
            &policy(&fixture),
            &[branch],
            Calibration::Uncalibrated,
            forecast_limits(),
        ),
        Err(ForecastError::InvalidDemand)
    ));
}

#[test]
fn expiration_is_inclusive_and_tick_units_are_explicit() {
    let fixture = Fixture::new();
    let candidates = [build_candidate(&fixture, 0)];
    for ticks in [10, 11, u64::MAX] {
        assert!(matches!(
            qualify_forecast(
                fixture.context(),
                LogicalTime {
                    ticks,
                    ticks_per_second: 1
                },
                Some(fixture.permitted()),
                &policy(&fixture),
                &candidates,
                Calibration::Uncalibrated,
                forecast_limits(),
            ),
            Err(ForecastError::Expired)
        ));
    }
    for ticks_per_second in [0, 1000] {
        assert!(matches!(
            qualify_forecast(
                fixture.context(),
                LogicalTime {
                    ticks: 9,
                    ticks_per_second
                },
                Some(fixture.permitted()),
                &policy(&fixture),
                &candidates,
                Calibration::Uncalibrated,
                forecast_limits(),
            ),
            Err(ForecastError::InvalidTimebase)
        ));
    }
}

#[test]
fn nested_capacity_precedes_missing_inventory_stale_comparison_and_ready_prefix() {
    let fixture = Fixture::new();
    let mut oversized = fixture.demands[0].clone();
    oversized.id = record(35);
    oversized.key.references = vec![fixture.assets[0].clone(); 4097];
    oversized.key.identity = label("stale");
    let candidates = [
        build_candidate(&fixture, 0),
        ForecastCandidate {
            demand: &oversized,
            ..build_candidate(&fixture, 0)
        },
    ];
    for permitted in [None, Some(fixture.permitted())] {
        assert!(matches!(
            qualify_forecast(
                fixture.context(),
                now(),
                permitted,
                &policy(&fixture),
                &candidates,
                Calibration::Unavailable,
                forecast_limits(),
            ),
            Err(ForecastError::Capacity)
        ));
    }
    let mut hidden = Fixture::new();
    hidden.moment.audience = AudienceScope::Members(vec![
        df_types::MemberId::from_bytes(&[77; 16])
            .unwrap();
        4097
    ]);
    let hidden_candidates = [build_candidate(&hidden, u32::MAX)];
    assert!(matches!(
        qualify_forecast(
            hidden.context(),
            now(),
            None,
            &policy(&hidden),
            &hidden_candidates,
            Calibration::Unavailable,
            forecast_limits(),
        ),
        Err(ForecastError::Capacity)
    ));
    let mut invalid = forecast_limits();
    invalid.maximum_items = 0;
    assert!(matches!(
        qualify_forecast(
            fixture.context(),
            now(),
            None,
            &policy(&fixture),
            &[],
            Calibration::Unavailable,
            invalid,
        ),
        Err(ForecastError::InvalidLimits)
    ));
}

#[test]
fn actual_permitted_fallback_is_identical_across_rank_missing_failed_and_expired_forecasts() {
    for mode in [
        ExecutionMode::Live,
        ExecutionMode::PreparedOnly,
        ExecutionMode::Replay,
    ] {
        let mut fixture = Fixture::new();
        fixture.mode = mode;
        fixture.demands[0].mode = mode;
        // The approved fallback has no speculative demands and does not wait for forecasts.
        let alternatives = [MomentAlternative {
            demands: &[],
            ..fixture.alternative()
        }];
        for scenario in 0..6 {
            let mut forecast = fixture.demands[0].clone();
            if scenario == 3 {
                forecast.expires = now();
            }
            if scenario == 4 {
                forecast.key.schema = 99;
            }
            let candidates = [ForecastCandidate {
                demand: &forecast,
                ..build_candidate(&fixture, if scenario == 0 { 0 } else { u32::MAX })
            }];
            let result = qualify_forecast(
                fixture.context(),
                now(),
                if scenario == 2 {
                    None
                } else {
                    Some(fixture.permitted())
                },
                &policy(&fixture),
                if scenario == 5 { &[] } else { &candidates },
                if scenario == 1 {
                    Calibration::Unavailable
                } else {
                    Calibration::Uncalibrated
                },
                forecast_limits(),
            );
            match scenario {
                0 | 1 => assert!(result.is_ok()),
                2 => assert!(matches!(result, Err(ForecastError::InventoryUnavailable))),
                3 => assert!(matches!(result, Err(ForecastError::Expired))),
                4 => assert!(matches!(result, Err(ForecastError::InvalidDemand))),
                _ => assert!(matches!(result, Err(ForecastError::ForecastUnavailable))),
            }
            let selected = select_moment_plan(
                fixture.context(),
                now(),
                Some(fixture.permitted()),
                &alternatives,
                limits(),
            )
            .unwrap();
            assert_eq!(selected.disposition, MomentDisposition::Selected);
            assert!(std::ptr::eq(selected.selected.unwrap(), &alternatives[0]));
        }
        assert_eq!(
            select_moment_plan(fixture.context(), now(), None, &alternatives, limits(),)
                .unwrap()
                .disposition,
            MomentDisposition::Unavailable
        );
    }
}

#[test]
fn high_rank_cannot_admit_hidden_or_substituted_assets_and_never_changes_history() {
    let fixture = Fixture::new();
    let history = fixture.demands.clone();
    for changed in 0..4 {
        let mut demand = fixture.demands[0].clone();
        match changed {
            0 => demand.key.references[0].digest = ContentDigest([99; 32]),
            1 => demand.key.references[0].key = label("private-future"),
            2 => demand.key.references[0].byte_length += 1,
            _ => demand.policy.entry = label("hidden-policy"),
        }
        let candidates = [ForecastCandidate {
            demand: &demand,
            ..build_candidate(&fixture, u32::MAX)
        }];
        assert!(matches!(
            qualify_forecast(
                fixture.context(),
                now(),
                Some(fixture.permitted()),
                &policy(&fixture),
                &candidates,
                Calibration::Unavailable,
                forecast_limits(),
            ),
            Err(ForecastError::Unpermitted | ForecastError::StaleContext)
        ));
    }
    assert_eq!(fixture.demands, history);
    assert_eq!(
        fixture.demands[0].budget_reservation,
        label("no-live-spend")
    );
}
