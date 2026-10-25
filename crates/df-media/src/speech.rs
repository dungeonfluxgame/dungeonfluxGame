use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use df_model::checkpoint::{Basis, JobId, NativeFailure};
use df_types::OperationId;

use crate::schedule::{
    AdmissionRefusal, DispatchOutcome, MediaDispatch, MediaPriority, MediaRequest, MediaScheduler,
    ScheduleError, ScheduleLimits, ScheduleSnapshot,
};

/// Existing canonical identifiers for one admitted job, not a public audio/cue schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeechIdentity {
    pub basis: Basis,
    pub job: JobId,
    pub operation: OperationId,
    pub generation: u64,
}

/// Global response bounds, separate from scheduler command-byte/execution bounds.
#[derive(Clone, Copy, Debug)]
pub struct SpeechLimits {
    pub maximum_chunks: usize,
    pub maximum_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechError {
    InvalidLimits,
    InvalidGeneration,
    Stale,
    IdentityMismatch,
    NotStarted,
    DuplicateChunk,
    SequenceRange,
    ChunkCapacity,
    ByteCapacity,
    DuplicateEnd,
    Closed,
    IncompleteEof,
    Waiting,
    WrongAcknowledgement,
    RevisionNotAdvanced,
    Failed(NativeFailure),
    Schedule(ScheduleError),
}

/// Admission preserves either the unchanged bytes or the scheduler's original refusal.
pub enum SpeechAdmissionRefusal {
    Invalid(SpeechError, Box<[u8]>),
    Schedule(Box<AdmissionRefusal<JobId>>),
}

impl fmt::Debug for SpeechAdmissionRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason, bytes) => formatter
                .debug_struct("SpeechAdmissionRefusal")
                .field("reason", reason)
                .field("payload_bytes", &bytes.len())
                .finish(),
            Self::Schedule(refusal) => fmt::Debug::fmt(refusal, formatter),
        }
    }
}

/// Refused response bytes remain owned by their sender and redacted from diagnostics.
pub struct SpeechChunkRefusal {
    pub reason: SpeechError,
    pub bytes: Box<[u8]>,
}

impl fmt::Debug for SpeechChunkRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpeechChunkRefusal")
            .field("reason", &self.reason)
            .field("payload_bytes", &self.bytes.len())
            .finish()
    }
}

/// Cloneable receipt tied privately to one queued request and its eventual actual dispatch.
/// A dropped receipt does not release work; only finish/stop or owner teardown does.
#[derive(Clone, Debug)]
pub struct SpeechReceipt {
    identity: SpeechIdentity,
    instance: Arc<()>,
}

impl SpeechReceipt {
    pub fn identity(&self) -> SpeechIdentity {
        self.identity
    }
}

/// An internal mismatch retains the exact dispatch and its occupied slot, failing closed.
#[derive(Debug)]
pub enum SpeechBeginRefusal {
    Schedule(ScheduleError),
    Owned {
        reason: SpeechError,
        dispatch: Box<MediaDispatch<JobId>>,
    },
}

/// Borrowed output stays owned and counted until the exact ordinal is acknowledged.
pub struct SpeechChunk<'a> {
    pub sequence: u64,
    pub bytes: &'a [u8],
}

/// Explicit disposition, without claiming provider cancellation or playback completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechStopReason {
    Cancelled,
    Failed(NativeFailure),
}

