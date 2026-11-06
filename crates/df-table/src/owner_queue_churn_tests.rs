//! A plain model of the owner queue under churn: seats join and leave, inputs of every seat kind
//! arrive (taps, free text, pauses, answers), and served inputs finish, are abandoned or are reaped,
//! all at the default settings. The model is written the long way round (per-seat lists, a cursor
//! per ring, a scan for the next seat) so the queue's rings, holds and counts are checked against
//! something that shares none of their structure.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use crate::config::Settings;
use crate::envelope::Sender;
use crate::event::FailureCause;
use crate::identity::{OperationId, SeatId, TableId};
use crate::outcome::OutcomeKind;
use crate::owner_queue::{AdmitError, Lane, OwnerQueue, QueueLimits, ServedInput};
use crate::owner_queue_property_tests::XorShift;
use crate::owner_queue_tests::{
    NullSink, operation, pending_answer, safety_card, settings, tap, text_line,
};

const SEEDS: u64 = 300;
const PENDING_LIMIT: usize = 3;
const BURST_MILLITOKENS: u128 = 3_000;
const LEASE_MS: u64 = 2_000;

struct ModelSeat {
    membership: u64,
    lines: VecDeque<(u64, bool)>,
    controls: VecDeque<u64>,
    lines_in_flight: usize,
    text_in_flight: usize,
    held_millitokens: u128,
}

impl ModelSeat {
    fn pending(&self) -> usize {
        self.lines.len() + self.lines_in_flight
    }

    fn line_eligible(&self) -> bool {
        self.lines
            .front()
            .is_some_and(|(_, text)| !(*text && self.text_in_flight > 0))
    }

    fn control_eligible(&self) -> bool {
        !self.controls.is_empty()
    }
}

struct Flight {
    served: ServedInput,
    served_ms: u64,
    membership: u64,
    text: bool,
}

/// The seat after the cursor in seat order that qualifies, wrapping to the smallest.
fn next_seat(
    model: &BTreeMap<u64, ModelSeat>,
    cursor: Option<u64>,
    qualifies: fn(&ModelSeat) -> bool,
) -> Option<u64> {
    cursor
        .and_then(|last| {
            model
                .range(last + 1..)
                .find(|(_, seat)| qualifies(seat))
                .map(|(number, _)| *number)
        })
        .or_else(|| {
            model
                .iter()
                .find(|(_, seat)| qualifies(seat))
                .map(|(number, _)| *number)
        })
}

fn seat_number(served: &ServedInput) -> u64 {
    served.envelope.seat().expect("a seat input").get()
}

fn number_of(operation_id: OperationId) -> u64 {
    (u128::from_be_bytes(*operation_id.as_bytes()) - 1) as u64
}

/// Gives a modelled seat back what an ended line held, if the seat is still the same membership.
fn release(model: &mut BTreeMap<u64, ModelSeat>, flight: &Flight) {
    if flight.served.lane != Lane::Player {
        return;
    }
    if let Some(seat) = model.get_mut(&seat_number(&flight.served))
        && seat.membership == flight.membership
    {
        seat.lines_in_flight -= 1;
        if flight.text {
            seat.text_in_flight -= 1;
        }
    }
}

