//! A served input that never ends, where a waiting input stands (K.6), and the round-robin cursor
//! when seats come and go.

use std::sync::Arc;

use crate::envelope::Sender;
use crate::event::FailureCause;
use crate::identity::{SeatId, TableId};
use crate::limits::MAXIMUM_IN_PROGRESS_MS;
use crate::outcome::OutcomeKind;
use crate::owner_queue::{
    AdmitError, FinishError, Lane, OperationStatus, OwnerQueue, QueueLimits, ServedInput,
};
use crate::owner_queue_tests::{
    NullSink, SEAT_ONE, operation, proposal, safety_card, seated_queue, settings, tap, text_line,
    timer,
};
use crate::testkit::RecordingSink;

fn quiet_queue(seats: u64) -> OwnerQueue {
    seated_queue(seats, &settings(3, 9, 1), Arc::new(NullSink))
}

fn serve_at(queue: &mut OwnerQueue, now_ms: u64) -> ServedInput {
    queue
        .serve_next(now_ms)
        .expect("serves")
        .expect("an input waits")
}

fn end(queue: &mut OwnerQueue, served: &ServedInput) {
    queue
        .finish(
            served.envelope.sender(),
            served.envelope.operation_id(),
            OutcomeKind::Applied,
        )
        .expect("finishes");
}

fn serve_order(queue: &mut OwnerQueue) -> Vec<u64> {
    let mut seats = Vec::new();
    while let Some(served) = queue.serve_next(0).expect("serves") {
        seats.push(served.envelope.seat().expect("a seat input").get());
        end(queue, &served);
    }
    seats
}

fn leak_three_taps(queue: &mut OwnerQueue) {
    for number in 1..=3 {
        queue.admit(tap(number, 1), 0).expect("admitted");
    }
    for number in 1..=3 {
        let leaked = serve_at(queue, number * 1_000);
        drop(leaked);
    }
}

#[test]
fn a_leaked_input_locks_its_seat_but_is_found_by_its_age() {
    let mut queue = quiet_queue(1);
    leak_three_taps(&mut queue);
    assert!(matches!(
        queue.admit(tap(4, 1), 5_000),
        Err(AdmitError::SeatBusy { .. })
    ));
    let stuck = queue.in_progress(10_000);
    let ages: Vec<u64> = stuck.iter().map(|stuck| stuck.age_ms).collect();
    assert_eq!(ages, [9_000, 8_000, 7_000], "oldest first");
    assert_eq!(stuck[0].sender, SEAT_ONE);
    assert_eq!(stuck[0].operation_id, operation(1));
    assert_eq!(stuck[0].turn_seq.get(), 1);
}

#[test]
fn abandoning_a_leaked_input_frees_the_seat_and_says_so() {
    let sink = Arc::new(RecordingSink::default());
    let mut queue = seated_queue(1, &settings(3, 9, 1), sink.clone());
    leak_three_taps(&mut queue);
    queue
        .abandon(SEAT_ONE, operation(1), FailureCause::TimedOut, 11_000)
        .expect("abandons");
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(2));
    assert_eq!(queue.in_progress(11_000).len(), 2);
    assert!(
        queue.admit(tap(4, 1), 11_000).is_ok(),
        "the seat has room again"
    );
    assert!(matches!(
        queue.status(SEAT_ONE, operation(1)),
        Some(OperationStatus::Abandoned { .. })
    ));
    let lines: Vec<String> = sink
        .events()
        .iter()
        .map(|event| event.render_audit_line())
        .filter(|line| line.starts_with("INPUT abandoned"))
        .collect();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].ends_with("turn_seq=1 epoch=1 revision=1 cause=timed_out ms=10000"),
        "{}",
        lines[0]
    );
}

