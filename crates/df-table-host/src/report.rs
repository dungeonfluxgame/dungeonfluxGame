//! Turning the API's typed answers into the text a person reads at the terminal.
//!
//! It owns wording and layout only. It never sends a request and never reads a file.

use crate::view::CanvasView;
use crate::wire::{EventsReply, Reply};

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
