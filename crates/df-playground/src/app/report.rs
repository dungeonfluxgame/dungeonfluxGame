//! What this page tells the host it shows and plays, and what it showed and met since its last
//! report. The dev CLI reads it: an agent cannot see or hear the page.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlImageElement;

use super::state::{log_error, try_with_app, with_app_mut};
use crate::audio;
use crate::dom::{document, js_message};
use crate::media::{ClientReport, ClientReportPost};
use crate::net::post_json;
use crate::page_log::{now_ms, with_log};
use crate::path;
use crate::timeline::{ErrorSource, LinkState, TimelineRecord, ViewportPx};

/// The pictures on the page that loaded without error, by media name.
fn images_shown() -> Vec<String> {
    let nodes = match document().and_then(|document| document.query_selector_all("img[data-media]"))
    {
        Ok(nodes) => nodes,
        Err(error) => {
            log_error(
                ErrorSource::Draw,
                &format!("could not list the pictures: {}", js_message(&error)),
            );
            return Vec::new();
        }
    };
    let mut shown = Vec::new();
    for index in 0..nodes.length() {
        if let Some(node) = nodes.item(index)
            && let Ok(image) = node.dyn_into::<HtmlImageElement>()
            && image.complete()
            && image.natural_width() > 0
            && let Some(name) = image.get_attribute("data-media")
            && !shown.contains(&name)
        {
            shown.push(name);
        }
    }
    shown
}

/// The size of the window, in CSS pixels, when the browser says.
fn viewport_px() -> Option<ViewportPx> {
    let window = web_sys::window()?;
    let size = |value: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
        value.ok()?.as_f64().map(|pixels| pixels.max(0.0) as u32)
    };
    Some(ViewportPx {
        width: size(window.inner_width())?,
        height: size(window.inner_height())?,
    })
}

/// The report without its timeline and clock, or `None` when the page's state is in use.
fn build_report() -> Option<ClientReport> {
    let status = audio::status();
    let errors = with_log(|log| log.errors()).unwrap_or_default();
    let images_shown = images_shown();
    try_with_app(|app| ClientReport {
        audio: status.state,
        last_cue: status.last_cue,
        music: status.music,
        images_shown,
        errors,
        frame: app.frame,
        boot: app.gate.boot_id().map(str::to_owned),
        failures: app.failures,
        link: if app.connection_problem.is_some() {
            LinkState::Down
        } else {
            LinkState::Up
        },
        asset_revision: app.asset_revision.clone(),
        fixture: app.loaded_name.clone(),
        fixture_error: match &app.fixture {
            Some(Err(error)) => Some(error.cause()),
            _ => None,
        },
        device: app.controls.device,
        viewport_px: viewport_px(),
        pace: app.controls.pace,
        page_clock_ms: 0,
        timeline: Vec::new(),
    })
}

/// Tells the host what this page shows and plays, when that has changed, when there are timeline
/// records to deliver, or when `force` says to (a sign of life, a change of link or revision, a
/// panic). The records are forgotten only once a report carrying them is delivered.
pub async fn send_report(force: bool) {
    let Some(report) = build_report() else {
        return;
    };
    let Some(client) = try_with_app(|app| app.client_id.clone()) else {
        return;
    };
    let (timeline, through): (Vec<TimelineRecord>, u64) =
        with_log(|log| log.unreported()).unwrap_or_default();
    let compared = match serde_json::to_string(&report) {
        Ok(text) => text,
        Err(error) => {
            log_error(
                ErrorSource::Report,
                &format!("could not write the report: {error}"),
            );
            return;
        }
    };
    let changed =
        try_with_app(|app| app.last_report.as_deref() != Some(compared.as_str())).unwrap_or(true);
    if !(changed || force || !timeline.is_empty()) {
        return;
    }
    let post = ClientReportPost {
        client,
        report: ClientReport {
            page_clock_ms: now_ms(),
            timeline,
            ..report
        }
        .bounded(),
    };
    let text = match serde_json::to_string(&post) {
        Ok(text) => text,
        Err(error) => {
            log_error(
                ErrorSource::Report,
                &format!("could not write the report: {error}"),
            );
            return;
        }
    };
    match post_json(path::CLIENT_REPORT, &text).await {
        Ok(_) => {
            with_log(|log| log.acknowledge(through));
            with_app_mut(|app| app.last_report = Some(compared));
        }
        Err(error) => {
            log_error(
                ErrorSource::Report,
                &format!("could not report to the host: {error}"),
            );
        }
    }
}

pub fn schedule_report() {
    spawn_local(send_report(false));
}

/// Sends a report even when nothing changed: the link went down or came back, or the files did.
pub fn schedule_forced_report() {
    spawn_local(send_report(true));
}
