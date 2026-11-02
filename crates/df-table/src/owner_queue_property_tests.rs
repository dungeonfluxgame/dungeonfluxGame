//! Property-style tests of the owner queue: a small deterministic generator, many seeds, and
//! reference models that say what must be true after every step. Bounds are small and seats come
//! and go, so the cases a huge bound hides (a seat refused `table_busy` mid-round, a seat leaving
//! with the cursor past it) are the cases that run.

use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::config::Settings;
use crate::envelope::{InputEnvelope, Sender};
use crate::identity::{OperationId, SeatId, TableId};
use crate::outcome::OutcomeKind;
use crate::owner_queue::{
    AdmitError, Lane, OperationStatus, OwnerQueue, QueueLimits, ServedInput, TokenBucket,
};
use crate::owner_queue_tests::{
    NullSink, operation, pending_answer, proposal, resource_result, safety_card, seated_queue,
    settings, tap, timer,
};

const SEEDS: u64 = 300;
const HUGE: u32 = 1_000_000;

/// A xorshift64 generator: the same seed always gives the same numbers.
pub(crate) struct XorShift(u64);

impl XorShift {
    pub(crate) fn seeded(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub(crate) fn next_number(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.0 = state;
        state
    }

    pub(crate) fn below(&mut self, bound: u64) -> u64 {
        self.next_number() % bound
    }

    pub(crate) fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// The number a test gave an operation: `operation(n)` is the id of the number n.
fn number_of(operation_id: OperationId) -> u64 {
    (u128::from_be_bytes(*operation_id.as_bytes()) - 1) as u64
}

fn seat_of(served: &ServedInput) -> u64 {
    served.envelope.seat().expect("a player input").get()
}

fn serve(queue: &mut OwnerQueue, now_ms: u64) -> Option<ServedInput> {
    queue.serve_next(now_ms).expect("serves")
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

/// The per-seat first-in first-out order, and how many serves went to other seats since each seat
/// last had to wait for its turn.
#[derive(Default)]
struct FairnessModel {
    waiting: BTreeMap<u64, VecDeque<OperationId>>,
    passed_over: BTreeMap<u64, u64>,
}

impl FairnessModel {
    fn admitted(&mut self, seat: u64, operation_id: OperationId) {
        let line = self.waiting.entry(seat).or_default();
        if line.is_empty() {
            self.passed_over.insert(seat, 0);
        }
        line.push_back(operation_id);
    }

    /// Checks one serve against the model: the seat's own order, and that no seat with input
    /// waiting has been passed over by more distinct seats than the table has.
    fn served(&mut self, seat: u64, operation_id: OperationId, seats: u64) {
        let next = self.waiting.get_mut(&seat).and_then(VecDeque::pop_front);
        assert_eq!(
            next,
            Some(operation_id),
            "seat {seat} was served out of order"
        );
        self.passed_over.remove(&seat);
        for (other, count) in &mut self.passed_over {
            *count += 1;
            assert!(
                *count < seats,
                "seat {other} was passed over {count} times with {seats} seats"
            );
        }
        if self.waiting.get(&seat).is_some_and(|line| !line.is_empty()) {
            self.passed_over.insert(seat, 0);
        }
    }
}

fn random_settings(random: &mut XorShift) -> Settings {
    match random.below(3) {
        0 => settings(HUGE, HUGE, 1),
        1 => settings(3, 3, 1),
        _ => settings(
            1 + random.below(4) as u32,
            1 + random.below(4) as u32,
            1 + random.below(3) as u32,
        ),
    }
}

#[test]
fn no_seat_waits_more_than_one_round_at_any_bound_and_a_refusal_never_costs_a_place() {
    for seed in 0..SEEDS {
        let mut random = XorShift::seeded(seed);
        let seats = 2 + random.below(11);
        let mut queue = seated_queue(seats, &random_settings(&mut random), Arc::new(NullSink));
        let mut model = FairnessModel::default();
        let mut in_flight: Vec<ServedInput> = Vec::new();
        let mut next_operation = 0;
        let mut now_ms = 0_u64;
        for _ in 0..600 {
            now_ms += random.below(1_500);
            match random.below(10) {
                0..=5 => {
                    let seat = if random.chance(70) {
                        1
                    } else {
                        1 + random.below(seats)
                    };
                    next_operation += 1;
                    let before = queue.seat_pending(SeatId::new(seat));
                    match queue.admit(tap(next_operation, seat), now_ms) {
                        Ok(_) => model.admitted(seat, operation(next_operation)),
                        Err(AdmitError::SeatBusy { .. } | AdmitError::RateLimited { .. }) => {
                            assert_eq!(queue.seat_pending(SeatId::new(seat)), before);
                            assert_eq!(
                                queue.status(
                                    Sender::Seat(SeatId::new(seat)),
                                    operation(next_operation)
                                ),
                                None
                            );
                        }
                        Err(other) => panic!("seed {seed}: unexpected refusal {other:?}"),
                    }
                }
                6..=8 => {
                    if let Some(served) = serve(&mut queue, now_ms) {
                        model.served(seat_of(&served), served.envelope.operation_id(), seats);
                        in_flight.push(served);
                    }
                }
                _ => {
                    if !in_flight.is_empty() {
                        let index = random.below(in_flight.len() as u64) as usize;
                        let served = in_flight.swap_remove(index);
                        end(&mut queue, &served);
                    }
                }
            }
        }
        for served in in_flight.drain(..) {
            end(&mut queue, &served);
        }
        while let Some(served) = serve(&mut queue, now_ms) {
            model.served(seat_of(&served), served.envelope.operation_id(), seats);
            end(&mut queue, &served);
        }
        assert_eq!(queue.depth(Lane::Player), 0, "seed {seed}: a seat starved");
        assert!(
            model.waiting.values().all(VecDeque::is_empty),
            "seed {seed}"
        );
    }
}

#[test]
fn a_flooding_seat_cannot_delay_a_quiet_seat_beyond_one_round() {
    for seed in 0..SEEDS {
        let mut random = XorShift::seeded(seed);
        let seats = 2 + random.below(11);
        let mut queue = seated_queue(seats, &random_settings(&mut random), Arc::new(NullSink));
        let mut next_operation = 0;
        let mut flood = |queue: &mut OwnerQueue, count: u64| {
            for _ in 0..count {
                next_operation += 1;
                // A refusal here is the bound doing its job; the flood only has to try.
                queue.admit(tap(next_operation, 1), 0).ok();
            }
        };
        flood(&mut queue, 200 + random.below(800));
        for _ in 0..random.below(60) {
            let Some(served) = serve(&mut queue, 0) else {
                break;
            };
            end(&mut queue, &served);
        }
        let quiet_seat = 2 + random.below(seats - 1);
        let quiet_operation = 1_000_000 + seed;
        if queue.admit(tap(quiet_operation, quiet_seat), 0).is_err() {
            continue;
        }
        let mut served_before_it = 0;
        loop {
            let served = serve(&mut queue, 0).expect("input waits");
            if served.envelope.operation_id() == operation(quiet_operation) {
                end(&mut queue, &served);
                break;
            }
            end(&mut queue, &served);
            served_before_it += 1;
            flood(&mut queue, 1);
        }
        assert!(
            served_before_it < seats,
            "seed {seed}: {served_before_it} other inputs were served first with {seats} seats"
        );
    }
}

#[test]
fn nothing_exceeds_its_bound_and_every_refusal_is_typed_and_expected() {
    for seed in 0..SEEDS {
        let mut random = XorShift::seeded(seed);
        let seats = 1 + random.below(4);
        let pending_limit = 1 + random.below(4);
        let limits = QueueLimits {
            seats: 4,
            engine_lane_length: 1 + random.below(4) as usize,
            timers_and_results_lane_length: 1 + random.below(4) as usize,
            seat_controls_pending: 1 + random.below(4) as usize,
            controls_lane_length: 1 + random.below(6) as usize,
            remembered_operations: random.below(5) as usize,
            ..QueueLimits::default()
        };
        let mut queue = OwnerQueue::new(
            TableId::new(1),
            &settings(pending_limit as u32, 6, 1),
            Arc::new(NullSink),
        )
        .expect("builds")
        .with_limits(limits);
        for seat in 1..=seats {
            queue.join_seat(SeatId::new(seat), 0).expect("joins");
        }
        let mut pending: BTreeMap<u64, u64> = (1..=seats).map(|seat| (seat, 0)).collect();
        let mut controls: BTreeMap<u64, usize> = (1..=seats).map(|seat| (seat, 0)).collect();
        let mut in_flight: Vec<ServedInput> = Vec::new();
        let mut next_operation = 0;
        for _ in 0..400 {
            match random.below(11) {
                0..=2 => {
                    let seat = 1 + random.below(seats);
                    next_operation += 1;
                    // The clock never moves, so a seat's bucket runs dry after its burst and a line
                    // that passes the pending bound may still be rate limited.
                    let result = queue.admit(tap(next_operation, seat), 0);
                    let held = pending[&seat];
                    match result {
                        Err(AdmitError::SeatBusy { pending, limit }) => {
                            assert_eq!(held as usize, pending);
                            assert_eq!(pending_limit as usize, limit);
                        }
                        Err(AdmitError::RateLimited { .. }) => {
                            assert!(held < pending_limit, "seed {seed}: checked after the bound");
                        }
                        Ok(_) => {
                            assert!(held < pending_limit, "seed {seed}");
                            pending.insert(seat, held + 1);
                        }
                        Err(other) => panic!("seed {seed}: {other:?}"),
                    }
                    if held >= pending_limit {
                        assert!(
                            matches!(result, Err(AdmitError::SeatBusy { .. })),
                            "seed {seed}"
                        );
                    }
                }
                3..=4 => {
                    let seat = 1 + random.below(seats);
                    next_operation += 1;
                    let envelope = if random.chance(50) {
                        safety_card(next_operation, seat)
                    } else {
                        pending_answer(next_operation, seat)
                    };
                    let result = queue.admit(envelope, 0);
                    let held = controls[&seat];
                    let table_wide: usize = controls.values().sum();
                    if held >= limits.seat_controls_pending {
                        let expected = AdmitError::LaneStalled {
                            lane: Lane::SeatControls,
                            limit: limits.seat_controls_pending,
                        };
                        assert_eq!(result, Err(expected), "seed {seed}");
                    } else if table_wide >= limits.controls_lane_length {
                        let expected = AdmitError::LaneStalled {
                            lane: Lane::SeatControls,
                            limit: limits.controls_lane_length,
                        };
                        assert_eq!(result, Err(expected), "seed {seed}");
                    } else {
                        assert!(
                            result.is_ok(),
                            "seed {seed}: a control was refused {result:?}"
                        );
                        controls.insert(seat, held + 1);
                    }
                }
                5 => {
                    next_operation += 1;
                    let (envelope, lane, cap) = match random.below(3) {
                        0 => (
                            proposal(next_operation),
                            Lane::Engine,
                            limits.engine_lane_length,
                        ),
                        1 => (
                            timer(next_operation),
                            Lane::TimersAndResults,
                            limits.timers_and_results_lane_length,
                        ),
                        _ => (
                            resource_result(next_operation),
                            Lane::TimersAndResults,
                            limits.timers_and_results_lane_length,
                        ),
                    };
                    let full = queue.depth(lane) >= cap;
                    let result = queue.admit(envelope, 0);
                    if full {
                        assert_eq!(result, Err(AdmitError::LaneStalled { lane, limit: cap }));
                    } else {
                        assert!(result.is_ok(), "seed {seed}: {result:?}");
                    }
                }
                6..=7 => {
                    if let Some(served) = serve(&mut queue, 0) {
                        if served.lane == Lane::SeatControls {
                            let seat = seat_of(&served);
                            controls.insert(seat, controls[&seat] - 1);
                        }
                        in_flight.push(served);
                    }
                }
                _ => {
                    if !in_flight.is_empty() {
                        let index = random.below(in_flight.len() as u64) as usize;
                        let served = in_flight.swap_remove(index);
                        end(&mut queue, &served);
                        if served.lane == Lane::Player {
                            let seat = seat_of(&served);
                            pending.insert(seat, pending[&seat] - 1);
                        }
                    }
                }
            }
            for (seat, held) in &pending {
                assert_eq!(queue.seat_pending(SeatId::new(*seat)), Some(*held as usize));
                assert!(*held <= pending_limit, "seed {seed}");
            }
            assert!(queue.depth(Lane::Engine) <= limits.engine_lane_length);
            assert!(queue.depth(Lane::TimersAndResults) <= limits.timers_and_results_lane_length);
            assert!(
                controls
                    .values()
                    .all(|held| *held <= limits.seat_controls_pending)
            );
            assert!(controls.values().sum::<usize>() <= limits.controls_lane_length);
            let live = queue.depth(Lane::SeatControls)
                + queue.depth(Lane::Engine)
                + queue.depth(Lane::TimersAndResults)
                + queue.depth(Lane::Player)
                + in_flight.len();
            assert!(
                queue.remembered_operations() <= limits.remembered_operations + live,
                "seed {seed}: {} remembered, {live} live",
                queue.remembered_operations()
            );
        }
    }
}

fn envelope_of_random_kind(random: &mut XorShift, number: u64, seats: u64) -> InputEnvelope {
    match random.below(6) {
        0 => proposal(number),
        1 => timer(number),
        2 => resource_result(number),
        3 => safety_card(number, 1 + random.below(seats)),
        _ => tap(number, 1 + random.below(seats)),
    }
}

#[test]
fn one_operation_id_runs_once_per_sender_however_often_it_is_sent_and_whenever_the_drain_stops() {
    for seed in 0..SEEDS {
        let mut random = XorShift::seeded(seed);
        let seats = 1 + random.below(5);
        let mut queue = seated_queue(seats, &settings(HUGE, HUGE, 1), Arc::new(NullSink));
        let mut known: BTreeMap<(Sender, u64), OperationStatus> = BTreeMap::new();
        let mut times_served: BTreeMap<(Sender, u64), u32> = BTreeMap::new();
        let mut in_flight: Vec<(Sender, u64)> = Vec::new();
        let mut fresh = 0;
        for _ in 0..500 {
            match random.below(10) {
                0..=4 => {
                    let resend = fresh > 0 && random.chance(50);
                    let number = if resend {
                        1 + random.below(fresh)
                    } else {
                        fresh += 1;
                        fresh
                    };
                    let envelope = envelope_of_random_kind(&mut random, number, seats);
                    let key = (envelope.sender(), number);
                    let result = queue.admit(envelope, 0);
                    match known.get(&key) {
                        Some(first) => {
                            assert_eq!(result, Err(AdmitError::Duplicate { first: *first }));
                        }
                        None => {
                            let admitted = result.expect("a new id of this sender is admitted");
                            known.insert(
                                key,
                                OperationStatus::Queued {
                                    lane: admitted.lane,
                                },
                            );
                        }
                    }
                }
                5..=7 => {
                    for _ in 0..random.below(4) {
                        let Some(served) = serve(&mut queue, 0) else {
                            break;
                        };
                        let key = (
                            served.envelope.sender(),
                            number_of(served.envelope.operation_id()),
                        );
                        *times_served.entry(key).or_default() += 1;
                        known.insert(
                            key,
                            OperationStatus::InProgress {
                                turn_seq: served.turn_seq,
                            },
                        );
                        in_flight.push(key);
                    }
                }
                _ => {
                    if !in_flight.is_empty() {
                        let index = random.below(in_flight.len() as u64) as usize;
                        let (sender, number) = in_flight.swap_remove(index);
                        queue
                            .finish(sender, operation(number), OutcomeKind::Pending)
                            .expect("finishes");
                        if let Some(OperationStatus::InProgress { turn_seq }) =
                            known.get(&(sender, number))
                        {
                            known.insert(
                                (sender, number),
                                OperationStatus::Finished {
                                    turn_seq: *turn_seq,
                                    outcome: OutcomeKind::Pending,
                                },
                            );
                        }
                    }
                }
            }
        }
        while let Some(served) = serve(&mut queue, 0) {
            let key = (
                served.envelope.sender(),
                number_of(served.envelope.operation_id()),
            );
            *times_served.entry(key).or_default() += 1;
        }
        assert!(
            times_served.values().all(|times| *times == 1),
            "seed {seed}: {times_served:?}"
        );
        assert_eq!(
            times_served.len(),
            known.len(),
            "seed {seed}: an admitted id never ran"
        );
    }
}

/// What one scenario did, as text, so two runs can be compared exactly.
fn scenario_trace(seed: u64) -> Vec<String> {
    let mut random = XorShift::seeded(seed);
    let seats = 1 + random.below(6);
    let mut queue = seated_queue(seats, &settings(4, 3, 2), Arc::new(NullSink));
    let mut trace = Vec::new();
    let mut now_ms = 0_u64;
    let mut fresh = 0;
    for _ in 0..400 {
        now_ms += random.below(700);
        if random.chance(65) {
            fresh += 1;
            let envelope = envelope_of_random_kind(&mut random, fresh, seats);
            trace.push(format!(
                "admit {fresh} at {now_ms}: {:?}",
                queue.admit(envelope, now_ms)
            ));
        } else if let Some(served) = serve(&mut queue, now_ms) {
            trace.push(format!(
                "serve {} turn {} lane {:?} waited {}",
                served.envelope.operation_id(),
                served.turn_seq,
                served.lane,
                served.waited_ms
            ));
            end(&mut queue, &served);
        }
    }
    trace
}

#[test]
fn the_same_inputs_and_the_same_clock_give_the_same_order_and_the_same_turn_numbers() {
    let mut distinct = std::collections::BTreeSet::new();
    for seed in 0..SEEDS {
        let first = scenario_trace(seed);
        assert_eq!(
            first,
            scenario_trace(seed),
            "seed {seed} was not repeatable"
        );
        assert!(
            first.iter().any(|line| line.starts_with("serve")),
            "seed {seed} served nothing"
        );
        distinct.insert(first);
    }
    assert!(distinct.len() > 1, "every seed gave the same scenario");
}

#[test]
fn turn_numbers_are_consecutive_from_one_with_no_gap_and_no_repeat() {
    for seed in 0..SEEDS {
        let trace = scenario_trace(seed);
        let turns: Vec<u64> = trace
            .iter()
            .filter(|line| line.starts_with("serve"))
            .map(|line| {
                let after = line.split(" turn ").nth(1).expect("a turn");
                after
                    .split(' ')
                    .next()
                    .expect("a number")
                    .parse()
                    .expect("a number")
            })
            .collect();
        let expected: Vec<u64> = (1..=turns.len() as u64).collect();
        assert_eq!(turns, expected, "seed {seed}");
    }
}

#[test]
fn the_highest_lane_with_input_is_always_served_first_and_engine_lanes_keep_their_order() {
    for seed in 0..SEEDS {
        let mut random = XorShift::seeded(seed);
        let seats = 1 + random.below(4);
        let mut queue = seated_queue(seats, &settings(HUGE, HUGE, 1), Arc::new(NullSink));
        let mut controls = 0_u64;
        let mut engine: VecDeque<OperationId> = VecDeque::new();
        let mut timers: VecDeque<OperationId> = VecDeque::new();
        let mut players = 0_u64;
        let mut fresh = 0;
        for _ in 0..500 {
            if random.chance(55) {
                fresh += 1;
                match random.below(4) {
                    0 => {
                        queue.admit(proposal(fresh), 0).expect("admitted");
                        engine.push_back(operation(fresh));
                    }
                    1 => {
                        let envelope = if random.chance(50) {
                            timer(fresh)
                        } else {
                            resource_result(fresh)
                        };
                        queue.admit(envelope, 0).expect("admitted");
                        timers.push_back(operation(fresh));
                    }
                    2 => {
                        queue
                            .admit(safety_card(fresh, 1 + random.below(seats)), 0)
                            .expect("admitted");
                        controls += 1;
                    }
                    _ => {
                        queue
                            .admit(tap(fresh, 1 + random.below(seats)), 0)
                            .expect("admitted");
                        players += 1;
                    }
                }
            } else if let Some(served) = serve(&mut queue, 0) {
                let served_id = served.envelope.operation_id();
                if controls > 0 {
                    assert_eq!(served.lane, Lane::SeatControls, "seed {seed}");
                    controls -= 1;
                } else if let Some(expected) = engine.pop_front() {
                    assert_eq!(
                        (served.lane, served_id),
                        (Lane::Engine, expected),
                        "seed {seed}"
                    );
                } else if let Some(expected) = timers.pop_front() {
                    assert_eq!(
                        (served.lane, served_id),
                        (Lane::TimersAndResults, expected),
                        "seed {seed}"
                    );
                } else {
                    assert_eq!(served.lane, Lane::Player, "seed {seed}");
                    players -= 1;
                }
                end(&mut queue, &served);
            } else {
                assert!(
                    controls == 0 && engine.is_empty() && timers.is_empty() && players == 0,
                    "seed {seed}"
                );
            }
        }
    }
}

#[test]
fn a_waiting_inputs_position_is_exactly_how_many_are_served_before_it() {
    for seed in 0..SEEDS {
        let mut random = XorShift::seeded(seed);
        let seats = 1 + random.below(6);
        let mut queue = seated_queue(seats, &settings(HUGE, HUGE, 1), Arc::new(NullSink));
        let mut waiting: Vec<(Sender, OperationId)> = Vec::new();
        for number in 1..=(5 + random.below(60)) {
            let envelope = match random.below(8) {
                0 => proposal(number),
                1 => timer(number),
                2 => pending_answer(number, 1 + random.below(seats)),
                _ => tap(number, 1 + random.below(seats)),
            };
            let key = (envelope.sender(), envelope.operation_id());
            queue.admit(envelope, 0).expect("admitted");
            waiting.push(key);
            if random.chance(25) {
                let served = serve(&mut queue, 0).expect("input waits");
                waiting.retain(|key| key.1 != served.envelope.operation_id());
                end(&mut queue, &served);
            }
        }
        let promised: BTreeMap<OperationId, usize> = waiting
            .iter()
            .map(|(sender, operation_id)| {
                let position = queue.position(*sender, *operation_id).expect("it waits");
                assert!(!position.held_behind_own_text);
                (*operation_id, position.ahead)
            })
            .collect();
        let mut index = 0;
        while let Some(served) = serve(&mut queue, 0) {
            let id = served.envelope.operation_id();
            assert_eq!(promised.get(&id), Some(&index), "seed {seed}");
            index += 1;
            end(&mut queue, &served);
        }
        assert_eq!(index, promised.len(), "seed {seed}");
    }
}

fn reference_wait_ms(missing_millitokens: u128, per_s: u128) -> u128 {
    missing_millitokens.div_ceil(per_s)
}

#[test]
fn the_token_bucket_matches_exact_arithmetic_and_never_gives_more_than_burst_plus_rate_times_time()
{
    for seed in 0..SEEDS {
        let mut random = XorShift::seeded(seed);
        let burst = 1 + random.below(10) as u32;
        let fastest = if random.chance(10) {
            u64::from(u32::MAX)
        } else {
            20
        };
        let per_s = 1 + random.below(fastest) as u32;
        let mut bucket = TokenBucket::full(
            NonZeroU32::new(burst).expect("nonzero"),
            NonZeroU32::new(per_s).expect("nonzero"),
            0,
        );
        let capacity = u128::from(burst) * 1_000;
        let mut held = capacity;
        let mut now_ms = 0_u64;
        let mut taken = 0_u128;
        for _ in 0..300 {
            let step = match random.below(6) {
                0 => 0,
                1 => random.below(3),
                2 => 1_000,
                3 => random.below(5_000),
                4 => random.below(u64::MAX / 600),
                _ => u64::MAX,
            };
            let before_ms = now_ms;
            now_ms = now_ms.saturating_add(step);
            let elapsed = u128::from(now_ms - before_ms);
            held = (held + elapsed * u128::from(per_s)).min(capacity);
            let result = bucket.try_take(now_ms);
            if held >= 1_000 {
                held -= 1_000;
                taken += 1;
                assert_eq!(result, Ok(()), "seed {seed}");
            } else {
                let wait = reference_wait_ms(1_000 - held, u128::from(per_s));
                assert_eq!(result, Err(wait as u64), "seed {seed}");
            }
            assert_eq!(u128::from(bucket.held_millitokens()), held, "seed {seed}");
            assert!(
                taken * 1_000 <= capacity + u128::from(now_ms) * u128::from(per_s),
                "seed {seed}: more tokens than burst plus rate times time"
            );
        }
    }
}
