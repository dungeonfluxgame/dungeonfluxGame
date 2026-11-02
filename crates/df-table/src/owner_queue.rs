//! The owner's input queue: three lanes, one total order (RES22, H.2, C.1, K.6).
//!
//! It owns admission and serving. `admit` decides whether an `InputEnvelope` may wait; `serve_next`
//! hands the owner the next one and assigns its `turn_seq` at that moment, so the serve order is the
//! single total order that H.2 records. Three lanes exist and are served in this strict order: the
//! engine lane (proposals and hand-offs), the timers and results lane, then the player lane. The
//! engine lanes are first-in first-out; the player lane is served round-robin by seat (K.6), so a
//! seat that floods can delay a quiet seat by at most one pass over the seats that have input
//! waiting. Nothing here reads a clock or draws a die: time arrives as `now_ms`.
//!
//! The bounds are by design and named. A seat holds at most `seat_pending_limit` inputs admitted and
//! not yet finished, counting the one that is in INTERPRET (RES22); over it the input is refused
//! `table_busy` for that seat only (K.6). Each seat also has a token bucket (`seat_rate_burst` tokens,
//! refilled at `seat_rate_sustained_per_s`); an empty bucket refuses `rate_limited` and says when to
//! retry. A full engine lane or timers lane is a stall and not a decline (RES22). The lane caps, the
//! seat count and the remembered operations are `QueueLimits`, huge by default (owner, 2026-11-02).
//!
//! A duplicate operation id is refused with the status of the first one, so it runs once and the
//! second submit learns where the first stands (H.2). A refused input is not remembered, so the
//! player may send it again. The record of a finished operation is kept until
//! `MAXIMUM_REMEMBERED_OPERATIONS` newer ones have finished; after that an old id is new again.
//!
//! It never decides what an input means and never runs the pipeline. The caller must `finish` every
//! served input, or its seat stays busy. `received`, `declined`, `refused` and `served` events go to
//! the `EventSink` as they happen. `turn_seq` of an event before service is `NOT_YET_SERVED`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::NonZeroU32;
use std::ops::Bound;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::{SettingId, Settings};
use crate::envelope::{EngineInput, InputEnvelope, InputOrigin};
use crate::event::TableEvent;
use crate::identity::{Correlation, OperationId, SeatId, TableId, TableScope, TurnSeq};
use crate::listed::listed_enum;
use crate::outcome::OutcomeKind;
use crate::ports::EventSink;

mod errors;
mod token_bucket;

pub use errors::{AdmitError, FinishError, QueueConfigError, RefusalReport, SeatError, ServeError};
pub(crate) use token_bucket::TokenBucket;

/// The `turn_seq` of an event about an input that has not been served yet. The first input served
/// is number 1.
pub const NOT_YET_SERVED: TurnSeq = TurnSeq::new(0);

/// The most seats one queue tracks. Deliberately huge so it never limits play; scope it down later
/// (owner, 2026-11-02).
pub const MAXIMUM_SEATS: usize = 1_000_000;

/// The most inputs the engine lane holds. The plan gives no number: J.4 leaves the lane caps to the
/// first run over real adapters. Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_ENGINE_LANE_LENGTH: usize = 1_000_000;

/// The most inputs the timers and results lane holds. Same status as the engine lane cap.
pub const MAXIMUM_TIMERS_AND_RESULTS_LANE_LENGTH: usize = 1_000_000;

/// The most finished operations whose result a duplicate can still find (O.7); the oldest finished
/// one is forgotten first. Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_REMEMBERED_OPERATIONS: usize = 1_000_000;

listed_enum! {
    /// The three lanes of H.2, declared in the order they are served.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Lane {
        /// Engine proposals and machine hand-offs (RES21).
        Engine,
        /// Timers, and results of resource requests that ran off the owner (RES22).
        TimersAndResults,
        /// Seat input, served round-robin by seat.
        Player,
    }
}

impl Lane {
    pub fn key(self) -> &'static str {
        match self {
            Self::Engine => "engine",
            Self::TimersAndResults => "timers_and_results",
            Self::Player => "player",
        }
    }
}

listed_enum! {
    /// Why an input was refused without a decline code: nothing a player is told, so the event
    /// names the cause instead.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum NotDeclinedCause {
        /// The seat is not at this table.
        UnknownSeat,
        /// The operation id was admitted before; the first one's status is the answer.
        Duplicate,
        /// An engine lane is full: a stall, not a decline (RES22).
        LaneStalled,
    }
}

