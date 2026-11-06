use std::num::{NonZeroU32, NonZeroU128};
use std::sync::Arc;

use crate::config::Settings;
use crate::envelope::{
    EngineInput, InputEnvelope, PendingId, PlayerInput, ProposalOrigin, ProposerRef, Sender,
    TimerName, UtteranceId, ViewStamp,
};
use crate::event::{AUDIT_LINE_CAPACITY_BYTES, EventClass, TableEvent};
use crate::identity::{
    Correlation, OperationId, RecoveryEpoch, SeatId, SessionRevision, TableId, TableScope, TurnSeq,
};
use crate::outcome::{DeclineCode, OutcomeKind};
use crate::owner_queue::{
    AdmitError, FinishError, Lane, NOT_YET_SERVED, NotDeclinedCause, OperationStatus, OwnerQueue,
    QueueConfigError, QueueLimits, RefusalReport, SeatError, ServeError, TokenBucket,
};
use crate::ports::{ActionId, EventSink, Readiness, ResourceHandle};
use crate::private_data::PlayerText;
use crate::testkit::RecordingSink;

pub(crate) struct NullSink;

impl EventSink for NullSink {
    fn record(&self, _event: &TableEvent) {}
}

pub(crate) fn settings(pending: u32, burst: u32, sustained_per_s: u32) -> Settings {
    let configuration = serde_json::json!({
        "seat_pending_limit": pending,
        "seat_rate_burst": burst,
        "seat_rate_sustained_per_s": sustained_per_s,
    });
    let object = configuration.as_object().expect("a JSON object");
    Settings::load(object)
        .expect("the specs are valid")
        .settings
}

pub(crate) fn operation(number: u64) -> OperationId {
    OperationId::from_nonzero_u128(NonZeroU128::MIN.saturating_add(u128::from(number)))
}

pub(crate) fn view() -> ViewStamp {
    ViewStamp::new(SessionRevision::new(RecoveryEpoch::FIRST, 1))
}

pub(crate) fn tap(operation_number: u64, seat: u64) -> InputEnvelope {
    InputEnvelope::from_player(
        operation(operation_number),
        SeatId::new(seat),
        view(),
        PlayerInput::Tap {
            action: ActionId::new("look").expect("valid action"),
        },
    )
}

pub(crate) const SEAT_ONE: Sender = Sender::Seat(SeatId::new(1));

pub(crate) fn safety_card(operation_number: u64, seat: u64) -> InputEnvelope {
    InputEnvelope::from_player(
        operation(operation_number),
        SeatId::new(seat),
        view(),
        PlayerInput::SafetyCard,
    )
}

pub(crate) fn pending_answer(operation_number: u64, seat: u64) -> InputEnvelope {
    InputEnvelope::from_player(
        operation(operation_number),
        SeatId::new(seat),
        view(),
        PlayerInput::PendingAnswer {
            pending: PendingId::new(1),
            choice: ActionId::new("accept").expect("valid action"),
        },
    )
}

/// A free-text line, the kind that goes to INTERPRET.
pub(crate) fn text_line(operation_number: u64, seat: u64) -> InputEnvelope {
    InputEnvelope::from_player(
        operation(operation_number),
        SeatId::new(seat),
        view(),
        PlayerInput::SubmitText {
            utterance: UtteranceId::new(operation_number),
            text: PlayerText::new("I look around"),
        },
    )
}

pub(crate) fn proposal(operation_number: u64) -> InputEnvelope {
    InputEnvelope::from_engine(
        operation(operation_number),
        view(),
        EngineInput::Proposal {
            origin: ProposalOrigin::World,
            proposer: ProposerRef::new("alarm").expect("valid proposer"),
        },
    )
}

pub(crate) fn timer(operation_number: u64) -> InputEnvelope {
    InputEnvelope::from_engine(
        operation(operation_number),
        view(),
        EngineInput::Timer {
            timer: TimerName::new("idle_nudge").expect("valid timer"),
        },
    )
}

pub(crate) fn resource_result(operation_number: u64) -> InputEnvelope {
    InputEnvelope::from_engine(
        operation(operation_number),
        view(),
        EngineInput::ResourceResult {
            handle: ResourceHandle::new(operation_number),
            readiness: Readiness::Ready,
        },
    )
}

/// A queue for table 1 with seats 1 to `seats` already at the table at time zero.
pub(crate) fn seated_queue(
    seats: u64,
    settings: &Settings,
    sink: Arc<dyn EventSink>,
) -> OwnerQueue {
    let mut queue = OwnerQueue::new(TableId::new(1), settings, sink).expect("settings are valid");
    for seat in 1..=seats {
        queue
            .join_seat(SeatId::new(seat), 0)
            .expect("the seat joins");
    }
    queue
}

