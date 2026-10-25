//! Native ownership contract for an admitted microphone capture lease.
//!
//! This owner accepts observations from a separate capture adapter. It does not
//! request browser permission or claim that a microphone is available. Chunk bytes
//! remain owned and bounded until finish, cancel, or dispose.

use df_model::checkpoint::{CaptureLease, WindowId};
use df_types::{ClientBindingId, MemberId};

use crate::capabilities::select_capture_codec;

const MAXIMUM_CAPTURE_BYTES: usize = 32 * 1024 * 1024;
const MAXIMUM_CAPTURE_CHUNKS: u64 = 65_536;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureError {
    /// The supplied admission facts do not match the lease.
    InvalidLease,
    /// The caller supplied an obsolete lease, binding, or generation.
    StaleLease,
    /// The platform start callback was not invoked inside a user gesture.
    UserGestureRequired,
    /// Platform acquisition has not reached a terminal result.
    Pending,
    /// The platform explicitly denied microphone permission.
    PermissionDenied,
    /// This target or adapter does not implement microphone acquisition.
    Unsupported,
    /// The platform adapter reported an acquisition operation failure.
    OperationFailed,
    /// The adapter did not report the exact format required by the capture lease.
    UnsupportedFormat,
    /// The requested lifecycle operation is invalid in the current state.
    InvalidState,
    /// A configured resource limit is zero, inconsistent, or too large.
    InvalidLimits,
    /// Empty chunks cannot advance a capture sequence.
    EmptyChunk,
    /// One chunk exceeds the configured per-chunk byte bound.
    ChunkTooLarge,
    /// The chunk sequence does not match the next expected sequence.
    SequenceMismatch,
    /// The chunk offset does not advance the capture timeline.
    OffsetMismatch,
    /// A chunk would exceed the configured session capacity.
    Capacity,
    /// A sequence or byte counter cannot be incremented safely.
    CounterOverflow,
}

/// Platform acquisition result observed by this lease owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureAcquisition {
    Pending,
    Opened,
    PermissionDenied,
    Unsupported,
    OperationFailed,
}

/// Gesture state observed by an adapter that invokes a gesture-gated API.
///
/// This value is policy input only. `SynchronousApiInvocationObserved` must
/// describe an actual synchronous API call made by the adapter; passing this
/// variant does not make that call, grant permission, or qualify a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureGestureObservation {
    Missing,
    SynchronousApiInvocationObserved,
}

/// Terminal and active states for one capture lease owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureState {
    Ready,
    Capturing,
    Finished,
    Cancelled,
    Disposed,
}

/// Hard limits for one capture session. Total owned bytes may not exceed 32 MiB.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureLimits {
    /// Maximum number of chunks retained by the session.
    pub maximum_chunks: u64,
    /// Maximum bytes in any one chunk.
    pub maximum_chunk_bytes: usize,
    /// Maximum sum of bytes retained across all chunks.
    pub maximum_total_bytes: usize,
}

/// Current capture lifecycle and retained resource counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureSnapshot {
    /// Current lifecycle state.
    pub state: CaptureState,
    /// Sequence number expected for the next accepted chunk.
    pub next_sequence: u64,
    /// Offset of the most recently accepted chunk, if one exists.
    pub next_offset_frames: Option<u64>,
    /// Number of chunks currently owned by the session.
    pub chunk_count: u64,
    /// Number of bytes currently owned by the session.
    pub retained_bytes: usize,
}

/// One owned chunk already delivered by the platform capture adapter.
#[derive(Debug, Eq, PartialEq)]
pub struct CaptureChunk {
    sequence: u64,
    offset_frames: u64,
    bytes: Vec<u8>,
}