/// Catches: a `finish` that refuses after an abandon (the outcome is lost and a retry runs twice), or
/// that records the late outcome twice or frees the seat a second time.
#[test]
fn a_finish_after_the_abandon_is_recorded_once_and_a_retry_of_the_same_id_learns_it() {
    let sink = Arc::new(RecordingSink::default());
    let mut queue = seated_queue(1, &settings(3, 9, 1), sink.clone());
    queue.admit(tap(1, 1), 0).expect("admitted");
    let served = serve_at(&mut queue, 0);
    queue
        .abandon(SEAT_ONE, operation(1), FailureCause::TimedOut, 1)
        .expect("abandons");
    let abandoned = OperationStatus::Abandoned {
        turn_seq: served.turn_seq,
    };
    assert_eq!(
        queue.admit(tap(1, 1), 2),
        Err(AdmitError::Duplicate { first: abandoned }),
        "until the outcome arrives the id hears it was abandoned"
    );
    assert_eq!(
        queue.finish(SEAT_ONE, operation(1), OutcomeKind::Applied),
        Ok(()),
        "the late outcome is not lost"
    );
    let finished = OperationStatus::Finished {
        turn_seq: served.turn_seq,
        outcome: OutcomeKind::Applied,
    };
    assert_eq!(queue.status(SEAT_ONE, operation(1)), Some(finished));
    assert_eq!(
        queue.admit(tap(1, 1), 3),
        Err(AdmitError::Duplicate { first: finished }),
        "a retry of the same id learns the outcome and nothing runs twice"
    );
    assert_eq!(
        queue.finish(SEAT_ONE, operation(1), OutcomeKind::Declined),
        Err(FinishError::NotInProgress { status: finished }),
        "the late outcome is recorded once"
    );
    assert_eq!(
        queue.seat_pending(SeatId::new(1)),
        Some(0),
        "the seat was freed at the abandon"
    );
    let late: Vec<String> = sink
        .events()
        .iter()
        .map(|event| event.render_audit_line())
        .filter(|line| line.starts_with("INPUT finished_late"))
        .collect();
    assert_eq!(late.len(), 1);
    assert!(
        late[0].ends_with("turn_seq=1 epoch=1 revision=1 outcome=applied"),
        "{}",
        late[0]
    );
}

#[test]
fn reaping_abandons_only_what_has_run_for_the_lease_and_oldest_first() {
    let limits = QueueLimits {
        in_progress_lease_ms: 5_000,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(TableId::new(1), &settings(3, 9, 1), Arc::new(NullSink))
        .expect("builds")
        .with_limits(limits);
    queue.join_seat(SeatId::new(1), 0).expect("joins");
    for number in 1..=3 {
        queue.admit(tap(number, 1), 0).expect("admitted");
    }
    let _first = serve_at(&mut queue, 0);
    let _second = serve_at(&mut queue, 3_000);
    let _third = serve_at(&mut queue, 6_000);
    assert!(
        queue.reap_expired(4_999).is_empty(),
        "nothing has run 5 s yet"
    );
    let reaped = queue.reap_expired(7_000);
    assert_eq!(reaped.len(), 1);
    assert_eq!(
        (reaped[0].operation_id, reaped[0].age_ms),
        (operation(1), 7_000)
    );
    let reaped = queue.reap_expired(9_000);
    assert_eq!(reaped.len(), 1);
    assert_eq!(
        (reaped[0].operation_id, reaped[0].age_ms),
        (operation(2), 6_000)
    );
    assert_eq!(queue.in_progress(9_000).len(), 1);
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(1));
}

#[test]
fn the_default_lease_is_one_named_large_constant() {
    assert_eq!(
        QueueLimits::default().in_progress_lease_ms,
        MAXIMUM_IN_PROGRESS_MS
    );
    let mut queue = quiet_queue(1);
    queue.admit(tap(1, 1), 0).expect("admitted");
    let _served = serve_at(&mut queue, 0);
    assert!(queue.reap_expired(MAXIMUM_IN_PROGRESS_MS - 1).is_empty());
    assert_eq!(queue.reap_expired(MAXIMUM_IN_PROGRESS_MS).len(), 1);
}

