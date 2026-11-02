//! The broker's scheduler: queues, leases, epochs, deadlines, retries, preemption and the ledger.
//!
//! It is a deterministic state machine over an injected `now_ms`, like the owner queue: it spawns
//! nothing, sleeps nothing, awaits nothing and calls no provider (the hub does not spawn; the
//! worker that owns the provider calls pulls work with `claim`). A request returns a handle at
//! once and settles later; readiness is read with `status` or pushed with `take_completions`, so
//! nothing on the owner's side ever waits for a provider.
//!
//! The rules, each pinned by a test:
//! - Classes are served live, plate, look-ahead, background. A request of a higher class that
//!   finds the provider slots of its kind full preempts the lowest-class attempt running; the
//!   victim goes back to the front of its queue (its deadline keeps running) and the provider
//!   call is the worker's to cancel. Non-live requests never take the slot reserved for live.
//! - A request carries the scene epoch it is for. A request for an epoch that has passed is
//!   refused; when the epoch advances every request for an earlier epoch is superseded, its
//!   attempt is abandoned, and a result that arrives for it is rejected. A request for a later
//!   epoch is how look-ahead builds the next scene before the table gets there.
//! - Every request has a deadline from its purpose. At the deadline it fails `timed_out`
//!   whether it waits, backs off or runs; a call that was sent and not answered is an unknown
//!   dispatch, so it is never retried and its spend stays recorded.
//! - A failure the provider reports is retried up to the purpose's retries with a doubling backoff
//!   plus a deterministic jitter, never for a refusal and never past the deadline.
//! - An attempt holds a lease (`LEASE_MS`, renewed by `heartbeat`). An expired lease puts a call
//!   that was never sent back in its queue (up to three attempts) and fails one that may have been
//!   sent (H.9).
//! - A second request for a key already in flight joins it and shares its handle; the flight
//!   is cancelled only when every holder has cancelled.
//! - The ledger records every request and every call sent and never refuses anything.
//!
//! It records every change through `BrokerAudit`, in one place (`record`).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use df_table::event::FailureCause;
use df_table::identity::Correlation;
use df_table::ports::{PurposeName, ResourceHandle};

mod lifecycle;
mod reads;

use super::events::{AbandonReason, BrokerAudit, BrokerEvent, RefusalKind, StaleReason};
use super::ledger::{CostLedger, LedgerEnd};
use super::limits::{
    DEFAULT_IMAGE_SLOTS, DEFAULT_TEXT_SLOTS, LEASE_MS, MAXIMUM_KEPT_SETTLED,
    MAXIMUM_PENDING_COMPLETIONS, MAXIMUM_REMEMBERED_ABANDONED, RESERVED_LIVE_TEXT_SLOTS,
};
use super::purpose::{PurposeSpec, PurposeTable, QueueClass, ResourceKind, SizeRule};
use super::queues::{ClassQueues, Pushed, QueueBounds};
use super::request::{
    AttemptId, Completion, Epoch, NewRequest, ProviderFault, RequestError, RequestKey,
    RequestStatus, ResourceResult, WorkerId,
};

/// How many providers may run at once, and how many requests each class may keep waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokerLimits {
    pub text_slots: usize,
    pub image_slots: usize,
    pub reserved_live_text_slots: usize,
    pub queues: QueueBounds,
}

impl Default for BrokerLimits {
    fn default() -> Self {
        Self {
            text_slots: DEFAULT_TEXT_SLOTS,
            image_slots: DEFAULT_IMAGE_SLOTS,
            reserved_live_text_slots: RESERVED_LIVE_TEXT_SLOTS,
            queues: QueueBounds::default(),
        }
    }
}

impl BrokerLimits {
    fn slots(self, kind: ResourceKind) -> usize {
        match kind {
            ResourceKind::Text => self.text_slots,
            ResourceKind::Image => self.image_slots,
        }
    }

    fn reserved_for_live(self, kind: ResourceKind) -> usize {
        match kind {
            ResourceKind::Text => self.reserved_live_text_slots,
            ResourceKind::Image => 0,
        }
    }
}

