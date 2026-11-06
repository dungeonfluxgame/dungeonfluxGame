//! Turning the API's typed answers into the text a person reads at the terminal.
//!
//! It owns wording and layout only. It never sends a request and never reads a file.

mod clients;
mod host;

use serde::Deserialize;

pub use self::clients::{format_clients, format_timeline};
pub use self::host::{
    FrameShapeError, format_frame, format_frame_unchanged, format_host_status, format_media_head,
    frame_count,
};
use crate::view::CanvasView;
use crate::wire::{ControlsReply, EventsReply};

/// The text of a change that was made (`Reply::Done`): its revision and its notices. A refusal
/// prints as its `DevError::Rejected`.
pub fn describe_done(revision: u64, notices: &[String]) -> String {
    let mut text = format!("ok (revision {revision})");
    for notice in notices {
        text.push_str(&format!("\n  {notice}"));
    }
    text
}

/// The canvas as readable JSON.
pub fn format_canvas(view: &CanvasView) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(view)
}

/// Each kind with its fields, a star marking a required one.
pub fn format_kinds(view: &CanvasView) -> String {
    if view.kinds.is_empty() {
        return "no kinds loaded (add a file to the kind directory, then: reload)".to_owned();
    }
    let mut lines = Vec::new();
    for kind in &view.kinds {
        lines.push(format!(
            "{} ({:?} {}x{}) {}",
            kind.name, kind.shape, kind.width_units, kind.height_units, kind.description
        ));
        for field in &kind.fields {
            let required = if field.required { "*" } else { "" };
            let default = field
                .default
                .as_ref()
                .map_or_else(String::new, |default| format!(" default {default}"));
            lines.push(format!(
                "  {}{required}: {}{default}",
                field.name, field.description
            ));
        }
    }
    lines.join("\n")
}

/// Event lines oldest first, each with the time it was recorded (`at_ms`, milliseconds since the
/// Unix epoch), a note when the host has restarted since the reader last looked and a
/// `forgotten=N` header when the log has dropped lines to keep its bounds.
pub fn format_events(reply: &EventsReply, after: Option<u64>) -> String {
    let restarted = after.is_some_and(|after| after > reply.latest);
    let mut lines: Vec<String> = Vec::new();
    if reply.forgotten > 0 {
        lines.push(format!("forgotten={}", reply.forgotten));
    }
    if reply.events.is_empty() {
        lines.push("no events".to_owned());
        return lines.join("\n");
    }
    if restarted {
        lines.push("(the host has restarted; showing the newest events)".to_owned());
    }
    lines.extend(
        reply
            .events
            .iter()
            .map(|line| format!("{:>4}  {:>13}  {}", line.sequence, line.at_ms, line.text)),
    );
    lines.join("\n")
}

/// What the media library holds against its bounds, as the listing reports it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MediaLimits {
    pub used_bytes: u64,
    pub files: u64,
    pub maximum_file_bytes: u64,
    pub maximum_upload_body_bytes: u64,
}

/// The media listing: its files, and the library's limits when the host reports them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum MediaListing {
    /// A host that lists only the files.
    Files(Vec<df_playground::media::MediaEntry>),
    Library {
        #[serde(alias = "media")]
        entries: Vec<df_playground::media::MediaEntry>,
        limits: MediaLimits,
    },
}

/// The media library, one file per line, then its limits when the listing has them.
pub fn format_media_listing(listing: &MediaListing) -> String {
    match listing {
        MediaListing::Files(entries) => format_media(entries),
        MediaListing::Library { entries, limits } => format!(
            "{}\nlimits: used_bytes={} files={} maximum_file_bytes={} \
             maximum_upload_body_bytes={}",
            format_media(entries),
            limits.used_bytes,
            limits.files,
            limits.maximum_file_bytes,
            limits.maximum_upload_body_bytes
        ),
    }
}

/// What the playground is showing, one setting per line, and the fixtures it can show.
pub fn format_playground(reply: &ControlsReply, fixtures: &[String]) -> String {
    let controls = &reply.controls;
    let device = controls.device.key();
    let pace = controls.pace.key();
    let fixture = controls.fixture.as_deref().unwrap_or("(none)");
    let scene = changed_parts(&reply.scene);
    let listed = if fixtures.is_empty() {
        "(none)".to_owned()
    } else {
        fixtures.join(", ")
    };
    format!(
        "fixture: {fixture}
device: {device}
pace: {pace}
status: {}
unavailable: {}
grid: {}
sam: {}
zoom_limit: {}
transition: {} {} ms
scene: {scene}
revision: {}
fixtures: {listed}",
        controls.status_key,
        controls.show_unavailable,
        controls.show_grid,
        controls.show_sam,
        controls.zoom_limit.key(),
        controls.transition.style.key(),
        controls.transition.duration_ms,
        reply.revision
    )
}

/// Which parts of the scene have live changes on top of the fixture.
fn changed_parts(scene: &df_playground::scene::SceneOverlay) -> String {
    let parts = scene.changed_slots();
    if parts.is_empty() {
        "as the fixture has it".to_owned()
    } else {
        format!("changed: {}", parts.join(", "))
    }
}

/// The media library, one file per line.
pub fn format_media(entries: &[df_playground::media::MediaEntry]) -> String {
    if entries.is_empty() {
        return "no media (add one with: media add FILE)".to_owned();
    }
    entries
        .iter()
        .map(|entry| {
            format!(
                "{:<32} {:<5} {} bytes",
                entry.name,
                entry.kind.key(),
                entry.bytes
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
