use df_table::event::FailureCause;
use df_table::ports::{Readiness, ResourceHandle};
use df_table::private_data::Narration;

use super::events::{AbandonReason, BrokerEvent, RefusalKind, StaleReason};
use super::ledger::LedgerEnd;
use super::purpose::{Price, PurposeFacts, PurposeSpec, PurposeTable, QueueClass, ResourceKind};
use super::queues::QueueBounds;
use super::request::{
    AttemptId, Epoch, ProviderFault, RequestError, RequestStatus, ResourceResult, ResultError,
    WorkerId,
};
use super::scheduler::{BrokerLimits, CancelOutcome, CompleteOutcome};
use super::tests_support::{core_over, core_with, facts, name, request_for};

const WORKER: WorkerId = WorkerId::new(1);

fn text_ok() -> Result<ResourceResult, ProviderFault> {
    Ok(ResourceResult::Text(Narration::new("a line")))
}

fn image_ok() -> Result<ResourceResult, ProviderFault> {
    Ok(ResourceResult::Image {
        width: 64,
        height: 64,
        bytes: 4_096,
    })
}

fn wide_images() -> BrokerLimits {
    BrokerLimits {
        image_slots: 10,
        ..BrokerLimits::default()
    }
}

fn one_image_slot() -> BrokerLimits {
    BrokerLimits {
        image_slots: 1,
        ..BrokerLimits::default()
    }
}

fn table_with_deadline(deadline_ms: u64) -> PurposeTable {
    let spec = PurposeSpec::new(
        "text_live",
        facts(ResourceKind::Text, QueueClass::Live, deadline_ms),
    )
    .expect("valid");
    PurposeTable::new(vec![spec]).expect("valid")
}

#[test]
fn a_request_returns_a_handle_and_stays_pending_until_a_worker_answers() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    assert_eq!(core.readiness(handle), Some(Readiness::Pending));
    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Queued {
            class: QueueClass::Live
        })
    );
    assert_eq!(core.take_result(handle), Err(ResultError::NotSettled));
}

#[test]
fn an_unknown_purpose_is_refused_and_recorded() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let error = core.request(request_for("no_such_purpose", 1, 1), 0);
    assert!(matches!(error, Err(RequestError::UnknownPurpose { .. })));
    assert_eq!(
        audit.count(|event| matches!(
            event,
            BrokerEvent::Refused {
                kind: RefusalKind::UnknownPurpose,
                ..
            }
        )),
        1
    );
}

#[test]
fn classes_are_claimed_live_then_plate_then_lookahead_then_background() {
    let (mut core, _) = core_with(wide_images());
    for (key, purpose) in [
        (1, "image_background"),
        (2, "image_lookahead"),
        (3, "image_plate"),
        (4, "image_live"),
    ] {
        core.request(request_for(purpose, key, 1), 0)
            .expect("accepted");
    }
    let claimed: Vec<String> = std::iter::from_fn(|| core.claim(ResourceKind::Image, WORKER, 0))
        .map(|call| call.purpose.to_string())
        .collect();
    assert_eq!(
        claimed,
        [
            "image_live",
            "image_plate",
            "image_lookahead",
            "image_background"
        ]
    );
}

#[test]
fn within_a_class_the_oldest_request_is_claimed_first() {
    let (mut core, _) = core_with(wide_images());
    let first = core
        .request(request_for("image_plate", 1, 1), 0)
        .expect("accepted");
    let second = core
        .request(request_for("image_plate", 2, 1), 5)
        .expect("accepted");
    let order: Vec<ResourceHandle> =
        std::iter::from_fn(|| core.claim(ResourceKind::Image, WORKER, 5))
            .map(|call| call.handle)
            .collect();
    assert_eq!(order, [first, second]);
}