impl NotDeclinedCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::UnknownSeat => "unknown_seat",
            Self::Duplicate => "duplicate",
            Self::LaneStalled => "lane_stalled",
        }
    }
}

/// The bounds of one queue other than the per-seat settings. Each is a limit on a collection that
/// grows with play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueLimits {
    pub seats: usize,
    pub engine_lane_length: usize,
    pub timers_and_results_lane_length: usize,
    /// Finished operations remembered for duplicate detection; zero remembers none.
    pub remembered_operations: usize,
}

impl Default for QueueLimits {
    fn default() -> Self {
        Self {
            seats: MAXIMUM_SEATS,
            engine_lane_length: MAXIMUM_ENGINE_LANE_LENGTH,
            timers_and_results_lane_length: MAXIMUM_TIMERS_AND_RESULTS_LANE_LENGTH,
            remembered_operations: MAXIMUM_REMEMBERED_OPERATIONS,
        }
    }
}

/// Where an operation stands. A duplicate submit is answered with this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    Queued {
        lane: Lane,
    },
    /// Served and not finished: the input is in the pipeline, perhaps in INTERPRET.
    InProgress {
        turn_seq: TurnSeq,
    },
    /// Finished; `turn_seq` names the turn whose record holds the full result.
    Finished {
        turn_seq: TurnSeq,
        outcome: OutcomeKind,
    },
}

/// An input that was admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admitted {
    pub lane: Lane,
}

/// An input the owner now runs, with the place it took in the total order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedInput {
    pub envelope: InputEnvelope,
    pub turn_seq: TurnSeq,
    pub lane: Lane,
    /// From admission to service, in milliseconds.
    pub waited_ms: u64,
}

impl ServedInput {
    pub fn correlation(&self) -> Correlation {
        Correlation {
            operation_id: self.envelope.operation_id(),
            turn_seq: self.turn_seq,
            revision: self.envelope.view().revision(),
        }
    }
}

/// Which seat membership an operation counts against, so an input that finishes after its seat left
/// and came back does not change the new membership's count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeatHold {
    seat: SeatId,
    membership: u64,
}

struct Waiting {
    envelope: InputEnvelope,
    admitted_ms: u64,
    holder: Option<SeatHold>,
}

struct SeatState {
    membership: u64,
    waiting: VecDeque<Waiting>,
    in_flight: usize,
    bucket: TokenBucket,
}

impl SeatState {
    fn pending(&self) -> usize {
        self.waiting.len().saturating_add(self.in_flight)
    }

    fn take_front_into_flight(&mut self) -> Option<Waiting> {
        let waiting = self.waiting.pop_front()?;
        self.in_flight = self.in_flight.saturating_add(1);
        Some(waiting)
    }
}

struct OperationRecord {
    status: OperationStatus,
    holder: Option<SeatHold>,
}

struct FifoLane {
    waiting: VecDeque<Waiting>,
    limit: usize,
}

/// Which engine lane an input joins.
#[derive(Debug, Clone, Copy)]
enum EngineLane {
    Proposals,
    TimersAndResults,
}

impl EngineLane {
    fn of(input: &EngineInput) -> Self {
        match input {
            EngineInput::Proposal { .. } => Self::Proposals,
            EngineInput::Timer { .. } | EngineInput::ResourceResult { .. } => {
                Self::TimersAndResults
            }
        }
    }

    fn lane(self) -> Lane {
        match self {
            Self::Proposals => Lane::Engine,
            Self::TimersAndResults => Lane::TimersAndResults,
        }
    }
}

/// The owner's queue of one table.
pub struct OwnerQueue {
    table: TableId,
    sink: Arc<dyn EventSink>,
    limits: QueueLimits,
    seat_pending_limit: usize,
    seat_burst: NonZeroU32,
    seat_sustained_per_s: NonZeroU32,
    seats: BTreeMap<SeatId, SeatState>,
    next_membership: u64,
    /// The seats that have input waiting; the player lane serves them cyclically by seat id.
    active_seats: BTreeSet<SeatId>,
    last_served_seat: Option<SeatId>,
    player_waiting: usize,
    engine: FifoLane,
    timers_and_results: FifoLane,
    operations: BTreeMap<OperationId, OperationRecord>,
    finished: VecDeque<OperationId>,
    last_turn_seq: TurnSeq,
    probes: u64,
}

