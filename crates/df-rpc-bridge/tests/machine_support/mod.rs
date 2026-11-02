//! A sample machine server shared by the native round-trip test and the browser example.
//!
//! It is not part of the crate and not a game server. It answers `ActionService.SubmitMachineIntent`
//! and `SessionService.WatchMachine` from the hand-built samples of `df_api::machine::sample`, and
//! says `unimplemented` to every other method, so the bytes that cross the bridge are the real
//! generated messages and the real HTTP/2 stream semantics.
pub mod server;