#[test]
fn a_higher_class_preempts_the_lowest_class_running_when_the_slots_are_full() {
    let (mut core, audit) = core_with(one_image_slot());
    let background = core
        .request(request_for("image_background", 1, 1), 0)
        .expect("accepted");
    let running = core.claim(ResourceKind::Image, WORKER, 0).expect("claimed");
    core.mark_sent(running.attempt).expect("known");
    let plate = core
        .request(request_for("image_plate", 2, 1), 10)
        .expect("accepted");

    let urgent = core
        .claim(ResourceKind::Image, WORKER, 10)
        .expect("preempts");

    assert_eq!(urgent.handle, plate);
    assert_eq!(
        core.status(background),
        Some(RequestStatus::Queued {
            class: QueueClass::Background
        })
    );
    assert_eq!(core.take_cancellations(), [running.attempt]);
    assert_eq!(
        audit.count(|event| matches!(event, BrokerEvent::Preempted { handle, by, .. } if *handle == background && *by == plate)),
        1
    );
    assert_eq!(
        core.complete(running.attempt, image_ok(), 20),
        CompleteOutcome::Rejected {
            reason: AbandonReason::Preempted
        }
    );
}

#[test]
fn a_preempted_request_runs_again_when_the_slot_frees_and_keeps_its_deadline() {
    let (mut core, _) = core_with(one_image_slot());
    let background = core
        .request(request_for("image_background", 1, 1), 0)
        .expect("accepted");
    core.claim(ResourceKind::Image, WORKER, 0).expect("claimed");
    core.request(request_for("image_plate", 2, 1), 10)
        .expect("accepted");
    let urgent = core
        .claim(ResourceKind::Image, WORKER, 10)
        .expect("preempts");
    assert_eq!(core.claim(ResourceKind::Image, WORKER, 11), None);

    assert_eq!(
        core.complete(urgent.attempt, image_ok(), 12),
        CompleteOutcome::Accepted
    );
    let again = core
        .claim(ResourceKind::Image, WORKER, 12)
        .expect("runs again");

    assert_eq!(again.handle, background);
    assert_eq!(again.number, 2);
    core.tick(10_000);
    assert_eq!(
        core.status(background),
        Some(RequestStatus::Failed {
            cause: FailureCause::TimedOut
        })
    );
}

#[test]
fn a_request_never_preempts_the_same_class_or_a_higher_one() {
    let (mut core, _) = core_with(one_image_slot());
    core.request(request_for("image_plate", 1, 1), 0)
        .expect("accepted");
    core.claim(ResourceKind::Image, WORKER, 0).expect("claimed");
    core.request(request_for("image_plate", 2, 1), 1)
        .expect("accepted");
    core.request(request_for("image_background", 3, 1), 1)
        .expect("accepted");

    assert_eq!(core.claim(ResourceKind::Image, WORKER, 2), None);
    assert!(core.take_cancellations().is_empty());
}

#[test]
fn work_that_is_not_live_never_takes_the_text_slot_reserved_for_live() {
    let limits = BrokerLimits {
        text_slots: 2,
        reserved_live_text_slots: 1,
        ..BrokerLimits::default()
    };
    let (mut core, _) = core_with(limits);
    core.request(request_for("text_lookahead", 1, 1), 0)
        .expect("accepted");
    core.request(request_for("text_lookahead", 2, 1), 0)
        .expect("accepted");

    assert!(core.claim(ResourceKind::Text, WORKER, 0).is_some());
    assert_eq!(core.claim(ResourceKind::Text, WORKER, 0), None);

    core.request(request_for("text_live", 3, 1), 0)
        .expect("accepted");
    let live = core
        .claim(ResourceKind::Text, WORKER, 0)
        .expect("live may use it");
    assert_eq!(live.purpose, name("text_live"));
}

