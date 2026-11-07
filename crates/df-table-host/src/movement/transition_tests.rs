//! Tests of `transition`.

use super::transition::{Deadlines, Transition, TransitionError, format_record, format_trace};
use crate::movement::types::{TransitionFailure, TransitionStage};

fn started_at(now_ms: u64, deadlines: Deadlines) -> Transition {
    Transition::start(7, "we walk to the town square", now_ms, deadlines)
}

#[test]
fn happy_path_visits_every_stage_in_order_and_records_each_step() {
    let mut transition = started_at(100, Deadlines::default());
    assert_eq!(transition.stage(), TransitionStage::Interpreting);

    let times = [110, 120, 130, 140, 150, 160];
    for (at_ms, expected) in times.iter().zip(TransitionStage::ALL.iter().skip(1)) {
        assert_eq!(transition.advance(*at_ms, "step"), Ok(*expected));
    }

    assert_eq!(transition.stage(), TransitionStage::Live);
    assert!(transition.is_finished());
    assert_eq!(transition.failure(), None);
    let stages: Vec<TransitionStage> = transition.records().iter().map(|r| r.stage).collect();
    assert_eq!(stages, TransitionStage::ALL.to_vec());
    let at_ms: Vec<u64> = transition.records().iter().map(|r| r.at_ms).collect();
    assert_eq!(at_ms, vec![100, 110, 120, 130, 140, 150, 160]);
    assert_eq!(
        transition.advance(170, "after live"),
        Err(TransitionError::AlreadyFinished {
            stage: TransitionStage::Live
        })
    );
    assert_eq!(transition.records().len(), 7);
}

#[test]
fn failure_at_generating_keeps_the_stage_and_stores_the_typed_failure() {
    let mut transition = started_at(0, Deadlines::default());
    for at_ms in [10, 20, 30] {
        transition
            .advance(at_ms, "step")
            .expect("the stages before generating advance");
    }
    assert_eq!(transition.stage(), TransitionStage::Generating);

    transition.fail(
        500,
        "provider_refused",
        "the image backend refused the prompt",
    );

    assert_eq!(transition.stage(), TransitionStage::Generating);
    assert!(transition.is_finished());
    assert_eq!(
        transition.failure(),
        Some(&TransitionFailure {
            stage: TransitionStage::Generating,
            key: "provider_refused".to_owned(),
            message: "the image backend refused the prompt".to_owned(),
        })
    );
    assert_eq!(
        transition.advance(600, "after failure"),
        Err(TransitionError::AlreadyFinished {
            stage: TransitionStage::Generating
        })
    );
    transition.fail(700, "other", "a late failure");
    assert_eq!(
        transition.failure().map(|f| f.key.as_str()),
        Some("provider_refused")
    );
    assert_eq!(transition.records().len(), 5);
}

#[test]
fn deadline_expires_one_ms_over_the_limit_but_not_at_it() {
    let deadlines = Deadlines {
        interpreting_ms: 50,
        ..Deadlines::default()
    };
    let mut transition = started_at(1000, deadlines);

    assert_eq!(transition.check_deadline(1050), None);
    assert_eq!(
        transition
            .check_deadline(1051)
            .map(|f| (f.stage, f.key.as_str())),
        Some((TransitionStage::Interpreting, "deadline"))
    );
}

#[test]
fn a_deadline_counts_from_the_start_of_the_current_stage() {
    let deadlines = Deadlines {
        interpreting_ms: 50,
        ..Deadlines::default()
    };
    let mut transition = started_at(1000, deadlines);
    transition
        .advance(1040, "interpreted")
        .expect("interpreting advances");

    assert_eq!(transition.check_deadline(1051), None);
    assert_eq!(transition.stage(), TransitionStage::Designing);
}

#[test]
fn deadline_check_is_idempotent_and_records_the_failure_once() {
    let deadlines = Deadlines {
        interpreting_ms: 50,
        ..Deadlines::default()
    };
    let mut transition = started_at(1000, deadlines);
    let first = transition.check_deadline(1051).cloned();

    assert!(first.is_some());
    assert_eq!(transition.check_deadline(5000).cloned(), first);
    assert_eq!(transition.records().len(), 2);
}

#[test]
fn clock_going_backwards_is_refused_without_panicking() {
    let mut transition = started_at(1000, Deadlines::default());

    assert_eq!(
        transition.advance(999, "backwards"),
        Err(TransitionError::ClockWentBack {
            now_ms: 999,
            last_ms: 1000
        })
    );
    assert_eq!(transition.stage(), TransitionStage::Interpreting);
    assert_eq!(transition.records().len(), 1);
    assert_eq!(transition.check_deadline(0), None);
    assert_eq!(transition.elapsed_ms(0), 0);
    assert_eq!(
        TransitionError::ClockWentBack {
            now_ms: 999,
            last_ms: 1000
        }
        .key(),
        "clock_went_back"
    );
}

#[test]
fn custom_deadlines_replace_the_defaults_and_live_has_none() {
    let defaults = Deadlines::default();
    assert_eq!(defaults.of(TransitionStage::Generating), Some(900_000));
    assert_eq!(defaults.of(TransitionStage::Live), None);

    let deadlines = Deadlines::default().with(TransitionStage::Placing, 3);
    assert_eq!(deadlines.of(TransitionStage::Placing), Some(3));
    let mut transition = started_at(0, deadlines);
    for at_ms in [1, 2, 3, 4, 5] {
        transition
            .advance(at_ms, "step")
            .expect("the stages before placing advance");
    }
    assert_eq!(transition.stage(), TransitionStage::Placing);
    assert_eq!(transition.check_deadline(8), None);
    assert_eq!(
        transition.check_deadline(9).map(|f| f.key.as_str()),
        Some("deadline")
    );
}

#[test]
fn replay_rebuilds_the_stage_and_the_trace_of_a_transition() {
    let mut transition = started_at(100, Deadlines::default());
    transition
        .advance(110, "designed")
        .expect("interpreting advances");
    transition
        .advance(120, "briefed")
        .expect("designing advances");

    let replayed = Transition::replay(transition.records()).expect("an in-order trace replays");

    assert_eq!(replayed.stage(), TransitionStage::Briefing);
    assert_eq!(replayed.records(), transition.records());
    assert_eq!(
        format_trace(replayed.records()),
        format_trace(transition.records())
    );
}

#[test]
fn replay_refuses_an_empty_or_out_of_order_trace() {
    let mut transition = started_at(100, Deadlines::default());
    transition
        .advance(110, "designed")
        .expect("interpreting advances");
    let mut records = transition.records().to_vec();
    records[1].at_ms = 50;

    assert_eq!(
        Transition::replay(&records),
        Err(TransitionError::TraceOutOfOrder { index: 1 })
    );
    assert_eq!(Transition::replay(&[]), Err(TransitionError::TraceEmpty));
}

#[test]
fn record_lines_put_a_multi_line_detail_on_one_line() {
    let mut transition = started_at(100, Deadlines::default());
    transition
        .advance(120, "first line\nsecond line\r\nthird line")
        .expect("interpreting advances");

    assert_eq!(
        format_record(&transition.records()[0]),
        "TRANSITION id=7 stage=interpreting at_ms=100 detail=line=we walk to the town square"
    );
    assert_eq!(
        format_record(&transition.records()[1]),
        "TRANSITION id=7 stage=designing at_ms=120 detail=first line second line third line"
    );
    assert_eq!(
        format_trace(transition.records()),
        format!(
            "{}\n{}",
            format_record(&transition.records()[0]),
            format_record(&transition.records()[1])
        )
    );
}
