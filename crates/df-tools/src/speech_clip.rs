//! The contract of one spoken clip between the phone and the server: routes, headers, the
//! closed set of outcomes, limits, and the WAV container the clip travels in.
//!
//! Both sides use this one definition so they cannot drift apart. The browser records with
//! `MediaRecorder`, decodes the recording at 16 kHz, and uploads mono 16-bit PCM in a minimal
//! WAV container; the server checks that container and hands the PCM to the speech provider
//! adapter, which accepts exactly that format. The clip carries sound only; nothing about the
//! speaker is read from it by either side.
use std::time::Duration;

/// `POST` a clip here; `GET` the status path first to learn whether speech input exists.
pub(crate) const SPEECH_PATH: &str = "/gameplay/speech";
pub(crate) const STATUS_PATH: &str = "/gameplay/speech/status";
/// Response header naming the [`SpeechOutcome`] of an upload or status request.
pub(crate) const OUTCOME_HEADER: &str = "x-df-speech";
/// Request header carrying the player's room binding, as for generated RPC calls.
pub(crate) const BINDING_HEADER: &str = "x-df-local-binding";
pub(crate) const CLIP_CONTENT_TYPE: &str = "audio/wav";

pub(crate) const SAMPLE_RATE: u32 = 16_000;
const BYTES_PER_SAMPLE: usize = 2;
#[cfg(any(target_arch = "wasm32", test))]
const WAV_HEADER_BYTES: usize = 44;
/// The hard stop of a recording on the phone.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) const MAXIMUM_RECORDING: Duration = Duration::from_secs(15);
/// Shorter clips are discarded on the phone and refused by the server.
pub(crate) const MINIMUM_CLIP: Duration = Duration::from_millis(400);
/// The server accepts a little more than the phone sends, so a clip stopped exactly at the
/// cap is never refused for rounding.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const MAXIMUM_ACCEPTED_CLIP: Duration = Duration::from_secs(16);
/// Largest upload body.
pub(crate) const MAXIMUM_CLIP_BYTES: usize = 1024 * 1024;

/// Every way an upload or status request can end. The label travels in [`OUTCOME_HEADER`];
/// callers never parse prose to decide behaviour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpeechOutcome {
    Heard,
    NothingHeard,
    Available,
    Unavailable,
    BudgetSpent,
    Busy,
    TooLarge,
    TooLong,
    TooShort,
    UnsupportedType,
    Malformed,
    Unauthenticated,
    Refused,
    ProviderFailed,
    TimedOut,
}

impl SpeechOutcome {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Heard => "heard",
            Self::NothingHeard => "nothing_heard",
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::BudgetSpent => "budget_spent",
            Self::Busy => "busy",
            Self::TooLarge => "too_large",
            Self::TooLong => "too_long",
            Self::TooShort => "too_short",
            Self::UnsupportedType => "unsupported_type",
            Self::Malformed => "malformed",
            Self::Unauthenticated => "unauthenticated",
            Self::Refused => "refused",
            Self::ProviderFailed => "provider_failed",
            Self::TimedOut => "timed_out",
        }
    }

    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn from_label(label: &str) -> Option<Self> {
        [
            Self::Heard,
            Self::NothingHeard,
            Self::Available,
            Self::Unavailable,
            Self::BudgetSpent,
            Self::Busy,
            Self::TooLarge,
            Self::TooLong,
            Self::TooShort,
            Self::UnsupportedType,
            Self::Malformed,
            Self::Unauthenticated,
            Self::Refused,
            Self::ProviderFailed,
            Self::TimedOut,
        ]
        .into_iter()
        .find(|outcome| outcome.label() == label)
    }

    /// The HTTP status that carries this outcome.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const fn status(self) -> u16 {
        match self {
            Self::Heard | Self::NothingHeard | Self::Available => 200,
            Self::Malformed => 400,
            Self::Unauthenticated => 401,
            Self::Refused => 403,
            Self::Busy => 409,
            Self::TooLarge | Self::TooLong => 413,
            Self::UnsupportedType => 415,
            Self::TooShort => 422,
            Self::BudgetSpent => 429,
            Self::ProviderFailed => 502,
            Self::Unavailable => 503,
            Self::TimedOut => 504,
        }
    }
}

/// The exact duration of `pcm_bytes` of 16 kHz mono 16-bit samples: 62.5 microseconds each,
/// the figure the speech provider adapter checks the request against.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pcm_duration(pcm_bytes: usize) -> Duration {
    let samples = (pcm_bytes / BYTES_PER_SAMPLE) as u64;
    Duration::from_nanos(samples * 1_000_000_000 / u64::from(SAMPLE_RATE))
}

/// Mono 16-bit little-endian PCM at 16 kHz from samples in `-1.0..=1.0`, wrapped in a WAV
/// container. Non-finite samples are silence; out-of-range samples are clamped.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_bytes = samples.len() * BYTES_PER_SAMPLE;
    let mut wav = Vec::with_capacity(WAV_HEADER_BYTES + data_bytes);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * BYTES_PER_SAMPLE as u32).to_le_bytes());
    wav.extend_from_slice(&(BYTES_PER_SAMPLE as u16).to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for sample in samples {
        let value = if sample.is_finite() {
            (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
        } else {
            0
        };
        wav.extend_from_slice(&value.to_le_bytes());
    }
    wav
}

/// Why a body is not an acceptable clip container.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContainerFault {
    NotWav,
    WrongFormat,
}