impl OwnerQueue {
    /// An empty queue for `table`, with the per-seat bounds and rates of `settings` and the default
    /// `QueueLimits`.
    pub fn new(
        table: TableId,
        settings: &Settings,
        sink: Arc<dyn EventSink>,
    ) -> Result<Self, QueueConfigError> {
        let seat_burst = NonZeroU32::new(settings.seat_rate_burst())
            .ok_or(QueueConfigError::ZeroSetting(SettingId::SeatRateBurst))?;
        let seat_sustained_per_s = NonZeroU32::new(settings.seat_rate_sustained_per_s()).ok_or(
            QueueConfigError::ZeroSetting(SettingId::SeatRateSustainedPerS),
        )?;
        let limits = QueueLimits::default();
        Ok(Self {
            table,
            sink,
            limits,
            seat_pending_limit: usize::try_from(settings.seat_pending_limit())
                .unwrap_or(usize::MAX),
            seat_burst,
            seat_sustained_per_s,
            seats: BTreeMap::new(),
            next_membership: 0,
            active_seats: BTreeSet::new(),
            last_served_seat: None,
            player_waiting: 0,
            engine: FifoLane::with_limit(limits.engine_lane_length),
            timers_and_results: FifoLane::with_limit(limits.timers_and_results_lane_length),
            operations: BTreeMap::new(),
            finished: VecDeque::new(),
            last_turn_seq: NOT_YET_SERVED,
            probes: 0,
        })
    }

    /// Replaces the limits of a queue that has not been used yet.
    pub fn with_limits(mut self, limits: QueueLimits) -> Self {
        self.limits = limits;
        self.engine.limit = limits.engine_lane_length;
        self.timers_and_results.limit = limits.timers_and_results_lane_length;
        self
    }

    /// Continues the order of a table that already served inputs up to `last`, such as after a
    /// restart: the next input served is `last` plus one.
    pub fn resuming_after(mut self, last: TurnSeq) -> Self {
        self.last_turn_seq = last;
        self
    }

    /// Seats a member at `now_ms` with a full token bucket.
    pub fn join_seat(&mut self, seat: SeatId, now_ms: u64) -> Result<(), SeatError> {
        if self.seats.contains_key(&seat) {
            return Err(SeatError::AlreadySeated { seat });
        }
        if self.seats.len() >= self.limits.seats {
            return Err(SeatError::TooManySeats {
                limit: self.limits.seats,
            });
        }
        self.next_membership = self.next_membership.saturating_add(1);
        self.seats.insert(
            seat,
            SeatState {
                membership: self.next_membership,
                waiting: VecDeque::new(),
                in_flight: 0,
                bucket: TokenBucket::full(self.seat_burst, self.seat_sustained_per_s, now_ms),
            },
        );
        Ok(())
    }

    /// Removes a seat and returns the operation ids of its inputs that were still waiting, which
    /// are forgotten: the caller records them. An input already served stays in progress and can
    /// still be finished.
    pub fn leave_seat(&mut self, seat: SeatId) -> Result<Vec<OperationId>, SeatError> {
        let state = self
            .seats
            .remove(&seat)
            .ok_or(SeatError::UnknownSeat { seat })?;
        self.active_seats.remove(&seat);
        let discarded: Vec<OperationId> = state
            .waiting
            .iter()
            .map(|waiting| waiting.envelope.operation_id())
            .collect();
        for operation_id in &discarded {
            self.operations.remove(operation_id);
        }
        self.player_waiting = self.player_waiting.saturating_sub(discarded.len());
        Ok(discarded)
    }

    /// Admits an input or refuses it with a typed reason, and records what happened. Refusals are
    /// checked in this order: seat, duplicate, the seat's pending bound, its token bucket.
    pub fn admit(&mut self, envelope: InputEnvelope, now_ms: u64) -> Result<Admitted, AdmitError> {
        let correlation = Correlation {
            operation_id: envelope.operation_id(),
            turn_seq: NOT_YET_SERVED,
            revision: envelope.view().revision(),
        };
        self.record(TableEvent::InputReceived {
            scope: self.scope(),
            correlation,
            length: envelope.length_class(),
            evidence: envelope.evidence(),
        });
        let admission = match envelope.origin() {
            InputOrigin::Player { seat, .. } => self.place_player(*seat, envelope, now_ms),
            InputOrigin::Engine(input) => {
                let lane = EngineLane::of(input);
                self.place_engine(lane, envelope, now_ms)
            }
        };
        if let Err(refusal) = &admission {
            self.record_refusal(correlation, refusal);
        }
        admission
    }