fn quiet_queue(seats: u64, settings: &Settings) -> OwnerQueue {
    seated_queue(seats, settings, Arc::new(NullSink))
}

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("nonzero")
}

fn bucket(burst: u32, per_s: u32, now_ms: u64) -> TokenBucket {
    TokenBucket::full(nonzero(burst), nonzero(per_s), now_ms)
}

#[test]
fn a_new_bucket_holds_the_whole_burst_then_refuses_with_the_exact_wait() {
    let mut bucket = bucket(3, 1, 0);
    assert_eq!(bucket.held_millitokens(), 3_000);
    for _ in 0..3 {
        assert_eq!(bucket.try_take(0), Ok(()));
    }
    assert_eq!(bucket.try_take(0), Err(1_000));
    assert_eq!(bucket.held_millitokens(), 0);
}

#[test]
fn the_wait_a_bucket_reports_is_exactly_long_enough_and_not_a_millisecond_longer() {
    let mut bucket = bucket(1, 3, 0);
    assert_eq!(bucket.try_take(0), Ok(()));
    assert_eq!(bucket.try_take(0), Err(334));
    assert_eq!(bucket.try_take(333), Err(1));
    assert_eq!(bucket.held_millitokens(), 999);
    assert_eq!(bucket.try_take(334), Ok(()));
    assert_eq!(
        bucket.held_millitokens(),
        0,
        "the 2 extra millitokens passed the burst"
    );
}

#[test]
fn a_bucket_refills_exactly_and_never_past_its_burst() {
    let mut bucket = bucket(2, 1, 0);
    assert_eq!(bucket.try_take(0), Ok(()));
    assert_eq!(bucket.try_take(0), Ok(()));
    assert_eq!(bucket.try_take(500), Err(500));
    assert_eq!(bucket.held_millitokens(), 500);
    assert_eq!(bucket.try_take(1_500), Ok(()));
    assert_eq!(bucket.held_millitokens(), 500);
    assert_eq!(bucket.try_take(1_500 + 60_000), Ok(()));
    assert_eq!(bucket.held_millitokens(), 1_000);
}

#[test]
fn a_huge_elapsed_time_saturates_at_the_burst_without_overflowing() {
    let mut small = bucket(3, u32::MAX, 0);
    small.try_take(0).expect("a token is held");
    small.try_take(u64::MAX).expect("a token is held");
    assert_eq!(small.held_millitokens(), 2_000);
    let mut widest = bucket(u32::MAX, u32::MAX, 1);
    widest.try_take(u64::MAX).expect("a token is held");
    assert_eq!(
        widest.held_millitokens(),
        u64::from(u32::MAX) * 1_000 - 1_000
    );
}

#[test]
fn a_clock_that_moves_back_adds_nothing_and_does_not_move_the_refill_point_back() {
    let mut bucket = bucket(1, 1, 10_000);
    assert_eq!(bucket.try_take(10_000), Ok(()));
    assert_eq!(bucket.try_take(5_000), Err(1_000));
    assert_eq!(bucket.try_take(10_999), Err(1));
    assert_eq!(bucket.try_take(11_000), Ok(()));
}

#[test]
fn a_seat_may_send_its_burst_at_once_then_is_rate_limited_with_a_retry_time() {
    let mut queue = quiet_queue(2, &settings(100, 3, 1));
    for number in 1..=3 {
        assert!(queue.admit(tap(number, 1), 5_000).is_ok());
    }
    let refused = queue.admit(tap(4, 1), 5_000);
    assert_eq!(
        refused,
        Err(AdmitError::RateLimited {
            retry_after_ms: 1_000
        })
    );
    assert_eq!(
        refused.expect_err("refused").decline_code(),
        Some(DeclineCode::RateLimited)
    );
    assert!(
        queue.admit(tap(5, 2), 5_000).is_ok(),
        "another seat has its own bucket"
    );
    assert!(
        queue.admit(timer(6), 5_000).is_ok(),
        "engine input is not rate limited"
    );
    assert!(
        queue.admit(tap(7, 1), 6_000).is_ok(),
        "one second refills one token"
    );
}

#[test]
fn a_refused_input_is_not_remembered_so_the_same_id_can_be_sent_again() {
    let mut queue = quiet_queue(1, &settings(100, 1, 1));
    assert!(queue.admit(tap(1, 1), 0).is_ok());
    assert!(matches!(
        queue.admit(tap(2, 1), 0),
        Err(AdmitError::RateLimited { .. })
    ));
    assert_eq!(queue.status(SEAT_ONE, operation(2)), None);
    assert!(queue.admit(tap(2, 1), 1_000).is_ok());
}