#[derive(Debug)]
pub struct SpeechStopped {
    pub identity: SpeechIdentity,
    pub reason: SpeechStopReason,
    pub discarded_chunks: usize,
    pub discarded_bytes: usize,
    pub request: MediaRequest<JobId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeechSnapshot {
    pub schedule: ScheduleSnapshot,
    pub buffered_chunks: usize,
    pub buffered_bytes: usize,
}

struct SpeechJob {
    identity: SpeechIdentity,
    instance: Arc<()>,
    dispatch: Option<MediaDispatch<JobId>>,
    chunks: BTreeMap<u64, Box<[u8]>>,
    next_sequence: u64,
    end: Option<u64>,
    closed: bool,
    failure: Option<NativeFailure>,
}

/// Pure single-owner scheduling over the actual media queue and dispatch tokens.
///
/// Source ordinals and explicit end/EOF are scheduling facts, not a speech codec contract.
/// Chunks stay private until the whole bounded sequence reaches end plus clean EOF. A qualified
/// producer must supply safe text/audio, timebase and current rights before public delivery;
/// no such producer, provider, recording publication or audible output is implemented here.
/// The native job owner supplies deadline/shutdown stop events independently of RPC waits.
/// Command bytes remain counted by MediaScheduler; response bytes remain counted through
/// borrowed delivery until acknowledgement. Copying into a downstream queue needs its own
/// admission bound. Dropping this owner tears down all local buffers, never refunds Unknown spend.
pub struct SpeechScheduler {
    current: Basis,
    scheduler: MediaScheduler<JobId>,
    limits: SpeechLimits,
    jobs: BTreeMap<JobId, SpeechJob>,
    buffered_chunks: usize,
    buffered_bytes: usize,
}

impl SpeechScheduler {
    pub fn new(
        current: Basis,
        schedule: ScheduleLimits,
        limits: SpeechLimits,
    ) -> Result<Self, SpeechError> {
        if limits.maximum_chunks == 0
            || limits.maximum_chunks > 4096
            || limits.maximum_bytes == 0
            || limits.maximum_bytes > 16 * 1024 * 1024
        {
            return Err(SpeechError::InvalidLimits);
        }
        Ok(Self {
            current,
            scheduler: MediaScheduler::new(schedule).map_err(SpeechError::Schedule)?,
            limits,
            jobs: BTreeMap::new(),
            buffered_chunks: 0,
            buffered_bytes: 0,
        })
    }

    pub fn admit(
        &mut self,
        identity: SpeechIdentity,
        command: Box<[u8]>,
    ) -> Result<SpeechReceipt, SpeechAdmissionRefusal> {
        if identity.basis != self.current {
            return Err(SpeechAdmissionRefusal::Invalid(SpeechError::Stale, command));
        }
        if identity.generation == 0 {
            return Err(SpeechAdmissionRefusal::Invalid(
                SpeechError::InvalidGeneration,
                command,
            ));
        }
        self.scheduler
            .admit(
                identity.job,
                identity.operation,
                MediaPriority::Speech,
                command,
            )
            .map_err(|refusal| SpeechAdmissionRefusal::Schedule(Box::new(refusal)))?;
        let instance = Arc::new(());
        let receipt = SpeechReceipt {
            identity,
            instance: Arc::clone(&instance),
        };
        self.jobs.insert(
            identity.job,
            SpeechJob {
                identity,
                instance,
                dispatch: None,
                chunks: BTreeMap::new(),
                next_sequence: 0,
                end: None,
                closed: false,
                failure: None,
            },
        );
        Ok(receipt)
    }

    /// Begins current queued work in admission order without dispatching obsolete commands.
    /// Obsolete commands retain their queue budget until explicit receipt-fenced cancellation.
    pub fn begin(&mut self) -> Result<Option<SpeechReceipt>, SpeechBeginRefusal> {
        let jobs = &self.jobs;
        let current_basis = self.current;
        let dispatch = match self.scheduler.begin_eligible(|request| {
            jobs.get(request.id())
                .is_some_and(|job| job.identity.basis == current_basis)
        }) {
            Ok(Some(dispatch)) => dispatch,
            Ok(None) => return Ok(None),
            Err(reason) => return Err(SpeechBeginRefusal::Schedule(reason)),
        };
        let job_id = *dispatch.request().id();
        let Some(job) = self.jobs.get_mut(&job_id) else {
            return Err(SpeechBeginRefusal::Owned {
                reason: SpeechError::NotStarted,
                dispatch: Box::new(dispatch),
            });
        };
        let receipt = SpeechReceipt {
            identity: job.identity,
            instance: Arc::clone(&job.instance),
        };
        job.dispatch = Some(dispatch);
        Ok(Some(receipt))
    }

    fn check(
        &self,
        receipt: &SpeechReceipt,
        require_current_basis: bool,
    ) -> Result<&SpeechJob, SpeechError> {
        let job = self
            .jobs
            .get(&receipt.identity.job)
            .ok_or(SpeechError::Stale)?;
        if job.identity != receipt.identity || !Arc::ptr_eq(&job.instance, &receipt.instance) {
            return Err(SpeechError::Stale);
        }
        if require_current_basis && job.identity.basis != self.current {
            return Err(SpeechError::Stale);
        }
        Ok(job)
    }

