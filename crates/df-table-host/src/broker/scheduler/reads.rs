//! What a request's owner reads from the scheduler: statuses, results, completions and the
//! accessors the reports use.
//!
//! Nothing here changes what the scheduler decides. `take_result`, `take_completions` and
//! `take_cancellations` only hand over what has settled, once.

use df_table::ports::{PurposeName, Readiness, ResourceHandle};

use crate::broker::purpose::QueueClass;
use crate::broker::request::{AttemptId, Completion, RequestStatus, ResourceResult, ResultError};

use super::BrokerCore;

/// One attempt that holds a lease, as the reports read it.
#[derive(Debug, Clone, Copy)]
pub(in crate::broker) struct RunningAttempt {
    pub(in crate::broker) attempt: AttemptId,
    pub(in crate::broker) handle: ResourceHandle,
    pub(in crate::broker) class: QueueClass,
    pub(in crate::broker) sent: bool,
    pub(in crate::broker) lease_expires_ms: u64,
}

impl BrokerCore {
    /// Where a request stands, or `None` for a handle that is unknown or long forgotten.
    pub fn status(&self, handle: ResourceHandle) -> Option<RequestStatus> {
        self.requests
            .get(&handle.get())
            .map(|request| request.status)
    }

    pub fn readiness(&self, handle: ResourceHandle) -> Option<Readiness> {
        self.status(handle).map(RequestStatus::readiness)
    }

    /// Takes the result of a request that is ready. It can be taken once.
    pub fn take_result(&mut self, handle: ResourceHandle) -> Result<ResourceResult, ResultError> {
        let request = self
            .requests
            .get_mut(&handle.get())
            .ok_or(ResultError::Unknown)?;
        if !request.status.is_settled() {
            return Err(ResultError::NotSettled);
        }
        request.result.take().ok_or(ResultError::NoResult {
            status: request.status,
        })
    }

    /// The attempts abandoned since the last call, oldest first, for the worker to cancel at the
    /// provider (a preempted, cancelled, superseded or timed-out call is not worth finishing).
    pub fn take_cancellations(&mut self) -> Vec<AttemptId> {
        self.to_cancel.drain(..).collect()
    }

    /// The requests that settled since the last call, oldest first: readiness pushed to the owner.
    pub fn take_completions(&mut self) -> Vec<Completion> {
        self.completions.drain(..).collect()
    }

    /// Completions dropped because nobody took them within `MAXIMUM_PENDING_COMPLETIONS`.
    pub fn dropped_completions(&self) -> u64 {
        self.dropped_completions
    }

    pub(in crate::broker) fn waiting_in(
        &self,
        class: QueueClass,
    ) -> impl Iterator<Item = ResourceHandle> + '_ {
        self.queues.iter(class)
    }

    pub(in crate::broker) fn dropped_in(&self, class: QueueClass) -> u64 {
        self.queues.dropped(class)
    }

    pub(in crate::broker) fn requested_ms(&self, handle: ResourceHandle) -> Option<u64> {
        self.requests
            .get(&handle.get())
            .map(|request| request.requested_ms)
    }

    pub(in crate::broker) fn running(&self) -> impl Iterator<Item = RunningAttempt> + '_ {
        self.in_flight.iter().map(|(id, attempt)| RunningAttempt {
            attempt: *id,
            handle: attempt.handle,
            class: attempt.class,
            sent: attempt.sent,
            lease_expires_ms: attempt.lease_expires_ms,
        })
    }

    pub(in crate::broker) fn purpose_of(&self, handle: ResourceHandle) -> Option<&PurposeName> {
        self.requests
            .get(&handle.get())
            .map(|request| request.spec.name())
    }
}
