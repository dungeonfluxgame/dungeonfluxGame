//! What the page does when someone clicks, changes a control or presses a key, and when a picture
//! loads or fails.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlImageElement, HtmlSelectElement};

use super::frames::send_change;
use super::page::{on_animation_end, place_bubbles, render};
use super::report::schedule_report;
use super::sound;
use super::state::with_app;
use super::voice;
use crate::audio;
use crate::controls::{ControlsChange, Device};
use crate::copy::ReadingPace;
use crate::page_log;
use crate::timeline::ErrorSource;

fn action_of(event: &Event) -> Option<(Element, String)> {
    let target = event.target()?.dyn_into::<Element>().ok()?;
    let carrier = target.closest("[data-action]").ok()??;
    let action = carrier.get_attribute("data-action")?;
    Some((carrier, action))
}

/// A click or a key press is what lets the browser play sound. Redraws the toolbar when it is the
/// first one, so the sound button says so, and tells the host.
pub fn note_gesture() {
    if audio::unlock() {
        render();
        schedule_report();
    }
}

/// A key press is a gesture too, and Enter in the composer's box sends.
pub fn on_key(event: &Event) {
    note_gesture();
    voice::on_enter(event);
}

/// Someone typed in the composer's box, or moved the volume slider.
pub fn on_input(event: &Event) {
    if sound::on_input(event) {
        return;
    }
    voice::on_input(event);
}

/// The page was hidden or shown.
pub fn on_visibility_change() {
    voice::on_visibility_change();
}

pub fn on_media_event(event: &Event, failed: bool) {
    let Some(target) = event.target() else {
        return;
    };
    let Ok(image) = target.dyn_into::<HtmlImageElement>() else {
        return;
    };
    let Some(name) = image.get_attribute("data-media") else {
        return;
    };
    if let Err(error) = crate::view::plate_fade::on_image(&image, failed) {
        page_log::note_error(ErrorSource::Draw, &crate::dom::js_message(&error));
    }
    if failed {
        page_log::note_error(
            ErrorSource::Asset,
            &format!("picture '{name}' did not load"),
        );
    } else {
        // A speaker's portrait can change the size of the bubble that holds it.
        place_bubbles();
    }
    schedule_report();
}

pub fn on_click(event: &Event) {
    let action = action_of(event);
    note_gesture();
    let Some((_, action)) = action else {
        return;
    };
    if voice::on_action(&action) || sound::on_action(&action) {
        return;
    }
    let change = with_app(|app| match action.split_once(':') {
        Some(("device", key)) => Device::from_key(key).map(ControlsChange::Device),
        Some(("pace", key)) => ReadingPace::from_key(key).map(ControlsChange::Pace),
        Some(("toggle", "unavailable")) => Some(ControlsChange::ShowUnavailable(
            !app.controls.show_unavailable,
        )),
        Some(("toggle", "grid")) => Some(ControlsChange::ShowGrid(!app.controls.show_grid)),
        Some(("toggle", "sam")) => Some(ControlsChange::ShowSam(!app.controls.show_sam)),
        Some(("toggle", "tts")) => Some(ControlsChange::Tts(!app.controls.tts_on)),
        Some(("toggle", "stt")) => Some(ControlsChange::Stt(!app.controls.stt_on)),
        _ => None,
    });
    if let Some(change) = change {
        spawn_local(send_change(change));
    }
}

pub fn on_change(event: &Event) {
    let Some((carrier, action)) = action_of(event) else {
        return;
    };
    let Ok(select) = carrier.dyn_into::<HtmlSelectElement>() else {
        return;
    };
    let value = select.value();
    if action == "talk-to" {
        voice::select(value);
        return;
    }
    let change = match action.as_str() {
        "fixture" => Some(ControlsChange::Fixture(value)),
        "status" => Some(ControlsChange::Status(value)),
        _ => None,
    };
    if let Some(change) = change {
        spawn_local(send_change(change));
    }
}

pub fn on_animation_ended(event: &Event) {
    if let Some(target) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
    {
        on_animation_end(&target);
    }
}

/// The clue board's fold has settled at its new width: the bubbles are placed for that size.
pub fn on_transition_ended(event: &Event) {
    let on_board = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .is_some_and(|target| {
            target
                .class_name()
                .split_whitespace()
                .any(|name| name == "kit-clues")
        });
    if on_board {
        place_bubbles();
    }
}
