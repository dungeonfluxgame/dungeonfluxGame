use df_types::OperationId;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

/// Ranked server-side demand, distinct from provider authorization or game authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPriority {
    Speech,
    Required,
    Likely,
    Optional,
}

impl MediaPriority {
    fn index(self) -> usize {
        match self {
            Self::Speech => 0,
            Self::Required => 1,
            Self::Likely => 2,
            Self::Optional => 3,
        }
    }
}

/// Explicit owner policy; byte bounds include queued and outstanding dispatch payloads.
#[derive(Debug, Clone, Copy)]
pub struct ScheduleLimits {
    pub queue_items: usize,
    pub queue_bytes: usize,
    pub speech_items: usize,
    pub speech_bytes: usize,
    pub execution_slots: usize,
    pub speech_slots: usize,
}

/// Typed refusal, without provider or transport success claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleError {
    InvalidLimits,
    ItemCapacity,
    ByteCapacity,
    Duplicate,
    IdentityExhausted,
    ForeignDispatch,
}

/// Opaque caller-owned command bytes and canonical diagnostic correlation.
pub struct MediaRequest<JobId> {
    id: JobId,
    operation: OperationId,
    priority: MediaPriority,
    payload: Box<[u8]>,
}

impl<JobId> fmt::Debug for MediaRequest<JobId> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaRequest")
            .field("operation", &self.operation)
            .field("priority", &self.priority)
            .field("payload_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}

impl<JobId> MediaRequest<JobId> {
    pub fn id(&self) -> &JobId {
        &self.id
    }

    pub fn operation(&self) -> OperationId {
        self.operation
    }

    pub fn priority(&self) -> MediaPriority {
        self.priority
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// A refused admission returns ownership of the unchanged request.
#[derive(Debug)]
pub struct AdmissionRefusal<JobId> {
    pub reason: ScheduleError,
    pub request: MediaRequest<JobId>,
}

/// Non-cloneable dispatch ownership, returned to the same scheduler on termination.
///
/// The private owner marker survives transfer to an owned native execution worker.
/// Dropping a dispatch does not release its slot. The owner must report a terminal
/// outcome, or tear down the scheduler. This fails closed on lost asynchronous work.
#[derive(Debug)]
pub struct MediaDispatch<JobId> {
    owner: Arc<()>,
    generation: u64,
    request: MediaRequest<JobId>,
}

impl<JobId> MediaDispatch<JobId> {
    pub fn request(&self) -> &MediaRequest<JobId> {
        &self.request
    }
}

/// Rejected completion retains the token so its actual owner can terminate it.
#[derive(Debug)]
pub struct DispatchRefusal<JobId> {
    pub reason: ScheduleError,
    pub dispatch: MediaDispatch<JobId>,
}

/// Caller-observed termination; this does not undo a committed provider job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    Completed,
    Cancelled,
    Failed,
}

/// Payload-free facts for the owning service's shared instrumentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleSnapshot {
    pub queued_items: usize,
    pub queued_bytes: usize,
    pub active_dispatches: usize,
    pub active_bytes: usize,
    pub active_non_speech: usize,
    pub refused_admissions: u64,
    pub completed_dispatches: u64,
    pub cancelled_dispatches: u64,
    pub failed_dispatches: u64,
}

struct Active<JobId> {
    id: JobId,
    generation: u64,
    priority: MediaPriority,
    bytes: usize,
}

/// Bounded single-owner queue with FIFO within each ranked demand class.
///
/// Weighted round robin gives eligible Speech/Required/Likely/Optional queues
/// eight/four/two/one turns per cycle. Non-speech cannot use the speech reserves.
/// Job identity remains caller-owned; this type creates no service DTO or job ID.
pub struct MediaScheduler<JobId> {
    limits: ScheduleLimits,
    queues: [VecDeque<MediaRequest<JobId>>; 4],
    queued_bytes: usize,
    non_speech_bytes: usize,
    active: Vec<Active<JobId>>,
    owner: Arc<()>,
    generation: u64,
    cursor: usize,
    refused: u64,
    terminal: [u64; 3],
}

const TURNS: [usize; 15] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3];