#[test]
fn a_result_for_a_stale_epoch_is_rejected() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.mark_sent(call.attempt).expect("known");

    assert_eq!(core.advance_epoch(Epoch::new(2)), Ok(1));
    let outcome = core.complete(call.attempt, text_ok(), 5);

    assert_eq!(
        outcome,
        CompleteOutcome::Rejected {
            reason: AbandonReason::EpochAdvanced
        }
    );
    assert_eq!(core.status(handle), Some(RequestStatus::Superseded));
    assert_eq!(
        core.readiness(handle),
        Some(Readiness::Failed(FailureCause::Stale))
    );
    assert!(core.take_result(handle).is_err());
    let reasons: Vec<StaleReason> = audit
        .events()
        .iter()
        .filter_map(|event| match event {
            BrokerEvent::Stale { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        [StaleReason::EpochAdvanced, StaleReason::LateResult]
    );
}

#[test]
fn advancing_the_epoch_supersedes_waiting_requests_and_tells_their_owner() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    core.advance_epoch(Epoch::new(2)).expect("forward");

    assert_eq!(core.status(handle), Some(RequestStatus::Superseded));
    let completions = core.take_completions();
    assert_eq!(completions.len(), 1);
    assert_eq!(
        completions[0].readiness,
        Readiness::Failed(FailureCause::Stale)
    );
    assert!(core.take_completions().is_empty());
}

#[test]
fn a_request_for_an_epoch_that_has_passed_is_refused() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    core.advance_epoch(Epoch::new(3)).expect("forward");

    let refused = core.request(request_for("text_live", 1, 2), 0);

    assert_eq!(
        refused,
        Err(RequestError::StaleEpoch {
            requested: Epoch::new(2),
            current: Epoch::new(3)
        })
    );
    assert_eq!(
        audit.count(|event| matches!(
            event,
            BrokerEvent::Stale {
                reason: StaleReason::RefusedAtRequest,
                ..
            }
        )),
        1
    );
}

#[test]
fn the_epoch_only_moves_forward() {
    let (mut core, _) = core_with(BrokerLimits::default());
    core.advance_epoch(Epoch::new(2)).expect("forward");
    assert!(core.advance_epoch(Epoch::new(2)).is_err());
    assert!(core.advance_epoch(Epoch::new(1)).is_err());
    assert_eq!(core.epoch(), Epoch::new(2));
}

#[test]
fn a_look_ahead_request_for_the_next_epoch_survives_the_advance_to_it() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let next_scene = core
        .request(request_for("text_lookahead", 1, 2), 0)
        .expect("accepted");
    let this_scene = core
        .request(request_for("text_live", 2, 1), 0)
        .expect("accepted");
    let build = core
        .claim(ResourceKind::Text, WORKER, 0)
        .expect("live first");
    assert_eq!(build.handle, this_scene);

    core.advance_epoch(Epoch::new(2)).expect("forward");

    assert_eq!(core.status(this_scene), Some(RequestStatus::Superseded));
    let call = core
        .claim(ResourceKind::Text, WORKER, 1)
        .expect("still waiting");
    assert_eq!(call.handle, next_scene);
    assert_eq!(
        core.complete(call.attempt, text_ok(), 2),
        CompleteOutcome::Accepted
    );
    assert_eq!(core.status(next_scene), Some(RequestStatus::Ready));
    core.advance_epoch(Epoch::new(3)).expect("forward");
    assert_eq!(core.status(next_scene), Some(RequestStatus::Ready));
}

#[test]
fn a_deadline_miss_fails_a_running_request_with_a_typed_timeout_and_cancels_its_call() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.mark_sent(call.attempt).expect("known");

    core.tick(9_999);
    assert!(matches!(
        core.status(handle),
        Some(RequestStatus::InFlight { .. })
    ));
    core.tick(10_000);

    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::TimedOut
        })
    );
    assert_eq!(core.take_cancellations(), [call.attempt]);
    let failure = audit
        .events()
        .into_iter()
        .find_map(|event| match event {
            BrokerEvent::Failed {
                cause,
                ms,
                unknown_dispatch,
                ..
            } => Some((cause, ms, unknown_dispatch)),
            _ => None,
        })
        .expect("a failure was recorded");
    assert_eq!(failure, (FailureCause::TimedOut, 10_000, true));
    assert_eq!(
        core.complete(call.attempt, text_ok(), 10_001),
        CompleteOutcome::Rejected {
            reason: AbandonReason::DeadlineExceeded
        }
    );
    assert_eq!(
        core.readiness(handle),
        Some(Readiness::Failed(FailureCause::TimedOut))
    );
}