/// The work a provider call is for, handed to the worker that claimed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedCall {
    pub attempt: AttemptId,
    pub handle: ResourceHandle,
    pub purpose: PurposeName,
    pub kind: ResourceKind,
    pub size: SizeRule,
    pub key: RequestKey,
    pub epoch: Epoch,
    pub correlation: Correlation,
    /// 1 for the first call of the request.
    pub number: u32,
}

/// What `complete` did with a provider's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteOutcome {
    Accepted,
    /// The attempt had been abandoned, so the answer is discarded.
    Rejected {
        reason: AbandonReason,
    },
    /// The attempt is not known (never claimed, or forgotten).
    Unknown,
}

/// What `cancel` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    Cancelled,
    /// Other holders of the same flight still want it.
    StillHeld {
        holders: u32,
    },
    /// The request is unknown or had already settled.
    NotActive,
}

/// Why an attempt could not be marked sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownAttempt;

#[derive(Debug)]
struct Request {
    spec: PurposeSpec,
    key: RequestKey,
    epoch: Epoch,
    correlation: Correlation,
    requested_ms: u64,
    deadline_ms: u64,
    status: RequestStatus,
    retries: u8,
    attempts: u32,
    lease_requeues: u8,
    holders: u32,
    result: Option<ResourceResult>,
}

#[derive(Debug, Clone, Copy)]
struct Attempt {
    handle: ResourceHandle,
    kind: ResourceKind,
    class: QueueClass,
    lease_expires_ms: u64,
    sent: bool,
    started_ms: u64,
}

#[derive(Debug, Default)]
struct AbandonedAttempts {
    order: VecDeque<AttemptId>,
    reasons: HashMap<AttemptId, (AbandonReason, ResourceHandle)>,
}

impl AbandonedAttempts {
    fn remember(&mut self, attempt: AttemptId, reason: AbandonReason, handle: ResourceHandle) {
        self.order.push_back(attempt);
        self.reasons.insert(attempt, (reason, handle));
        while self.order.len() > MAXIMUM_REMEMBERED_ABANDONED {
            if let Some(oldest) = self.order.pop_front() {
                self.reasons.remove(&oldest);
            }
        }
    }
}

/// The scheduler of one table's resource requests.
pub struct BrokerCore {
    purposes: PurposeTable,
    limits: BrokerLimits,
    audit: Arc<dyn BrokerAudit>,
    epoch: Epoch,
    closed: bool,
    next_handle: u64,
    next_attempt: u64,
    requests: BTreeMap<u64, Request>,
    active: BTreeSet<u64>,
    flights: HashMap<RequestKey, ResourceHandle>,
    queues: ClassQueues,
    in_flight: BTreeMap<AttemptId, Attempt>,
    abandoned: AbandonedAttempts,
    to_cancel: VecDeque<AttemptId>,
    settled_order: VecDeque<u64>,
    completions: VecDeque<Completion>,
    dropped_completions: u64,
    ledger: CostLedger,
}

impl BrokerCore {
    pub fn new(purposes: PurposeTable, limits: BrokerLimits, audit: Arc<dyn BrokerAudit>) -> Self {
        Self {
            purposes,
            limits,
            audit,
            epoch: Epoch::FIRST,
            closed: false,
            next_handle: 0,
            next_attempt: 0,
            requests: BTreeMap::new(),
            active: BTreeSet::new(),
            flights: HashMap::new(),
            queues: ClassQueues::new(limits.queues),
            in_flight: BTreeMap::new(),
            abandoned: AbandonedAttempts::default(),
            to_cancel: VecDeque::new(),
            settled_order: VecDeque::new(),
            completions: VecDeque::new(),
            dropped_completions: 0,
            ledger: CostLedger::default(),
        }
    }

