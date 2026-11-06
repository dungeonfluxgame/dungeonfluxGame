//! What this page tells the host it shows and plays. The dev CLI reads it: an agent cannot see or
//! hear the page.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlImageElement;

use super::state::{log_error, with_app, with_app_mut};
use crate::audio;
use crate::dom::{document, js_message};
use crate::media::{ClientReport, ClientReportPost};
use crate::net::post_json;
use crate::path;

/// The pictures on the page that loaded without error, by media name.
fn images_shown() -> Vec<String> {
    let nodes = match document().and_then(|document| document.query_selector_all("img[data-media]"))
    {
        Ok(nodes) => nodes,
        Err(error) => {
            log_error(&format!(
                "could not list the pictures: {}",
                js_message(&error)
            ));
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

fn build_report() -> ClientReport {
    let status = audio::status();
    ClientReport {
        audio: status.state,
        last_cue: status.last_cue,
        music: status.music,
        images_shown: images_shown(),
        errors: status.errors,
    }
    .bounded()
}

/// Tells the host what this page shows and plays, when that has changed (or when `force` says to,
/// as a sign of life).
pub async fn send_report(force: bool) {
    let post = ClientReportPost {
        client: with_app(|app| app.client_id.clone()),
        report: build_report(),
    };
    let text = match serde_json::to_string(&post) {
        Ok(text) => text,
        Err(error) => {
            log_error(&format!("could not write the report: {error}"));
            return;
        }
    };
    let send = with_app_mut(|app| {
        let changed = app.last_report.as_deref() != Some(text.as_str());
        if changed || force {
            app.last_report = Some(text.clone());
        }
        changed || force
    });
    if send && let Err(error) = post_json(path::CLIENT_REPORT, &text).await {
        log_error(&format!("could not report to the host: {error}"));
    }
}

pub fn schedule_report() {
    spawn_local(send_report(false));
}