#[test]
fn abandoning_an_input_of_a_membership_that_left_changes_nothing_for_the_new_one() {
    let mut queue = quiet_queue(1);
    queue.admit(tap(1, 1), 0).expect("admitted");
    let _old = serve_at(&mut queue, 0);
    queue.leave_seat(SeatId::new(1), 0).expect("leaves");
    queue.join_seat(SeatId::new(1), 0).expect("joins again");
    queue.admit(tap(2, 1), 0).expect("admitted");
    let _new = serve_at(&mut queue, 0);
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(1));
    assert_eq!(
        queue.abandon(SEAT_ONE, operation(1), FailureCause::TimedOut, 10),
        Err(FinishError::UnknownOperation)
    );
    assert_eq!(
        queue.seat_pending(SeatId::new(1)),
        Some(1),
        "the new input still counts"
    );
}

#[test]
fn reaping_a_free_text_line_in_interpret_lets_the_seats_next_line_run() {
    let mut queue = quiet_queue(1);
    queue.admit(text_line(1, 1), 0).expect("admitted");
    queue.admit(text_line(2, 1), 0).expect("admitted");
    let _stuck = serve_at(&mut queue, 0);
    assert!(queue.serve_next(10).expect("serves").is_none());
    queue.reap_expired(MAXIMUM_IN_PROGRESS_MS);
    assert_eq!(
        serve_at(&mut queue, MAXIMUM_IN_PROGRESS_MS)
            .envelope
            .operation_id(),
        operation(2)
    );
}

#[test]
fn the_position_of_a_waiting_input_counts_the_lanes_ahead_of_it() {
    let mut queue = quiet_queue(2);
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 2), 0).expect("admitted");
    queue.admit(timer(3), 0).expect("admitted");
    queue.admit(proposal(4), 0).expect("admitted");
    queue.admit(safety_card(5, 2), 0).expect("admitted");
    let ahead = |sender, number| {
        queue
            .position(sender, operation(number))
            .map(|position| (position.lane, position.ahead))
    };
    assert_eq!(
        ahead(Sender::Seat(SeatId::new(2)), 5),
        Some((Lane::SeatControls, 0))
    );
    assert_eq!(ahead(Sender::Engine, 4), Some((Lane::Engine, 1)));
    assert_eq!(ahead(Sender::Engine, 3), Some((Lane::TimersAndResults, 2)));
    assert_eq!(
        ahead(Sender::Seat(SeatId::new(1)), 1),
        Some((Lane::Player, 3))
    );
    assert_eq!(
        ahead(Sender::Seat(SeatId::new(2)), 2),
        Some((Lane::Player, 4))
    );
    assert_eq!(ahead(Sender::Seat(SeatId::new(1)), 99), None);
    let served = serve_at(&mut queue, 0);
    assert_eq!(
        queue.position(Sender::Seat(SeatId::new(2)), served.envelope.operation_id()),
        None
    );
    end(&mut queue, &served);
    assert_eq!(
        queue
            .position(Sender::Engine, operation(4))
            .map(|position| position.ahead),
        Some(0)
    );
}

#[test]
fn a_line_held_behind_its_own_free_text_says_so() {
    let mut queue = quiet_queue(2);
    queue.admit(text_line(1, 1), 0).expect("admitted");
    queue.admit(text_line(2, 1), 0).expect("admitted");
    queue.admit(tap(3, 2), 0).expect("admitted");
    let first = serve_at(&mut queue, 0);
    let held = queue
        .position(Sender::Seat(SeatId::new(1)), operation(2))
        .expect("it waits");
    assert!(held.held_behind_own_text);
    let free = queue
        .position(Sender::Seat(SeatId::new(2)), operation(3))
        .expect("it waits");
    assert!(!free.held_behind_own_text);
    assert_eq!(free.ahead, 0);
    end(&mut queue, &first);
    let released = queue
        .position(Sender::Seat(SeatId::new(1)), operation(2))
        .expect("it waits");
    assert!(!released.held_behind_own_text);
}

