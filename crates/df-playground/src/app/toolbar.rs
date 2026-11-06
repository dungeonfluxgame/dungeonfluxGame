//! The toolbar: the scenario, the devices shown, what to simulate (a status and the reading pace),
//! the overlays on the screen and the sound. Each control only names the action a click or a
//! change on it asks for; `events` turns that into a change sent to the host.
//!
//! Controls come in groups that never break inside, and three kinds look different: a choice of
//! one (a segment of options), an on/off switch, and a one-shot action.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use super::state::App;
use crate::audio;
use crate::controls::{Device, FIXTURE_STATUS_KEY};
use crate::copy::{ReadingPace, STATUS_KEYS};
use crate::dom::{append, element, set_action, text_element};
use crate::media::AudioState;

fn option(
    document: &Document,
    value: &str,
    label: &str,
    selected: bool,
) -> Result<Element, JsValue> {
    let option = text_element(document, "option", "", label)?;
    option.set_attribute("value", value)?;
    if selected {
        option.set_attribute("selected", "selected")?;
    }
    Ok(option)
}

fn select(document: &Document, label: &str, action: &str) -> Result<Element, JsValue> {
    let select = element(document, "select", "")?;
    select.set_attribute("aria-label", label)?;
    set_action(&select, action)?;
    Ok(select)
}

/// A labelled control inside a group: the small label, then the control.
fn field(document: &Document, label: &str, control: &Element) -> Result<Element, JsValue> {
    let field = element(document, "label", "studio-field")?;
    append(
        &field,
        &text_element(document, "span", "studio-sublabel", label)?,
    )?;
    append(&field, control)?;
    Ok(field)
}

/// A group of controls under a small label. Rows of the toolbar break only between groups.
fn group(document: &Document, label: &str, children: &[Element]) -> Result<Element, JsValue> {
    let group = element(document, "div", "studio-group")?;
    group.set_attribute("role", "group")?;
    group.set_attribute("aria-label", label)?;
    append(
        &group,
        &text_element(document, "span", "studio-label", label)?,
    )?;
    let controls = element(document, "div", "studio-controls")?;
    for child in children {
        append(&controls, child)?;
    }
    append(&group, &controls)?;
    Ok(group)
}

/// A choice of one: the options of a segment, the chosen one filled. `label` names the segment
/// as a group of its own; it is `None` when the group around it already says the same.
fn segment(
    document: &Document,
    label: Option<&str>,
    options: &[(String, &str, bool)],
) -> Result<Element, JsValue> {
    let segment = element(document, "div", "studio-segment")?;
    if let Some(label) = label {
        segment.set_attribute("role", "group")?;
        segment.set_attribute("aria-label", label)?;
    }
    for (action, text, chosen) in options {
        append(&segment, &segment_option(document, action, text, *chosen)?)?;
    }
    Ok(segment)
}

fn segment_option(
    document: &Document,
    action: &str,
    label: &str,
    chosen: bool,
) -> Result<Element, JsValue> {
    let button = text_element(document, "button", "studio-option", label)?;
    button.set_attribute("type", "button")?;
    button.set_attribute("aria-pressed", if chosen { "true" } else { "false" })?;
    set_action(&button, action)?;
    Ok(button)
}

fn switch(document: &Document, action: &str, label: &str, on: bool) -> Result<Element, JsValue> {
    let button = text_element(document, "button", "studio-switch", label)?;
    button.set_attribute("type", "button")?;
    button.set_attribute("aria-pressed", if on { "true" } else { "false" })?;
    set_action(&button, action)?;
    Ok(button)
}

fn action_button(document: &Document, action: &str, label: &str) -> Result<Element, JsValue> {
    let button = text_element(document, "button", "studio-action", label)?;
    button.set_attribute("type", "button")?;
    set_action(&button, action)?;
    Ok(button)
}