    fn check_event(
        &self,
        receipt: &SpeechReceipt,
        identity: SpeechIdentity,
    ) -> Result<&SpeechJob, SpeechError> {
        let job = self.check(receipt, true)?;
        if identity != job.identity {
            return Err(SpeechError::IdentityMismatch);
        }
        if job.dispatch.is_none() {
            return Err(SpeechError::NotStarted);
        }
        if let Some(reason) = job.failure {
            return Err(SpeechError::Failed(reason));
        }
        Ok(job)
    }

    /// Out-of-order data is private and bounded; every refusal returns identical owned bytes.
    pub fn queue_chunk(
        &mut self,
        receipt: &SpeechReceipt,
        identity: SpeechIdentity,
        sequence: u64,
        bytes: Box<[u8]>,
    ) -> Result<(), SpeechChunkRefusal> {
        let validation = self.check_event(receipt, identity).and_then(|job| {
            if job.closed {
                return Err(SpeechError::Closed);
            }
            if job.chunks.contains_key(&sequence) {
                return Err(SpeechError::DuplicateChunk);
            }
            if sequence >= self.limits.maximum_chunks as u64
                || job.end.is_some_and(|count| sequence >= count)
            {
                return Err(SpeechError::SequenceRange);
            }
            if self.buffered_chunks >= self.limits.maximum_chunks {
                return Err(SpeechError::ChunkCapacity);
            }
            if bytes.len() > self.limits.maximum_bytes - self.buffered_bytes {
                return Err(SpeechError::ByteCapacity);
            }
            Ok(())
        });
        if let Err(reason) = validation {
            return Err(SpeechChunkRefusal { reason, bytes });
        }
        let Some(job) = self.jobs.get_mut(&identity.job) else {
            return Err(SpeechChunkRefusal {
                reason: SpeechError::Stale,
                bytes,
            });
        };
        self.buffered_chunks += 1;
        self.buffered_bytes += bytes.len();
        job.chunks.insert(sequence, bytes);
        Ok(())
    }

    /// An end marker may overtake data, but cannot declare away queued trailing chunks.
    pub fn end(
        &mut self,
        receipt: &SpeechReceipt,
        identity: SpeechIdentity,
        count: u64,
    ) -> Result<(), SpeechError> {
        let job = self.check_event(receipt, identity)?;
        if job.closed {
            return Err(SpeechError::Closed);
        }
        if job.end.is_some() {
            return Err(SpeechError::DuplicateEnd);
        }
        if count > self.limits.maximum_chunks as u64
            || job
                .chunks
                .last_key_value()
                .is_some_and(|(sequence, _)| *sequence >= count)
        {
            return Err(SpeechError::SequenceRange);
        }
        self.jobs
            .get_mut(&identity.job)
            .ok_or(SpeechError::Stale)?
            .end = Some(count);
        Ok(())
    }

    /// No marker, gaps or wrong count fail EOF; partial bytes stay owned until explicit stop.
    pub fn eof(
        &mut self,
        receipt: &SpeechReceipt,
        identity: SpeechIdentity,
    ) -> Result<(), SpeechError> {
        let job = self.check_event(receipt, identity)?;
        if job.closed {
            return Err(SpeechError::Closed);
        }
        let is_complete = job.end.is_some_and(|count| {
            count == job.chunks.len() as u64 && job.chunks.keys().copied().eq(0..count)
        });
        let job = self.jobs.get_mut(&identity.job).ok_or(SpeechError::Stale)?;
        if !is_complete {
            job.failure = Some(NativeFailure::Rejected);
            return Err(SpeechError::IncompleteEof);
        }
        job.closed = true;
        Ok(())
    }

    /// Repeated reads borrow the same next chunk without removing it or freeing its budget.
    pub fn next(&self, receipt: &SpeechReceipt) -> Result<Option<SpeechChunk<'_>>, SpeechError> {
        let job = self.check_event(receipt, receipt.identity)?;
        if !job.closed {
            return Ok(None);
        }
        Ok(job.chunks.get(&job.next_sequence).map(|bytes| SpeechChunk {
            sequence: job.next_sequence,
            bytes,
        }))
    }

