//! The unified AI service boundary: text requests with durable attempts and their telemetry,
//! listener-safe expression requests, recording-stream admission and read-only artifact lookup.
//! Provider backends live elsewhere; this crate sends no network or process call itself.
pub mod admission;
pub mod expression;
pub mod lookup;
pub mod text;
pub mod text_attempts;
pub mod text_fixture;
#[cfg(not(target_arch = "wasm32"))]
pub mod text_telemetry;