/// Catches: a seat left in a ring after it leaves or after its hold is released; a control served
/// after a line; a free-text line served while another is in INTERPRET; an abandon, reap or leave that
/// frees the wrong seat count; a leave that keeps the seat's in-progress records.
#[test]
fn with_seats_joining_leaving_and_every_kind_of_input_the_queue_matches_a_plain_model() {
    for seed in 0..SEEDS {
        let mut random = XorShift::seeded(seed);
        let limits = QueueLimits {
            in_progress_lease_ms: LEASE_MS,
            ..QueueLimits::default()
        };
        let mut queue = OwnerQueue::new(TableId::new(1), &Settings::default(), Arc::new(NullSink))
            .expect("builds")
            .with_limits(limits);
        let mut model: BTreeMap<u64, ModelSeat> = BTreeMap::new();
        let mut line_cursor: Option<u64> = None;
        let mut control_cursor: Option<u64> = None;
        let mut memberships = 0;
        let mut flights: Vec<Flight> = Vec::new();
        let mut next_operation = 0;
        let mut now_ms = 0_u64;
        for _ in 0..600 {
            let before_ms = now_ms;
            now_ms += random.below(1_400);
            let elapsed = u128::from(now_ms - before_ms);
            for seat in model.values_mut() {
                seat.held_millitokens = (seat.held_millitokens + elapsed).min(BURST_MILLITOKENS);
            }
            let seat = 1 + random.below(6);
            match random.below(14) {
                0 => {
                    let joined = queue.join_seat(SeatId::new(seat), now_ms);
                    match model.entry(seat) {
                        std::collections::btree_map::Entry::Occupied(_) => {
                            assert!(joined.is_err(), "seed {seed}");
                        }
                        std::collections::btree_map::Entry::Vacant(vacant) => {
                            joined.expect("joins");
                            memberships += 1;
                            vacant.insert(ModelSeat {
                                membership: memberships,
                                lines: VecDeque::new(),
                                controls: VecDeque::new(),
                                lines_in_flight: 0,
                                text_in_flight: 0,
                                held_millitokens: BURST_MILLITOKENS,
                            });
                        }
                    }
                }
                1 => {
                    let left = queue.leave_seat(SeatId::new(seat), now_ms);
                    match model.remove(&seat) {
                        Some(gone) => {
                            let mut expected: Vec<u64> =
                                gone.lines.iter().map(|(number, _)| *number).collect();
                            expected.extend(gone.controls.iter());
                            let expected: Vec<OperationId> =
                                expected.into_iter().map(operation).collect();
                            assert_eq!(left, Ok(expected), "seed {seed}");
                            flights.retain(|flight| seat_number(&flight.served) != seat);
                        }
                        None => assert!(left.is_err(), "seed {seed}"),
                    }
                }
                2..=7 => {
                    next_operation += 1;
                    let kind = random.below(4);
                    let envelope = match kind {
                        0 => tap(next_operation, seat),
                        1 => text_line(next_operation, seat),
                        2 => safety_card(next_operation, seat),
                        _ => pending_answer(next_operation, seat),
                    };
                    let result = queue.admit(envelope, now_ms);
                    let Some(held) = model.get_mut(&seat) else {
                        let expected = AdmitError::UnknownSeat {
                            seat: SeatId::new(seat),
                        };
                        assert_eq!(result, Err(expected), "seed {seed}");
                        continue;
                    };
                    if kind >= 2 {
                        assert!(result.is_ok(), "seed {seed}: a control was refused");
                        held.controls.push_back(next_operation);
                    } else if held.pending() >= PENDING_LIMIT {
                        let expected = AdmitError::SeatBusy {
                            pending: held.pending(),
                            limit: PENDING_LIMIT,
                        };
                        assert_eq!(result, Err(expected), "seed {seed}");
                    } else if held.held_millitokens < 1_000 {
                        let expected = AdmitError::RateLimited {
                            retry_after_ms: (1_000 - held.held_millitokens) as u64,
                        };
                        assert_eq!(result, Err(expected), "seed {seed}");
                    } else {
                        assert!(result.is_ok(), "seed {seed}: {result:?}");
                        held.held_millitokens -= 1_000;
                        held.lines.push_back((next_operation, kind == 1));
                    }
                }
                8..=10 => {
                    let expected = next_seat(&model, control_cursor, ModelSeat::control_eligible)
                        .map(|seat| (seat, Lane::SeatControls))
                        .or_else(|| {
                            next_seat(&model, line_cursor, ModelSeat::line_eligible)
                                .map(|seat| (seat, Lane::Player))
                        });
                    let served = queue.serve_next(now_ms).expect("serves");
                    let Some(served) = served else {
                        assert_eq!(
                            expected, None,
                            "seed {seed}: nothing served, model had input"
                        );
                        continue;
                    };
                    assert_eq!(
                        expected,
                        Some((seat_number(&served), served.lane)),
                        "seed {seed}"
                    );
                    let number = number_of(served.envelope.operation_id());
                    let held = model
                        .get_mut(&seat_number(&served))
                        .expect("a modelled seat");
                    let mut text = false;
                    if served.lane == Lane::SeatControls {
                        assert_eq!(held.controls.pop_front(), Some(number), "seed {seed}");
                        control_cursor = Some(seat_number(&served));
                    } else {
                        let (front, front_text) = held.lines.pop_front().expect("a line");
                        assert_eq!(front, number, "seed {seed}");
                        held.lines_in_flight += 1;
                        text = front_text;
                        if text {
                            held.text_in_flight += 1;
                        }
                        line_cursor = Some(seat_number(&served));
                    }
                    let membership = held.membership;
                    flights.push(Flight {
                        served,
                        served_ms: now_ms,
                        membership,
                        text,
                    });
                }
                11 => {
                    if !flights.is_empty() {
                        let flight =
                            flights.swap_remove(random.below(flights.len() as u64) as usize);
                        queue
                            .finish(
                                flight.served.envelope.sender(),
                                flight.served.envelope.operation_id(),
                                OutcomeKind::Applied,
                            )
                            .expect("finishes");
                        release(&mut model, &flight);
                    }
                }
                12 => {
                    if !flights.is_empty() {
                        let flight =
                            flights.swap_remove(random.below(flights.len() as u64) as usize);
                        queue
                            .abandon(
                                flight.served.envelope.sender(),
                                flight.served.envelope.operation_id(),
                                FailureCause::TimedOut,
                                now_ms,
                            )
                            .expect("abandons");
                        release(&mut model, &flight);
                    }
                }
                _ => {
                    let mut expired: Vec<&Flight> = flights
                        .iter()
                        .filter(|flight| now_ms - flight.served_ms >= LEASE_MS)
                        .collect();
                    expired.sort_by_key(|flight| flight.served.turn_seq);
                    let expected: Vec<OperationId> = expired
                        .iter()
                        .map(|flight| flight.served.envelope.operation_id())
                        .collect();
                    let reaped: Vec<OperationId> = queue
                        .reap_expired(now_ms)
                        .iter()
                        .map(|report| report.operation_id)
                        .collect();
                    assert_eq!(reaped, expected, "seed {seed}");
                    let (gone, kept): (Vec<Flight>, Vec<Flight>) = flights
                        .into_iter()
                        .partition(|flight| now_ms - flight.served_ms >= LEASE_MS);
                    for flight in &gone {
                        release(&mut model, flight);
                    }
                    flights = kept;
                }
            }
            for (number, held) in &model {
                assert_eq!(
                    queue.seat_pending(SeatId::new(*number)),
                    Some(held.pending()),
                    "seed {seed}"
                );
            }
            let lines: usize = model.values().map(|held| held.lines.len()).sum();
            let controls: usize = model.values().map(|held| held.controls.len()).sum();
            assert_eq!(queue.depth(Lane::Player), lines, "seed {seed}");
            assert_eq!(queue.depth(Lane::SeatControls), controls, "seed {seed}");
            assert_eq!(
                queue.in_progress(now_ms).len(),
                flights.len(),
                "seed {seed}"
            );
        }
    }
}