    /// Acknowledge synchronous consumption of exactly the borrowed ordinal.
    pub fn acknowledge(
        &mut self,
        receipt: &SpeechReceipt,
        sequence: u64,
    ) -> Result<(), SpeechError> {
        let job = self.check_event(receipt, receipt.identity)?;
        if !job.closed || sequence != job.next_sequence || !job.chunks.contains_key(&sequence) {
            return Err(SpeechError::WrongAcknowledgement);
        }
        let job = self
            .jobs
            .get_mut(&receipt.identity.job)
            .ok_or(SpeechError::Stale)?;
        let bytes = job
            .chunks
            .remove(&sequence)
            .ok_or(SpeechError::WrongAcknowledgement)?;
        job.next_sequence += 1; // At most the validated 4096 ordinals.
        self.buffered_chunks -= 1;
        self.buffered_bytes -= bytes.len();
        Ok(())
    }

    /// Scheduling completion requires whole-tail acknowledgement, not playback or asset Ready.
    pub fn finish(&mut self, receipt: &SpeechReceipt) -> Result<MediaRequest<JobId>, SpeechError> {
        let job = self.check_event(receipt, receipt.identity)?;
        if !job.closed || job.end != Some(job.next_sequence) || !job.chunks.is_empty() {
            return Err(SpeechError::Waiting);
        }
        self.release(receipt, DispatchOutcome::Completed)
    }

    /// Explicitly disposes local buffered bytes and releases this exact dispatch once.
    /// Stop is allowed for obsolete jobs; the instance fence still rejects old/foreign receipts.
    pub fn stop(
        &mut self,
        receipt: &SpeechReceipt,
        reason: SpeechStopReason,
    ) -> Result<SpeechStopped, SpeechError> {
        let job = self.check(receipt, false)?;
        let reason = job.failure.map(SpeechStopReason::Failed).unwrap_or(reason);
        let discarded_chunks = job.chunks.len();
        let discarded_bytes = job.chunks.values().map(|bytes| bytes.len()).sum();
        let outcome = match reason {
            SpeechStopReason::Cancelled => DispatchOutcome::Cancelled,
            SpeechStopReason::Failed(_) => DispatchOutcome::Failed,
        };
        let request = self.release(receipt, outcome)?;
        Ok(SpeechStopped {
            identity: receipt.identity,
            reason,
            discarded_chunks,
            discarded_bytes,
            request,
        })
    }

    fn release(
        &mut self,
        receipt: &SpeechReceipt,
        outcome: DispatchOutcome,
    ) -> Result<MediaRequest<JobId>, SpeechError> {
        let mut job = self
            .jobs
            .remove(&receipt.identity.job)
            .ok_or(SpeechError::Stale)?;
        let Some(dispatch) = job.dispatch.take() else {
            self.jobs.insert(receipt.identity.job, job);
            return Err(SpeechError::NotStarted);
        };
        match self.scheduler.finish(dispatch, outcome) {
            Ok(request) => {
                self.buffered_chunks -= job.chunks.len();
                self.buffered_bytes -= job.chunks.values().map(|bytes| bytes.len()).sum::<usize>();
                Ok(request)
            }
            Err(refusal) => {
                job.dispatch = Some(refusal.dispatch);
                self.jobs.insert(receipt.identity.job, job);
                Err(SpeechError::Schedule(refusal.reason))
            }
        }
    }

    /// A delayed queued cancellation needs the exact private admission receipt, too.
    pub fn cancel_queued(
        &mut self,
        receipt: &SpeechReceipt,
    ) -> Result<Option<MediaRequest<JobId>>, SpeechError> {
        let job = self.check(receipt, false)?;
        if job.dispatch.is_some() {
            return Ok(None);
        }
        let Some(request) = self.scheduler.cancel_queued(receipt.identity.job) else {
            return Err(SpeechError::NotStarted);
        };
        self.jobs.remove(&receipt.identity.job);
        Ok(Some(request))
    }

    /// Advance trusted canonical freshness without silently releasing or discarding old work.
    /// Existing receipts can retire old buffers; they cannot drain or accept more old data.
    pub fn replace_basis(&mut self, current: Basis) -> Result<(), SpeechError> {
        if current.revision <= self.current.revision {
            return Err(SpeechError::RevisionNotAdvanced);
        }
        self.current = current;
        Ok(())
    }

    pub fn snapshot(&self) -> SpeechSnapshot {
        SpeechSnapshot {
            schedule: self.scheduler.snapshot(),
            buffered_chunks: self.buffered_chunks,
            buffered_bytes: self.buffered_bytes,
        }
    }
}
