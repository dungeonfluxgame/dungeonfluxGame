//! The owner's input queue: four lanes, one total order (RES22, H.2, C.1, K.6).
//!
//! It owns admission and serving. `admit` decides whether an `InputEnvelope` may wait; `serve_next`
//! hands the owner the next one and assigns its `turn_seq` at that moment, so the serve order is the
//! single total order that H.2 records. The lanes are served in this strict order: seat controls
//! (the pause card and the answer to a pending), the engine lane (proposals and hand-offs), the
//! timers and results lane, then the player lane. Controls and player input are round-robin by
//! seat (K.6), so a seat that floods can delay a quiet seat by at most one pass over the seats
//! that have input waiting; the engine lanes are first-in first-out. Nothing here reads a clock or
//! draws a die: time arrives as `now_ms`.
//!
//! The player lane holds what costs a model call or a turn. A seat holds at most
//! `seat_pending_limit` of them admitted and not yet finished, counting the one in INTERPRET (RES22);
//! over it the input is refused `table_busy` for that seat only (K.6). Each seat also has a token
//! bucket (`seat_rate_burst` tokens, refilled at `seat_rate_sustained_per_s`); an empty bucket
//! refuses `rate_limited` and says when to retry. C.1 also limits a seat to one free-text line in
//! INTERPRET at a time, and the queue owns that: while a `SubmitText` or `Say` of a seat is in
//! progress, the seat's next input waits at the head of its own line (its order is kept) and the
//! seat is skipped until `finish` ends the first. Seat controls are not limited by any of this
//! (decision recorded against H.2, which names three lanes: a pause or the answer that releases a
//! held turn must never be told `table_busy` and makes no model call, K35, F.7). They have no
//! bucket and two bounds: `seat_controls_pending` per seat and `controls_lane_length` for the whole
//! table, so one seat cannot keep the other lanes waiting behind its controls for longer than the
//! table-wide count allows. Overflow of either is a stall.
//!
//! A full engine lane, timers lane or controls bound is a stall and not a decline (RES22). The lane
//! caps, the seat count, the controls bounds, the remembered operations and the lease are
//! `QueueLimits`, huge by default (owner, 2026-11-02).
//!
//! Operation ids are minted by their sender: a phone numbers its own inputs and the engine its own.
//! Duplicates are found per sender and per membership of that sender, so an id one seat used means
//! nothing to another seat or to the engine, and a seat that leaves and joins again (or another
//! player who takes its seat id) starts with no remembered ids: ids restart per membership,
//! `leave_seat` abandons what the seat had in progress and forgets what it had waiting. A duplicate
//! is refused with the status of the first one, so it runs once and the second submit learns where
//! the first stands (H.2). A refused input is not remembered, so it may be sent again. The record
//! of an ended operation is kept until `MAXIMUM_REMEMBERED_OPERATIONS` newer ones have ended; after
//! that an old id is new again.
//!
//! A served input is in progress until the caller ends it with `finish`. If the caller never does
//! (an early return in the pipeline, a lost job), `in_progress` lists what is stuck and for how long
//! and `reap_expired` abandons whatever has run longer than the lease, which frees its seat. The
//! watchdog is the caller's: it calls these with the time. An abandoned operation stays remembered
//! as `Abandoned`; if its `finish` arrives after all, the outcome is recorded once as its result
//! (a `finished_late` event), so a retry of the same id learns it and nothing runs twice. The commit
//! step checks the status before it applies (C3): an input that is `Abandoned` must not commit.
//! `position` answers K.6.
//!
//! It never decides what an input means and never runs the pipeline. `received`, `declined`,
//! `refused`, `served` and `abandoned` events go to the `EventSink` as they happen. `turn_seq` of an
//! event before service is `NOT_YET_SERVED`.

use std::num::NonZeroU32;
use std::sync::Arc;

