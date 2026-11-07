//! The bounds of the speech feature. A bound tied to the provider's contract says so; the rest
//! only protect against mistakes and are generous.

use std::time::Duration;

use df_playground::voice_wire::MissingSetting;

/// The variables the speech supplier's key is read from, in order.
pub const KEY_VARIABLES: [&str; 2] = [MissingSetting::SpeechKey.variable(), "ELEVENLABS_API_KEY"];

/// The content type of a transcription body: raw signed 16-bit little-endian mono PCM at 16 kHz.
pub use df_playground::path::SPEECH_CONTENT_TYPE as AUDIO_CONTENT_TYPE;
/// Bytes of audio in one millisecond of 16 kHz 16-bit mono PCM (the input contract).
pub const AUDIO_BYTES_PER_MS: usize = 32;
/// Shortest audio the speech-to-text supplier takes, in milliseconds (its contract).
pub const MINIMUM_AUDIO_MS: u64 = 100;
/// Longest audio the speech-to-text supplier takes, in milliseconds (its contract).
pub const MAXIMUM_AUDIO_MS: u64 = 60_000;
/// Most key terms one transcription is biased with (the supplier adapter's contract).
pub const MAXIMUM_KEY_TERMS: usize = 32;
/// Most characters of one key term (the supplier adapter's contract).
pub const MAXIMUM_KEY_TERM_CHARACTERS: usize = 49;
/// Most words of one key term (the supplier adapter's contract).
pub const MAXIMUM_KEY_TERM_WORDS: usize = 5;

/// Most characters one synthesis voices (the supplier adapter's contract).
pub const MAXIMUM_TEXT_CHARACTERS: usize = 4096;
/// Longest voice id, in bytes (the supplier adapter's contract).
pub const MAXIMUM_VOICE_ID_BYTES: usize = 128;
/// Most voice clips the host keeps in the media library; the oldest it made go first.
pub const MAXIMUM_KEPT_CLIPS: usize = 500;
/// Most characters with a voice of their own in the voices file.
pub const MAXIMUM_VOICES: usize = 100_000;
/// Largest voices file read, in bytes.
pub const MAXIMUM_VOICES_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// How long one speech call may take from admission to its answer.
pub const SPEECH_DEADLINE: Duration = Duration::from_secs(5 * 60);
/// Largest request one speech call sends, in bytes (60 s of audio is 1.92 MB plus its envelope).
pub const MAXIMUM_PROVIDER_REQUEST_BYTES: usize = 8 * 1024 * 1024;
/// Largest answer one speech call keeps, in bytes.
pub const MAXIMUM_PROVIDER_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
/// Largest payload the request owner admits for a transcription, in bytes.
pub const MAXIMUM_ADMITTED_AUDIO_BYTES: usize = 2_000_000;
/// Largest payload the request owner admits for a synthesis, in bytes (four bytes a character).
pub const MAXIMUM_ADMITTED_TEXT_BYTES: usize = 4 * MAXIMUM_TEXT_CHARACTERS;
