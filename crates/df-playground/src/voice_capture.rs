//! What a microphone recording is and why one fails, apart from the browser that records it.
//!
//! The browser binding (`app::voice::capture`) asks for the microphone and hands over blocks of
//! samples; this module owns the rest: the typed reasons a recording cannot start or go on, how a
//! browser's exception name maps to one, and the buffer that holds a recording to the longest the
//! speech adapter takes and turns it into the PCM it reads. The audio lives in this buffer, in
//! memory, until it is sent or dropped: it is never kept, named or logged.
//!
//! `df_audio::capture::CaptureSession` is not used: it is native-only and needs a server-issued
//! `CaptureLease` and client binding that the dev page does not have.

use std::fmt;

use crate::limits::{MAXIMUM_CAPTURE_MS, MINIMUM_CAPTURE_MS};
use crate::voice_pcm::{duration_ms, level_permille, samples_in, to_pcm16_le, to_speech_rate};

/// Why a recording could not start, go on or be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneError {
    /// The person (or the browser's settings) refused the microphone.
    PermissionDenied,
    /// There is no microphone.
    NoDevice,
    /// This browser, or this address, cannot record (no microphone API, or not a secure page).
    Unsupported,
    /// The microphone was taken away or stopped working while it was in use.
    DeviceLost,
    /// The recording is shorter than the speech adapter takes.
    TooShort,
    /// The browser failed in some other way.
    Failed,
}

impl MicrophoneError {
    pub const ALL: [Self; 6] = [
        Self::PermissionDenied,
        Self::NoDevice,
        Self::Unsupported,
        Self::DeviceLost,
        Self::TooShort,
        Self::Failed,
    ];

    /// The closed key the page notes in its report.
    pub fn key(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::NoDevice => "no_device",
            Self::Unsupported => "unsupported",
            Self::DeviceLost => "device_lost",
            Self::TooShort => "too_short",
            Self::Failed => "failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|error| error.key() == key)
    }
}

impl fmt::Display for MicrophoneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PermissionDenied => "the microphone was not allowed",
            Self::NoDevice => "there is no microphone",
            Self::Unsupported => "this browser cannot record from a microphone here",
            Self::DeviceLost => "the microphone stopped working",
            Self::TooShort => "the recording is too short",
            Self::Failed => "the recording failed",
        })
    }
}

impl std::error::Error for MicrophoneError {}

/// The error for an exception the browser threw when asked for the microphone, from its name.
pub fn classify_media_error(exception_name: &str) -> MicrophoneError {
    match exception_name {
        "NotAllowedError" | "SecurityError" | "PermissionDeniedError" => {
            MicrophoneError::PermissionDenied
        }
        "NotFoundError" | "DevicesNotFoundError" => MicrophoneError::NoDevice,
        "NotReadableError" | "TrackStartError" | "AbortError" => MicrophoneError::DeviceLost,
        "NotSupportedError" | "TypeError" => MicrophoneError::Unsupported,
        _ => MicrophoneError::Failed,
    }
}

/// What taking a block of samples did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// The recording goes on.
    Going,
    /// The recording reached the longest allowed; the rest of the block was dropped.
    ReachedLongest,
}

/// A recording in progress: the browser's samples at the browser's rate, held to the longest the
/// speech adapter takes.
pub struct CaptureBuffer {
    rate_hz: u32,
    samples: Vec<f32>,
    longest_samples: usize,
    level_permille: u16,
}

impl CaptureBuffer {
    /// A buffer for a recording read at `rate_hz`.
    pub fn new(rate_hz: u32) -> Self {
        Self {
            rate_hz,
            samples: Vec::new(),
            longest_samples: samples_in(MAXIMUM_CAPTURE_MS, rate_hz),
            level_permille: 0,
        }
    }

    /// Takes a block of samples. Samples past the longest recording are dropped.
    pub fn take(&mut self, block: &[f32]) -> Taken {
        self.level_permille = level_permille(block);
        let room = self.longest_samples.saturating_sub(self.samples.len());
        let kept = block.len().min(room);
        self.samples.extend_from_slice(&block[..kept]);
        if self.samples.len() >= self.longest_samples {
            Taken::ReachedLongest
        } else {
            Taken::Going
        }
    }

    /// How much has been recorded, in milliseconds.
    pub fn elapsed_ms(&self) -> u64 {
        duration_ms(self.samples.len(), self.rate_hz)
    }

    /// How loud the newest block was, from 0 to 1000.
    pub fn level_permille(&self) -> u16 {
        self.level_permille
    }

    /// The recording as the speech adapter reads it, or `TooShort` when it is under the shortest
    /// the adapter takes. The samples are cleared either way.
    pub fn finish(mut self) -> Result<Vec<u8>, MicrophoneError> {
        let recorded_ms = self.elapsed_ms();
        let speech = to_speech_rate(&self.samples, self.rate_hz);
        self.samples.fill(0.0);
        if recorded_ms < MINIMUM_CAPTURE_MS {
            return Err(MicrophoneError::TooShort);
        }
        Ok(to_pcm16_le(&speech))
    }
}

impl Drop for CaptureBuffer {
    fn drop(&mut self) {
        // The audio is request-lived: scrub it rather than leave it in freed memory.
        self.samples.fill(0.0);
    }
}