fn queue_with_two_taps_each(seats: &[u64]) -> OwnerQueue {
    let mut queue = quiet_queue(0);
    let mut number = 0;
    for seat in seats {
        queue.join_seat(SeatId::new(*seat), 0).expect("joins");
    }
    for _round in 0..2 {
        for seat in seats {
            number += 1;
            queue.admit(tap(number, *seat), 0).expect("admitted");
        }
    }
    queue
}

/// Catches the mutation that restarts the player cursor at the smallest seat when the seat it
/// stands on leaves.
#[test]
fn when_the_seat_served_last_leaves_the_next_seat_after_it_goes_next() {
    let mut queue = queue_with_two_taps_each(&[1, 2, 3]);
    for expected in [1, 2] {
        let served = serve_at(&mut queue, 0);
        assert_eq!(served.envelope.seat(), Some(SeatId::new(expected)));
        end(&mut queue, &served);
    }
    queue.leave_seat(SeatId::new(2), 0).expect("leaves");
    assert_eq!(serve_order(&mut queue), [3, 1, 3]);
}

#[test]
fn a_seat_joining_below_the_cursor_is_served_after_the_seats_above_it() {
    let mut queue = quiet_queue(0);
    for seat in [2, 3] {
        queue.join_seat(SeatId::new(seat), 0).expect("joins");
    }
    queue.admit(tap(1, 2), 0).expect("admitted");
    queue.admit(tap(2, 2), 0).expect("admitted");
    queue.admit(tap(3, 3), 0).expect("admitted");
    let first = serve_at(&mut queue, 0);
    assert_eq!(first.envelope.seat(), Some(SeatId::new(2)));
    end(&mut queue, &first);
    queue.join_seat(SeatId::new(1), 0).expect("joins");
    queue.admit(tap(4, 1), 0).expect("admitted");
    assert_eq!(serve_order(&mut queue), [3, 1, 2]);
}

#[test]
fn a_seat_leaving_with_input_waiting_loses_its_turn_and_nobody_else_does() {
    let mut queue = queue_with_two_taps_each(&[1, 2, 3]);
    let first = serve_at(&mut queue, 0);
    end(&mut queue, &first);
    let discarded = queue.leave_seat(SeatId::new(2), 0).expect("leaves");
    assert_eq!(discarded, [operation(2), operation(5)]);
    assert_eq!(serve_order(&mut queue), [3, 1, 3]);
}

#[test]
fn a_seat_with_an_id_below_a_cursor_that_wrapped_is_still_served_in_its_turn() {
    let mut queue = queue_with_two_taps_each(&[1, 2, 3]);
    for _ in 0..3 {
        let served = serve_at(&mut queue, 0);
        end(&mut queue, &served);
    }
    queue.leave_seat(SeatId::new(1), 0).expect("leaves");
    assert_eq!(serve_order(&mut queue), [2, 3]);
}

#[test]
fn leaving_and_joining_with_the_same_id_while_the_old_input_runs_starts_a_clean_seat() {
    let mut queue = quiet_queue(2);
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 2), 0).expect("admitted");
    let old = serve_at(&mut queue, 0);
    assert_eq!(old.envelope.seat(), Some(SeatId::new(1)));
    queue.leave_seat(SeatId::new(1), 0).expect("leaves");
    queue.join_seat(SeatId::new(1), 0).expect("joins again");
    queue.admit(tap(3, 1), 0).expect("admitted");
    queue.admit(tap(4, 1), 0).expect("admitted");
    queue
        .admit(tap(5, 1), 0)
        .expect("the old input does not count against the new seat");
    assert_eq!(
        queue.finish(
            old.envelope.sender(),
            old.envelope.operation_id(),
            OutcomeKind::Applied
        ),
        Err(FinishError::UnknownOperation)
    );
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(3));
    assert_eq!(serve_order(&mut queue), [2, 1, 1, 1]);
}

