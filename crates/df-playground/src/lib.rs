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
pub mod contrast;
pub mod controls;
pub mod copy;
pub mod decisions;
pub mod draw_plan;
pub mod figure;
pub mod fixture;
pub mod fixture_checks;
pub mod frame;
pub mod home_adventure;
pub mod home_campaigns;
pub mod home_change;
pub mod home_checks;
pub mod home_chrome;
pub mod home_configure;
pub mod home_configure_changes;
pub mod home_configure_icons;
pub mod home_glyph;
pub mod home_model;
pub mod home_nav;
pub mod layout;
pub mod limits;
pub mod lobby_change;
pub mod lobby_model;
pub mod media;
pub mod motion_style;
pub mod overlay;
pub mod overlay_changes;
pub mod overlay_checks;
pub mod overlay_rules;
pub mod scene;
mod scene_apply;
mod scene_apply_ui;
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
mod view_brand;
#[cfg(target_arch = "wasm32")]
mod view_home;
#[cfg(target_arch = "wasm32")]
mod view_home_actions;
#[cfg(target_arch = "wasm32")]
mod view_home_adventure;
#[cfg(target_arch = "wasm32")]
mod view_home_campaigns;
#[cfg(target_arch = "wasm32")]
mod view_home_chrome;
#[cfg(target_arch = "wasm32")]
mod view_home_configure;
#[cfg(target_arch = "wasm32")]
mod view_home_kit;
#[cfg(target_arch = "wasm32")]
mod view_icons;
#[cfg(target_arch = "wasm32")]
mod view_kit;
#[cfg(target_arch = "wasm32")]
mod view_lobby;
#[cfg(target_arch = "wasm32")]
mod view_lobby_page;
#[cfg(target_arch = "wasm32")]
mod view_lobby_panels;
#[cfg(target_arch = "wasm32")]
mod view_lobby_seat;
#[cfg(target_arch = "wasm32")]
mod view_narration;
#[cfg(target_arch = "wasm32")]
mod view_overlays;
#[cfg(target_arch = "wasm32")]
mod view_phone;
#[cfg(target_arch = "wasm32")]
mod view_pills;
#[cfg(target_arch = "wasm32")]
mod view_roleplay;
#[cfg(target_arch = "wasm32")]
mod view_seats;
#[cfg(target_arch = "wasm32")]
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
mod contrast_tests;
#[cfg(test)]
mod controls_tests;
#[cfg(test)]
mod copy_tests;
#[cfg(test)]
mod css_tests;
#[cfg(test)]
mod decisions_tests;
#[cfg(test)]
mod draw_plan_tests;
#[cfg(test)]
mod figure_tests;
#[cfg(test)]
mod fixture_checks_tests;
#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod home_configure_tests;
#[cfg(test)]
mod home_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod lobby_change_tests;
#[cfg(test)]
mod lobby_tests;
#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod motion_style_tests;
#[cfg(test)]
mod overlay_rules_tests;
#[cfg(test)]
mod overlay_scene_tests;
#[cfg(test)]
mod overlay_tests;
#[cfg(test)]
mod scene_media_tests;
#[cfg(test)]
mod scene_tests;
#[cfg(test)]
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