#[test]
fn a_deadline_miss_fails_a_waiting_request_without_an_unknown_dispatch() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");

    core.tick(10_000);

    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::TimedOut
        })
    );
    assert_eq!(core.claim(ResourceKind::Text, WORKER, 10_001), None);
    assert_eq!(
        audit.count(|event| matches!(
            event,
            BrokerEvent::Failed {
                unknown_dispatch: false,
                cause: FailureCause::TimedOut,
                ..
            }
        )),
        1
    );
    assert_eq!(core.ledger().entry(handle).expect("recorded").calls_sent, 0);
}

#[test]
fn a_deadline_miss_fails_a_request_that_is_backing_off() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.complete(call.attempt, Err(ProviderFault::Unavailable), 10);
    assert!(matches!(
        core.status(handle),
        Some(RequestStatus::BackingOff { .. })
    ));

    core.tick(10_000);

    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::TimedOut
        })
    );
}

#[test]
fn a_reported_failure_is_retried_with_a_doubling_backoff_until_the_retries_run_out() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let first = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.complete(first.attempt, Err(ProviderFault::Unavailable), 0);
    let RequestStatus::BackingOff { until_ms } = core.status(handle).expect("known") else {
        panic!("the first failure should back off");
    };
    assert!((100..=125).contains(&until_ms), "first backoff {until_ms}");
    assert_eq!(core.claim(ResourceKind::Text, WORKER, until_ms - 1), None);

    let second = core
        .claim(ResourceKind::Text, WORKER, until_ms)
        .expect("claimed again");
    assert_eq!(second.number, 2);
    core.complete(second.attempt, Err(ProviderFault::Unavailable), until_ms);
    let RequestStatus::BackingOff { until_ms: later } = core.status(handle).expect("known") else {
        panic!("the second failure should back off");
    };
    assert!(
        (until_ms + 200..=until_ms + 250).contains(&later),
        "second backoff {later}"
    );

    let third = core
        .claim(ResourceKind::Text, WORKER, later)
        .expect("claimed a third time");
    core.complete(third.attempt, Err(ProviderFault::Unavailable), later);

    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::Unavailable
        })
    );
    assert_eq!(
        audit.count(|event| matches!(event, BrokerEvent::Retrying { .. })),
        2
    );
    assert_eq!(core.ledger().entry(handle).expect("recorded").calls_sent, 0);
}

#[test]
fn a_refusal_is_never_retried() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");

    core.complete(call.attempt, Err(ProviderFault::Refused), 5);

    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::Refused
        })
    );
    assert_eq!(
        audit.count(|event| matches!(event, BrokerEvent::Retrying { .. })),
        0
    );
}

#[test]
fn a_retry_that_would_end_past_the_deadline_is_not_made() {
    let (mut core, _) = core_over(table_with_deadline(150), BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");

    core.complete(call.attempt, Err(ProviderFault::Unavailable), 100);

    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::Unavailable
        })
    );
}

#[test]
fn an_answer_outside_the_size_rule_is_an_invalid_answer_and_is_not_paid_for_again() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.mark_sent(call.attempt).expect("known");
    let too_long = Ok(ResourceResult::Text(Narration::new("a".repeat(101))));

    core.complete(call.attempt, too_long, 5);

    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::InvalidAnswer
        })
    );
    assert_eq!(
        audit.count(|event| matches!(event, BrokerEvent::Retrying { .. })),
        0,
        "another call would only repeat the same oversize answer"
    );
    assert_eq!(core.ledger().entry(handle).expect("recorded").calls_sent, 1);
}

