//! Bounded decoded presentation resources, separate from production playback.
//!
//! The queue owns PCM and preserves supplied frame offsets. Canonical media identities,
//! output leases and immutable asset references do not authorize audio by themselves.
//! No provider bytes, decoder, browser node or game rule is implemented here.
//! A mounted playback adapter must stop its actual nodes before confirming stop,
//! and explicitly dispose this owner before unmounting. Queue callbacks report
//! resource lifetime facts; they never attest that a speaker was audible.

#[cfg(not(target_arch = "wasm32"))]
mod capabilities;
#[cfg(not(target_arch = "wasm32"))]
mod capture;
#[cfg(not(target_arch = "wasm32"))]
mod native;
mod pcm;
mod presentation;
mod queue;

#[cfg(not(target_arch = "wasm32"))]
pub use capture::{
    CaptureAcquisition, CaptureChunk, CaptureError, CaptureGestureObservation, CaptureLimits,
    CaptureRecording, CaptureSession, CaptureSnapshot, CaptureState, ChunkRefusal,
};
#[cfg(not(target_arch = "wasm32"))]
pub use native::{AudioQueue, PcmView};
pub use presentation::{
    PcmSource, PlaybackBasis, PlaybackCue, PlaybackDestination, PlaybackOutput,
};
#[cfg(target_arch = "wasm32")]
pub use queue::{PresentationPcmView as PcmView, PresentationQueue as AudioQueue};

pub use pcm::{PcmBuffer, PcmFormat, PcmRefusal};
pub use queue::{
    AudioReceipt, BufferCompletion, BufferTicket, Cancellation, CaptionBinding, CaptionError,
    DriftDirection, EnqueueRefusal, MissingPlaybackPrerequisite, PlaybackDrift,
    PresentationPcmView, PresentationQueue, QueueError, QueueLimits, QueueSnapshot, QueueState,
    REQUIRED_PLAYBACK_PREREQUISITES, Replacement, ServerPlaybackTimeline, TimelineError,
};

pub mod browser_playback;
