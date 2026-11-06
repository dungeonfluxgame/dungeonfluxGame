//! The time-driven and epoch-driven endings of a request: deadlines, leases, epoch advances,
//! cancellation and closing the broker.
//!
//! Each ends a request or puts it back in its queue, abandons the running attempt it held, and
//! records the change through `BrokerCore::record`. They are the parts of the scheduler that
//! start from the clock or from the table's scene rather than from a worker's call.

use df_table::event::FailureCause;
use df_table::ports::ResourceHandle;

use crate::broker::events::{AbandonReason, BrokerEvent, StaleReason};
use crate::broker::ledger::LedgerEnd;
use crate::broker::limits::MAXIMUM_LEASE_ATTEMPTS;
use crate::broker::request::{AttemptId, Epoch, EpochError, RequestStatus};

use super::{BrokerCore, CancelOutcome};

impl BrokerCore {
    /// Enforces the deadlines and the leases. Call it with the time as often as a deadline should
    /// be noticed; it does bounded work over the active requests. `claim` and `complete` enforce
    /// the deadline themselves, so a request is never started or accepted past it.
    pub fn tick(&mut self, now_ms: u64) {
        self.enforce_deadlines(now_ms);
        self.enforce_leases(now_ms);
    }

    /// Fails every active request whose deadline has passed.
    pub(super) fn enforce_deadlines(&mut self, now_ms: u64) {
        let overdue: Vec<ResourceHandle> = self
            .active
            .iter()
            .filter(|handle| {
                self.requests
                    .get(handle)
                    .is_some_and(|request| request.deadline_ms <= now_ms)
            })
            .map(|handle| ResourceHandle::new(*handle))
            .collect();
        for handle in overdue {
            self.time_out(handle, now_ms);
        }
    }

    fn enforce_leases(&mut self, now_ms: u64) {
        let expired: Vec<AttemptId> = self
            .in_flight
            .iter()
            .filter(|(_, attempt)| attempt.lease_expires_ms <= now_ms)
            .map(|(id, _)| *id)
            .collect();
        for attempt in expired {
            self.expire_lease(attempt, now_ms);
        }
    }

    fn time_out(&mut self, handle: ResourceHandle, now_ms: u64) {
        let Some(request) = self.requests.get(&handle.get()) else {
            return;
        };
        let class = request.spec.class();
        let mut unknown_dispatch = false;
        if let RequestStatus::InFlight { attempt } = request.status {
            unknown_dispatch = self
                .abandon(attempt, AbandonReason::DeadlineExceeded)
                .is_some_and(|abandoned| abandoned.sent);
        } else {
            self.queues.remove(class, handle);
        }
        self.fail(handle, FailureCause::TimedOut, now_ms, unknown_dispatch);
    }

    fn expire_lease(&mut self, attempt: AttemptId, now_ms: u64) {
        let Some(abandoned) = self.abandon(attempt, AbandonReason::LeaseExpired) else {
            return;
        };
        let handle = abandoned.handle;
        let Some(request) = self.requests.get_mut(&handle.get()) else {
            return;
        };
        let requeue = !abandoned.sent && request.lease_requeues + 1 < MAXIMUM_LEASE_ATTEMPTS;
        let class = request.spec.class();
        if requeue {
            request.lease_requeues += 1;
            request.status = RequestStatus::Queued { class };
            request.claimable_ms = now_ms;
        }
        self.record(BrokerEvent::LeaseExpired {
            hub: self.hub,
            handle,
            attempt,
            sent: abandoned.sent,
            requeued: requeue,
        });
        if requeue {
            self.queues.push_front(class, handle);
        } else if abandoned.sent {
            self.fail(handle, FailureCause::TimedOut, now_ms, true);
        } else {
            self.fail(handle, FailureCause::Unavailable, now_ms, false);
        }
    }

    /// Moves the scene epoch forward and supersedes every request for an earlier one: its attempt
    /// is abandoned and any answer that arrives for it is rejected.
    pub fn advance_epoch(&mut self, to: Epoch) -> Result<usize, EpochError> {
        if to <= self.epoch {
            return Err(EpochError::NotForward {
                current: self.epoch,
                requested: to,
            });
        }
        let from = self.epoch;
        self.epoch = to;
        let stale: Vec<ResourceHandle> = self
            .active
            .iter()
            .filter(|handle| {
                self.requests
                    .get(handle)
                    .is_some_and(|request| request.epoch < to)
            })
            .map(|handle| ResourceHandle::new(*handle))
            .collect();
        for handle in &stale {
            self.supersede(*handle);
        }
        self.record(BrokerEvent::EpochAdvanced {
            hub: self.hub,
            from,
            to,
            superseded: stale.len(),
        });
        Ok(stale.len())
    }

    pub(super) fn supersede(&mut self, handle: ResourceHandle) {
        let Some(request) = self.requests.get(&handle.get()) else {
            return;
        };
        let (class, purpose, epoch) = (
            request.spec.class(),
            request.spec.name().clone(),
            request.epoch,
        );
        if let RequestStatus::InFlight { attempt } = request.status {
            self.abandon(attempt, AbandonReason::EpochAdvanced);
        } else {
            self.queues.remove(class, handle);
        }
        self.settle(
            handle,
            RequestStatus::Superseded,
            LedgerEnd::Superseded,
            true,
        );
        self.record(BrokerEvent::Stale {
            hub: self.hub,
            handle: Some(handle),
            purpose,
            epoch,
            current: self.epoch,
            reason: StaleReason::EpochAdvanced,
        });
    }

    /// Gives up one holder's interest in a request. The flight is cancelled when the last holder
    /// gives up: a waiting request leaves its queue and a running attempt is abandoned.
    pub fn cancel(&mut self, handle: ResourceHandle) -> CancelOutcome {
        let Some(request) = self.requests.get_mut(&handle.get()) else {
            return CancelOutcome::NotActive;
        };
        if request.status.is_settled() {
            return CancelOutcome::NotActive;
        }
        request.holders = request.holders.saturating_sub(1);
        if request.holders > 0 {
            return CancelOutcome::StillHeld {
                holders: request.holders,
            };
        }
        self.cancel_flight(handle);
        CancelOutcome::Cancelled
    }

    fn cancel_flight(&mut self, handle: ResourceHandle) {
        let Some(request) = self.requests.get(&handle.get()) else {
            return;
        };
        let (class, purpose) = (request.spec.class(), request.spec.name().clone());
        if let RequestStatus::InFlight { attempt } = request.status {
            self.abandon(attempt, AbandonReason::Cancelled);
        } else {
            self.queues.remove(class, handle);
        }
        self.settle(
            handle,
            RequestStatus::Cancelled,
            LedgerEnd::Cancelled,
            false,
        );
        self.record(BrokerEvent::Cancelled {
            hub: self.hub,
            handle,
            purpose,
        });
    }

    /// Closes the broker with its table: every request is cancelled whoever holds it, and no new
    /// one is accepted. Returns how many were cancelled.
    pub fn close(&mut self) -> usize {
        self.closed = true;
        let open: Vec<ResourceHandle> = self
            .active
            .iter()
            .map(|handle| ResourceHandle::new(*handle))
            .collect();
        for handle in &open {
            self.cancel_flight(*handle);
        }
        self.record(BrokerEvent::Closed {
            hub: self.hub,
            cancelled: open.len(),
        });
        open.len()
    }
}
