//! The tempo engine on the dev host, and the music director that follows it.
//!
//! A journey (a lone traveller's day, a quest) posts typed impulses to the tempo engine of
//! `df-tempo`; the engine keeps one intensity with its inertia, fatigue and phase, advanced at
//! the host's clock. A music director reads the engine at each boundary (an impulse, a tick, a
//! scheduled wake-up) and plays the profile of the music table that the tempo calls for: the
//! table is data (`development/market-threads/free-roam.music.json`), the tracks come from the
//! music engine's reuse-or-compose flow, and a track change is a rate-limited dip fade of the
//! page's one music track. Nothing in the host knows a story or a style.
//!
//! Routes (each answers a `TempoReply`, the shared dev envelope with a typed `TempoError`):
//!
//! - `GET /dev/tempo`: a `TempoView` (phase, intensity, inertia, momentum, recent impulses, the
//!   cue that plays and the newest decision).
//! - `POST /dev/tempo/impulse`: `{"kind", "intensity"?, "note"?}`, answers the new `TempoView`.
//! - `POST /dev/tempo/tick`, `POST /dev/tempo/reset`: a boundary with no impulse; a reset starts
//!   the journey again and plays the base profile.
//! - `POST /dev/tempo/table`: the table file as the body; answers `TableLoaded`.
//! - `POST /dev/tempo/prepare`: finds or composes every profile's track; answers a
//!   `PrepareReport`. Only tracks prepared this way are ever played.
//!
//! The dev CLI's `tempo` verbs are these routes. Reading order: `limits`, `table` (the data
//! file), `request` (bodies, replies, `TempoError`), `run` (the engine), `cue` (the choice),
//! `routes` (the boundary, the fade and the routes) and `cli`.

pub mod cli;
mod cue;
pub mod limits;
pub mod request;
pub mod routes;
mod run;
pub mod table;

pub use self::cli::{TempoCommand, parse};
pub use self::request::TempoError;

#[cfg(test)]
mod cue_tests;
#[cfg(test)]
mod routes_tests;
#[cfg(test)]
mod table_tests;