#[test]
fn a_seat_over_its_pending_bound_is_busy_and_only_that_seat() {
    let mut queue = quiet_queue(2, &settings(2, 100, 1));
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 1), 0).expect("admitted");
    let refused = queue.admit(tap(3, 1), 0);
    assert_eq!(
        refused,
        Err(AdmitError::SeatBusy {
            pending: 2,
            limit: 2
        })
    );
    assert_eq!(
        refused.expect_err("refused").decline_code(),
        Some(DeclineCode::TableBusy)
    );
    assert!(
        queue.admit(tap(4, 2), 0).is_ok(),
        "another seat is not affected"
    );
    assert!(
        queue.admit(proposal(5), 0).is_ok(),
        "engine lanes are not affected"
    );
}

#[test]
fn an_input_in_progress_still_counts_against_its_seat_until_it_finishes() {
    let mut queue = quiet_queue(1, &settings(2, 100, 1));
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 1), 0).expect("admitted");
    let served = queue
        .serve_next(0)
        .expect("serves")
        .expect("an input waits");
    assert_eq!(queue.depth(Lane::Player), 1);
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(2));
    assert!(matches!(
        queue.admit(tap(3, 1), 0),
        Err(AdmitError::SeatBusy { .. })
    ));
    queue
        .finish(
            served.envelope.sender(),
            served.envelope.operation_id(),
            OutcomeKind::Applied,
        )
        .expect("finishes");
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(1));
    assert!(queue.admit(tap(3, 1), 0).is_ok());
}

#[test]
fn a_full_engine_lane_is_a_stall_with_no_decline_code() {
    let limits = QueueLimits {
        engine_lane_length: 1,
        timers_and_results_lane_length: 2,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(TableId::new(1), &settings(3, 3, 1), Arc::new(NullSink))
        .expect("builds")
        .with_limits(limits);
    queue.admit(proposal(1), 0).expect("admitted");
    let stalled = queue.admit(proposal(2), 0).expect_err("the lane is full");
    assert_eq!(
        stalled,
        AdmitError::LaneStalled {
            lane: Lane::Engine,
            limit: 1
        }
    );
    assert_eq!(stalled.decline_code(), None);
    assert_eq!(
        stalled.report(),
        RefusalReport::NotDeclined(NotDeclinedCause::LaneStalled)
    );
    queue.admit(timer(3), 0).expect("admitted");
    queue.admit(resource_result(4), 0).expect("admitted");
    assert_eq!(
        queue.admit(timer(5), 0),
        Err(AdmitError::LaneStalled {
            lane: Lane::TimersAndResults,
            limit: 2
        })
    );
}

#[test]
fn a_table_seats_at_most_its_limit_and_a_seat_joins_once() {
    let limits = QueueLimits {
        seats: 2,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(TableId::new(1), &settings(3, 3, 1), Arc::new(NullSink))
        .expect("builds")
        .with_limits(limits);
    queue.join_seat(SeatId::new(1), 0).expect("joins");
    assert_eq!(
        queue.join_seat(SeatId::new(1), 0),
        Err(SeatError::AlreadySeated {
            seat: SeatId::new(1)
        })
    );
    queue.join_seat(SeatId::new(2), 0).expect("joins");
    assert_eq!(
        queue.join_seat(SeatId::new(3), 0),
        Err(SeatError::TooManySeats { limit: 2 })
    );
}

#[test]
fn an_input_from_a_seat_that_is_not_at_the_table_is_refused_before_anything_else() {
    let mut queue = quiet_queue(1, &settings(3, 3, 1));
    queue.admit(tap(1, 1), 0).expect("admitted");
    let refused = queue.admit(tap(1, 9), 0);
    assert_eq!(
        refused,
        Err(AdmitError::UnknownSeat {
            seat: SeatId::new(9)
        })
    );
    assert_eq!(refused.expect_err("refused").decline_code(), None);
}

#[test]
fn leaving_returns_the_waiting_inputs_and_lets_their_ids_be_sent_again() {
    let mut queue = quiet_queue(2, &settings(5, 5, 1));
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 1), 0).expect("admitted");
    queue.admit(tap(3, 2), 0).expect("admitted");
    assert_eq!(
        queue.leave_seat(SeatId::new(1), 0),
        Ok(vec![operation(1), operation(2)])
    );
    assert_eq!(queue.depth(Lane::Player), 1);
    assert_eq!(queue.status(SEAT_ONE, operation(1)), None);
    assert_eq!(
        queue.leave_seat(SeatId::new(1), 0),
        Err(SeatError::UnknownSeat {
            seat: SeatId::new(1)
        })
    );
    let served = queue.serve_next(0).expect("serves").expect("one waits");
    assert_eq!(served.envelope.seat(), Some(SeatId::new(2)));
}