#[test]
fn an_answer_of_the_wrong_kind_or_an_empty_text_is_not_accepted() {
    for answer in [image_ok(), Ok(ResourceResult::Text(Narration::new("")))] {
        let (mut core, _) = core_with(BrokerLimits::default());
        let handle = core
            .request(request_for("text_live", 1, 1), 0)
            .expect("accepted");
        let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");

        core.complete(call.attempt, answer, 1);

        assert_eq!(
            core.status(handle),
            Some(RequestStatus::Failed {
                cause: FailureCause::InvalidAnswer
            })
        );
    }
}

#[test]
fn a_request_past_its_deadline_is_never_claimed_or_charged() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");

    let claimed = core.claim(ResourceKind::Text, WORKER, 10_000);

    assert_eq!(claimed, None, "the deadline passed while it waited");
    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::TimedOut
        })
    );
    assert_eq!(core.ledger().entry(handle).expect("recorded").calls_sent, 0);
    assert_eq!(
        audit.count(|event| matches!(
            event,
            BrokerEvent::Spent { .. } | BrokerEvent::Claimed { .. }
        )),
        0
    );
}

#[test]
fn a_request_that_waited_out_a_retry_backoff_past_its_deadline_is_not_claimed_either() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let first = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.complete(first.attempt, Err(ProviderFault::Unavailable), 100);

    assert_eq!(core.claim(ResourceKind::Text, WORKER, 10_000), None);

    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::TimedOut
        })
    );
}

#[test]
fn an_answer_that_arrives_after_the_deadline_is_rejected_as_timed_out_without_a_tick() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.mark_sent(call.attempt).expect("known");

    let outcome = core.complete(call.attempt, text_ok(), 10_000);

    assert_eq!(
        outcome,
        CompleteOutcome::Rejected {
            reason: AbandonReason::DeadlineExceeded
        }
    );
    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::TimedOut
        })
    );
    assert!(matches!(
        core.take_result(handle),
        Err(ResultError::NoResult { .. })
    ));
    assert_eq!(
        audit.count(|event| matches!(
            event,
            BrokerEvent::Failed {
                cause: FailureCause::TimedOut,
                unknown_dispatch: true,
                ..
            }
        )),
        1
    );
}

#[test]
fn an_answer_one_millisecond_inside_the_deadline_is_accepted() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");

    assert_eq!(
        core.complete(call.attempt, text_ok(), 9_999),
        CompleteOutcome::Accepted
    );
    assert_eq!(core.status(handle), Some(RequestStatus::Ready));
}

#[test]
fn an_expired_lease_puts_an_unsent_call_back_up_to_three_attempts_and_then_fails_it() {
    let (mut core, audit) = core_over(table_with_deadline(1_000_000), BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");

    let first = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.tick(60_000);
    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Queued {
            class: QueueClass::Live
        })
    );
    let second = core
        .claim(ResourceKind::Text, WORKER, 60_000)
        .expect("claimed again");
    core.tick(120_000);
    let third = core
        .claim(ResourceKind::Text, WORKER, 120_000)
        .expect("claimed a third time");
    core.tick(180_000);

    assert_eq!((first.number, second.number, third.number), (1, 2, 3));
    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::Unavailable
        })
    );
    assert_eq!(
        audit.count(|event| matches!(event, BrokerEvent::LeaseExpired { sent: false, .. })),
        3
    );
    assert_eq!(core.ledger().entry(handle).expect("recorded").calls_sent, 0);
}

