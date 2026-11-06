//! The text the dev command line prints for the event and settings verbs.
//!
//! Every list is numbered from 1 like the other engine answers. An event is printed as `#id A
//! 1730000000000 AREA verb key=value ...`: the sink's number, the class (A always on, B
//! verbose), the time it was seen in milliseconds since the Unix epoch, and the audit line
//! as the event rendered it. It formats and never decides, and holds no player text because no
//! report does.

use crate::observe::{OutletStatus, SinkHealth};

use super::{list_or_none, numbered, submission_text, turn_text};
use crate::engine::report::observe::{
    Applies, ConfigReport, EventView, HistoryReport, ReplayReport, RetentionView, SettingView,
    TraceReport,
};

fn event_text(event: &EventView) -> String {
    format!(
        "#{} {} {} {}",
        event.audit_id, event.class, event.observed_ms, event.line
    )
}

fn retention_text(retention: &RetentionView) -> String {
    let range = match (retention.oldest_audit_id, retention.newest_audit_id) {
        (Some(oldest), Some(newest)) => format!(", #{oldest} to #{newest}"),
        _ => String::new(),
    };
    format!(
        "{} events retained{range}, {} forgotten since the run began",
        retention.retained, retention.evicted
    )
}

fn outlet_text(outlet: &OutletStatus) -> String {
    let mut text = format!(
        "outlet {}: {} accepted, {} confirmed, {} pending, lag {} ms, lost {} class A and {} class \
         B",
        outlet.name,
        outlet.accepted,
        outlet.confirmed,
        outlet.pending,
        outlet.lag_ms,
        outlet.lost_always,
        outlet.lost_verbose
    );
    if outlet.evicted > 0 {
        text.push_str(&format!(", forgot {}", outlet.evicted));
    }
    if outlet.unconfirmed > 0 {
        text.push_str(&format!(", {} receipts failed", outlet.unconfirmed));
    }
    text
}

/// The lines `health` adds for the event stream, numbered from `first`.
pub fn stream_lines(stream: &SinkHealth, first: usize) -> String {
    let summary = format!(
        "events at level {}: {} seen ({} class A, {} class B), {} class B left out by the level, \
         {} not recordable; ingestion lag {} ms; {} class A and {} class B lost; {}",
        stream.level,
        stream.seen,
        stream.always,
        stream.verbose,
        stream.filtered_verbose,
        stream.unrenderable,
        stream.lag_ms,
        stream.lost_always,
        stream.lost_verbose,
        if stream.degraded {
            "DEGRADED: this is not a complete record"
        } else {
            "complete so far"
        },
    );
    std::iter::once(summary)
        .chain(stream.outlets.iter().map(outlet_text))
        .enumerate()
        .map(|(index, line)| format!("{}: {line}", first + index))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_history(report: &HistoryReport) -> String {
    let events = list_or_none(report.events.iter().map(event_text), "no event is retained");
    format!(
        "{}; showing the newest {} of the {} that match\n{events}\nevent stream:\n{}",
        retention_text(&report.retention),
        report.events.len(),
        report.matched,
        stream_lines(&report.stream, 1),
    )
}

pub fn format_trace(report: &TraceReport) -> String {
    let events = if report.events.is_empty() {
        format!(
            "no event of operation {} is retained ({}); an event may have been forgotten, or the \
             input never reached the host",
            report.operation,
            retention_text(&report.retention)
        )
    } else {
        format!(
            "operation {}: {} events, in the order they were recorded ({})\n{}",
            report.operation,
            report.events.len(),
            retention_text(&report.retention),
            numbered(report.events.iter().map(event_text)),
        )
    };
    let queue = list_or_none(
        report
            .submissions
            .iter()
            .map(submission_text)
            .chain(report.turns.iter().map(turn_text)),
        "the queue remembers nothing of it",
    );
    format!("{events}\nwhat the queue did:\n{queue}")
}

fn setting_text(setting: &SettingView) -> String {
    let applies = match setting.applies {
        Applies::NextInput => "changes the next input",
        Applies::Restart => "read at start, cannot change while running",
    };
    format!(
        "{} = {} (default {}, {} to {} {}; {}; cost effect {}; {applies})",
        setting.name,
        setting.value,
        setting.default,
        setting.minimum,
        setting.maximum,
        setting.unit,
        setting.owner,
        setting.cost_effect,
    )
}

pub fn format_config(report: &ConfigReport) -> String {
    let settings = numbered(report.settings.iter().map(setting_text));
    match &report.changed {
        Some(change) => format!(
            "changed {} from {} to {}\n{settings}",
            change.name, change.from, change.to
        ),
        None => settings,
    }
}

pub fn format_replay(report: &ReplayReport) -> String {
    let verdict = match report.first_difference_turn {
        Some(turn) => format!("differs from the record at turn {turn}"),
        None if report.matches_recorded => "matches the record".to_owned(),
        None => "does not match the record".to_owned(),
    };
    let head = format!(
        "1: replayed {} turns of {}, final state {}, {verdict}",
        report.turns_replayed, report.log, report.final_state_hash
    );
    if report.differences.is_empty() {
        return head;
    }
    let differences = report
        .differences
        .iter()
        .enumerate()
        .map(|(index, line)| format!("{}: {line}", index + 2))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{head}\n{differences}")
}