/// The scenario menu: each option shows the scenario's title, or its file name when the file is
/// not a valid scenario, and keeps the file name as its value and its tooltip.
fn scenario_field(document: &Document, app: &App) -> Result<Element, JsValue> {
    let menu = select(document, "Scenario", "fixture")?;
    let chosen = app.controls.fixture.as_deref().unwrap_or("");
    for fixture in &app.fixtures {
        let text = fixture.title.as_deref().unwrap_or(&fixture.file);
        let item = option(document, &fixture.file, text, fixture.file == chosen)?;
        item.set_attribute("title", &fixture.file)?;
        append(&menu, &item)?;
    }
    group(document, "Scenario", &[menu])
}

fn view_segment(document: &Document, app: &App) -> Result<Element, JsValue> {
    let options = [
        (Device::Both, "Both"),
        (Device::Tv, "Screen"),
        (Device::Phone, "Phone"),
    ]
    .map(|(device, label)| {
        (
            format!("device:{}", device.key()),
            label,
            app.controls.device == device,
        )
    });
    group(document, "View", &[segment(document, None, &options)?])
}

/// The words a status key shows in the menu: its key in sentence case.
fn status_label(key: &str) -> String {
    let words = key.replace('_', " ");
    let mut letters = words.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => words,
    }
}

fn pace_label(pace: ReadingPace) -> &'static str {
    match pace {
        ReadingPace::Slow => "Slow",
        ReadingPace::Normal => "Normal",
        ReadingPace::Fast => "Fast",
    }
}

/// The status to try and the reading pace. A status other than the scenario's own marks its menu
/// as changed, so an override is visible.
fn simulate_group(document: &Document, app: &App) -> Result<Element, JsValue> {
    let status = &app.controls.status_key;
    let menu = select(document, "Status", "status")?;
    append(
        &menu,
        &option(
            document,
            FIXTURE_STATUS_KEY,
            "As in scenario",
            status == FIXTURE_STATUS_KEY,
        )?,
    )?;
    for key in STATUS_KEYS {
        append(
            &menu,
            &option(document, key, &status_label(key), status == key)?,
        )?;
    }
    if status != FIXTURE_STATUS_KEY {
        menu.set_attribute("data-modified", "true")?;
    }
    let paces = ReadingPace::ALL.map(|pace| {
        (
            format!("pace:{}", pace.key()),
            pace_label(pace),
            app.controls.pace == pace,
        )
    });
    let pace = element(document, "div", "studio-field")?;
    append(
        &pace,
        &text_element(document, "span", "studio-sublabel", "Pace")?,
    )?;
    append(&pace, &segment(document, Some("Reading pace"), &paces)?)?;
    group(
        document,
        "Simulate",
        &[field(document, "Status", &menu)?, pace],
    )
}

fn overlay_switches(document: &Document, app: &App) -> Result<Element, JsValue> {
    let controls = &app.controls;
    group(
        document,
        "Overlays",
        &[
            switch(
                document,
                "toggle:unavailable",
                "Unavailable",
                controls.show_unavailable,
            )?,
            switch(document, "toggle:grid", "Grid", controls.show_grid)?,
            switch(document, "toggle:sam", "SAM", controls.show_sam)?,
        ],
    )
}

/// The sound: a button that lets the browser play sound, and once it has, a plain "Sound on".
fn sound_control(document: &Document) -> Result<Element, JsValue> {
    let control = if audio::state() == AudioState::Unlocked {
        let state = text_element(document, "span", "studio-state", "Sound on")?;
        state.set_attribute("role", "status")?;
        state
    } else {
        action_button(document, "audio:unlock", "Enable sound")?
    };
    group(document, "Sound", &[control])
}

pub fn toolbar(document: &Document, app: &App) -> Result<Element, JsValue> {
    let bar = element(document, "header", "studio-bar")?;
    append(
        &bar,
        &text_element(document, "h1", "studio-title", "Playground")?,
    )?;
    append(&bar, &scenario_field(document, app)?)?;
    append(&bar, &view_segment(document, app)?)?;
    append(&bar, &simulate_group(document, app)?)?;
    append(&bar, &overlay_switches(document, app)?)?;
    append(&bar, &sound_control(document)?)?;
    Ok(bar)
}