impl CaptureChunk {
    /// Takes ownership of bytes from an adapter event; the session validates bounds.
    pub fn new(sequence: u64, offset_frames: u64, bytes: Vec<u8>) -> Self {
        Self {
            sequence,
            offset_frames,
            bytes,
        }
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn offset_frames(&self) -> u64 {
        self.offset_frames
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn scrub(&mut self) {
        self.bytes.fill(0);
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct ChunkRefusal {
    /// Why the chunk was refused.
    pub error: CaptureError,
    /// Original chunk ownership returned to the caller.
    pub chunk: CaptureChunk,
}

/// Completed capture bytes and their canonical lease metadata.
#[derive(Debug, Eq, PartialEq)]
pub struct CaptureRecording {
    lease: CaptureLease,
    chunks: Vec<CaptureChunk>,
    retained_bytes: usize,
}

impl CaptureRecording {
    /// Canonical server-issued capture lease for this recording.
    pub fn lease(&self) -> &CaptureLease {
        &self.lease
    }

    pub fn format(&self) -> &df_types::RevisionLabel {
        &self.lease.format
    }

    pub fn chunks(&self) -> &[CaptureChunk] {
        &self.chunks
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

impl Drop for CaptureRecording {
    fn drop(&mut self) {
        for chunk in &mut self.chunks {
            chunk.scrub();
        }
    }
}

/// Owns chunks for one canonical capture lease and one client binding generation.
///
/// `select_codec` compares adapter-reported formats with the canonical lease format;
/// it does not establish device support. `start` records the platform adapter's actual
/// acquisition result. This crate has no microphone acquisition implementation, and
/// every later operation must present the same current lease.
pub struct CaptureSession {
    lease: CaptureLease,
    binding: ClientBindingId,
    generation: u64,
    limits: CaptureLimits,
    selected_format: Option<df_types::RevisionLabel>,
    state: CaptureState,
    next_sequence: u64,
    next_offset_frames: Option<u64>,
    retained_bytes: usize,
    chunks: Vec<CaptureChunk>,
}

impl CaptureSession {
    pub fn new(
        lease: CaptureLease,
        current_binding: ClientBindingId,
        current_generation: u64,
        current_member: MemberId,
        current_offer: Option<WindowId>,
        limits: CaptureLimits,
    ) -> Result<Self, CaptureError> {
        if lease.binding != current_binding
            || lease.generation == 0
            || lease.generation != current_generation
            || lease.member != current_member
            || lease
                .permitted_offer
                .is_some_and(|permitted_offer| current_offer != Some(permitted_offer))
        {
            return Err(CaptureError::InvalidLease);
        }
        if limits.maximum_chunks == 0
            || limits.maximum_chunks > MAXIMUM_CAPTURE_CHUNKS
            || limits.maximum_chunk_bytes == 0
            || limits.maximum_total_bytes == 0
            || limits.maximum_chunk_bytes > limits.maximum_total_bytes
            || limits.maximum_total_bytes > MAXIMUM_CAPTURE_BYTES
        {
            return Err(CaptureError::InvalidLimits);
        }
        let chunk_capacity =
            usize::try_from(limits.maximum_chunks).map_err(|_| CaptureError::InvalidLimits)?;
        let mut chunks = Vec::new();
        chunks
            .try_reserve_exact(chunk_capacity)
            .map_err(|_| CaptureError::InvalidLimits)?;
        Ok(Self {
            lease,
            binding: current_binding,
            generation: current_generation,
            limits,
            selected_format: None,
            state: CaptureState::Ready,
            next_sequence: 0,
            next_offset_frames: None,
            retained_bytes: 0,
            chunks,
        })
    }

    /// Selects only the exact codec named by this lease from adapter-reported formats.
    /// A mismatch is explicit; this method performs no device probe or fallback and
    /// does not qualify that a microphone can open or deliver audio.
    pub fn select_codec(
        &mut self,
        adapter_formats: &[df_types::RevisionLabel],
    ) -> Result<(), CaptureError> {
        if self.state != CaptureState::Ready {
            return Err(CaptureError::InvalidState);
        }
        self.selected_format = None;
        let selected_format = select_capture_codec(&self.lease.format, adapter_formats)?;
        self.selected_format = Some(selected_format.clone());
        Ok(())
    }

    /// Records gesture and acquisition observations supplied by the platform adapter.
    /// These observations do not themselves invoke an API or qualify a device.
    pub fn start(
        &mut self,
        current_lease: &CaptureLease,
        current_binding: ClientBindingId,
        current_generation: u64,
        gesture: CaptureGestureObservation,
        acquisition: CaptureAcquisition,
    ) -> Result<(), CaptureError> {
        self.check_current(current_lease, current_binding, current_generation)?;
        if self.state != CaptureState::Ready {
            return Err(CaptureError::InvalidState);
        }
        if gesture == CaptureGestureObservation::Missing {
            return Err(CaptureError::UserGestureRequired);
        }
        match acquisition {
            CaptureAcquisition::Pending => Err(CaptureError::Pending),
            CaptureAcquisition::PermissionDenied => Err(CaptureError::PermissionDenied),
            CaptureAcquisition::Unsupported => Err(CaptureError::Unsupported),
            CaptureAcquisition::OperationFailed => Err(CaptureError::OperationFailed),
            CaptureAcquisition::Opened => {
                if self.selected_format.as_ref() != Some(&self.lease.format) {
                    return Err(CaptureError::UnsupportedFormat);
                }
                self.state = CaptureState::Capturing;
                Ok(())
            }
        }
    }

    pub fn chunk(
        &mut self,
        current_lease: &CaptureLease,
        current_binding: ClientBindingId,
        current_generation: u64,
        chunk: CaptureChunk,
    ) -> Result<(), ChunkRefusal> {
        if let Err(error) = self.check_current(current_lease, current_binding, current_generation) {
            return Err(ChunkRefusal { error, chunk });
        }
        if self.state != CaptureState::Capturing {
            return Err(ChunkRefusal {
                error: CaptureError::InvalidState,
                chunk,
            });
        }
        if chunk.bytes.is_empty() {
            return Err(ChunkRefusal {
                error: CaptureError::EmptyChunk,
                chunk,
            });
        }
        // The session retains the Vec allocation as well as its initialized bytes.
        // Bound the allocation before storing it so a tiny logical chunk cannot
        // smuggle an oversized backing buffer into the session.
        let chunk_capacity_bytes = chunk.bytes.capacity();
        if chunk_capacity_bytes > self.limits.maximum_chunk_bytes {
            return Err(ChunkRefusal {
                error: CaptureError::ChunkTooLarge,
                chunk,
            });
        }
        if chunk.sequence != self.next_sequence {
            return Err(ChunkRefusal {
                error: CaptureError::SequenceMismatch,
                chunk,
            });
        }
        if self
            .next_offset_frames
            .is_some_and(|offset| chunk.offset_frames <= offset)
        {
            return Err(ChunkRefusal {
                error: CaptureError::OffsetMismatch,
                chunk,
            });
        }
        let Some(retained_bytes) = self.retained_bytes.checked_add(chunk_capacity_bytes) else {
            return Err(ChunkRefusal {
                error: CaptureError::CounterOverflow,
                chunk,
            });
        };
        let maximum_chunks = match usize::try_from(self.limits.maximum_chunks) {
            Ok(maximum_chunks) => maximum_chunks,
            Err(_) => {
                return Err(ChunkRefusal {
                    error: CaptureError::InvalidLimits,
                    chunk,
                });
            }
        };
        if self.chunks.len() >= maximum_chunks || retained_bytes > self.limits.maximum_total_bytes {
            return Err(ChunkRefusal {
                error: CaptureError::Capacity,
                chunk,
            });
        }
        let Some(next_sequence) = self.next_sequence.checked_add(1) else {
            return Err(ChunkRefusal {
                error: CaptureError::CounterOverflow,
                chunk,
            });
        };
        self.next_offset_frames = Some(chunk.offset_frames);
        self.next_sequence = next_sequence;
        self.retained_bytes = retained_bytes;
        self.chunks.push(chunk);
        Ok(())
    }

    pub fn finish(
        &mut self,
        current_lease: &CaptureLease,
        current_binding: ClientBindingId,
        current_generation: u64,
    ) -> Result<CaptureRecording, CaptureError> {
        self.check_current(current_lease, current_binding, current_generation)?;
        if self.state != CaptureState::Capturing || self.chunks.is_empty() {
            return Err(CaptureError::InvalidState);
        }
        self.state = CaptureState::Finished;
        Ok(CaptureRecording {
            lease: self.lease.clone(),
            chunks: std::mem::take(&mut self.chunks),
            retained_bytes: std::mem::take(&mut self.retained_bytes),
        })
    }

    pub fn cancel(
        &mut self,
        current_lease: &CaptureLease,
        current_binding: ClientBindingId,
        current_generation: u64,
    ) -> Result<(), CaptureError> {
        self.check_current(current_lease, current_binding, current_generation)?;
        if matches!(self.state, CaptureState::Finished | CaptureState::Disposed) {
            return Err(CaptureError::InvalidState);
        }
        self.clear_chunks();
        self.state = CaptureState::Cancelled;
        Ok(())
    }

    pub fn dispose(
        &mut self,
        current_lease: &CaptureLease,
        current_binding: ClientBindingId,
        current_generation: u64,
    ) -> Result<(), CaptureError> {
        self.check_current(current_lease, current_binding, current_generation)?;
        if self.state == CaptureState::Disposed {
            return Err(CaptureError::InvalidState);
        }
        self.clear_chunks();
        self.state = CaptureState::Disposed;
        Ok(())
    }

    pub fn snapshot(&self) -> CaptureSnapshot {
        CaptureSnapshot {
            state: self.state,
            next_sequence: self.next_sequence,
            next_offset_frames: self.next_offset_frames,
            chunk_count: self.chunks.len() as u64,
            retained_bytes: self.retained_bytes,
        }
    }

    fn check_current(
        &self,
        current_lease: &CaptureLease,
        current_binding: ClientBindingId,
        current_generation: u64,
    ) -> Result<(), CaptureError> {
        if self.state == CaptureState::Disposed {
            return Err(CaptureError::InvalidState);
        }
        if current_lease != &self.lease
            || current_binding != self.binding
            || current_generation != self.generation
        {
            return Err(CaptureError::StaleLease);
        }
        Ok(())
    }

    fn clear_chunks(&mut self) {
        for chunk in &mut self.chunks {
            chunk.scrub();
        }
        self.chunks.clear();
        self.retained_bytes = 0;
        self.next_sequence = 0;
        self.next_offset_frames = None;
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.clear_chunks();
    }
}
