//! Voice chat for the dev host: the player's speech to a transcript, and a character's line to a
//! voice clip.
//!
//! One thin port (`provider::SpeechPort`) stands in front of the supplier, so the planned
//! `AudioService` can replace the ElevenLabs adapter without touching a route. The page never
//! talks to the supplier and never holds a key.
//!
//! Routes (each answers a `SpeechReply`, the shared dev envelope with a typed `SpeechError`):
//!
//! - `GET /dev/speech/status`: whether the service is set up (`configured`; the page enables its
//!   microphone on it), the models, the voices and the limits.
//! - `POST /dev/speech/transcribe`: the raw body is 16 kHz mono signed 16-bit little-endian PCM
//!   (`Content-Type: audio/L16;rate=16000`), 0.1 s to 60 s; repeated `?terms=` query keys bias
//!   recognition toward names (by default the shown scene's cast and place). It answers the
//!   transcript, the language and the times. The recording and the transcript live in memory for
//!   the request only: never on disk, never in an audit line, never in the text attempt store.
//!   A transcript is a draft the player edits; it is never an action.
//! - `POST /dev/speech/speak`: `{text, character, voice}` voiced in the character's voice and
//!   played as `scene sound` plays a clip. A dev operator route: only text the operator typed
//!   goes through it. A character's own line is voiced by `persona ask`/`say` with `speak`.
//! - `POST /dev/speech/voice`, `GET /dev/speech/clips`, `POST /dev/speech/clear`: a character's
//!   voice in `voices.json`, the host's `voice-<n>.mp3` clips in the media library, and their
//!   removal.
//!
//! The dev CLI's `speech` verbs are these routes. Reading order: `request` (bodies, replies and
//! `SpeechError`), `provider` (the port), `service` (the work and its audit lines), `routes`,
//! `voices` and `clips` (the two stores), `cli` and `format`.

pub mod cli;
mod clips;
mod format;
pub mod limits;
pub mod provider;
pub mod request;
pub mod routes;
mod service;
mod voices;

pub use self::cli::{SPEECH_USAGE, SpeechCommand, parse};
pub use self::format::format_voiced;
pub use self::provider::SpeechPort;
pub use self::request::{SpeechError, SpeechFeature, Voiced};
pub use self::routes::router_with;
pub use self::service::Speech;
pub(crate) use self::service::sound_change;

#[cfg(test)]
pub(crate) mod fixtures;
#[cfg(test)]
mod speech_tests;