#[test]
fn an_expired_lease_of_a_call_that_was_sent_fails_as_an_unknown_dispatch_and_is_not_retried() {
    let (mut core, audit) = core_over(table_with_deadline(1_000_000), BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.mark_sent(call.attempt).expect("known");

    core.tick(60_000);

    assert_eq!(
        core.status(handle),
        Some(RequestStatus::Failed {
            cause: FailureCause::TimedOut
        })
    );
    assert_eq!(
        audit.count(|event| matches!(
            event,
            BrokerEvent::Failed {
                unknown_dispatch: true,
                ..
            }
        )),
        1
    );
    assert_eq!(core.ledger().entry(handle).expect("recorded").calls_sent, 1);
}

#[test]
fn a_heartbeat_renews_the_lease() {
    let (mut core, _) = core_over(table_with_deadline(1_000_000), BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");

    assert!(core.heartbeat(call.attempt, 50_000));
    core.tick(100_000);

    assert!(matches!(
        core.status(handle),
        Some(RequestStatus::InFlight { .. })
    ));
    assert!(!core.heartbeat(AttemptId::new(999), 100_000));
}

#[test]
fn a_second_request_for_a_key_joins_the_flight_and_it_ends_only_when_every_holder_cancels() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    let first = core
        .request(request_for("image_plate", 7, 1), 0)
        .expect("accepted");
    let second = core
        .request(request_for("image_plate", 7, 1), 1)
        .expect("joined");

    assert_eq!(first, second);
    assert_eq!(
        audit.count(|event| matches!(event, BrokerEvent::Joined { holders: 2, .. })),
        1
    );
    assert_eq!(core.ledger().entries().count(), 1);
    assert_eq!(core.cancel(first), CancelOutcome::StillHeld { holders: 1 });
    assert!(matches!(
        core.status(first),
        Some(RequestStatus::Queued { .. })
    ));
    assert_eq!(core.cancel(first), CancelOutcome::Cancelled);
    assert_eq!(core.status(first), Some(RequestStatus::Cancelled));
}

#[test]
fn a_key_that_has_settled_starts_a_new_flight() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let first = core
        .request(request_for("text_live", 7, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.complete(call.attempt, text_ok(), 1);

    let second = core
        .request(request_for("text_live", 7, 1), 2)
        .expect("accepted");

    assert_ne!(first, second);
}

#[test]
fn cancelling_a_waiting_request_removes_it_and_a_running_one_is_abandoned() {
    let (mut core, _) = core_with(wide_images());
    let waiting = core
        .request(request_for("image_plate", 1, 1), 0)
        .expect("accepted");
    let running = core
        .request(request_for("image_live", 2, 1), 0)
        .expect("accepted");
    let call = core
        .claim(ResourceKind::Image, WORKER, 0)
        .expect("live first");
    assert_eq!(call.handle, running);

    assert_eq!(core.cancel(waiting), CancelOutcome::Cancelled);
    assert_eq!(core.cancel(running), CancelOutcome::Cancelled);

    assert_eq!(core.claim(ResourceKind::Image, WORKER, 1), None);
    assert_eq!(core.take_cancellations(), [call.attempt]);
    assert_eq!(
        core.complete(call.attempt, image_ok(), 2),
        CompleteOutcome::Rejected {
            reason: AbandonReason::Cancelled
        }
    );
    assert_eq!(core.cancel(waiting), CancelOutcome::NotActive);
    assert_eq!(
        core.cancel(ResourceHandle::new(404)),
        CancelOutcome::NotActive
    );
}

#[test]
fn a_full_plate_queue_drops_its_oldest_request_and_says_so() {
    let limits = BrokerLimits {
        queues: QueueBounds {
            plate: 2,
            ..QueueBounds::default()
        },
        ..BrokerLimits::default()
    };
    let (mut core, audit) = core_with(limits);
    let oldest = core
        .request(request_for("image_plate", 1, 1), 0)
        .expect("accepted");
    let middle = core
        .request(request_for("image_plate", 2, 1), 1)
        .expect("accepted");
    let newest = core
        .request(request_for("image_plate", 3, 1), 2)
        .expect("accepted");

    assert_eq!(core.status(oldest), Some(RequestStatus::Dropped));
    assert!(matches!(
        core.status(middle),
        Some(RequestStatus::Queued { .. })
    ));
    assert!(matches!(
        core.status(newest),
        Some(RequestStatus::Queued { .. })
    ));
    assert_eq!(
        audit.count(|event| matches!(event, BrokerEvent::Dropped { .. })),
        1
    );
    assert_eq!(core.queues_report(2).classes[1].dropped, 1);
    let completions = core.take_completions();
    assert_eq!(
        completions[0].readiness,
        Readiness::Failed(FailureCause::Stale)
    );
}

#[test]
fn a_full_live_queue_refuses_the_new_request_instead_of_dropping_one() {
    let limits = BrokerLimits {
        queues: QueueBounds {
            live: 2,
            ..QueueBounds::default()
        },
        ..BrokerLimits::default()
    };
    let (mut core, _) = core_with(limits);
    let first = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    core.request(request_for("text_live", 2, 1), 0)
        .expect("accepted");

    let refused = core.request(request_for("text_live", 3, 1), 0);

    assert_eq!(
        refused,
        Err(RequestError::QueueFull {
            class: QueueClass::Live
        })
    );
    assert!(matches!(
        core.status(first),
        Some(RequestStatus::Queued { .. })
    ));
}

#[test]
fn the_ledger_records_every_request_and_every_call_sent_and_never_refuses() {
    let priced = PurposeFacts {
        price: Price::Priced { micro_dollars: 250 },
        ..facts(ResourceKind::Image, QueueClass::Plate, 10_000)
    };
    let table = PurposeTable::new(vec![
        PurposeSpec::new("priced_plate", priced).expect("valid"),
        PurposeSpec::new(
            "free_text",
            facts(ResourceKind::Text, QueueClass::Live, 10_000),
        )
        .expect("valid"),
    ])
    .expect("valid");
    let limits = BrokerLimits {
        image_slots: 1_000,
        queues: QueueBounds {
            plate: 1_000,
            ..QueueBounds::default()
        },
        ..BrokerLimits::default()
    };
    let (mut core, _) = core_over(table, limits);
    for key in 1..=500 {
        core.request(request_for("priced_plate", key, 1), 0)
            .expect("never refused");
    }
    core.request(request_for("free_text", 9_999, 1), 0)
        .expect("accepted");
    while let Some(call) = core.claim(ResourceKind::Image, WORKER, 0) {
        core.mark_sent(call.attempt).expect("known");
        core.mark_sent(call.attempt).expect("charged once");
        core.complete(call.attempt, image_ok(), 1);
    }
    let text = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.mark_sent(text.attempt).expect("known");

    let priced_totals = core.ledger().totals_for(&name("priced_plate"));
    assert_eq!(priced_totals.requests, 500);
    assert_eq!(priced_totals.calls_sent, 500);
    assert_eq!(priced_totals.spent_micro_dollars, 125_000);
    let text_totals = core.ledger().totals_for(&name("free_text"));
    assert_eq!((text_totals.calls_sent, text_totals.unpriced_calls), (1, 1));
    assert_eq!(text_totals.spent_micro_dollars, 0);
    let entry = core.ledger().entries().next().expect("an entry");
    assert_eq!((entry.calls_sent, entry.end), (1, LedgerEnd::Ready));
}

#[test]
fn a_call_that_was_sent_is_charged_even_if_the_request_is_then_cancelled() {
    let priced = PurposeFacts {
        price: Price::Priced { micro_dollars: 40 },
        ..facts(ResourceKind::Image, QueueClass::Plate, 10_000)
    };
    let table = PurposeTable::new(vec![
        PurposeSpec::new("priced_plate", priced).expect("valid"),
    ])
    .expect("valid");
    let (mut core, _) = core_over(table, BrokerLimits::default());
    let handle = core
        .request(request_for("priced_plate", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Image, WORKER, 0).expect("claimed");
    core.mark_sent(call.attempt).expect("known");

    core.cancel(handle);

    assert_eq!(
        core.ledger()
            .totals_for(&name("priced_plate"))
            .spent_micro_dollars,
        40
    );
    assert_eq!(
        core.ledger().entry(handle).expect("recorded").end,
        LedgerEnd::Cancelled
    );
}

#[test]
fn closing_the_broker_cancels_every_request_whoever_holds_it_and_refuses_new_ones() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let waiting = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    core.request(request_for("text_live", 1, 1), 0)
        .expect("joined");
    let running = core
        .request(request_for("image_plate", 2, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Image, WORKER, 0).expect("claimed");

    assert_eq!(core.close(), 2);

    assert_eq!(core.status(waiting), Some(RequestStatus::Cancelled));
    assert_eq!(core.status(running), Some(RequestStatus::Cancelled));
    assert_eq!(core.take_cancellations(), [call.attempt]);
    assert_eq!(
        core.request(request_for("text_live", 3, 1), 0),
        Err(RequestError::Closed)
    );
}

#[test]
fn a_result_can_be_taken_once() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let handle = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.complete(call.attempt, text_ok(), 1);

    assert!(matches!(
        core.take_result(handle),
        Ok(ResourceResult::Text(_))
    ));
    assert!(matches!(
        core.take_result(handle),
        Err(ResultError::NoResult { .. })
    ));
    assert_eq!(
        core.take_result(ResourceHandle::new(404)),
        Err(ResultError::Unknown)
    );
}

#[test]
fn settled_requests_are_pushed_once_as_completions() {
    let (mut core, _) = core_with(BrokerLimits::default());
    let ready = core
        .request(request_for("text_live", 1, 1), 0)
        .expect("accepted");
    let failed = core
        .request(request_for("text_live", 2, 1), 0)
        .expect("accepted");
    let first = core.claim(ResourceKind::Text, WORKER, 0).expect("claimed");
    core.complete(first.attempt, text_ok(), 1);
    let second = core.claim(ResourceKind::Text, WORKER, 1).expect("claimed");
    core.complete(second.attempt, Err(ProviderFault::Refused), 2);

    let completions = core.take_completions();

    assert_eq!(completions.len(), 2);
    assert_eq!(
        (completions[0].handle, completions[0].readiness),
        (ready, Readiness::Ready)
    );
    assert_eq!(
        (completions[1].handle, completions[1].readiness),
        (failed, Readiness::Failed(FailureCause::Refused))
    );
    assert!(core.take_completions().is_empty());
}

#[test]
fn every_change_is_recorded_as_one_line_in_the_resource_or_spend_area() {
    let (mut core, audit) = core_with(BrokerLimits::default());
    core.request(request_for("image_plate", 1, 1), 0)
        .expect("accepted");
    let call = core.claim(ResourceKind::Image, WORKER, 0).expect("claimed");
    core.mark_sent(call.attempt).expect("known");
    core.complete(call.attempt, image_ok(), 30);

    let lines = audit.lines();

    assert!(
        lines[0]
            .starts_with("RESOURCE request purpose=image_plate class=plate handle=1 epoch=1 key=")
    );
    assert_eq!(
        lines[1],
        "RESOURCE claimed purpose=image_plate handle=1 attempt=1 worker=1 number=1"
    );
    assert_eq!(
        lines[2],
        "SPEND commit purpose=image_plate handle=1 attempt=1 price=unpriced"
    );
    assert_eq!(
        lines[3],
        "RESOURCE ready purpose=image_plate handle=1 ms=30"
    );
    assert!(lines.iter().all(|line| !line.contains('\n')));
}

#[test]
fn an_unknown_attempt_is_not_accepted_and_cannot_be_marked_sent() {
    let (mut core, _) = core_with(BrokerLimits::default());
    assert_eq!(
        core.complete(AttemptId::new(5), text_ok(), 0),
        CompleteOutcome::Unknown
    );
    assert!(core.mark_sent(AttemptId::new(5)).is_err());
}