/// Catches: a `leave_seat` that keeps a seat's finished records under its seat id.
#[test]
fn a_seat_that_leaves_and_rejoins_can_resend_an_id_and_learns_nothing_of_the_first_membership() {
    let mut queue = quiet_queue(1);
    queue.admit(tap(1, 1), 0).expect("admitted");
    let _served = serve_at(&mut queue, 0);
    queue
        .finish(SEAT_ONE, operation(1), OutcomeKind::Declined)
        .expect("finishes");
    queue.leave_seat(SeatId::new(1), 0).expect("leaves");
    assert_eq!(
        queue.status(SEAT_ONE, operation(1)),
        None,
        "no seat, no history"
    );
    queue
        .join_seat(SeatId::new(1), 0)
        .expect("a new player takes the seat id");
    assert_eq!(queue.status(SEAT_ONE, operation(1)), None);
    assert_eq!(
        queue.admit(tap(1, 1), 0).map(|admitted| admitted.lane),
        Ok(Lane::Player),
        "the new membership's id 1 is a real input"
    );
    assert_eq!(
        queue.status(SEAT_ONE, operation(1)),
        Some(OperationStatus::Queued { lane: Lane::Player }),
        "it was not told the first membership's outcome"
    );
}

/// Catches: a `leave_seat` that keeps a seat's in-progress records, or abandons them silently.
#[test]
fn an_id_still_in_progress_when_its_seat_left_is_abandoned_and_free_for_the_next_membership() {
    let sink = Arc::new(RecordingSink::default());
    let mut queue = seated_queue(1, &settings(3, 9, 1), sink.clone());
    queue.admit(tap(1, 1), 0).expect("admitted");
    let _running = serve_at(&mut queue, 100);
    queue.leave_seat(SeatId::new(1), 350).expect("leaves");
    assert!(queue.in_progress(400).is_empty());
    queue.join_seat(SeatId::new(1), 400).expect("joins again");
    assert!(queue.admit(tap(1, 1), 400).is_ok());
    let abandoned: Vec<String> = sink
        .events()
        .iter()
        .map(|event| event.render_audit_line())
        .filter(|line| line.starts_with("INPUT abandoned"))
        .collect();
    assert_eq!(abandoned.len(), 1);
    assert!(
        abandoned[0].ends_with("cause=stale ms=250"),
        "{}",
        abandoned[0]
    );
}

#[test]
fn a_departed_seats_operations_never_reach_the_engine_or_another_seat() {
    let mut queue = quiet_queue(2);
    queue.admit(timer(1), 0).expect("admitted");
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(1, 2), 0).expect("admitted");
    queue.leave_seat(SeatId::new(1), 0).expect("leaves");
    assert_eq!(
        queue.status(Sender::Engine, operation(1)),
        Some(OperationStatus::Queued {
            lane: Lane::TimersAndResults
        })
    );
    assert_eq!(
        queue.status(Sender::Seat(SeatId::new(2)), operation(1)),
        Some(OperationStatus::Queued { lane: Lane::Player })
    );
}

/// Catches: a `reap_expired` that stops at the first entry younger than the lease.
#[test]
fn a_clock_that_stepped_back_cannot_hide_an_expired_input_behind_a_younger_one() {
    let limits = QueueLimits {
        in_progress_lease_ms: 3_600_000,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(TableId::new(1), &settings(3, 9, 1), Arc::new(NullSink))
        .expect("builds")
        .with_limits(limits);
    queue.join_seat(SeatId::new(1), 0).expect("joins");
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 1), 0).expect("admitted");
    let _later = serve_at(&mut queue, 10_800_000);
    let _earlier = serve_at(&mut queue, 1_000);
    let reaped = queue.reap_expired(3_601_000);
    assert_eq!(reaped.len(), 1, "the second input has run the whole lease");
    assert_eq!(reaped[0].operation_id, operation(2));
    assert_eq!(reaped[0].turn_seq.get(), 2);
    assert_eq!(queue.in_progress(3_601_000).len(), 1);
}

