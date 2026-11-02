//! A playground for gameplay display: the shared screen and a phone, drawn in the browser from
//! fixture files, so the table experience can be designed and judged without the engine.
//!
//! The model (`fixture`, `copy`, `choices`, `layout`, `figure`) is plain Rust that native tests cover. The
//! drawing (`dom`, `net`, `app` and the views) is browser-only and thin: it turns the model into
//! elements, and it reloads styles and fixtures when their files change, so a visual change needs
//! no rebuild. Browser crates do not depend on the engine, so this crate holds its own view data.

pub mod backoff;
pub mod camera;
pub mod checks;
pub mod choices;
pub mod continuity;
pub mod controls;
pub mod copy;
pub mod draw_plan;
pub mod figure;
pub mod fixture;
pub mod fixture_checks;
pub mod layout;
pub mod limits;
pub mod media;
pub mod motion_style;
pub mod scene;
mod scene_apply;
pub mod sound_plan;
pub mod style;

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod audio;
#[cfg(target_arch = "wasm32")]
mod dom;
#[cfg(target_arch = "wasm32")]
mod net;
#[cfg(target_arch = "wasm32")]
mod view_phone;
#[cfg(target_arch = "wasm32")]
mod view_tv;

#[cfg(test)]
mod backoff_tests;
#[cfg(test)]
mod camera_tests;
#[cfg(test)]
mod choices_tests;
#[cfg(test)]
mod continuity_tests;
#[cfg(test)]
mod controls_tests;
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod css_tests;
#[cfg(test)]
mod draw_plan_tests;
#[cfg(test)]
mod figure_tests;
#[cfg(test)]
mod fixture_checks_tests;
#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod motion_style_tests;
#[cfg(test)]
mod scene_media_tests;
#[cfg(test)]
mod scene_tests;
#[cfg(test)]
mod shipped_fixtures_tests;
#[cfg(test)]
mod slide_tests;
#[cfg(test)]
mod sound_plan_tests;
#[cfg(test)]
mod style_tests;

/// Starts the playground when the page loads.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    app::start()
}