use crate::config::{SettingId, Settings};
use crate::envelope::{InputEnvelope, InputOrigin, Sender};
use crate::event::{FailureCause, TableEvent};
use crate::identity::{Correlation, OperationId, SeatId, TableId, TableScope, TurnSeq};
use crate::outcome::OutcomeKind;
use crate::ports::EventSink;

mod counted;
mod errors;
mod lane;
mod position;
mod reports;
mod state;
mod token_bucket;

use counted::{Book, Fifo, Ring};
#[cfg(test)]
pub(crate) use counted::{reset_touched_entries, touched_entries};
pub use errors::{AdmitError, FinishError, QueueConfigError, RefusalReport, SeatError, ServeError};
pub use lane::{
    Lane, MAXIMUM_CONTROLS_LANE_LENGTH, MAXIMUM_ENGINE_LANE_LENGTH, MAXIMUM_IN_PROGRESS_MS,
    MAXIMUM_REMEMBERED_OPERATIONS, MAXIMUM_SEAT_CONTROLS_PENDING, MAXIMUM_SEATS,
    MAXIMUM_TIMERS_AND_RESULTS_LANE_LENGTH, NotDeclinedCause, QueueLimits,
};
pub use reports::{Admitted, InProgressOperation, OperationStatus, QueuePosition, ServedInput};
use state::{
    Ending, EngineLane, FifoLane, InProgressEntry, OperationKey, OperationRecord, Placement,
    PlayerRoute, SeatHold, SeatState, Waiting,
};
pub(crate) use token_bucket::TokenBucket;

/// The `turn_seq` of an event about an input that has not been served yet. The first input served
/// is number 1.
pub const NOT_YET_SERVED: TurnSeq = TurnSeq::new(0);