#[test]
fn a_finish_for_an_input_of_a_membership_that_left_is_unknown_and_changes_nothing_for_the_new_one()
{
    let mut queue = quiet_queue(1, &settings(2, 5, 1));
    queue.admit(tap(1, 1), 0).expect("admitted");
    let _in_progress = queue.serve_next(0).expect("serves").expect("one waits");
    queue.leave_seat(SeatId::new(1), 0).expect("leaves");
    queue.join_seat(SeatId::new(1), 0).expect("joins again");
    queue.admit(tap(2, 1), 0).expect("admitted");
    let _in_progress = queue.serve_next(0).expect("serves").expect("one waits");
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(1));
    assert_eq!(
        queue.finish(SEAT_ONE, operation(1), OutcomeKind::Applied),
        Err(FinishError::UnknownOperation),
        "the old membership's input was abandoned when its seat left"
    );
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(1));
    queue
        .finish(SEAT_ONE, operation(2), OutcomeKind::Applied)
        .expect("finishes");
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(0));
}

#[test]
fn the_lanes_are_served_engine_then_timers_and_results_then_players() {
    let mut queue = quiet_queue(2, &settings(9, 9, 1));
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(timer(2), 0).expect("admitted");
    queue.admit(tap(3, 2), 0).expect("admitted");
    queue.admit(resource_result(4), 0).expect("admitted");
    queue.admit(proposal(5), 0).expect("admitted");
    queue.admit(proposal(6), 0).expect("admitted");
    let order: Vec<(u64, Lane)> = std::iter::from_fn(|| queue.serve_next(0).expect("serves"))
        .map(|served| (served.turn_seq.get(), served.lane))
        .collect();
    assert_eq!(
        order,
        [
            (1, Lane::Engine),
            (2, Lane::Engine),
            (3, Lane::TimersAndResults),
            (4, Lane::TimersAndResults),
            (5, Lane::Player),
            (6, Lane::Player),
        ]
    );
}

#[test]
fn an_engine_input_admitted_late_is_served_before_player_input_already_waiting() {
    let mut queue = quiet_queue(1, &settings(9, 9, 1));
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 1), 0).expect("admitted");
    queue.admit(proposal(3), 10).expect("admitted");
    let first = queue.serve_next(10).expect("serves").expect("one waits");
    assert_eq!(first.envelope.operation_id(), operation(3));
    assert_eq!(first.lane, Lane::Engine);
}

#[test]
fn every_kind_of_input_goes_to_the_lane_the_plan_names() {
    let mut queue = quiet_queue(1, &settings(9, 9, 1));
    let text = || PlayerText::new("a line");
    let player = |number, input| {
        InputEnvelope::from_player(operation(number), SeatId::new(1), view(), input)
    };
    let admitted = [
        (tap(1, 1), Lane::Player),
        (
            player(
                2,
                PlayerInput::SubmitText {
                    utterance: UtteranceId::new(1),
                    text: text(),
                },
            ),
            Lane::Player,
        ),
        (
            player(
                3,
                PlayerInput::Say {
                    utterance: UtteranceId::new(2),
                    text: text(),
                },
            ),
            Lane::Player,
        ),
        (
            player(
                4,
                PlayerInput::PrivateMessage {
                    utterance: UtteranceId::new(3),
                    to: SeatId::new(2),
                    text: text(),
                },
            ),
            Lane::Player,
        ),
        (player(5, PlayerInput::SafetyCard), Lane::SeatControls),
        (
            player(
                6,
                PlayerInput::PendingAnswer {
                    pending: PendingId::new(1),
                    choice: ActionId::new("accept").expect("valid action"),
                },
            ),
            Lane::SeatControls,
        ),
        (proposal(7), Lane::Engine),
        (timer(8), Lane::TimersAndResults),
        (resource_result(9), Lane::TimersAndResults),
    ];
    assert_eq!(admitted.len(), crate::envelope::InputKind::ALL.len());
    for (envelope, lane) in admitted {
        let kind = envelope.kind();
        assert_eq!(
            queue.admit(envelope, 0),
            Ok(crate::owner_queue::Admitted { lane }),
            "{kind:?}"
        );
    }
}

