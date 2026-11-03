//! The text of `director show` and of a verb's answer. The log rows are the audit lines of the
//! entries themselves, so the screen and the event log cannot differ.

use df_table::director_log::EndsReport;
use df_table::element::ElementId;

use super::request::{ActReply, ShowReply};

/// An element's name, or what is said in place of one the audience may not be told.
fn name(element: &Option<ElementId>) -> String {
    element
        .as_ref()
        .map_or_else(|| "(withheld)".to_owned(), ToString::to_string)
}

/// A verb's answer: the lines it wrote, or that nothing moved, and anything the playground refused.
pub fn format_act(reply: &ActReply) -> String {
    let mut text = if reply.lines.is_empty() {
        "no move".to_owned()
    } else {
        reply.lines.join("\n")
    };
    for problem in &reply.problems {
        text.push_str(&format!("\nnot applied: {problem}"));
    }
    text
}

/// `director show`.
pub fn format_show(show: &ShowReply) -> String {
    let report = &show.report;
    let on_off = |on: bool| if on { "on" } else { "off" };
    let mut text = format!(
        "director: clock {} ms, {} moves made, {}, auto camera {}\n  zoom ceiling {}% ({})\n",
        report.now_ms,
        report.serial,
        if report.paused { "paused" } else { "running" },
        on_off(report.auto_camera),
        report.ceiling_percent,
        show.view.plate,
    );
    match &report.active {
        Some(active) => {
            let ends = match active.ends {
                EndsReport::At(at) => format!("returns wide at {at} ms"),
                EndsReport::WhenReasonEnds(limit) => {
                    format!("returns when the reason ends (safety limit {limit} ms)")
                }
            };
            text.push_str(&format!(
                "  holding: {} for {} (move {}, {}%, since {} ms), {ends}\n",
                name(&active.element),
                active.reason,
                active.serial,
                active.zoom_percent,
                active.since_ms,
            ));
        }
        None => text.push_str("  holding: nothing, the camera is wide\n"),
    }
    match &report.waiting {
        Some(waiting) => text.push_str(&format!(
            "  waiting: {} for {} since {} ms\n",
            name(&waiting.element),
            waiting.reason,
            waiting.since_ms
        )),
        None => text.push_str("  waiting: nothing\n"),
    }
    text.push_str(&format!(
        "  directions: {} queued, {} applied, {} dropped\n",
        report.queued, report.applied, report.dropped
    ));
    let list = |items: &[String]| {
        if items.is_empty() {
            "none".to_owned()
        } else {
            items.join(", ")
        }
    };
    text.push_str(&format!(
        "  sees: hidden {}; heroes {}; collapsed beat {}; reduced effects {}; stage {} px",
        list(&show.view.hidden),
        list(&show.view.heroes),
        on_off(show.view.collapsed_beat),
        on_off(show.view.reduced_effects),
        show.view.stage_px,
    ));
    if let Some(beat) = &show.view.beat {
        text.push_str(&format!("; {beat}"));
    }
    text.push_str("\n  log, newest last:");
    if report.log.is_empty() {
        text.push_str(" empty");
    }
    for entry in &report.log {
        for line in entry.lines(report.audience) {
            text.push_str(&format!(
                "\n    #{} @{} ms  {line}",
                entry.sequence, entry.at_ms
            ));
        }
    }
    text
}