/// A seat that has a free-text line in INTERPRET and whose next waiting line is also free text.
fn text_in_flight_then_waiting_text(queue: &mut OwnerQueue, seat: u64, first: u64) {
    queue.admit(text_line(first, seat), 0).expect("admitted");
    let running = queue
        .serve_next(0)
        .expect("serves")
        .expect("the line waits");
    assert_eq!(running.envelope.operation_id(), operation(first));
    queue
        .admit(text_line(first + 1, seat), 0)
        .expect("admitted");
}

/// Catches: a position that counts a held seat wrongly (it may overstate, never understate), a
/// missing `held_behind_own_text`, or a position that ignores seats that joined and left.
#[test]
fn a_waiting_inputs_position_is_exact_without_held_seats_and_never_understates_with_them() {
    for seed in 0..SEEDS {
        let mut random = XorShift::seeded(seed);
        let seats = 2 + random.below(5);
        let mut queue = OwnerQueue::new(
            TableId::new(1),
            &settings(1_000_000, 1_000_000, 1),
            Arc::new(NullSink),
        )
        .expect("builds");
        for seat in 1..=seats {
            queue.join_seat(SeatId::new(seat), 0).expect("joins");
        }
        let mut next_operation = 0;
        let mut held_heads: BTreeMap<u64, u64> = BTreeMap::new();
        let mut any_text = false;
        for seat in 1..=seats {
            if random.chance(40) {
                next_operation += 2;
                text_in_flight_then_waiting_text(&mut queue, seat, next_operation - 1);
                held_heads.insert(seat, next_operation);
                any_text = true;
            }
        }
        for _ in 0..random.below(6) {
            let seat = 1 + random.below(seats);
            if random.chance(50) {
                queue.leave_seat(SeatId::new(seat), 0).ok();
                held_heads.remove(&seat);
                if queue.join_seat(SeatId::new(seat), 0).is_err() {
                    continue;
                }
            }
        }
        let mut waiting: Vec<(Sender, OperationId)> = Vec::new();
        for _ in 0..(3 + random.below(40)) {
            next_operation += 1;
            let seat = 1 + random.below(seats);
            let (envelope, text) = match random.below(6) {
                0 => (text_line(next_operation, seat), true),
                1 => (pending_answer(next_operation, seat), false),
                _ => (tap(next_operation, seat), false),
            };
            let key = (envelope.sender(), envelope.operation_id());
            if queue.admit(envelope, 0).is_ok() {
                waiting.push(key);
                any_text |= text;
            }
        }
        let promised: Vec<((Sender, OperationId), usize, bool)> = waiting
            .iter()
            .map(|key| {
                let position = queue.position(key.0, key.1).expect("it waits");
                (*key, position.ahead, position.held_behind_own_text)
            })
            .collect();
        for (seat, head) in &held_heads {
            let position = queue
                .position(Sender::Seat(SeatId::new(*seat)), operation(*head))
                .expect("the second line waits");
            assert!(position.held_behind_own_text, "seed {seed}");
        }
        let mut served_at: BTreeMap<OperationId, usize> = BTreeMap::new();
        let mut index = 0;
        while let Some(served) = queue.serve_next(0).expect("serves") {
            served_at.insert(served.envelope.operation_id(), index);
            index += 1;
            queue
                .finish(
                    served.envelope.sender(),
                    served.envelope.operation_id(),
                    OutcomeKind::Applied,
                )
                .expect("finishes");
        }
        for ((_, id), ahead, _) in &promised {
            let Some(actual) = served_at.get(id) else {
                assert!(
                    any_text,
                    "seed {seed}: an input with no text anywhere never ran"
                );
                continue;
            };
            if any_text {
                assert!(
                    actual <= ahead,
                    "seed {seed}: {actual} served before it, {ahead} promised"
                );
            } else {
                assert_eq!(actual, ahead, "seed {seed}");
            }
        }
    }
}