#[cfg(not(target_arch = "wasm32"))]
fn le_u16(bytes: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(..2)?.try_into().ok()?))
}

#[cfg(not(target_arch = "wasm32"))]
fn le_u32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
}

/// The PCM samples of a WAV that is exactly mono 16-bit integer PCM at 16 kHz. Other chunks
/// are skipped; a data chunk that claims more than the body holds is refused.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn wav_pcm(body: &[u8]) -> Result<&[u8], ContainerFault> {
    if body.get(..4) != Some(b"RIFF") || body.get(8..12) != Some(b"WAVE") {
        return Err(ContainerFault::NotWav);
    }
    let mut format_ok = false;
    let mut at = 12;
    while let (Some(id), Some(size)) = (
        body.get(at..at + 4),
        body.get(at + 4..)
            .and_then(le_u32)
            .map(|size| size as usize),
    ) {
        let start = at + 8;
        let end = start.checked_add(size).ok_or(ContainerFault::NotWav)?;
        let chunk = body.get(start..end).ok_or(ContainerFault::NotWav)?;
        match id {
            b"fmt " => {
                let fields = (
                    le_u16(chunk),
                    chunk.get(2..).and_then(le_u16),
                    chunk.get(4..).and_then(le_u32),
                    chunk.get(14..).and_then(le_u16),
                );
                if fields != (Some(1), Some(1), Some(SAMPLE_RATE), Some(16)) {
                    return Err(ContainerFault::WrongFormat);
                }
                format_ok = true;
            }
            b"data" => {
                if !format_ok {
                    return Err(ContainerFault::NotWav);
                }
                if chunk.len() % BYTES_PER_SAMPLE != 0 {
                    return Err(ContainerFault::NotWav);
                }
                return Ok(chunk);
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        at = end + (size & 1);
    }
    Err(ContainerFault::NotWav)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|index| (index as f32 / 50.0).sin())
            .collect()
    }

    #[test]
    fn encoded_clip_round_trips_through_the_server_container_check() {
        let wav = encode_wav(&tone(16_000));

        let pcm = wav_pcm(&wav).unwrap();

        assert_eq!(pcm.len(), 32_000);
        assert_eq!(pcm_duration(pcm.len()), Duration::from_secs(1));
    }

    #[test]
    fn samples_are_clamped_and_non_finite_samples_are_silent() {
        let wav = encode_wav(&[2.0, -2.0, f32::NAN, f32::INFINITY]);

        let pcm = wav_pcm(&wav).unwrap();

        assert_eq!(pcm, [0xff, 0x7f, 0x01, 0x80, 0, 0, 0, 0]);
    }

    #[test]
    fn container_that_is_not_the_exact_format_is_refused() {
        let wav = encode_wav(&tone(100));
        let mut stereo = wav.clone();
        stereo[22] = 2;
        let mut other_rate = wav.clone();
        other_rate[24] = 0x44;
        other_rate[25] = 0xac;
        let mut truncated = wav.clone();
        truncated.truncate(60);

        assert_eq!(wav_pcm(&stereo), Err(ContainerFault::WrongFormat));
        assert_eq!(wav_pcm(&other_rate), Err(ContainerFault::WrongFormat));
        assert_eq!(wav_pcm(&truncated), Err(ContainerFault::NotWav));
        assert_eq!(
            wav_pcm(b"OggS and some other bytes"),
            Err(ContainerFault::NotWav)
        );
        assert_eq!(wav_pcm(&[]), Err(ContainerFault::NotWav));
    }

    #[test]
    fn unknown_chunks_before_the_data_are_skipped() {
        let wav = encode_wav(&tone(10));
        let mut padded = wav[..36].to_vec();
        padded.extend_from_slice(b"LIST\x03\0\0\0abc\0");
        padded.extend_from_slice(&wav[36..]);

        assert_eq!(wav_pcm(&padded).unwrap().len(), 20);
    }

    #[test]
    fn every_outcome_label_is_unique_and_parses_back() {
        let labels = [
            SpeechOutcome::Heard,
            SpeechOutcome::NothingHeard,
            SpeechOutcome::Available,
            SpeechOutcome::Unavailable,
            SpeechOutcome::BudgetSpent,
            SpeechOutcome::Busy,
            SpeechOutcome::TooLarge,
            SpeechOutcome::TooLong,
            SpeechOutcome::TooShort,
            SpeechOutcome::UnsupportedType,
            SpeechOutcome::Malformed,
            SpeechOutcome::Unauthenticated,
            SpeechOutcome::Refused,
            SpeechOutcome::ProviderFailed,
            SpeechOutcome::TimedOut,
        ];
        for outcome in labels {
            assert_eq!(SpeechOutcome::from_label(outcome.label()), Some(outcome));
        }
        let mut names: Vec<_> = labels.iter().map(|outcome| outcome.label()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), labels.len());
        assert_eq!(SpeechOutcome::from_label("anything else"), None);
    }
}
