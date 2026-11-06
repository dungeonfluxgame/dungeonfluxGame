//! Seat controls, per-sender operation ids and the one free-text line in INTERPRET (C.1, H.2, F.7,
//! K.6): the rules that decide who may be refused and who may overtake whom.

use std::sync::Arc;

use crate::envelope::Sender;
use crate::identity::SeatId;
use crate::limits::{MAXIMUM_CONTROLS_LANE_LENGTH, MAXIMUM_SEAT_CONTROLS_PENDING};
use crate::outcome::OutcomeKind;
use crate::owner_queue::{
    AdmitError, FinishError, Lane, OperationStatus, OwnerQueue, QueueLimits, ServedInput,
};
use crate::owner_queue_tests::{
    NullSink, operation, pending_answer, proposal, safety_card, seated_queue, settings, tap,
    text_line, timer,
};
use crate::testkit::RecordingSink;

fn quiet_queue(seats: u64, pending: u32, burst: u32) -> OwnerQueue {
    seated_queue(seats, &settings(pending, burst, 1), Arc::new(NullSink))
}

fn seat(number: u64) -> Sender {
    Sender::Seat(SeatId::new(number))
}

fn serve(queue: &mut OwnerQueue) -> Option<ServedInput> {
    queue.serve_next(0).expect("serves")
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

fn served_seats(queue: &mut OwnerQueue) -> Vec<u64> {
    let mut seats = Vec::new();
    while let Some(served) = serve(queue) {
        seats.push(served.envelope.seat().expect("a seat input").get());
        end(queue, &served);
    }
    seats
}

#[test]
fn a_seat_at_its_pending_bound_can_still_pause_and_answer() {
    let mut queue = quiet_queue(1, 3, 9);
    let mut running = Vec::new();
    for number in 1..=3 {
        queue.admit(tap(number, 1), 0).expect("admitted");
        running.push(serve(&mut queue).expect("one waits"));
    }
    assert!(matches!(
        queue.admit(tap(4, 1), 0),
        Err(AdmitError::SeatBusy { .. })
    ));
    assert_eq!(
        queue.admit(safety_card(5, 1), 0).map(|a| a.lane),
        Ok(Lane::SeatControls)
    );
    assert_eq!(
        queue.admit(pending_answer(6, 1), 0).map(|a| a.lane),
        Ok(Lane::SeatControls)
    );
    assert_eq!(
        queue.seat_pending(SeatId::new(1)),
        Some(3),
        "controls are not counted"
    );
}

#[test]
fn controls_are_never_rate_limited() {
    let mut queue = quiet_queue(1, 3, 1);
    queue.admit(tap(1, 1), 0).expect("admitted");
    assert!(matches!(
        queue.admit(tap(2, 1), 0),
        Err(AdmitError::RateLimited { .. })
    ));
    for number in 10..60 {
        assert!(
            queue.admit(safety_card(number, 1), 0).is_ok(),
            "card {number}"
        );
    }
    assert_eq!(queue.depth(Lane::SeatControls), 50);
}

#[test]
fn a_pause_behind_queued_player_input_and_engine_input_is_served_first() {
    let mut queue = quiet_queue(2, 9, 9);
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 2), 0).expect("admitted");
    queue.admit(timer(3), 0).expect("admitted");
    queue.admit(proposal(4), 0).expect("admitted");
    queue.admit(safety_card(5, 2), 0).expect("admitted");
    let first = serve(&mut queue).expect("input waits");
    assert_eq!(
        (first.lane, first.envelope.operation_id()),
        (Lane::SeatControls, operation(5))
    );
    assert_eq!(first.turn_seq.get(), 1);
}

#[test]
fn the_answer_to_a_pending_overtakes_the_player_lane_too() {
    let mut queue = quiet_queue(1, 9, 9);
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(pending_answer(2, 1), 0).expect("admitted");
    let first = serve(&mut queue).expect("input waits");
    assert_eq!(first.envelope.operation_id(), operation(2));
}

