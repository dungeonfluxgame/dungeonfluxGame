//! The trusted per-host runner's launch layer (hosted runtime design 7): one isolated runtime
//! process per active campaign, started through a launcher port with a native adapter for
//! local development and a Docker adapter that talks to the body-checking proxy. The runner
//! holds no game, payment or provider credentials and starts every runtime with an empty
//! environment; its configuration reaches the runtime only as a read-only descriptor file.
#![forbid(unsafe_code)]
#![cfg(not(target_arch = "wasm32"))]

pub mod docker;
#[cfg(any(test, feature = "test-fakes"))]
pub mod docker_fake;
pub mod launcher;
pub mod native;
pub mod process;
pub mod spec;

#[cfg(test)]
mod docker_tests;
