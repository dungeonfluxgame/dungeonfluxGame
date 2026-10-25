//! The campaign service (`planning/campaigns-and-table-research.md`): campaigns, membership and
//! the role matrix, invitations and room codes, display pairing, the event feed and account
//! closure. Owns the rules and the ports; the PostgreSQL adapter lives in `df-persistence`
//! (`campaign_store`) and serves JSON through the route module. It never serves HTML, never runs
//! on a game owner thread and never calls a provider.
#![forbid(unsafe_code)]
#![cfg(not(target_arch = "wasm32"))]

// TEMPORARY (round 1): the pure policy files live in `df-auth` and `df-session`, whose module
// lists are held by B. They are compiled here by path until B declares them there; then these
// become `pub use df_auth::{roles, short_code, keyed_throttle}` and
// `pub use df_session::{join_invitation, display_pairing, lobby}` and nothing else changes. The files
// reach `crate::throttle`, which this crate provides below.
mod throttle {
    pub use df_auth::throttle::*;
}
#[path = "../../df-session/src/display_pairing.rs"]
pub mod display_pairing;
#[path = "../../df-session/src/join_invitation.rs"]
pub mod join_invitation;
#[path = "../../df-auth/src/keyed_throttle.rs"]
pub mod keyed_throttle;
#[path = "../../df-session/src/lobby.rs"]
pub mod lobby;
#[path = "../../df-auth/src/roles.rs"]
pub mod roles;
#[path = "../../df-auth/src/short_code.rs"]
pub mod short_code;

pub mod audit;
pub mod clock;
pub mod config;
pub mod draft;
pub mod errors;
#[cfg(any(test, feature = "test-fakes"))]
pub mod fakes;
pub mod ids;
pub mod lobby_state;
pub mod records;
#[cfg(feature = "routes")]
pub mod routes;
pub mod secrets;
pub mod service;
pub mod store;
pub mod throttles;