    /// Serves the next input in the total order, or `None` when nothing waits. The `turn_seq` it
    /// assigns is one more than the last. The input is in progress until `finish`.
    pub fn serve_next(&mut self, now_ms: u64) -> Result<Option<ServedInput>, ServeError> {
        if self.waiting_total() == 0 {
            return Ok(None);
        }
        let turn_seq = self.next_turn_seq()?;
        let Some((lane, waiting)) = self.pop_by_priority() else {
            return Ok(None);
        };
        self.last_turn_seq = turn_seq;
        let waited_ms = now_ms.saturating_sub(waiting.admitted_ms);
        self.operations.insert(
            waiting.envelope.operation_id(),
            OperationRecord {
                status: OperationStatus::InProgress { turn_seq },
                holder: waiting.holder,
            },
        );
        let served = ServedInput {
            envelope: waiting.envelope,
            turn_seq,
            lane,
            waited_ms,
        };
        self.record(TableEvent::InputServed {
            scope: self.scope(),
            correlation: served.correlation(),
            lane,
            ms: waited_ms,
        });
        Ok(Some(served))
    }

    /// Ends a served input with the kind of its outcome: its seat has room again, and a duplicate
    /// of it now learns the outcome.
    pub fn finish(
        &mut self,
        operation_id: OperationId,
        outcome: OutcomeKind,
    ) -> Result<(), FinishError> {
        let record = self
            .operations
            .get_mut(&operation_id)
            .ok_or(FinishError::UnknownOperation)?;
        let OperationStatus::InProgress { turn_seq } = record.status else {
            return Err(FinishError::NotInProgress {
                status: record.status,
            });
        };
        record.status = OperationStatus::Finished { turn_seq, outcome };
        let holder = record.holder;
        let seat_state = holder
            .and_then(|hold| self.seats.get_mut(&hold.seat).map(|state| (hold, state)))
            .filter(|(hold, state)| state.membership == hold.membership)
            .map(|(_, state)| state);
        if let Some(state) = seat_state {
            state.in_flight = state.in_flight.saturating_sub(1);
        }
        self.finished.push_back(operation_id);
        self.forget_oldest_finished();
        Ok(())
    }

    /// The status of an operation, if the queue knows it.
    pub fn status(&self, operation_id: &OperationId) -> Option<OperationStatus> {
        self.operations
            .get(operation_id)
            .map(|record| record.status)
    }

    /// How many inputs wait in a lane.
    pub fn depth(&self, lane: Lane) -> usize {
        match lane {
            Lane::Engine => self.engine.waiting.len(),
            Lane::TimersAndResults => self.timers_and_results.waiting.len(),
            Lane::Player => self.player_waiting,
        }
    }

    /// The inputs a seat has admitted and not finished (what `seat_pending_limit` bounds), or
    /// `None` when the seat is not at the table.
    pub fn seat_pending(&self, seat: SeatId) -> Option<usize> {
        self.seats.get(&seat).map(SeatState::pending)
    }

    /// The operations the queue remembers: waiting, in progress and finished. It never exceeds the
    /// remembered limit by more than the waiting and in-progress ones.
    pub fn remembered_operations(&self) -> usize {
        self.operations.len()
    }

    /// The `turn_seq` of the last input served; `NOT_YET_SERVED` before the first.
    pub fn last_turn_seq(&self) -> TurnSeq {
        self.last_turn_seq
    }

    /// How many lanes and seats `serve_next` looked at in all. Serving is linear when this stays
    /// within a constant times the number served.
    pub fn probes(&self) -> u64 {
        self.probes
    }

    fn scope(&self) -> TableScope {
        TableScope::of_table(self.table)
    }

    fn record(&self, event: TableEvent) {
        self.sink.record(&event);
    }

    fn record_refusal(&self, correlation: Correlation, refusal: &AdmitError) {
        let event = match refusal.report() {
            RefusalReport::Declined(code) => TableEvent::InputDeclined {
                scope: self.scope(),
                correlation,
                code,
            },
            RefusalReport::NotDeclined(cause) => TableEvent::InputRefused {
                scope: self.scope(),
                correlation,
                cause,
            },
        };
        self.record(event);
    }

    fn waiting_total(&self) -> usize {
        self.engine
            .waiting
            .len()
            .saturating_add(self.timers_and_results.waiting.len())
            .saturating_add(self.player_waiting)
    }

    fn next_turn_seq(&self) -> Result<TurnSeq, ServeError> {
        self.last_turn_seq
            .get()
            .checked_add(1)
            .map(TurnSeq::new)
            .ok_or(ServeError::TurnSequenceExhausted)
    }

