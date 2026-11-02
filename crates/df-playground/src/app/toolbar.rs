//! The toolbar: the scenario choice, the devices shown, the status to try, the reading pace, the
//! two display toggles and the sound switch. Each control only names the action a click or a
//! change on it asks for; `events` turns that into a change sent to the host.

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

fn select(
    document: &Document,
    label: &str,
    action: &str,
    options: &[(String, String)],
    chosen: &str,
) -> Result<Element, JsValue> {
    let wrapper = element(document, "label", "control")?;
    append(
        &wrapper,
        &text_element(document, "span", "control-label", label)?,
    )?;
    let select = element(document, "select", "")?;
    set_action(&select, action)?;
    for (value, text) in options {
        append(&select, &option(document, value, text, value == chosen)?)?;
    }
    append(&wrapper, &select)?;
    Ok(wrapper)
}

fn button(document: &Document, action: &str, label: &str, on: bool) -> Result<Element, JsValue> {
    let button = text_element(
        document,
        "button",
        if on { "toggle on" } else { "toggle" },
        label,
    )?;
    button.set_attribute("type", "button")?;
    button.set_attribute("aria-pressed", if on { "true" } else { "false" })?;
    set_action(&button, action)?;
    Ok(button)
}

fn scenario_select(document: &Document, app: &App) -> Result<Element, JsValue> {
    let names: Vec<(String, String)> = app
        .fixture_names
        .iter()
        .map(|name| (name.clone(), name.clone()))
        .collect();
    select(
        document,
        "Scenario",
        "fixture",
        &names,
        app.controls.fixture.as_deref().unwrap_or(""),
    )
}

fn device_buttons(document: &Document, app: &App) -> Result<Element, JsValue> {
    let devices = element(document, "div", "control")?;
    append(
        &devices,
        &text_element(document, "span", "control-label", "Show")?,
    )?;
    let segment = element(document, "div", "segment")?;
    for (device, label) in [
        (Device::Both, "Both"),
        (Device::Tv, "Screen"),
        (Device::Phone, "Phone"),
    ] {
        append(
            &segment,
            &button(
                document,
                &format!("device:{}", device.key()),
                label,
                app.controls.device == device,
            )?,
        )?;
    }
    append(&devices, &segment)?;
    Ok(devices)
}

fn status_select(document: &Document, app: &App) -> Result<Element, JsValue> {
    let mut statuses = vec![(
        FIXTURE_STATUS_KEY.to_owned(),
        "As in the scenario".to_owned(),
    )];
    statuses.extend(
        STATUS_KEYS
            .iter()
            .map(|key| ((*key).to_owned(), key.replace('_', " "))),
    );
    select(
        document,
        "Try a status",
        "status",
        &statuses,
        &app.controls.status_key,
    )
}

fn pace_select(document: &Document, app: &App) -> Result<Element, JsValue> {
    let paces = [("slow", "Slow"), ("normal", "Normal"), ("fast", "Fast")]
        .map(|(key, label)| (key.to_owned(), label.to_owned()));
    let current_pace = match app.controls.pace {
        ReadingPace::Slow => "slow",
        ReadingPace::Normal => "normal",
        ReadingPace::Fast => "fast",
    };
    select(document, "Reading pace", "pace", &paces, current_pace)
}

pub fn toolbar(document: &Document, app: &App) -> Result<Element, JsValue> {
    let bar = element(document, "header", "toolbar")?;
    append(
        &bar,
        &text_element(document, "h1", "", "Gameplay display playground")?,
    )?;
    append(&bar, &scenario_select(document, app)?)?;
    append(&bar, &device_buttons(document, app)?)?;
    append(&bar, &status_select(document, app)?)?;
    append(&bar, &pace_select(document, app)?)?;
    append(
        &bar,
        &button(
            document,
            "toggle:unavailable",
            "Show unavailable",
            app.controls.show_unavailable,
        )?,
    )?;
    append(
        &bar,
        &button(document, "toggle:grid", "Show grid", app.controls.show_grid)?,
    )?;
    let unlocked = audio::state() == AudioState::Unlocked;
    append(
        &bar,
        &button(
            document,
            "audio:unlock",
            if unlocked { "Sound on" } else { "Enable sound" },
            unlocked,
        )?,
    )?;
    Ok(bar)
}
