//! The shared handle to one table's broker, and the `Broker` port over it.
//!
//! The owner thread and the worker that runs provider calls both reach the scheduler through the
//! hub. Every call holds the lock for a bounded amount of in-memory work and none holds it across
//! I/O, an `.await` or a provider call, so a request from the owner thread never waits for a
//! provider: it takes the lock for a moment, queues, and returns a handle. The hub reads the time
//! from the injected `Clock`, so tests and the dev engine can drive it with a manual one.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use df_table::event::FailureCause;
use df_table::identity::Correlation;
use df_table::ports::{Broker, Clock, PortError, PortName, PurposeName, Readiness, ResourceHandle};

use super::events::BrokerAudit;
use super::purpose::{PurposeTable, ResourceKind};
use super::report::{LedgerReport, QueuesReport};
use super::request::{
    AttemptId, Completion, Epoch, EpochError, NewRequest, ProviderFault, RequestError, RequestKey,
    RequestStatus, ResourceResult, ResultError, WorkerId,
};
use super::scheduler::{
    BrokerCore, BrokerLimits, CancelOutcome, ClaimedCall, CompleteOutcome, UnknownAttempt,
};

/// The broker of one table, shared by its owner and its worker.
pub struct BrokerHub {
    core: Mutex<BrokerCore>,
    clock: Arc<dyn Clock>,
}

impl BrokerHub {
    pub fn new(
        purposes: PurposeTable,
        limits: BrokerLimits,
        audit: Arc<dyn BrokerAudit>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            core: Mutex::new(BrokerCore::new(purposes, limits, audit)),
            clock,
        }
    }

    fn core(&self) -> MutexGuard<'_, BrokerCore> {
        self.core.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Asks for a resource with its own key, for a caller that knows the inputs (H.8).
    pub fn request_keyed(&self, new: NewRequest) -> Result<ResourceHandle, RequestError> {
        self.core().request(new, self.now_ms())
    }

    /// Whether the broker has a purpose of this name.
    pub fn knows_purpose(&self, purpose: &PurposeName) -> bool {
        self.core().purposes().get(purpose).is_some()
    }

    pub fn claim(&self, kind: ResourceKind, worker: WorkerId) -> Option<ClaimedCall> {
        self.core().claim(kind, worker, self.now_ms())
    }

    pub fn mark_sent(&self, attempt: AttemptId) -> Result<(), UnknownAttempt> {
        self.core().mark_sent(attempt)
    }

    pub fn heartbeat(&self, attempt: AttemptId) -> bool {
        self.core().heartbeat(attempt, self.now_ms())
    }

    pub fn complete(
        &self,
        attempt: AttemptId,
        answer: Result<ResourceResult, ProviderFault>,
    ) -> CompleteOutcome {
        self.core().complete(attempt, answer, self.now_ms())
    }

    /// Enforces deadlines and leases at the clock's time.
    pub fn tick(&self) {
        self.core().tick(self.now_ms());
    }

    pub fn advance_epoch(&self, to: Epoch) -> Result<usize, EpochError> {
        self.core().advance_epoch(to)
    }

    pub fn epoch(&self) -> Epoch {
        self.core().epoch()
    }

    pub fn cancel(&self, handle: ResourceHandle) -> CancelOutcome {
        self.core().cancel(handle)
    }

    /// Cancels everything and refuses new requests: the table is closing.
    pub fn close(&self) -> usize {
        self.core().close()
    }

    pub fn status(&self, handle: ResourceHandle) -> Option<RequestStatus> {
        self.core().status(handle)
    }

    pub fn take_result(&self, handle: ResourceHandle) -> Result<ResourceResult, ResultError> {
        self.core().take_result(handle)
    }

    pub fn take_cancellations(&self) -> Vec<AttemptId> {
        self.core().take_cancellations()
    }

    pub fn take_completions(&self) -> Vec<Completion> {
        self.core().take_completions()
    }

    pub fn queues_report(&self) -> QueuesReport {
        self.core().queues_report(self.now_ms())
    }

    pub fn ledger_report(&self, newest: usize) -> LedgerReport {
        self.core().ledger_report(newest)
    }
}

impl Broker for BrokerHub {
    /// The port knows only the purpose and the input it serves, so the key is made from those and
    /// the table's current epoch.
    fn request(
        &self,
        correlation: Correlation,
        purpose: &PurposeName,
    ) -> Result<ResourceHandle, PortError> {
        let mut core = self.core();
        let epoch = core.epoch();
        let new = NewRequest {
            purpose: purpose.clone(),
            key: RequestKey::of_operation(purpose, &correlation, epoch),
            epoch,
            correlation,
        };
        core.request(new, self.now_ms()).map_err(|error| {
            let cause = match error {
                RequestError::StaleEpoch { .. } => FailureCause::Stale,
                RequestError::UnknownPurpose { .. }
                | RequestError::QueueFull { .. }
                | RequestError::Closed => FailureCause::Unavailable,
            };
            PortError::new(PortName::Broker, cause)
        })
    }

    fn readiness(
        &self,
        _correlation: Correlation,
        handle: ResourceHandle,
    ) -> Result<Readiness, PortError> {
        self.core()
            .readiness(handle)
            .ok_or(PortError::new(PortName::Broker, FailureCause::Stale))
    }
}
