//! Turning the API's typed answers into the text a person reads at the terminal.
//!
//! It owns wording and layout only. It never sends a request and never reads a file.

use crate::view::CanvasView;
use crate::wire::{ClientView, ControlsReply, EventsReply, Reply};

/// The text of an answer to a change: `Ok` when the change was made, `Err` when it was not.
pub fn describe_reply(reply: &Reply) -> Result<String, String> {
    match reply {
        Reply::Done { revision, notices } => {
            let mut text = format!("ok (revision {revision})");
            for notice in notices {
                text.push_str(&format!("\n  {notice}"));
            }
            Ok(text)
        }
        Reply::Rejected {
            message, problems, ..
        } => {
            let mut text = message.clone();
            for problem in problems {
                text.push_str(&format!("\n  {problem}"));
            }
            Err(text)
        }
    }
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

/// Event lines oldest first, with a note when the host has restarted since the reader last looked.
pub fn format_events(reply: &EventsReply, after: Option<u64>) -> String {
    let restarted = after.is_some_and(|after| after > reply.latest);
    if reply.events.is_empty() {
        return "no events".to_owned();
    }
    let mut lines: Vec<String> = Vec::new();
    if restarted {
        lines.push("(the host has restarted; showing the newest events)".to_owned());
    }
    lines.extend(
        reply
            .events
            .iter()
            .map(|line| format!("{:>4}  {}", line.sequence, line.text)),
    );
    lines.join("\n")
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
transition: {} {} ms
scene: {scene}
revision: {}
fixtures: {listed}",
        controls.status_key,
        controls.show_unavailable,
        controls.show_grid,
        controls.transition.style.key(),
        controls.transition.duration_ms,
        reply.revision
    )
}

/// Which parts of the scene have live changes on top of the fixture.
fn changed_parts(scene: &df_playground::scene::SceneOverlay) -> String {
    let parts: Vec<&str> = [
        ("caption", scene.caption.is_some()),
        ("party", scene.party.is_some()),
        ("suggested", scene.suggested.is_some()),
        ("groups", scene.groups.is_some()),
        ("unavailable", scene.unavailable.is_some()),
        ("backdrop", scene.backdrop.is_some()),
        ("card", scene.card.is_some()),
        ("phone picture", scene.phone_image.is_some()),
        ("music", scene.music.is_some()),
    ]
    .into_iter()
    .filter_map(|(name, changed)| changed.then_some(name))
    .collect();
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

/// What each open page says it is showing and playing. An agent cannot see or hear a screen, so
/// this is how it checks that a picture or a sound arrived.
pub fn format_clients(clients: &[ClientView]) -> String {
    use df_playground::media::AudioState;
    if clients.is_empty() {
        return "clients: none (open the playground page)".to_owned();
    }
    let mut lines = Vec::new();
    for client in clients {
        let report = &client.report;
        let audio = match report.audio {
            AudioState::Unlocked => "audio unlocked",
            AudioState::Locked => {
                "audio LOCKED (the browser plays sound only after someone clicks the page)"
            }
        };
        let music = report.music.as_deref().unwrap_or("none");
        let shown = if report.images_shown.is_empty() {
            "none".to_owned()
        } else {
            report.images_shown.join(", ")
        };
        lines.push(format!(
            "client {} ({:.1} s ago): {audio}; last sound cue {}; music {music}; pictures shown {shown}",
            client.client,
            client.age_ms as f64 / 1000.0,
            report.last_cue
        ));
        for error in &report.errors {
            lines.push(format!("  error: {error}"));
        }
    }
    lines.join("\n")
}
