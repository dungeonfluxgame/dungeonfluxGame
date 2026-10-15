//! C01: compile-only browser surface checks; coordinator runs browser acceptance.
#![cfg(target_arch = "wasm32")]

#[path = "../src/gameplay_browser/hero_creator.rs"]
mod hero_creator;