impl<JobId: Copy + Eq> MediaScheduler<JobId> {
    /// Validates finite owner limits; no defaults or provider capacity promise.
    pub fn new(limits: ScheduleLimits) -> Result<Self, ScheduleError> {
        if limits.queue_items == 0
            || limits.queue_items > 4096
            || limits.queue_bytes == 0
            || limits.queue_bytes > 16 * 1024 * 1024
            || limits.speech_items == 0
            || limits.speech_items > limits.queue_items
            || limits.speech_bytes == 0
            || limits.speech_bytes > limits.queue_bytes
            || limits.execution_slots == 0
            || limits.execution_slots > 256
            || limits.speech_slots == 0
            || limits.speech_slots > limits.execution_slots
        {
            return Err(ScheduleError::InvalidLimits);
        }
        Ok(Self {
            limits,
            queues: std::array::from_fn(|_| VecDeque::new()),
            queued_bytes: 0,
            non_speech_bytes: 0,
            active: Vec::new(),
            owner: Arc::new(()),
            generation: 0,
            cursor: 0,
            refused: 0,
            terminal: [0; 3],
        })
    }

    /// Admits actual owned bytes, returning them unchanged on explicit refusal.
    pub fn admit(
        &mut self,
        id: JobId,
        operation: OperationId,
        priority: MediaPriority,
        payload: Box<[u8]>,
    ) -> Result<(), AdmissionRefusal<JobId>> {
        let request = MediaRequest {
            id,
            operation,
            priority,
            payload,
        };
        if let Err(reason) = self.admission_reason(&request) {
            self.refused = self.refused.saturating_add(1);
            return Err(AdmissionRefusal { reason, request });
        }
        self.queued_bytes += request.payload.len();
        if priority != MediaPriority::Speech {
            self.non_speech_bytes += request.payload.len();
        }
        self.queues[priority.index()].push_back(request);
        Ok(())
    }

    fn admission_reason(&self, request: &MediaRequest<JobId>) -> Result<(), ScheduleError> {
        if self.queues.iter().flatten().any(|job| job.id == request.id)
            || self.active.iter().any(|job| job.id == request.id)
        {
            return Err(ScheduleError::Duplicate);
        }
        let queued_items = self.queues.iter().map(VecDeque::len).sum::<usize>();
        let queued_non_speech_items = self.queues.iter().skip(1).map(VecDeque::len).sum::<usize>();
        if queued_items >= self.limits.queue_items
            || (request.priority != MediaPriority::Speech
                && queued_non_speech_items >= self.limits.queue_items - self.limits.speech_items)
        {
            return Err(ScheduleError::ItemCapacity);
        }
        let request_bytes = request.payload.len();
        let active_bytes = self.active.iter().map(|job| job.bytes).sum::<usize>();
        let active_non_speech_bytes = self
            .active
            .iter()
            .filter(|job| job.priority != MediaPriority::Speech)
            .map(|job| job.bytes)
            .sum::<usize>();
        if request_bytes > self.limits.queue_bytes - self.queued_bytes - active_bytes
            || (request.priority != MediaPriority::Speech
                && request_bytes
                    > self.limits.queue_bytes
                        - self.limits.speech_bytes
                        - self.non_speech_bytes
                        - active_non_speech_bytes)
        {
            return Err(ScheduleError::ByteCapacity);
        }
        Ok(())
    }

    /// Transfers one eligible request while preserving speech execution capacity.
    ///
    /// None means no eligible request or no execution slot. Provider admission,
    /// deadlines, scene fences and paid execution remain caller boundaries.
    pub fn begin(&mut self) -> Result<Option<MediaDispatch<JobId>>, ScheduleError> {
        self.begin_eligible(|_| true)
    }

