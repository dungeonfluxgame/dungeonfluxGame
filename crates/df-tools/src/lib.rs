//! Runnable S00 experiment. The fixture UI is Rust compiled to WebAssembly.
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
pub mod build_set;
#[cfg(not(target_arch = "wasm32"))]
pub mod dev_cli;
#[cfg(not(target_arch = "wasm32"))]
mod file_identity;
#[cfg(not(target_arch = "wasm32"))]
pub mod fixture;
#[cfg(not(target_arch = "wasm32"))]
pub mod gameplay;
#[cfg(target_arch = "wasm32")]
mod gameplay_browser;
#[cfg(not(target_arch = "wasm32"))]
pub mod runtime;
#[cfg(all(target_arch = "wasm32", feature = "public-scene-delivery-fixture"))]
pub use gameplay_browser::fixture as public_scene_delivery_fixture;
#[cfg(target_arch = "wasm32")]
mod generated_browser_bindings;
#[cfg(not(target_arch = "wasm32"))]
mod preview;
#[cfg(target_arch = "wasm32")]
mod qualification;
#[cfg(target_arch = "wasm32")]
mod qualification_verdict;

mod layout_contract;
mod music_score;
#[cfg(any(target_arch = "wasm32", test))]
mod reconnect;
#[cfg(any(target_arch = "wasm32", test))]
mod sound_switch;
mod speech_clip;
mod voice_line;

// PLAY-L01: when a same-revision display view may replace the shown one (generated art only).
#[cfg(any(target_arch = "wasm32", test))]
mod live_art;
#[cfg(any(target_arch = "wasm32", test))]
mod plate_layout;
#[cfg(any(target_arch = "wasm32", test))]
mod presentation;
#[cfg(any(target_arch = "wasm32", test))]
mod scene_script;
// FIGHT-001: pure dice-face plan shared by the TV overlay and the phone mini, unit-tested.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) mod dice_roll;
#[cfg(any(target_arch = "wasm32", test))]
mod speech_capture;
// PLAY-H01: the one definition of the health tiers, shared by the server and both clients.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) mod combat_text;
pub(crate) mod health_tier;
// PLAY-H01: shake sizes and timings (pure), used by the browser's health cues.
#[cfg(any(target_arch = "wasm32", test))]
mod health_cues;
#[cfg(any(target_arch = "wasm32", test))]
mod player_sheet;
mod stage_art;
#[cfg(any(target_arch = "wasm32", test))]
mod stage_conductor;
#[cfg(any(target_arch = "wasm32", test))]
mod stage_depth;
#[cfg(any(target_arch = "wasm32", test))]
mod stage_director;
#[cfg(any(target_arch = "wasm32", test))]
mod stage_machines;

/// Source identity injected by the reproducible fixture build command.
pub const BUILD_ID: &str = match option_env!("DF_FIXTURE_BUILD") {
    Some(value) => value,
    None => "unregistered-cargo-build",
};