#[test]
fn the_player_lane_serves_seats_in_turn_and_each_seat_in_the_order_it_sent() {
    let mut queue = quiet_queue(3, &settings(9, 9, 1));
    let mut number = 0;
    for _round in 0..3 {
        for seat in 1..=3 {
            number += 1;
            queue.admit(tap(number, seat), 0).expect("admitted");
        }
    }
    let order: Vec<(u64, u64)> = std::iter::from_fn(|| queue.serve_next(0).expect("serves"))
        .map(|served| {
            let seat = served.envelope.seat().expect("a player input").get();
            (seat, served.turn_seq.get())
        })
        .collect();
    let seats: Vec<u64> = order.iter().map(|(seat, _)| *seat).collect();
    assert_eq!(seats, [1, 2, 3, 1, 2, 3, 1, 2, 3]);
    let turns: Vec<u64> = order.iter().map(|(_, turn)| *turn).collect();
    assert_eq!(turns, (1..=9).collect::<Vec<u64>>());
}

#[test]
fn the_wait_of_a_served_input_runs_from_admission_to_service() {
    let mut queue = quiet_queue(1, &settings(9, 9, 1));
    queue.admit(tap(1, 1), 100).expect("admitted");
    let served = queue.serve_next(350).expect("serves").expect("one waits");
    assert_eq!(served.waited_ms, 250);
    queue.admit(tap(2, 1), 400).expect("admitted");
    let earlier_clock = queue.serve_next(300).expect("serves").expect("one waits");
    assert_eq!(
        earlier_clock.waited_ms, 0,
        "a clock that moved back never underflows"
    );
}

#[test]
fn a_duplicate_is_refused_with_where_the_first_one_stands() {
    let mut queue = quiet_queue(1, &settings(9, 9, 1));
    queue.admit(tap(1, 1), 0).expect("admitted");
    assert_eq!(
        queue.admit(tap(1, 1), 0),
        Err(AdmitError::Duplicate {
            first: OperationStatus::Queued { lane: Lane::Player }
        })
    );
    let served = queue.serve_next(0).expect("serves").expect("one waits");
    let in_progress = OperationStatus::InProgress {
        turn_seq: served.turn_seq,
    };
    assert_eq!(
        queue.admit(tap(1, 1), 0),
        Err(AdmitError::Duplicate { first: in_progress })
    );
    queue
        .finish(SEAT_ONE, operation(1), OutcomeKind::Declined)
        .expect("finishes");
    let finished = OperationStatus::Finished {
        turn_seq: served.turn_seq,
        outcome: OutcomeKind::Declined,
    };
    assert_eq!(
        queue.admit(tap(1, 1), 0),
        Err(AdmitError::Duplicate { first: finished })
    );
    assert_eq!(queue.status(SEAT_ONE, operation(1)), Some(finished));
    assert_eq!(queue.depth(Lane::Player), 0, "the duplicate never queued");
}

#[test]
fn a_duplicate_costs_the_seat_neither_a_token_nor_a_pending_slot() {
    let mut queue = quiet_queue(1, &settings(1, 1, 1));
    queue.admit(tap(1, 1), 0).expect("admitted");
    for _ in 0..10 {
        assert!(matches!(
            queue.admit(tap(1, 1), 0),
            Err(AdmitError::Duplicate { .. })
        ));
    }
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(1));
    let served = queue.serve_next(0).expect("serves").expect("one waits");
    queue
        .finish(
            served.envelope.sender(),
            served.envelope.operation_id(),
            OutcomeKind::Applied,
        )
        .expect("finishes");
    assert!(
        queue.admit(tap(2, 1), 1_000).is_ok(),
        "the one token refilled, no extra was spent"
    );
}

#[test]
fn engine_inputs_are_deduplicated_by_operation_id_too() {
    let mut queue = quiet_queue(0, &settings(9, 9, 1));
    queue.admit(timer(1), 0).expect("admitted");
    assert!(matches!(
        queue.admit(timer(1), 0),
        Err(AdmitError::Duplicate { .. })
    ));
    assert!(matches!(
        queue.admit(proposal(1), 0),
        Err(AdmitError::Duplicate { .. })
    ));
}

#[test]
fn finishing_an_unknown_queued_or_finished_operation_is_a_typed_refusal() {
    let mut queue = quiet_queue(1, &settings(9, 9, 1));
    assert_eq!(
        queue.finish(SEAT_ONE, operation(1), OutcomeKind::Applied),
        Err(FinishError::UnknownOperation)
    );
    queue.admit(tap(1, 1), 0).expect("admitted");
    assert_eq!(
        queue.finish(SEAT_ONE, operation(1), OutcomeKind::Applied),
        Err(FinishError::NotInProgress {
            status: OperationStatus::Queued { lane: Lane::Player }
        })
    );
    let served = queue.serve_next(0).expect("serves").expect("one waits");
    queue
        .finish(SEAT_ONE, operation(1), OutcomeKind::Applied)
        .expect("finishes");
    assert_eq!(
        queue.finish(SEAT_ONE, operation(1), OutcomeKind::Pending),
        Err(FinishError::NotInProgress {
            status: OperationStatus::Finished {
                turn_seq: served.turn_seq,
                outcome: OutcomeKind::Applied
            }
        })
    );
}