#[test]
fn with_every_lane_loaded_and_a_seat_flooding_the_player_lane_the_order_holds() {
    let mut queue = quiet_queue(3);
    let mut number = 0;
    let mut next = || {
        number += 1;
        number
    };
    let mut flood_ids = Vec::new();
    for _ in 0..3 {
        let id = next();
        flood_ids.push(id);
        queue.admit(tap(id, 1), 0).expect("admitted");
    }
    let quiet = next();
    queue.admit(tap(quiet, 2), 0).expect("admitted");
    let engine = next();
    queue.admit(proposal(engine), 0).expect("admitted");
    let timer_id = next();
    queue.admit(timer(timer_id), 0).expect("admitted");
    let pause = next();
    queue.admit(safety_card(pause, 3), 0).expect("admitted");
    let mut order = Vec::new();
    while let Some(served) = queue.serve_next(0).expect("serves") {
        order.push(served.envelope.operation_id());
        end(&mut queue, &served);
    }
    let expected = [
        pause,
        engine,
        timer_id,
        flood_ids[0],
        quiet,
        flood_ids[1],
        flood_ids[2],
    ]
    .map(operation);
    assert_eq!(order, expected);
}

#[test]
fn a_duplicate_at_exactly_the_remembered_limit_still_finds_its_result_and_one_more_forgets_it() {
    let limits = QueueLimits {
        remembered_operations: 2,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(TableId::new(1), &settings(9, 9, 1), Arc::new(NullSink))
        .expect("builds")
        .with_limits(limits);
    queue.join_seat(SeatId::new(1), 0).expect("joins");
    for number in 1..=2 {
        queue.admit(tap(number, 1), 0).expect("admitted");
        let served = serve_at(&mut queue, 0);
        end(&mut queue, &served);
    }
    assert!(matches!(
        queue.admit(tap(1, 1), 0),
        Err(AdmitError::Duplicate {
            first: OperationStatus::Finished { .. }
        })
    ));
    queue.admit(tap(3, 1), 0).expect("admitted");
    let served = serve_at(&mut queue, 0);
    assert!(
        matches!(
            queue.admit(tap(3, 1), 0),
            Err(AdmitError::Duplicate {
                first: OperationStatus::InProgress { .. }
            })
        ),
        "an input in progress is never forgotten"
    );
    end(&mut queue, &served);
    assert!(
        queue.admit(tap(1, 1), 0).is_ok(),
        "the oldest ended operation was forgotten"
    );
}

#[test]
fn a_released_input_forgets_its_operation_id_and_frees_its_seat() {
    let mut queue = quiet_queue(1);
    queue.admit(tap(7, 1), 0).expect("admitted");
    let served = serve_at(&mut queue, 10);
    let (sender, operation_id) = (served.envelope.sender(), served.envelope.operation_id());

    queue
        .release(sender, operation_id, FailureCause::Unavailable, 30)
        .expect("releases");

    assert_eq!(queue.status(sender, operation_id), None);
    assert_eq!(queue.seat_pending(SeatId::new(1)), Some(0));
    assert_eq!(
        queue.finish(sender, operation_id, OutcomeKind::Applied),
        Err(FinishError::UnknownOperation)
    );
    queue
        .admit(tap(7, 1), 40)
        .expect("the same id is a new operation after a release");
    let again = serve_at(&mut queue, 50);
    assert_eq!(again.envelope.operation_id(), operation_id);
    end(&mut queue, &again);
    assert!(matches!(
        queue.status(sender, operation_id),
        Some(OperationStatus::Finished { .. })
    ));
}

#[test]
fn releasing_an_input_that_is_not_in_progress_is_refused() {
    let mut queue = quiet_queue(1);
    let waiting = tap(3, 1);
    let (sender, operation_id) = (waiting.sender(), waiting.operation_id());
    queue.admit(waiting, 0).expect("admitted");

    assert!(matches!(
        queue.release(sender, operation_id, FailureCause::Unavailable, 1),
        Err(FinishError::NotInProgress { .. })
    ));
}
