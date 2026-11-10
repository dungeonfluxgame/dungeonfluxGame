//! The toolbar: the scenario, the devices shown, what to simulate (a status and the reading pace),
//! the overlays on the screen and the sound. Each control only names the action a click or a
//! change on it asks for; `events` turns that into a change sent to the host.
//!
//! Controls come in groups that never break inside, and three kinds look different: a choice of
//! one (a segment of options), an on/off switch, and a one-shot action.

use std::cell::Cell;

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element};

use super::sound::{MUSIC_ACTION, SOUND_ACTION, VOLUME_ACTION};
use super::state::{App, try_with_app};
use crate::audio;
use crate::controls::{Device, FIXTURE_STATUS_KEY};
use crate::copy::{ReadingPace, STATUS_KEYS};
use crate::dom::{append, button, element, now_ms, option, select, text_element};
use crate::media::AudioState;

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
    let button = button(document, "studio-option", label, action)?;
    button.set_attribute("aria-pressed", if chosen { "true" } else { "false" })?;
    Ok(button)
}

fn switch(document: &Document, action: &str, label: &str, on: bool) -> Result<Element, JsValue> {
    let button = button(document, "studio-switch", label, action)?;
    button.set_attribute("aria-pressed", if on { "true" } else { "false" })?;
    Ok(button)
}

fn action_button(document: &Document, action: &str, label: &str) -> Result<Element, JsValue> {
    button(document, "studio-action", label, action)
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

/// The two voice switches: lines spoken aloud (text to speech) and questions taken by voice
/// (speech to text). The host keeps them, so the dev CLI and every page see the same ones.
fn speech_switches(document: &Document, app: &App) -> Result<Element, JsValue> {
    let controls = &app.controls;
    group(
        document,
        "Speech",
        &[
            switch(document, "toggle:tts", "TTS", controls.tts_on)?,
            switch(document, "toggle:stt", "STT", controls.stt_on)?,
        ],
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

/// The sound: a button that lets the browser play sound, and once it has, a plain "Sound on";
/// then this viewer's own switches (all sound, the music) and master volume.
fn sound_control(document: &Document) -> Result<Element, JsValue> {
    let control = if audio::state() == AudioState::Unlocked {
        let state = text_element(document, "span", "studio-state", "Sound on")?;
        state.set_attribute("role", "status")?;
        state
    } else {
        action_button(document, "audio:unlock", "Enable sound")?
    };
    let prefs = audio::prefs();
    let volume = element(document, "input", "studio-volume")?;
    for (name, value) in [
        ("type", "range"),
        ("min", "0"),
        ("max", "100"),
        ("step", "1"),
        ("aria-label", "Volume"),
        ("data-action", VOLUME_ACTION),
    ] {
        volume.set_attribute(name, value)?;
    }
    volume.set_attribute("value", &prefs.volume_percent.to_string())?;
    group(
        document,
        "Sound",
        &[
            control,
            switch(document, SOUND_ACTION, "Sound", prefs.sound_on)?,
            switch(document, MUSIC_ACTION, "Music", prefs.music_on)?,
            field(document, "Volume", &volume)?,
        ],
    )
}

/// How often the plate timer's words are read again: once a second, so it never changes faster.
const PLATE_TICK_MS: u32 = 1_000;

thread_local! {
    /// The host clock of the plate timer last read and the page's clock when it was first seen,
    /// so the countdown runs on between the host's replies; and whether the tick runs.
    static PLATE_SEEN: Cell<Option<(u64, f64)>> = const { Cell::new(None) };
    static PLATE_TICKING: Cell<bool> = const { Cell::new(false) };
}

/// Notes that the page took a reply of the host whose plate timer carries `host_ms`, so the timer
/// counts on from the moment the reply came, not from the next tick.
pub fn note_plate_reply(host_ms: u64) {
    let page_ms = now_ms().unwrap_or_default();
    PLATE_SEEN.with(|seen| seen.set(Some((host_ms, page_ms))));
}

/// Writes the plate timer's words into the toolbar now, when they changed. The tick, a reply of the
/// host and a page shown again all call it, so the words are never older than the last of them.
/// The state being in use is skipped, not waited for: a panic here would end the tick for good.
pub fn refresh_plate() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(Some(timer)) = document.query_selector(".studio-plate") else {
        return;
    };
    let Some(words) = try_with_app(plate_words) else {
        return;
    };
    if timer.text_content().as_deref() != Some(words.as_str()) {
        timer.set_text_content(Some(&words));
    }
}

/// The plate timer's words now: the host's timer, as many milliseconds on as the page has run
/// since it took that reply.
fn plate_words(app: &App) -> String {
    let plate = &app.controls.plate;
    let page_ms = now_ms().unwrap_or_default();
    let seen = PLATE_SEEN.with(|seen| match seen.get() {
        Some((host_ms, page_seen)) if host_ms == plate.now_ms => page_seen,
        _ => {
            seen.set(Some((plate.now_ms, page_ms)));
            page_ms
        }
    });
    // A float of whole milliseconds since a moment before it, never negative.
    let elapsed_ms = (page_ms - seen).max(0.0) as u64;
    plate.shown_text(elapsed_ms)
}

/// The plate timer: the frame watch's schedule as the host says it (the page works out none), a
/// countdown to the next scheduled frame, the frame being made and the last frame's kind and age.
/// A tick, started once, rewrites its words once a second, only when they changed.
/// A page that was hidden is caught up when it is shown (`events::on_visibility_change`).
fn plate_timer(document: &Document, app: &App) -> Result<Element, JsValue> {
    let timer = text_element(
        document,
        "span",
        "studio-state studio-plate",
        &plate_words(app),
    )?;
    // A timer is not read out on every change.
    timer.set_attribute("role", "timer")?;
    if !PLATE_TICKING.with(|ticking| ticking.replace(true)) {
        spawn_local(async {
            loop {
                TimeoutFuture::new(PLATE_TICK_MS).await;
                refresh_plate();
            }
        });
    }
    group(document, "Plate", &[timer])
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
    append(&bar, &speech_switches(document, app)?)?;
    append(&bar, &plate_timer(document, app)?)?;
    Ok(bar)
}
