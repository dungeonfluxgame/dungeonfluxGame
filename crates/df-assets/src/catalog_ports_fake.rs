//! Faithful fakes of the catalog service's ports, for tests: a manual clock, F's attempt states
//! with the cancel-versus-claim rule, a dispatcher with a scripted answer, a validator that
//! derives two variants, and a chunked byte stream.

use crate::asset_lifecycle::RejectReason;
use crate::catalog_ports::{
    CancelOutcome, Clock, Dispatcher, FinanceAttempts, FinanceUnavailable, MediaValidator,
    ProviderAttemptState, RefetchAnswer, StagingStream, StreamError, ValidatedMedia,
};
use crate::catalog_record::AssetKind;
use crate::media_ids::{AssetId, AttemptId};
use crate::object_store::Deadline;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A clock that moves only when a test says so.
pub struct ManualClock(AtomicI64);

impl ManualClock {
    pub fn new(start_ms: i64) -> Self {
        Self(AtomicI64::new(start_ms))
    }

    pub fn advance(&self, ms: i64) {
        // Relaxed: tests read the clock on the thread that advanced it, or after joining.
        self.0.fetch_add(ms, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
struct FinanceState {
    attempts: HashMap<AttemptId, ProviderAttemptState>,
    cancels: Vec<AttemptId>,
    is_unavailable: bool,
    dispatcher_claims_first: bool,
}

/// F's view of provider attempts. An attempt not set reads as `Reserved`.
#[derive(Default)]
pub struct FakeFinance {
    state: Mutex<FinanceState>,
}

impl FakeFinance {
    pub fn set(&self, attempt: AttemptId, state: ProviderAttemptState) {
        locked(&self.state).attempts.insert(attempt, state);
    }

    pub fn set_unavailable(&self, is_unavailable: bool) {
        locked(&self.state).is_unavailable = is_unavailable;
    }

    /// The next cancel finds the dispatcher has just claimed the effect (the race F decides).
    pub fn set_dispatcher_claims_first(&self, claims_first: bool) {
        locked(&self.state).dispatcher_claims_first = claims_first;
    }

    /// Every attempt `cancel_unclaimed_effect` was called for, in order.
    pub fn cancel_calls(&self) -> Vec<AttemptId> {
        locked(&self.state).cancels.clone()
    }
}

impl FinanceAttempts for FakeFinance {
    async fn attempt_state(
        &self,
        attempt: AttemptId,
    ) -> Result<ProviderAttemptState, FinanceUnavailable> {
        let state = locked(&self.state);
        if state.is_unavailable {
            return Err(FinanceUnavailable);
        }
        Ok(state
            .attempts
            .get(&attempt)
            .copied()
            .unwrap_or(ProviderAttemptState::Reserved))
    }

    async fn cancel_unclaimed_effect(
        &self,
        attempt: AttemptId,
    ) -> Result<CancelOutcome, FinanceUnavailable> {
        let mut state = locked(&self.state);
        if state.is_unavailable {
            return Err(FinanceUnavailable);
        }
        state.cancels.push(attempt);
        if state.dispatcher_claims_first {
            state
                .attempts
                .insert(attempt, ProviderAttemptState::DispatchPrepared);
        }
        let current = state
            .attempts
            .get(&attempt)
            .copied()
            .unwrap_or(ProviderAttemptState::Reserved);
        match current {
            ProviderAttemptState::Reserved | ProviderAttemptState::Cancelled => {
                state
                    .attempts
                    .insert(attempt, ProviderAttemptState::Cancelled);
                Ok(CancelOutcome::Cancelled)
            }
            _ => Ok(CancelOutcome::AlreadyClaimed),
        }
    }
}

/// A dispatcher that answers every re-fetch request with the scripted answer.
pub struct FakeDispatcher {
    answer: Mutex<RefetchAnswer>,
    requests: Mutex<Vec<(AssetId, AttemptId)>>,
}

impl FakeDispatcher {
    pub fn answering(answer: RefetchAnswer) -> Self {
        Self {
            answer: Mutex::new(answer),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn set_answer(&self, answer: RefetchAnswer) {
        *locked(&self.answer) = answer;
    }

    pub fn requests(&self) -> Vec<(AssetId, AttemptId)> {
        locked(&self.requests).clone()
    }
}

impl Dispatcher for FakeDispatcher {
    async fn request_refetch(&self, asset: AssetId, attempt: AttemptId) -> RefetchAnswer {
        locked(&self.requests).push((asset, attempt));
        *locked(&self.answer)
    }
}

/// Accepts every input by default: the master is the staged bytes and the display variant is their
/// first half. `rejecting` makes it refuse with one reason.
pub struct FakeValidator {
    refusal: Mutex<Option<RejectReason>>,
    calls: Mutex<usize>,
}

impl FakeValidator {
    pub fn accepting() -> Self {
        Self {
            refusal: Mutex::new(None),
            calls: Mutex::new(0),
        }
    }

    pub fn rejecting(reason: RejectReason) -> Self {
        Self {
            refusal: Mutex::new(Some(reason)),
            calls: Mutex::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        *locked(&self.calls)
    }
}

impl MediaValidator for FakeValidator {
    async fn validate(
        &self,
        _kind: AssetKind,
        staged: &[u8],
    ) -> Result<ValidatedMedia, RejectReason> {
        *locked(&self.calls) += 1;
        if let Some(reason) = *locked(&self.refusal) {
            return Err(reason);
        }
        Ok(ValidatedMedia {
            master: staged.to_vec(),
            display: Some(staged[..staged.len() / 2 + 1].to_vec()),
            mime: "image/jpeg".to_owned(),
            width_px: Some(640),
            height_px: Some(480),
        })
    }
}

/// A byte stream cut into fixed-size chunks; optionally fails or stalls after some chunks, and
/// optionally takes a minimum real time per chunk (a slow provider).
pub struct ChunkedBytes {
    bytes: Vec<u8>,
    chunk_bytes: usize,
    position: usize,
    fail_after_chunks: Option<usize>,
    stall_after_chunks: Option<usize>,
    chunk_duration: std::time::Duration,
    delivered: usize,
}

impl ChunkedBytes {
    pub fn new(bytes: Vec<u8>, chunk_bytes: usize) -> Self {
        Self {
            bytes,
            chunk_bytes,
            position: 0,
            fail_after_chunks: None,
            stall_after_chunks: None,
            chunk_duration: std::time::Duration::ZERO,
            delivered: 0,
        }
    }

    /// After this many chunks the stream reports that the next chunk missed its deadline.
    pub fn stalling_after(mut self, chunks: usize) -> Self {
        self.stall_after_chunks = Some(chunks);
        self
    }

    /// Each chunk arrives no sooner than this after it was asked for (spins; never sleeps).
    pub fn taking_per_chunk(mut self, duration: std::time::Duration) -> Self {
        self.chunk_duration = duration;
        self
    }

    pub fn failing_after(mut self, chunks: usize) -> Self {
        self.fail_after_chunks = Some(chunks);
        self
    }

    /// Bytes handed out so far.
    pub fn position(&self) -> usize {
        self.position
    }
}

impl StagingStream for ChunkedBytes {
    async fn next_chunk(&mut self, deadline: Deadline) -> Result<Option<Vec<u8>>, StreamError> {
        if self.fail_after_chunks == Some(self.delivered) {
            return Err(StreamError::Failed);
        }
        if self.stall_after_chunks == Some(self.delivered) || deadline.has_passed() {
            return Err(StreamError::DeadlineExceeded);
        }
        let asked = std::time::Instant::now();
        while asked.elapsed() < self.chunk_duration {
            std::hint::spin_loop();
        }
        if self.position >= self.bytes.len() {
            return Ok(None);
        }
        let end = (self.position + self.chunk_bytes).min(self.bytes.len());
        let chunk = self.bytes[self.position..end].to_vec();
        self.position = end;
        self.delivered += 1;
        Ok(Some(chunk))
    }
}