#[test]
fn the_oldest_finished_operations_are_forgotten_first_and_then_their_ids_are_new_again() {
    let limits = QueueLimits {
        remembered_operations: 2,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(TableId::new(1), &settings(9, 9, 1), Arc::new(NullSink))
        .expect("builds")
        .with_limits(limits);
    queue.join_seat(SeatId::new(1), 0).expect("joins");
    for number in 1..=3 {
        queue.admit(tap(number, 1), 0).expect("admitted");
        let served = queue.serve_next(0).expect("serves").expect("one waits");
        queue
            .finish(
                served.envelope.sender(),
                served.envelope.operation_id(),
                OutcomeKind::Applied,
            )
            .expect("finishes");
    }
    assert_eq!(queue.remembered_operations(), 2);
    assert_eq!(queue.status(SEAT_ONE, operation(1)), None);
    assert!(queue.status(SEAT_ONE, operation(2)).is_some());
    assert!(queue.status(SEAT_ONE, operation(3)).is_some());
    assert!(
        queue.admit(tap(1, 1), 0).is_ok(),
        "a forgotten id is admitted as new"
    );
}

#[test]
fn the_first_input_served_is_number_one_and_a_resumed_queue_continues_after_the_last() {
    let mut fresh = quiet_queue(1, &settings(9, 9, 1));
    fresh.admit(tap(1, 1), 0).expect("admitted");
    assert_eq!(fresh.last_turn_seq(), NOT_YET_SERVED);
    assert_eq!(
        fresh
            .serve_next(0)
            .expect("serves")
            .expect("one waits")
            .turn_seq,
        TurnSeq::new(1)
    );
    let mut resumed = OwnerQueue::new(TableId::new(1), &settings(9, 9, 1), Arc::new(NullSink))
        .expect("builds")
        .resuming_after(TurnSeq::new(41));
    resumed.join_seat(SeatId::new(1), 0).expect("joins");
    resumed.admit(tap(1, 1), 0).expect("admitted");
    assert_eq!(
        resumed
            .serve_next(0)
            .expect("serves")
            .expect("one waits")
            .turn_seq,
        TurnSeq::new(42)
    );
}

#[test]
fn an_exhausted_turn_sequence_refuses_to_serve_and_keeps_the_input_queued() {
    let mut queue = OwnerQueue::new(TableId::new(1), &settings(9, 9, 1), Arc::new(NullSink))
        .expect("builds")
        .resuming_after(TurnSeq::new(u64::MAX));
    queue.join_seat(SeatId::new(1), 0).expect("joins");
    assert_eq!(
        queue.serve_next(0),
        Ok(None),
        "an empty queue serves nothing, exhausted or not"
    );
    queue.admit(tap(1, 1), 0).expect("admitted");
    assert_eq!(queue.serve_next(0), Err(ServeError::TurnSequenceExhausted));
    assert_eq!(queue.depth(Lane::Player), 1);
    assert_eq!(
        queue.status(SEAT_ONE, operation(1)),
        Some(OperationStatus::Queued { lane: Lane::Player })
    );
}

#[test]
fn a_zero_rate_setting_cannot_build_a_queue_and_the_defaults_can() {
    assert!(OwnerQueue::new(TableId::new(1), &Settings::default(), Arc::new(NullSink)).is_ok());
    assert_eq!(
        QueueConfigError::ZeroSetting(crate::config::SettingId::SeatRateBurst).to_string(),
        "the setting SeatRateBurst is zero"
    );
}

fn lines(sink: &RecordingSink) -> Vec<String> {
    sink.events()
        .iter()
        .map(TableEvent::render_audit_line)
        .collect()
}

#[test]
fn an_admitted_input_is_recorded_received_and_later_served_with_its_lane_and_wait() {
    let sink = Arc::new(RecordingSink::default());
    let mut queue = seated_queue(1, &settings(9, 9, 1), sink.clone());
    queue.admit(tap(1, 1), 100).expect("admitted");
    let _in_progress = queue.serve_next(350).expect("serves").expect("one waits");
    let id = operation(1);
    assert_eq!(
        lines(&sink),
        [
            format!(
                "INPUT received table=1 operation_id={id} turn_seq=0 epoch=1 revision=1 length=empty"
            ),
            format!(
                "INPUT served table=1 operation_id={id} turn_seq=1 epoch=1 revision=1 lane=player ms=250"
            ),
        ]
    );
}

#[test]
fn a_refusal_with_a_code_is_declined_and_one_without_is_refused_with_its_cause() {
    let sink = Arc::new(RecordingSink::default());
    let mut queue = seated_queue(1, &settings(1, 9, 1), sink.clone());
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 1), 0).expect_err("the seat is busy");
    queue.admit(tap(1, 1), 0).expect_err("a duplicate");
    queue.admit(tap(3, 7), 0).expect_err("an unknown seat");
    let id = |number| operation(number);
    let context = |number| {
        format!(
            "table=1 operation_id={} turn_seq=0 epoch=1 revision=1",
            id(number)
        )
    };
    assert_eq!(
        lines(&sink),
        [
            format!("INPUT received {} length=empty", context(1)),
            format!("INPUT received {} length=empty", context(2)),
            format!("INPUT declined {} code=table_busy", context(2)),
            format!("INPUT received {} length=empty", context(1)),
            format!("INPUT refused {} cause=duplicate", context(1)),
            format!("INPUT received {} length=empty", context(3)),
            format!("INPUT refused {} cause=unknown_seat", context(3)),
        ]
    );
}