    /// Leaves ineligible queued work owned and counted until explicit cancellation.
    pub(crate) fn begin_eligible(
        &mut self,
        eligible: impl Fn(&MediaRequest<JobId>) -> bool,
    ) -> Result<Option<MediaDispatch<JobId>>, ScheduleError> {
        if self.active.len() >= self.limits.execution_slots {
            return Ok(None);
        }
        let active_non_speech_count = self
            .active
            .iter()
            .filter(|job| job.priority != MediaPriority::Speech)
            .count();
        for offset in 0..TURNS.len() {
            let turn_index = (self.cursor + offset) % TURNS.len();
            let priority_index = TURNS[turn_index];
            if self.queues[priority_index].is_empty()
                || (priority_index != 0
                    && active_non_speech_count
                        >= self.limits.execution_slots - self.limits.speech_slots)
            {
                continue;
            }
            let Some(position) = self.queues[priority_index].iter().position(&eligible) else {
                continue;
            };
            let generation = self
                .generation
                .checked_add(1)
                .ok_or(ScheduleError::IdentityExhausted)?;
            if let Some(request) = self.queues[priority_index].remove(position) {
                self.remove_queued_bytes(&request);
                self.active.push(Active {
                    id: request.id,
                    generation,
                    priority: request.priority,
                    bytes: request.payload.len(),
                });
                self.generation = generation;
                self.cursor = (turn_index + 1) % TURNS.len();
                return Ok(Some(MediaDispatch {
                    owner: Arc::clone(&self.owner),
                    generation,
                    request,
                }));
            }
        }
        Ok(None)
    }

    /// Terminates only this owner's exact token and returns its original request.
    pub fn finish(
        &mut self,
        dispatch: MediaDispatch<JobId>,
        outcome: DispatchOutcome,
    ) -> Result<MediaRequest<JobId>, DispatchRefusal<JobId>> {
        if !Arc::ptr_eq(&self.owner, &dispatch.owner) {
            return Err(DispatchRefusal {
                reason: ScheduleError::ForeignDispatch,
                dispatch,
            });
        }
        let position = self.active.iter().position(|active| {
            active.generation == dispatch.generation && active.id == dispatch.request.id
        });
        let Some(position) = position else {
            return Err(DispatchRefusal {
                reason: ScheduleError::ForeignDispatch,
                dispatch,
            });
        };
        self.active.remove(position);
        let counter = match outcome {
            DispatchOutcome::Completed => 0,
            DispatchOutcome::Cancelled => 1,
            DispatchOutcome::Failed => 2,
        };
        self.terminal[counter] = self.terminal[counter].saturating_add(1);
        Ok(dispatch.request)
    }

    /// Cancels unstarted demand, returning exact bytes without affecting active work.
    pub fn cancel_queued(&mut self, id: JobId) -> Option<MediaRequest<JobId>> {
        let (index, position) = self.queues.iter().enumerate().find_map(|(index, queue)| {
            queue
                .iter()
                .position(|job| job.id == id)
                .map(|position| (index, position))
        })?;
        let request = self.queues[index].remove(position)?;
        self.remove_queued_bytes(&request);
        Some(request)
    }

    fn remove_queued_bytes(&mut self, request: &MediaRequest<JobId>) {
        self.queued_bytes -= request.payload.len();
        if request.priority != MediaPriority::Speech {
            self.non_speech_bytes -= request.payload.len();
        }
    }

    pub fn snapshot(&self) -> ScheduleSnapshot {
        ScheduleSnapshot {
            queued_items: self.queues.iter().map(VecDeque::len).sum(),
            queued_bytes: self.queued_bytes,
            active_dispatches: self.active.len(),
            active_bytes: self.active.iter().map(|job| job.bytes).sum(),
            active_non_speech: self
                .active
                .iter()
                .filter(|job| job.priority != MediaPriority::Speech)
                .count(),
            refused_admissions: self.refused,
            completed_dispatches: self.terminal[0],
            cancelled_dispatches: self.terminal[1],
            failed_dispatches: self.terminal[2],
        }
    }
}
