//! Direct Google OIDC. Only the server exchanges codes; no Google token survives validation.
#![forbid(unsafe_code)]
#![cfg(not(target_arch = "wasm32"))]

mod claims;
mod client;
#[cfg(any(test, feature = "test-fakes"))]
pub mod fake;

pub use client::{Config, ConfigError, Google};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod token_endpoint;