#[test]
fn a_stalled_lane_is_recorded_as_a_refusal_not_a_decline() {
    let sink = Arc::new(RecordingSink::default());
    let limits = QueueLimits {
        engine_lane_length: 0,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(TableId::new(1), &settings(9, 9, 1), sink.clone())
        .expect("builds")
        .with_limits(limits);
    queue
        .admit(proposal(1), 0)
        .expect_err("the lane holds nothing");
    let recorded = lines(&sink);
    assert_eq!(recorded.len(), 2);
    assert!(recorded[1].starts_with("INPUT refused "), "{}", recorded[1]);
    assert!(
        recorded[1].ends_with("cause=lane_stalled"),
        "{}",
        recorded[1]
    );
}

#[test]
fn typed_text_reaches_an_event_only_as_a_length_class() {
    const SENTINEL: &str = "SENTINEL-LINE-91ab";
    let sink = Arc::new(RecordingSink::default());
    let mut queue = seated_queue(1, &settings(9, 9, 1), sink.clone());
    let line = InputEnvelope::from_player(
        operation(1),
        SeatId::new(1),
        view(),
        PlayerInput::SubmitText {
            utterance: UtteranceId::new(1),
            text: PlayerText::new(SENTINEL),
        },
    );
    queue.admit(line, 0).expect("admitted");
    let _in_progress = queue.serve_next(0).expect("serves").expect("one waits");
    let everything = format!("{:?}{:?}", sink.events(), lines(&sink));
    assert!(!everything.contains(SENTINEL), "{everything}");
    assert!(lines(&sink)[0].ends_with("length=short"));
}

#[test]
fn the_new_events_are_class_a_and_round_trip_through_serde() {
    let sink = Arc::new(RecordingSink::default());
    let mut queue = seated_queue(1, &settings(1, 9, 1), sink.clone());
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(1, 1), 0).expect_err("a duplicate");
    let _in_progress = queue.serve_next(0).expect("serves").expect("one waits");
    for event in sink.events() {
        assert_eq!(event.class(), EventClass::Always);
        let json = serde_json::to_string(&event).expect("serializes");
        let back: TableEvent = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, event);
    }
}

#[test]
fn the_widest_possible_queue_event_lines_fit_the_reserved_capacity() {
    // The queue records the table only: an input is not yet in a scene or a machine.
    let scope = TableScope::of_table(TableId::new(u64::MAX));
    let correlation = Correlation {
        operation_id: OperationId::from_bytes(&[0xff; 16]).expect("valid id"),
        turn_seq: TurnSeq::new(u64::MAX),
        revision: SessionRevision::new(RecoveryEpoch::new(u64::MAX).expect("valid"), u64::MAX),
    };
    let widest = [
        TableEvent::InputServed {
            scope: scope.clone(),
            correlation,
            lane: Lane::TimersAndResults,
            ms: u64::MAX,
        },
        TableEvent::InputRefused {
            scope,
            correlation,
            cause: NotDeclinedCause::LaneStalled,
        },
    ];
    for event in widest {
        let rendered = event.render_audit_line();
        assert!(
            rendered.len() <= AUDIT_LINE_CAPACITY_BYTES,
            "{} bytes",
            rendered.len()
        );
    }
}

#[test]
fn the_four_lane_keys_and_the_three_causes_are_distinct_and_listed() {
    let lanes: Vec<&str> = Lane::ALL.iter().map(|lane| lane.key()).collect();
    assert_eq!(
        lanes,
        ["seat_controls", "engine", "timers_and_results", "player"]
    );
    let causes: Vec<&str> = NotDeclinedCause::ALL
        .iter()
        .map(|cause| cause.key())
        .collect();
    assert_eq!(causes, ["unknown_seat", "duplicate", "lane_stalled"]);
}