/// The owner's queue of one table.
pub struct OwnerQueue {
    table: TableId,
    sink: Arc<dyn EventSink>,
    limits: QueueLimits,
    seat_pending_limit: usize,
    seat_burst: NonZeroU32,
    seat_sustained_per_s: NonZeroU32,
    seats: Book<SeatId, SeatState>,
    next_membership: u64,
    line_ring: Ring,
    control_ring: Ring,
    lines_waiting: usize,
    controls_waiting: usize,
    engine: FifoLane,
    timers_and_results: FifoLane,
    operations: Book<OperationKey, OperationRecord>,
    in_progress: Book<TurnSeq, InProgressEntry>,
    ended: Fifo<OperationKey>,
    last_turn_seq: TurnSeq,
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
            seats: Book::new(),
            next_membership: 0,
            line_ring: Ring::new(),
            control_ring: Ring::new(),
            lines_waiting: 0,
            controls_waiting: 0,
            engine: FifoLane::with_limit(limits.engine_lane_length),
            timers_and_results: FifoLane::with_limit(limits.timers_and_results_lane_length),
            operations: Book::new(),
            in_progress: Book::new(),
            ended: Fifo::new(),
            last_turn_seq: NOT_YET_SERVED,
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
                lines: Fifo::new(),
                controls: Fifo::new(),
                lines_in_flight: 0,
                free_text_in_flight: 0,
                bucket: TokenBucket::full(self.seat_burst, self.seat_sustained_per_s, now_ms),
            },
        );
        Ok(())
    }

    /// Removes a seat at `now_ms`. Its inputs that were still waiting are forgotten and their
    /// operation ids returned, for the caller to record. Its inputs in progress are abandoned
    /// (cause `stale`: nobody is left to show the result to), and the seat's whole operation
    /// history goes with its membership, so a seat that joins again, or another player who takes
    /// the seat id, starts with no remembered ids and learns nothing of the one before.
    pub fn leave_seat(&mut self, seat: SeatId, now_ms: u64) -> Result<Vec<OperationId>, SeatError> {
        let membership = self
            .seats
            .get(&seat)
            .map(|state| state.membership)
            .ok_or(SeatError::UnknownSeat { seat })?;
        let running: Vec<OperationKey> = self
            .in_progress
            .iter()
            .map(|(_, entry)| entry.key)
            .filter(|(sender, held, _)| *sender == Sender::Seat(seat) && *held == membership)
            .collect();
        for key in running {
            // Each key was just read from the in-progress list, so ending it cannot be refused.
            self.abandon_key(key, FailureCause::Stale, now_ms).ok();
        }
        let state = self
            .seats
            .remove(&seat)
            .ok_or(SeatError::UnknownSeat { seat })?;
        self.line_ring.set_eligible(seat, false);
        self.control_ring.set_eligible(seat, false);
        let discarded: Vec<OperationId> = state
            .lines
            .iter()
            .chain(state.controls.iter())
            .map(|waiting| waiting.envelope.operation_id())
            .collect();
        for operation_id in &discarded {
            self.operations
                .remove(&(Sender::Seat(seat), membership, *operation_id));
        }
        self.lines_waiting = self.lines_waiting.saturating_sub(state.lines.len());
        self.controls_waiting = self.controls_waiting.saturating_sub(state.controls.len());
        Ok(discarded)
    }

    /// Admits an input or refuses it with a typed reason, and records what happened. Refusals are
    /// checked in this order: seat, duplicate, then the lane's own bound (for the player lane the
    /// seat's pending bound and then its token bucket).
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
        let placement = match envelope.origin() {
            InputOrigin::Player { seat, input } => Placement::Seat(*seat, PlayerRoute::of(input)),
            InputOrigin::Engine(input) => Placement::Engine(EngineLane::of(input)),
        };
        let admission = match placement {
            Placement::Seat(seat, route) => self.place_player(seat, route, envelope, now_ms),
            Placement::Engine(lane) => self.place_engine(lane, envelope, now_ms),
        };
        if let Err(refusal) = &admission {
            self.record_refusal(correlation, refusal);
        }
        admission
    }

    /// Serves the next input in the total order, or `None` when nothing can be served (nothing
    /// waits, or every waiting seat is held behind its own free-text line). The `turn_seq` it
    /// assigns is one more than the last. The input is in progress until it is ended.
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
        let envelope = waiting.envelope;
        let membership = waiting.holder.map_or(0, |hold| hold.membership);
        let key = (envelope.sender(), membership, envelope.operation_id());
        let revision = envelope.view().revision();
        self.operations.insert(
            key,
            OperationRecord {
                status: OperationStatus::InProgress { turn_seq },
                holder: waiting.holder,
                lane,
                free_text: waiting.free_text,
                revision,
            },
        );
        self.in_progress.insert(
            turn_seq,
            InProgressEntry {
                key,
                served_ms: now_ms,
                revision,
            },
        );
        let served = ServedInput {
            envelope,
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

    /// Ends a served input with the kind of its outcome: its seat has room again (and may serve its
    /// next free-text line), and a duplicate of it now learns the outcome. If the input was
    /// abandoned meanwhile (its lease ran out), the outcome is still recorded, once, as its result
    /// and a `finished_late` event says so, so a retry of the same id learns what happened and
    /// nothing runs twice. A seat that has left has no operations: `UnknownOperation`.
    pub fn finish(
        &mut self,
        sender: Sender,
        operation_id: OperationId,
        outcome: OutcomeKind,
    ) -> Result<(), FinishError> {
        let key = self
            .key_of(sender, operation_id)
            .ok_or(FinishError::UnknownOperation)?;
        let abandoned = self
            .operations
            .get_mut(&key)
            .filter(|record| matches!(record.status, OperationStatus::Abandoned { .. }));
        if let Some(record) = abandoned {
            let OperationStatus::Abandoned { turn_seq } = record.status else {
                return Err(FinishError::UnknownOperation);
            };
            record.status = OperationStatus::Finished { turn_seq, outcome };
            let revision = record.revision;
            self.record(TableEvent::InputFinishedLate {
                scope: self.scope(),
                correlation: Correlation {
                    operation_id,
                    turn_seq,
                    revision,
                },
                outcome,
            });
            return Ok(());
        }
        self.end_in_progress(key, Ending::Finished(outcome))
            .map(|_| ())
    }

    /// Gives up on a served input that will not finish, with the cause, and records it. It frees
    /// the seat exactly as `finish` does and a duplicate of it learns it was abandoned.
    pub fn abandon(
        &mut self,
        sender: Sender,
        operation_id: OperationId,
        cause: FailureCause,
        now_ms: u64,
    ) -> Result<(), FinishError> {
        let key = self
            .key_of(sender, operation_id)
            .ok_or(FinishError::UnknownOperation)?;
        self.abandon_key(key, cause, now_ms)
    }

    fn abandon_key(
        &mut self,
        key: OperationKey,
        cause: FailureCause,
        now_ms: u64,
    ) -> Result<(), FinishError> {
        let (turn_seq, entry) = self.end_in_progress(key, Ending::Abandoned)?;
        self.record(TableEvent::InputAbandoned {
            scope: self.scope(),
            correlation: Correlation {
                operation_id: key.2,
                turn_seq,
                revision: entry.revision,
            },
            cause,
            ms: now_ms.saturating_sub(entry.served_ms),
        });
        Ok(())
    }

    /// The inputs served and not ended, oldest first, with how long each has been running.
    pub fn in_progress(&self, now_ms: u64) -> Vec<InProgressOperation> {
        self.in_progress
            .iter()
            .map(|(turn_seq, entry)| in_progress_report(*turn_seq, entry, now_ms))
            .collect()
    }

    /// Abandons every input that has been in progress for the lease or longer, in serve order, with
    /// cause `timed_out`, and returns what it abandoned. It looks at every entry, so a clock that
    /// stepped back cannot hide an expired input behind a younger one. The watchdog calls it with
    /// the time.
    pub fn reap_expired(&mut self, now_ms: u64) -> Vec<InProgressOperation> {
        let lease_ms = self.limits.in_progress_lease_ms;
        let expired: Vec<InProgressOperation> = self
            .in_progress
            .iter()
            .map(|(turn_seq, entry)| in_progress_report(*turn_seq, entry, now_ms))
            .filter(|report| report.age_ms >= lease_ms)
            .collect();
        expired
            .into_iter()
            .filter(|report| {
                let key = (report.sender, report.membership, report.operation_id);
                self.abandon_key(key, FailureCause::TimedOut, now_ms)
                    .is_ok()
            })
            .collect()
    }

    /// The status of an operation of `sender`, if the queue knows it.
    pub fn status(&self, sender: Sender, operation_id: OperationId) -> Option<OperationStatus> {
        let key = self.key_of(sender, operation_id)?;
        self.operations.get(&key).map(|record| record.status)
    }

    /// How many inputs wait in a lane.
    pub fn depth(&self, lane: Lane) -> usize {
        match lane {
            Lane::SeatControls => self.controls_waiting,
            Lane::Engine => self.engine.waiting.len(),
            Lane::TimersAndResults => self.timers_and_results.waiting.len(),
            Lane::Player => self.lines_waiting,
        }
    }

    /// The player-lane inputs a seat has admitted and not ended (what `seat_pending_limit`
    /// bounds), or `None` when the seat is not at the table.
    pub fn seat_pending(&self, seat: SeatId) -> Option<usize> {
        self.seats.get(&seat).map(SeatState::pending)
    }

    /// The operations the queue remembers: waiting, in progress and ended. It never exceeds the
    /// remembered limit by more than the waiting and in-progress ones.
    pub fn remembered_operations(&self) -> usize {
        self.operations.len()
    }

    /// The `turn_seq` of the last input served; `NOT_YET_SERVED` before the first.
    pub fn last_turn_seq(&self) -> TurnSeq {
        self.last_turn_seq
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
        self.controls_waiting
            .saturating_add(self.engine.waiting.len())
            .saturating_add(self.timers_and_results.waiting.len())
            .saturating_add(self.lines_waiting)
    }

    fn next_turn_seq(&self) -> Result<TurnSeq, ServeError> {
        self.last_turn_seq
            .get()
            .checked_add(1)
            .map(TurnSeq::new)
            .ok_or(ServeError::TurnSequenceExhausted)
    }

    /// The key an operation of `sender` is remembered under now; a seat that is not at the table
    /// has none.
    fn key_of(&self, sender: Sender, operation_id: OperationId) -> Option<OperationKey> {
        match sender {
            Sender::Engine => Some((Sender::Engine, 0, operation_id)),
            Sender::Seat(seat) => self
                .seats
                .get(&seat)
                .map(|state| (sender, state.membership, operation_id)),
        }
    }

    /// Puts a seat into or out of each ring according to what it has waiting now.
    fn sync_rings(&mut self, seat: SeatId) {
        let (lines, controls) = self.seats.get(&seat).map_or((false, false), |state| {
            (state.lines_eligible(), !state.controls.is_empty())
        });
        self.line_ring.set_eligible(seat, lines);
        self.control_ring.set_eligible(seat, controls);
    }

    fn place_player(
        &mut self,
        seat: SeatId,
        route: PlayerRoute,
        envelope: InputEnvelope,
        now_ms: u64,
    ) -> Result<Admitted, AdmitError> {
        let pending_limit = self.seat_pending_limit;
        let controls_limit = self.limits.seat_controls_pending;
        let lane_limit = self.limits.controls_lane_length;
        let state = self
            .seats
            .get_mut(&seat)
            .ok_or(AdmitError::UnknownSeat { seat })?;
        let key = (
            Sender::Seat(seat),
            state.membership,
            envelope.operation_id(),
        );
        let earlier = self.operations.get(&key).map(|record| record.status);
        if let Some(first) = earlier {
            return Err(AdmitError::Duplicate { first });
        }
        let revision = envelope.view().revision();
        let holder = Some(SeatHold {
            seat,
            membership: state.membership,
        });
        let (lane, free_text) = match route {
            PlayerRoute::Line { free_text } => {
                let pending = state.pending();
                if pending >= pending_limit {
                    return Err(AdmitError::SeatBusy {
                        pending,
                        limit: pending_limit,
                    });
                }
                state
                    .bucket
                    .try_take(now_ms)
                    .map_err(|retry_after_ms| AdmitError::RateLimited { retry_after_ms })?;
                state.lines.push_back(Waiting {
                    envelope,
                    admitted_ms: now_ms,
                    holder,
                    free_text,
                });
                self.lines_waiting = self.lines_waiting.saturating_add(1);
                (Lane::Player, free_text)
            }
            PlayerRoute::Control => {
                if state.controls.len() >= controls_limit {
                    return Err(AdmitError::LaneStalled {
                        lane: Lane::SeatControls,
                        limit: controls_limit,
                    });
                }
                if self.controls_waiting >= lane_limit {
                    return Err(AdmitError::LaneStalled {
                        lane: Lane::SeatControls,
                        limit: lane_limit,
                    });
                }
                state.controls.push_back(Waiting {
                    envelope,
                    admitted_ms: now_ms,
                    holder,
                    free_text: false,
                });
                self.controls_waiting = self.controls_waiting.saturating_add(1);
                (Lane::SeatControls, false)
            }
        };
        self.sync_rings(seat);
        self.operations.insert(
            key,
            OperationRecord {
                status: OperationStatus::Queued { lane },
                holder,
                lane,
                free_text,
                revision,
            },
        );
        Ok(Admitted { lane })
    }

    fn place_engine(
        &mut self,
        engine_lane: EngineLane,
        envelope: InputEnvelope,
        now_ms: u64,
    ) -> Result<Admitted, AdmitError> {
        let key = (Sender::Engine, 0, envelope.operation_id());
        let revision = envelope.view().revision();
        let lane = engine_lane.lane();
        if let Some(record) = self.operations.get(&key) {
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
            free_text: false,
        });
        self.operations.insert(
            key,
            OperationRecord {
                status: OperationStatus::Queued { lane },
                holder: None,
                lane,
                free_text: false,
                revision,
            },
        );
        Ok(Admitted { lane })
    }

    fn pop_by_priority(&mut self) -> Option<(Lane, Waiting)> {
        if let Some(waiting) = self.pop_control() {
            return Some((Lane::SeatControls, waiting));
        }
        if let Some(waiting) = self.engine.waiting.pop_front() {
            return Some((Lane::Engine, waiting));
        }
        if let Some(waiting) = self.timers_and_results.waiting.pop_front() {
            return Some((Lane::TimersAndResults, waiting));
        }
        self.pop_line().map(|waiting| (Lane::Player, waiting))
    }

    /// The next control, from the seat after the one served last. A seat in the ring with nothing
    /// to serve is dropped from it and skipped, which cannot lose an input.
    fn pop_control(&mut self) -> Option<Waiting> {
        loop {
            let seat = self.control_ring.next()?;
            let waiting = self
                .seats
                .get_mut(&seat)
                .and_then(|state| state.controls.pop_front());
            let Some(waiting) = waiting else {
                self.control_ring.set_eligible(seat, false);
                continue;
            };
            self.control_ring.served(seat);
            self.controls_waiting = self.controls_waiting.saturating_sub(1);
            self.sync_rings(seat);
            return Some(waiting);
        }
    }

    fn pop_line(&mut self) -> Option<Waiting> {
        loop {
            let seat = self.line_ring.next()?;
            let waiting = self.seats.get_mut(&seat).and_then(SeatState::take_line);
            let Some(waiting) = waiting else {
                self.line_ring.set_eligible(seat, false);
                continue;
            };
            self.line_ring.served(seat);
            self.lines_waiting = self.lines_waiting.saturating_sub(1);
            self.sync_rings(seat);
            return Some(waiting);
        }
    }

    fn end_in_progress(
        &mut self,
        key: OperationKey,
        ending: Ending,
    ) -> Result<(TurnSeq, InProgressEntry), FinishError> {
        let record = self
            .operations
            .get_mut(&key)
            .ok_or(FinishError::UnknownOperation)?;
        let OperationStatus::InProgress { turn_seq } = record.status else {
            return Err(FinishError::NotInProgress {
                status: record.status,
            });
        };
        let entry = self
            .in_progress
            .remove(&turn_seq)
            .ok_or(FinishError::UnknownOperation)?;
        record.status = match ending {
            Ending::Finished(outcome) => OperationStatus::Finished { turn_seq, outcome },
            Ending::Abandoned => OperationStatus::Abandoned { turn_seq },
        };
        let (holder, lane, free_text) = (record.holder, record.lane, record.free_text);
        if let Some(hold) = holder {
            self.release_seat(hold, lane, free_text);
        }
        self.ended.push_back(key);
        self.forget_oldest_ended();
        Ok((turn_seq, entry))
    }

    /// Gives a seat back what an ended input held, unless the seat left and came back since.
    fn release_seat(&mut self, hold: SeatHold, lane: Lane, free_text: bool) {
        let Some(state) = self.seats.get_mut(&hold.seat) else {
            return;
        };
        if state.membership != hold.membership {
            return;
        }
        state.release(lane, free_text);
        self.sync_rings(hold.seat);
    }

    fn forget_oldest_ended(&mut self) {
        while self.ended.len() > self.limits.remembered_operations {
            let Some(oldest) = self.ended.pop_front() else {
                return;
            };
            self.operations.remove(&oldest);
        }
    }
}

fn in_progress_report(
    turn_seq: TurnSeq,
    entry: &InProgressEntry,
    now_ms: u64,
) -> InProgressOperation {
    InProgressOperation {
        sender: entry.key.0,
        membership: entry.key.1,
        operation_id: entry.key.2,
        turn_seq,
        age_ms: now_ms.saturating_sub(entry.served_ms),
    }
}