#[test]
fn a_seat_flooding_controls_cannot_hide_another_seats_pause() {
    let mut queue = quiet_queue(3, 9, 9);
    for number in 1..=100 {
        queue.admit(safety_card(number, 1), 0).expect("admitted");
    }
    queue.admit(safety_card(1_000, 3), 0).expect("admitted");
    let order = served_seats(&mut queue);
    let place = order
        .iter()
        .position(|seat| *seat == 3)
        .expect("seat 3 was served");
    assert!(place <= 1, "the quiet seat waited behind {place} controls");
}

#[test]
fn a_flood_of_controls_from_one_seat_is_bounded_by_a_named_limit_for_that_seat_only() {
    assert_eq!(
        QueueLimits::default().seat_controls_pending,
        MAXIMUM_SEAT_CONTROLS_PENDING
    );
    let limits = QueueLimits {
        seat_controls_pending: 5,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(
        crate::identity::TableId::new(1),
        &settings(3, 3, 1),
        Arc::new(NullSink),
    )
    .expect("builds")
    .with_limits(limits);
    queue.join_seat(SeatId::new(1), 0).expect("joins");
    queue.join_seat(SeatId::new(2), 0).expect("joins");
    for number in 1..=5 {
        queue.admit(safety_card(number, 1), 0).expect("admitted");
    }
    assert_eq!(
        queue.admit(safety_card(6, 1), 0),
        Err(AdmitError::LaneStalled {
            lane: Lane::SeatControls,
            limit: 5
        })
    );
    assert!(queue.admit(safety_card(7, 2), 0).is_ok());
    let served = serve(&mut queue).expect("input waits");
    end(&mut queue, &served);
    assert!(
        queue.admit(safety_card(8, 1), 0).is_ok(),
        "serving made room"
    );
}

#[test]
fn controls_of_a_seat_that_leaves_are_returned_with_its_waiting_lines() {
    let mut queue = quiet_queue(1, 9, 9);
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(safety_card(2, 1), 0).expect("admitted");
    let discarded = queue.leave_seat(SeatId::new(1), 0).expect("leaves");
    assert_eq!(discarded.len(), 2);
    assert!(discarded.contains(&operation(1)) && discarded.contains(&operation(2)));
    assert_eq!(queue.depth(Lane::SeatControls), 0);
    assert_eq!(queue.depth(Lane::Player), 0);
    assert_eq!(serve(&mut queue).map(|served| served.turn_seq), None);
}

#[test]
fn another_seat_reusing_an_operation_id_is_admitted_and_never_told_the_first_seats_outcome() {
    let mut queue = quiet_queue(2, 9, 9);
    queue.admit(tap(1, 1), 0).expect("admitted");
    let served = serve(&mut queue).expect("one waits");
    queue
        .finish(seat(1), operation(1), OutcomeKind::Declined)
        .expect("finishes");
    assert_eq!(served.lane, Lane::Player);
    let admitted = queue.admit(tap(1, 2), 0);
    assert_eq!(
        admitted.map(|a| a.lane),
        Ok(Lane::Player),
        "seat 2's own input"
    );
    assert_eq!(
        queue.status(seat(2), operation(1)),
        Some(OperationStatus::Queued { lane: Lane::Player })
    );
    assert_eq!(
        queue.status(seat(1), operation(1)),
        Some(OperationStatus::Finished {
            turn_seq: served.turn_seq,
            outcome: OutcomeKind::Declined
        })
    );
    assert_eq!(
        queue.admit(tap(1, 2), 0),
        Err(AdmitError::Duplicate {
            first: OperationStatus::Queued { lane: Lane::Player }
        }),
        "seat 2's retry hears about seat 2's input"
    );
}

#[test]
fn an_engine_input_and_a_seat_input_may_share_an_operation_id() {
    let mut queue = quiet_queue(1, 9, 9);
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue
        .admit(timer(1), 0)
        .expect("the engine numbers its own ids");
    assert_eq!(
        queue.status(Sender::Engine, operation(1)),
        Some(OperationStatus::Queued {
            lane: Lane::TimersAndResults
        })
    );
    let engine = serve(&mut queue).expect("input waits");
    assert_eq!(engine.envelope.sender(), Sender::Engine);
    queue
        .finish(Sender::Engine, operation(1), OutcomeKind::Applied)
        .expect("finishes");
    assert_eq!(
        queue.status(seat(1), operation(1)),
        Some(OperationStatus::Queued { lane: Lane::Player }),
        "the seat's input is untouched"
    );
}

#[test]
fn ending_an_operation_needs_the_sender_that_owns_it() {
    let mut queue = quiet_queue(2, 9, 9);
    queue.admit(tap(1, 1), 0).expect("admitted");
    let served = serve(&mut queue).expect("one waits");
    assert_eq!(
        queue.finish(seat(2), operation(1), OutcomeKind::Applied),
        Err(FinishError::UnknownOperation)
    );
    assert_eq!(
        queue.finish(Sender::Engine, operation(1), OutcomeKind::Applied),
        Err(FinishError::UnknownOperation)
    );
    end(&mut queue, &served);
}

#[test]
fn a_second_free_text_line_waits_until_the_first_ends_and_keeps_its_place_in_the_order() {
    let mut queue = quiet_queue(1, 3, 9);
    for number in 1..=3 {
        queue.admit(text_line(number, 1), 0).expect("admitted");
    }
    let first = serve(&mut queue).expect("the first line is served");
    assert_eq!(first.envelope.operation_id(), operation(1));
    assert!(serve(&mut queue).is_none(), "the second line is held");
    assert_eq!(
        queue.seat_pending(SeatId::new(1)),
        Some(3),
        "held lines still count"
    );
    assert_eq!(queue.depth(Lane::Player), 2);
    end(&mut queue, &first);
    let second = serve(&mut queue).expect("the second line is served now");
    assert_eq!(second.envelope.operation_id(), operation(2));
    assert!(serve(&mut queue).is_none());
    end(&mut queue, &second);
    let third = serve(&mut queue).expect("the third line");
    assert_eq!(third.envelope.operation_id(), operation(3));
    end(&mut queue, &third);
}

#[test]
fn a_held_seat_does_not_hold_up_other_seats_and_an_all_held_queue_serves_nothing() {
    let mut queue = quiet_queue(2, 3, 9);
    queue.admit(text_line(1, 1), 0).expect("admitted");
    queue.admit(text_line(2, 1), 0).expect("admitted");
    queue.admit(tap(3, 2), 0).expect("admitted");
    let first = serve(&mut queue).expect("seat 1's first line");
    let other = serve(&mut queue).expect("seat 2 is not held");
    assert_eq!(other.envelope.seat(), Some(SeatId::new(2)));
    assert!(
        serve(&mut queue).is_none(),
        "only seat 1's second line waits, and it is held"
    );
    assert_eq!(queue.serve_next(0), Ok(None));
    end(&mut queue, &first);
    end(&mut queue, &other);
    assert!(serve(&mut queue).is_some());
}

#[test]
fn a_tap_behind_a_held_line_waits_for_it_so_the_seats_order_holds() {
    let mut queue = quiet_queue(1, 3, 9);
    queue.admit(text_line(1, 1), 0).expect("admitted");
    queue.admit(text_line(2, 1), 0).expect("admitted");
    queue.admit(tap(3, 1), 0).expect("admitted");
    let first = serve(&mut queue).expect("the first line");
    assert!(
        serve(&mut queue).is_none(),
        "the tap does not jump the held line"
    );
    end(&mut queue, &first);
    let order: Vec<_> = std::iter::from_fn(|| serve(&mut queue))
        .map(|served| served.envelope.operation_id())
        .collect();
    assert_eq!(order.first(), Some(&operation(2)));
}

#[test]
fn taps_and_private_messages_are_not_free_text_lines_and_a_spoken_line_is() {
    let mut queue = quiet_queue(1, 5, 9);
    queue.admit(tap(1, 1), 0).expect("admitted");
    queue.admit(tap(2, 1), 0).expect("admitted");
    let first = serve(&mut queue).expect("a tap");
    let second = serve(&mut queue).expect("taps run side by side");
    end(&mut queue, &first);
    end(&mut queue, &second);
    let say = crate::envelope::InputEnvelope::from_player(
        operation(3),
        SeatId::new(1),
        crate::owner_queue_tests::view(),
        crate::envelope::PlayerInput::Say {
            utterance: crate::envelope::UtteranceId::new(1),
            text: crate::private_data::PlayerText::new("Halt!"),
        },
    );
    queue.admit(say, 0).expect("admitted");
    queue.admit(text_line(4, 1), 0).expect("admitted");
    let spoken = serve(&mut queue).expect("the spoken line");
    assert!(
        serve(&mut queue).is_none(),
        "a spoken line is a free-text line too"
    );
    end(&mut queue, &spoken);
    assert!(serve(&mut queue).is_some());
}

#[test]
fn pause_cards_are_not_held_behind_a_free_text_line_in_interpret() {
    let mut queue = quiet_queue(1, 3, 9);
    queue.admit(text_line(1, 1), 0).expect("admitted");
    let _interpreting = serve(&mut queue).expect("the line");
    queue.admit(safety_card(2, 1), 0).expect("admitted");
    let card = serve(&mut queue).expect("the card is served at once");
    assert_eq!(card.lane, Lane::SeatControls);
}

#[test]
fn the_audit_trail_of_a_pause_shows_the_controls_lane() {
    let sink = Arc::new(RecordingSink::default());
    let mut queue = seated_queue(1, &settings(3, 3, 1), sink.clone());
    queue.admit(safety_card(1, 1), 0).expect("admitted");
    let served = serve(&mut queue).expect("served");
    end(&mut queue, &served);
    let lines: Vec<String> = sink
        .events()
        .iter()
        .map(|event| event.render_audit_line())
        .collect();
    assert!(lines[1].starts_with("INPUT served "), "{}", lines[1]);
    assert!(lines[1].contains("lane=seat_controls"), "{}", lines[1]);
}

fn queue_with_controls_bound(seats: u64, table_wide: usize) -> OwnerQueue {
    let limits = QueueLimits {
        controls_lane_length: table_wide,
        ..QueueLimits::default()
    };
    let mut queue = OwnerQueue::new(
        crate::identity::TableId::new(1),
        &settings(3, 3, 1),
        Arc::new(NullSink),
    )
    .expect("builds")
    .with_limits(limits);
    for seat in 1..=seats {
        queue.join_seat(SeatId::new(seat), 0).expect("joins");
    }
    queue
}

/// Catches the mutation that drops the table-wide check: the per-seat bound alone lets every seat
/// queue a million controls.
#[test]
fn the_table_wide_count_of_waiting_controls_never_passes_its_named_bound() {
    assert_eq!(
        QueueLimits::default().controls_lane_length,
        MAXIMUM_CONTROLS_LANE_LENGTH
    );
    let mut queue = queue_with_controls_bound(3, 5);
    let mut admitted = 0;
    for number in 1..=30 {
        let seat = 1 + number % 3;
        match queue.admit(safety_card(number, seat), 0) {
            Ok(_) => admitted += 1,
            Err(refusal) => assert_eq!(
                refusal,
                AdmitError::LaneStalled {
                    lane: Lane::SeatControls,
                    limit: 5
                }
            ),
        }
        assert!(queue.depth(Lane::SeatControls) <= 5);
    }
    assert_eq!(admitted, 5);
    assert!(
        queue.admit(tap(100, 1), 0).is_ok(),
        "other lanes are not full"
    );
    assert!(queue.admit(timer(101), 0).is_ok());
    let served = serve(&mut queue).expect("a control waits");
    end(&mut queue, &served);
    assert!(
        queue.admit(safety_card(102, 2), 0).is_ok(),
        "serving made room"
    );
}

/// Catches the mutation that lets a seat's controls pass the table-wide bound by a second route
/// (for example a pending answer): both kinds share the one count.
#[test]
fn pause_cards_and_pending_answers_share_the_one_table_wide_count() {
    let mut queue = queue_with_controls_bound(2, 4);
    for number in 1..=2 {
        queue.admit(safety_card(number, 1), 0).expect("admitted");
        queue
            .admit(pending_answer(10 + number, 2), 0)
            .expect("admitted");
    }
    assert!(matches!(
        queue.admit(safety_card(20, 1), 0),
        Err(AdmitError::LaneStalled { .. })
    ));
    assert!(matches!(
        queue.admit(pending_answer(21, 2), 0),
        Err(AdmitError::LaneStalled { .. })
    ));
}

/// Catches the mutation that serves controls without limit while other lanes wait: a flood delays
/// a timer by at most the table-wide bound.
#[test]
fn a_flood_of_controls_delays_a_timer_by_no_more_than_the_table_wide_bound() {
    let mut queue = queue_with_controls_bound(1, 7);
    for number in 1..=100 {
        queue.admit(safety_card(number, 1), 0).ok();
    }
    queue.admit(timer(500), 0).expect("admitted");
    queue.admit(tap(501, 1), 0).expect("admitted");
    let mut order = Vec::new();
    while let Some(served) = serve(&mut queue) {
        order.push(served.envelope.operation_id());
        end(&mut queue, &served);
    }
    let timer_at = order
        .iter()
        .position(|id| *id == operation(500))
        .expect("served");
    assert_eq!(
        timer_at, 7,
        "behind the seven controls the bound allowed, no more"
    );
    assert_eq!(order.last(), Some(&operation(501)));
}

fn queue_with_two_cards_each(seats: &[u64]) -> OwnerQueue {
    let mut queue = quiet_queue(0, 3, 3);
    let mut number = 0;
    for seat in seats {
        queue.join_seat(SeatId::new(*seat), 0).expect("joins");
    }
    for _round in 0..2 {
        for seat in seats {
            number += 1;
            queue
                .admit(safety_card(number, *seat), 0)
                .expect("admitted");
        }
    }
    queue
}

/// Catches the mutation that restarts the controls cursor at the smallest seat when the seat it
/// stands on leaves: the next control must come from the seat after the one that left.
#[test]
fn when_the_control_seat_served_last_leaves_the_next_seat_after_it_goes_next() {
    let mut queue = queue_with_two_cards_each(&[1, 2, 3]);
    for expected in [1, 2] {
        let served = serve(&mut queue).expect("a control");
        assert_eq!(served.envelope.seat(), Some(SeatId::new(expected)));
        end(&mut queue, &served);
    }
    queue.leave_seat(SeatId::new(2), 0).expect("leaves");
    assert_eq!(served_seats(&mut queue), [3, 1, 3]);
}

/// Catches the mutation that puts a joining seat's controls at the front instead of in cyclic
/// order after the cursor.
#[test]
fn a_seat_joining_below_the_controls_cursor_is_served_after_the_seats_above_it() {
    let mut queue = quiet_queue(0, 3, 3);
    for seat in [2, 3] {
        queue.join_seat(SeatId::new(seat), 0).expect("joins");
    }
    queue.admit(safety_card(1, 2), 0).expect("admitted");
    queue.admit(safety_card(2, 2), 0).expect("admitted");
    queue.admit(safety_card(3, 3), 0).expect("admitted");
    let first = serve(&mut queue).expect("a control");
    assert_eq!(first.envelope.seat(), Some(SeatId::new(2)));
    end(&mut queue, &first);
    queue.join_seat(SeatId::new(1), 0).expect("joins");
    queue.admit(safety_card(4, 1), 0).expect("admitted");
    assert_eq!(served_seats(&mut queue), [3, 1, 2]);
}

/// Catches the mutation where a seat that leaves with controls waiting stays in the ring and its
/// turn is spent on nothing.
#[test]
fn a_seat_leaving_with_controls_waiting_returns_them_and_loses_its_turn_only() {
    let mut queue = queue_with_two_cards_each(&[1, 2, 3]);
    let first = serve(&mut queue).expect("a control");
    end(&mut queue, &first);
    let discarded = queue.leave_seat(SeatId::new(2), 0).expect("leaves");
    assert_eq!(discarded, [operation(2), operation(5)]);
    assert_eq!(queue.depth(Lane::SeatControls), 3);
    assert_eq!(served_seats(&mut queue), [3, 1, 3]);
}