/// Admits `inputs` mixed inputs (taps, pause cards, timers) over twelve seats and serves them all
/// with the queue remembering only 1_000 ended operations, so eviction runs all the way through.
/// A seat leaves and comes back now and then, every 1_500th served input is left unfinished and
/// the watchdog reaps the leaked ones. Returns how many entries the queue touched in all.
fn touched_serving(inputs: u64) -> u64 {
    const SEATS: u64 = 12;
    let limits = QueueLimits {
        remembered_operations: 1_000,
        in_progress_lease_ms: 50,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(
        TableId::new(1),
        &settings(1_000_000, 1_000_000, 1),
        Arc::new(NullSink),
    )
    .expect("builds")
    .with_limits(limits);
    for seat in 1..=SEATS {
        queue.join_seat(SeatId::new(seat), 0).expect("joins");
    }
    crate::owner_queue::reset_touched_entries();
    for number in 1..=inputs {
        let seat = 1 + number % SEATS;
        let envelope = if number % 7 == 0 {
            safety_card(number, seat)
        } else if number % 11 == 0 {
            timer(number)
        } else {
            tap(number, seat)
        };
        queue.admit(envelope, 0).expect("admitted");
    }
    let mut served = 0_u64;
    let mut discarded = 0_u64;
    let mut reaped = 0_u64;
    let mut leaked = 0_u64;
    while let Some(input) = queue.serve_next(served).expect("serves") {
        served += 1;
        assert_eq!(input.turn_seq.get(), served);
        if served.is_multiple_of(1_500) {
            leaked += 1;
        } else {
            queue
                .finish(
                    input.envelope.sender(),
                    input.envelope.operation_id(),
                    OutcomeKind::Applied,
                )
                .expect("finishes");
        }
        if served.is_multiple_of(997) {
            let churned = SeatId::new(1 + served % SEATS);
            discarded += queue.leave_seat(churned, served).expect("leaves").len() as u64;
            queue.join_seat(churned, served).expect("joins again");
        }
        if served.is_multiple_of(1_000) {
            reaped += queue.reap_expired(served).len() as u64;
        }
    }
    reaped += queue.reap_expired(served + 1_000).len() as u64;
    assert_eq!(served + discarded, inputs);
    assert!(
        reaped <= leaked && reaped > 0,
        "the watchdog reaped {reaped} of {leaked} leaked inputs; a seat leaving abandons the rest"
    );
    assert!(queue.in_progress(served + 1_000).is_empty());
    assert!(
        queue.remembered_operations() <= 1_000,
        "ended operations were evicted down to the limit"
    );
    crate::owner_queue::touched_entries()
}

/// Catches the mutations that make serving cost more than a constant per input: an eviction that
/// scans the remembered operations, a `reap_expired` that rescans on every serve, or a serve that
/// walks every seat. The bound is about twice the cost measured when it was written.
#[test]
fn the_work_of_serving_grows_in_step_with_the_inputs_served() {
    let small = touched_serving(20_000);
    let large = touched_serving(80_000);
    assert!(
        large <= small * 45 / 10,
        "four times the input touched {large} entries against {small}"
    );
    assert!(
        large <= MEASURED_TOUCHES_PER_INPUT * 80_000,
        "{large} entries touched for 80_000 inputs"
    );
}

const MEASURED_TOUCHES_PER_INPUT: u64 = 25;

#[test]
fn a_hundred_thousand_inputs_over_twelve_seats_are_served_balanced() {
    const SEATS: u64 = 12;
    const INPUTS: u64 = 100_000;
    let mut queue = quiet_queue(SEATS, &settings(1_000_000, 1_000_000, 1));
    for number in 1..=INPUTS {
        queue
            .admit(tap(number, 1 + number % SEATS), 0)
            .expect("admitted");
    }
    assert_eq!(queue.depth(Lane::Player), 100_000);
    let mut served_per_seat = [0_u64; SEATS as usize];
    let mut served = 0_u64;
    while let Some(input) = queue.serve_next(0).expect("serves") {
        served += 1;
        assert_eq!(input.turn_seq.get(), served);
        let seat = input.envelope.seat().expect("a player input").get();
        served_per_seat[(seat - 1) as usize] += 1;
        let most = served_per_seat.iter().max().copied().unwrap_or(0);
        let least = served_per_seat.iter().min().copied().unwrap_or(0);
        assert!(most - least <= 1, "seats drifted apart at serve {served}");
        queue
            .finish(
                input.envelope.sender(),
                input.envelope.operation_id(),
                OutcomeKind::Applied,
            )
            .expect("finishes");
    }
    assert_eq!(served, INPUTS);
    assert_eq!(queue.depth(Lane::Player), 0);
}