    /// Every broker change goes through here: the one place a request's life is recorded.
    fn record(&self, event: BrokerEvent) {
        self.audit.record(&event);
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn purposes(&self) -> &PurposeTable {
        &self.purposes
    }

    pub fn limits(&self) -> BrokerLimits {
        self.limits
    }

    pub fn ledger(&self) -> &CostLedger {
        &self.ledger
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Asks for a resource. It never waits: the request is queued and its handle returned. A
    /// request for a key already in flight joins that flight and returns its handle.
    pub fn request(
        &mut self,
        new: NewRequest,
        now_ms: u64,
    ) -> Result<ResourceHandle, RequestError> {
        if self.closed {
            self.record(BrokerEvent::Refused {
                purpose: new.purpose,
                kind: RefusalKind::Closed,
            });
            return Err(RequestError::Closed);
        }
        let Some(spec) = self.purposes.get(&new.purpose).cloned() else {
            self.record(BrokerEvent::Refused {
                purpose: new.purpose.clone(),
                kind: RefusalKind::UnknownPurpose,
            });
            return Err(RequestError::UnknownPurpose {
                purpose: new.purpose,
            });
        };
        if new.epoch < self.epoch {
            self.record(BrokerEvent::Stale {
                handle: None,
                purpose: new.purpose,
                epoch: new.epoch,
                current: self.epoch,
                reason: StaleReason::RefusedAtRequest,
            });
            return Err(RequestError::StaleEpoch {
                requested: new.epoch,
                current: self.epoch,
            });
        }
        if let Some(joined) = self.join_flight(&new) {
            return Ok(joined);
        }
        let class = spec.class();
        if self.queues.refuses(class) {
            self.record(BrokerEvent::Refused {
                purpose: new.purpose,
                kind: RefusalKind::QueueFull,
            });
            return Err(RequestError::QueueFull { class });
        }
        self.next_handle += 1;
        let handle = ResourceHandle::new(self.next_handle);
        if let Ok(Pushed::DroppedOldest(oldest)) = self.queues.push_back(class, handle) {
            self.drop_waiting(oldest, class);
        }
        self.ledger
            .record_request(handle, &new.purpose, class, spec.price(), now_ms);
        self.record(BrokerEvent::Requested {
            handle,
            purpose: new.purpose.clone(),
            class,
            epoch: new.epoch,
            key: new.key,
            operation: new.correlation.operation_id,
        });
        self.requests.insert(
            handle.get(),
            Request {
                deadline_ms: now_ms.saturating_add(spec.deadline_ms()),
                spec,
                key: new.key,
                epoch: new.epoch,
                correlation: new.correlation,
                requested_ms: now_ms,
                status: RequestStatus::Queued { class },
                retries: 0,
                attempts: 0,
                lease_requeues: 0,
                holders: 1,
                result: None,
            },
        );
        self.active.insert(handle.get());
        self.flights.insert(new.key, handle);
        Ok(handle)
    }

    fn join_flight(&mut self, new: &NewRequest) -> Option<ResourceHandle> {
        let handle = *self.flights.get(&new.key)?;
        let request = self.requests.get_mut(&handle.get())?;
        if request.status.is_settled() {
            return None;
        }
        request.holders += 1;
        let holders = request.holders;
        self.record(BrokerEvent::Joined {
            handle,
            purpose: new.purpose.clone(),
            holders,
        });
        Some(handle)
    }

    fn drop_waiting(&mut self, handle: ResourceHandle, class: QueueClass) {
        let Some(purpose) = self
            .requests
            .get(&handle.get())
            .map(|request| request.spec.name().clone())
        else {
            return;
        };
        self.record(BrokerEvent::Dropped {
            handle,
            purpose,
            class,
        });
        self.settle(handle, RequestStatus::Dropped, LedgerEnd::Dropped, true);
    }

    /// The next call a worker should make for `kind`, or `None` when nothing waits or the
    /// provider slots are full of work at least as important. A higher class finding the slots
    /// full preempts the lowest-class attempt running.
    pub fn claim(
        &mut self,
        kind: ResourceKind,
        worker: WorkerId,
        now_ms: u64,
    ) -> Option<ClaimedCall> {
        self.enforce_deadlines(now_ms);
        let (class, handle) = self.best_waiting(kind, now_ms)?;
        if !self.has_room(kind, class) {
            let victim = self.preemption_victim(kind, class)?;
            self.preempt(victim, handle);
        }
        self.queues.remove(class, handle);
        self.next_attempt += 1;
        let attempt = AttemptId::new(self.next_attempt);
        let request = self.requests.get_mut(&handle.get())?;
        request.attempts += 1;
        request.status = RequestStatus::InFlight { attempt };
        let number = request.attempts;
        self.in_flight.insert(
            attempt,
            Attempt {
                handle,
                kind,
                class,
                lease_expires_ms: now_ms.saturating_add(LEASE_MS),
                sent: false,
                started_ms: now_ms,
            },
        );
        let request = self.requests.get(&handle.get())?;
        let call = ClaimedCall {
            attempt,
            handle,
            purpose: request.spec.name().clone(),
            kind,
            size: request.spec.size(),
            key: request.key,
            epoch: request.epoch,
            correlation: request.correlation,
            number,
        };
        self.record(BrokerEvent::Claimed {
            handle,
            purpose: call.purpose.clone(),
            attempt,
            worker,
            number,
        });
        Some(call)
    }

    /// The highest-class request of `kind` that may start now, oldest first within its class.
    fn best_waiting(
        &self,
        kind: ResourceKind,
        now_ms: u64,
    ) -> Option<(QueueClass, ResourceHandle)> {
        QueueClass::ALL.iter().find_map(|class| {
            self.queues
                .iter(*class)
                .find(|handle| self.may_start(*handle, kind, now_ms))
                .map(|handle| (*class, handle))
        })
    }

    fn may_start(&self, handle: ResourceHandle, kind: ResourceKind, now_ms: u64) -> bool {
        self.requests.get(&handle.get()).is_some_and(|request| {
            request.spec.kind() == kind
                && match request.status {
                    RequestStatus::Queued { .. } => true,
                    RequestStatus::BackingOff { until_ms } => until_ms <= now_ms,
                    _ => false,
                }
        })
    }

    /// Whether a request of `class` may take a provider slot of `kind` without preempting.
    fn has_room(&self, kind: ResourceKind, class: QueueClass) -> bool {
        let running = self.in_flight.values().filter(|a| a.kind == kind);
        let total = running.clone().count();
        if total >= self.limits.slots(kind) {
            return false;
        }
        if class == QueueClass::Live {
            return true;
        }
        let not_live = running.filter(|a| a.class != QueueClass::Live).count();
        let allowed = self
            .limits
            .slots(kind)
            .saturating_sub(self.limits.reserved_for_live(kind));
        not_live < allowed
    }

    /// The attempt to preempt for a waiting request of `class`: the lowest class running, and the
    /// newest of it. Nothing of the same or a higher class is ever preempted.
    fn preemption_victim(&self, kind: ResourceKind, class: QueueClass) -> Option<AttemptId> {
        self.in_flight
            .iter()
            .filter(|(_, attempt)| attempt.kind == kind && attempt.class.rank() > class.rank())
            .max_by_key(|(id, attempt)| (attempt.class.rank(), attempt.started_ms, **id))
            .map(|(id, _)| *id)
    }

    fn preempt(&mut self, victim: AttemptId, by: ResourceHandle) {
        let Some(attempt) = self.abandon(victim, AbandonReason::Preempted) else {
            return;
        };
        let Some(request) = self.requests.get_mut(&attempt.handle.get()) else {
            return;
        };
        let class = request.spec.class();
        let purpose = request.spec.name().clone();
        request.status = RequestStatus::Queued { class };
        self.queues.push_front(class, attempt.handle);
        self.record(BrokerEvent::Preempted {
            handle: attempt.handle,
            purpose,
            by,
        });
    }

    /// Takes an attempt out of the running set and remembers why, so its answer is rejected.
    fn abandon(&mut self, attempt: AttemptId, reason: AbandonReason) -> Option<Attempt> {
        let removed = self.in_flight.remove(&attempt)?;
        self.abandoned.remember(attempt, reason, removed.handle);
        self.to_cancel.push_back(attempt);
        while self.to_cancel.len() > MAXIMUM_REMEMBERED_ABANDONED {
            self.to_cancel.pop_front();
        }
        self.record(BrokerEvent::Abandoned {
            handle: removed.handle,
            attempt,
            reason,
        });
        Some(removed)
    }

    /// Says the worker is about to send the provider call: from here its price is spent and never
    /// refunded, and the attempt counts as dispatched. Saying it twice charges once.
    pub fn mark_sent(&mut self, attempt: AttemptId) -> Result<(), UnknownAttempt> {
        let running = self.in_flight.get_mut(&attempt).ok_or(UnknownAttempt)?;
        if running.sent {
            return Ok(());
        }
        running.sent = true;
        let handle = running.handle;
        let request = self.requests.get(&handle.get()).ok_or(UnknownAttempt)?;
        let (purpose, price) = (request.spec.name().clone(), request.spec.price());
        self.ledger.record_call_sent(handle, &purpose, price);
        self.record(BrokerEvent::Spent {
            handle,
            purpose,
            attempt,
            price,
        });
        Ok(())
    }

    /// Renews the lease of a running attempt. `false` when it is no longer running.
    pub fn heartbeat(&mut self, attempt: AttemptId, now_ms: u64) -> bool {
        match self.in_flight.get_mut(&attempt) {
            Some(running) => {
                running.lease_expires_ms = now_ms.saturating_add(LEASE_MS);
                true
            }
            None => false,
        }
    }

    /// Takes a provider's answer. An answer for an attempt that was abandoned (preempted,
    /// cancelled, superseded, timed out) is rejected and changes nothing.
    pub fn complete(
        &mut self,
        attempt: AttemptId,
        answer: Result<ResourceResult, ProviderFault>,
        now_ms: u64,
    ) -> CompleteOutcome {
        let Some(running) = self.in_flight.remove(&attempt) else {
            return self.reject_late(attempt);
        };
        let Some((epoch, size, deadline_ms)) = self
            .requests
            .get(&running.handle.get())
            .map(|request| (request.epoch, request.spec.size(), request.deadline_ms))
        else {
            return CompleteOutcome::Unknown;
        };
        if now_ms >= deadline_ms {
            self.fail(running.handle, FailureCause::TimedOut, now_ms, running.sent);
            return CompleteOutcome::Rejected {
                reason: AbandonReason::DeadlineExceeded,
            };
        }
        if epoch < self.epoch {
            self.abandoned
                .remember(attempt, AbandonReason::EpochAdvanced, running.handle);
            self.supersede(running.handle);
            return CompleteOutcome::Rejected {
                reason: AbandonReason::EpochAdvanced,
            };
        }
        let answer = answer.and_then(|result| check_size(size, result));
        match answer {
            Ok(result) => self.finish_ready(running.handle, result, now_ms),
            Err(fault) => self.after_failure(running.handle, fault, now_ms),
        }
        CompleteOutcome::Accepted
    }

    fn reject_late(&mut self, attempt: AttemptId) -> CompleteOutcome {
        let Some((reason, handle)) = self.abandoned.reasons.get(&attempt).copied() else {
            return CompleteOutcome::Unknown;
        };
        if let Some(request) = self.requests.get(&handle.get()) {
            self.record(BrokerEvent::Stale {
                handle: Some(handle),
                purpose: request.spec.name().clone(),
                epoch: request.epoch,
                current: self.epoch,
                reason: StaleReason::LateResult,
            });
        }
        CompleteOutcome::Rejected { reason }
    }

    fn finish_ready(&mut self, handle: ResourceHandle, result: ResourceResult, now_ms: u64) {
        let Some(request) = self.requests.get_mut(&handle.get()) else {
            return;
        };
        request.result = Some(result);
        let (purpose, ms) = (
            request.spec.name().clone(),
            now_ms.saturating_sub(request.requested_ms),
        );
        self.settle(handle, RequestStatus::Ready, LedgerEnd::Ready, true);
        self.record(BrokerEvent::Ready {
            handle,
            purpose,
            ms,
        });
    }

    fn after_failure(&mut self, handle: ResourceHandle, fault: ProviderFault, now_ms: u64) {
        let Some(request) = self.requests.get_mut(&handle.get()) else {
            return;
        };
        let retry = request.retries.saturating_add(1);
        let backoff_ms =
            jittered_backoff(request.key, request.spec.retry().backoff_ms(retry), retry);
        let may_retry = fault.is_retryable()
            && request.retries < request.spec.retry().maximum_retries
            && now_ms.saturating_add(backoff_ms) < request.deadline_ms;
        if !may_retry {
            self.fail(handle, fault.cause(), now_ms, false);
            return;
        }
        request.retries = retry;
        request.status = RequestStatus::BackingOff {
            until_ms: now_ms.saturating_add(backoff_ms),
        };
        let (class, purpose) = (request.spec.class(), request.spec.name().clone());
        self.queues.push_front(class, handle);
        self.record(BrokerEvent::Retrying {
            handle,
            purpose,
            retry,
            backoff_ms,
            cause: fault.cause(),
        });
    }

    fn fail(
        &mut self,
        handle: ResourceHandle,
        cause: FailureCause,
        now_ms: u64,
        unknown_dispatch: bool,
    ) {
        let Some(request) = self.requests.get(&handle.get()) else {
            return;
        };
        let (purpose, ms) = (
            request.spec.name().clone(),
            now_ms.saturating_sub(request.requested_ms),
        );
        self.settle(
            handle,
            RequestStatus::Failed { cause },
            LedgerEnd::Failed { cause },
            true,
        );
        self.record(BrokerEvent::Failed {
            handle,
            purpose,
            cause,
            ms,
            unknown_dispatch,
        });
    }

    /// Ends a request: it leaves the active set and its flight, the ledger notes how it ended,
    /// and its owner is told when `notify` is set. The oldest settled requests are forgotten past
    /// `MAXIMUM_KEPT_SETTLED`.
    fn settle(
        &mut self,
        handle: ResourceHandle,
        status: RequestStatus,
        end: LedgerEnd,
        notify: bool,
    ) {
        let Some(request) = self.requests.get_mut(&handle.get()) else {
            return;
        };
        request.status = status;
        let key = request.key;
        self.active.remove(&handle.get());
        if self.flights.get(&key) == Some(&handle) {
            self.flights.remove(&key);
        }
        self.ledger.record_end(handle, end);
        if notify {
            self.completions.push_back(Completion {
                handle,
                readiness: status.readiness(),
            });
            while self.completions.len() > MAXIMUM_PENDING_COMPLETIONS {
                self.completions.pop_front();
                self.dropped_completions += 1;
            }
        }
        self.settled_order.push_back(handle.get());
        while self.settled_order.len() > MAXIMUM_KEPT_SETTLED {
            if let Some(oldest) = self.settled_order.pop_front() {
                self.requests.remove(&oldest);
            }
        }
    }
}

/// Rejects an answer outside the purpose's size rule or of the wrong kind.
fn check_size(rule: SizeRule, result: ResourceResult) -> Result<ResourceResult, ProviderFault> {
    let fits = match (&result, rule) {
        (ResourceResult::Text(text), SizeRule::TextCharacters { maximum }) => {
            let length = text.expose().chars().count();
            length > 0 && u32::try_from(length).is_ok_and(|length| length <= maximum)
        }
        (
            ResourceResult::Image { width, height, .. },
            SizeRule::ImagePixels {
                width: w,
                height: h,
            },
        ) => *width > 0 && *height > 0 && *width <= w && *height <= h,
        _ => false,
    };
    if fits {
        Ok(result)
    } else {
        Err(ProviderFault::InvalidAnswer)
    }
}

/// The backoff plus a jitter of up to a quarter of it, taken from the request key and the retry
/// number so a replay waits the same time.
fn jittered_backoff(key: RequestKey, backoff_ms: u64, retry: u8) -> u64 {
    let mut mixed = key.get() ^ u64::from(retry).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^= mixed >> 31;
    backoff_ms.saturating_add(mixed % (backoff_ms / 4).saturating_add(1))
}