    fn place_player(
        &mut self,
        seat: SeatId,
        envelope: InputEnvelope,
        now_ms: u64,
    ) -> Result<Admitted, AdmitError> {
        let operation_id = envelope.operation_id();
        let limit = self.seat_pending_limit;
        let earlier = self
            .operations
            .get(&operation_id)
            .map(|record| record.status);
        let state = self
            .seats
            .get_mut(&seat)
            .ok_or(AdmitError::UnknownSeat { seat })?;
        if let Some(first) = earlier {
            return Err(AdmitError::Duplicate { first });
        }
        let pending = state.pending();
        if pending >= limit {
            return Err(AdmitError::SeatBusy { pending, limit });
        }
        state
            .bucket
            .try_take(now_ms)
            .map_err(|retry_after_ms| AdmitError::RateLimited { retry_after_ms })?;
        let holder = Some(SeatHold {
            seat,
            membership: state.membership,
        });
        state.waiting.push_back(Waiting {
            envelope,
            admitted_ms: now_ms,
            holder,
        });
        self.active_seats.insert(seat);
        self.player_waiting = self.player_waiting.saturating_add(1);
        self.operations.insert(
            operation_id,
            OperationRecord {
                status: OperationStatus::Queued { lane: Lane::Player },
                holder,
            },
        );
        Ok(Admitted { lane: Lane::Player })
    }

    fn place_engine(
        &mut self,
        engine_lane: EngineLane,
        envelope: InputEnvelope,
        now_ms: u64,
    ) -> Result<Admitted, AdmitError> {
        let operation_id = envelope.operation_id();
        let lane = engine_lane.lane();
        if let Some(record) = self.operations.get(&operation_id) {
            return Err(AdmitError::Duplicate {
                first: record.status,
            });
        }
        let fifo = match engine_lane {
            EngineLane::Proposals => &mut self.engine,
            EngineLane::TimersAndResults => &mut self.timers_and_results,
        };
        if fifo.waiting.len() >= fifo.limit {
            return Err(AdmitError::LaneStalled {
                lane,
                limit: fifo.limit,
            });
        }
        fifo.waiting.push_back(Waiting {
            envelope,
            admitted_ms: now_ms,
            holder: None,
        });
        self.operations.insert(
            operation_id,
            OperationRecord {
                status: OperationStatus::Queued { lane },
                holder: None,
            },
        );
        Ok(Admitted { lane })
    }

    fn pop_by_priority(&mut self) -> Option<(Lane, Waiting)> {
        self.probes = self.probes.saturating_add(1);
        if let Some(waiting) = self.engine.waiting.pop_front() {
            return Some((Lane::Engine, waiting));
        }
        self.probes = self.probes.saturating_add(1);
        if let Some(waiting) = self.timers_and_results.waiting.pop_front() {
            return Some((Lane::TimersAndResults, waiting));
        }
        self.pop_player().map(|waiting| (Lane::Player, waiting))
    }

    /// The seat after the one served last, cyclically, that has input waiting. A seat in the set
    /// with nothing waiting is dropped from it and skipped, which cannot lose an input.
    fn pop_player(&mut self) -> Option<Waiting> {
        loop {
            self.probes = self.probes.saturating_add(1);
            let seat = self.next_active_seat()?;
            let waiting = self
                .seats
                .get_mut(&seat)
                .and_then(|state| state.take_front_into_flight());
            let Some(waiting) = waiting else {
                self.active_seats.remove(&seat);
                continue;
            };
            let emptied = self
                .seats
                .get(&seat)
                .is_none_or(|state| state.waiting.is_empty());
            if emptied {
                self.active_seats.remove(&seat);
            }
            self.last_served_seat = Some(seat);
            self.player_waiting = self.player_waiting.saturating_sub(1);
            return Some(waiting);
        }
    }

    fn next_active_seat(&self) -> Option<SeatId> {
        let after_last = self.last_served_seat.and_then(|last| {
            self.active_seats
                .range((Bound::Excluded(last), Bound::Unbounded))
                .next()
        });
        after_last.or_else(|| self.active_seats.first()).copied()
    }

    fn forget_oldest_finished(&mut self) {
        while self.finished.len() > self.limits.remembered_operations {
            let Some(oldest) = self.finished.pop_front() else {
                return;
            };
            self.operations.remove(&oldest);
        }
    }
}

impl FifoLane {
    fn with_limit(limit: usize) -> Self {
        Self {
            waiting: VecDeque::new(),
            limit,
        }
    }
}
