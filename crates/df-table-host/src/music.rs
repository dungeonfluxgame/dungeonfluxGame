//! The music engine's dev verb: `scene music generate`, the background music of one narrative
//! scene, composed from a prompt through ElevenLabs music and played looping like `scene music`.
//!
//! A track is named per narrative scene (`as=NAME`). When the media library already holds a sound
//! of that name it is played again and nothing is composed, so a new track is made only when the
//! scene changes and the caller names a new one. Otherwise one instrumental of the asked length
//! (two minutes unless told) is composed, stored as `NAME.mp3` through the library's staged upload,
//! and set as the scene's looping music through the same change `scene music` sends.
//!
//! The composition runs on a task of its own under a deadline (`limits::COMPOSE_DEADLINE`), so a
//! client that leaves cannot lose music already paid for. A failure is a typed refusal and changes
//! nothing on the scene: the music that was playing keeps playing.
//!
//! Reading order: `limits`, `request` (the body, the reply and `MusicError`), `backend` (the
//! composer and its ElevenLabs adapter), `routes` (the route and the reuse-or-compose flow) and
//! `cli` (sending the verb and printing its answer). The grammar is in `command::scene`.

mod backend;
pub mod cli;
pub mod limits;
pub mod request;
pub mod routes;

#[cfg(test)]
pub(crate) use self::backend::{ComposeFuture, Composer};
pub use self::request::{MusicError, MusicPlayed, MusicProviderCause, MusicRequest, MusicSource};
pub use self::routes::MusicService;

#[cfg(test)]
mod music_tests;
