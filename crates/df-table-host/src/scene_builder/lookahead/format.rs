//! The text `scene lookahead` prints: the planner's settings, the scenes the story expects, the
//! book of built and building scenes, the counters of idea 1's metrics, and the audit lines of the
//! call. It formats and never decides; every list is numbered from 1 like the other scene answers.

use df_table::lookahead::{BookReport, LookaheadCounts, LookaheadReport, WantedReport};

use super::request::LookaheadReply;
use crate::scene_builder::format::{numbered, seconds, timed};

fn wanted_line(wanted: &WantedReport) -> String {
    let how = if wanted.certain {
        "certain".to_owned()
    } else {
        format!("{} thousandths likely", wanted.likelihood_permille)
    };
    format!(
        "scene {} ({}): {}, {how}",
        wanted.scene, wanted.place, wanted.expectation
    )
}

fn book_line(entry: &BookReport) -> String {
    let failure = entry
        .failure
        .as_ref()
        .map_or_else(String::new, |failure| format!(" ({failure})"));
    format!(
        "scene {} build {}: {}{failure}, {} old, {}",
        entry.scene,
        entry.token,
        entry.state,
        seconds(entry.age_ms),
        entry.expectation
    )
}

fn counts_line(counts: &LookaheadCounts) -> String {
    format!(
        "started {}, built {}, hit {}, redressed {}, stale {}, late {}, miss {}, failed {}, cancelled {}, expired {}, refused results {}",
        counts.started,
        counts.built,
        counts.hit,
        counts.redressed,
        counts.stale,
        counts.late,
        counts.miss,
        counts.failed,
        counts.cancelled,
        counts.expired,
        counts.refused_results
    )
}

/// The report alone, as `scene lookahead show` prints it.
pub fn format_report(now_ms: u64, report: &LookaheadReport) -> String {
    let state = match (report.enabled, report.paused) {
        (false, _) => "off",
        (true, true) => "paused",
        (true, false) => "on",
    };
    let mut text = format!(
        "look-ahead at {}: {state}, depth {}, a likely scene starts {} s after the scene is ready, a build is cancelled after {} s, a built scene is kept {} s",
        seconds(now_ms),
        report.depth,
        report.start_s,
        report.deadline_s,
        report.book_age_s
    );
    text.push_str("\nexpected next:");
    if report.wanted.is_empty() {
        text.push_str(" nothing");
    } else {
        text.push('\n');
        text.push_str(&numbered(report.wanted.iter().map(wanted_line)));
    }
    text.push_str("\nbook:");
    if report.book.is_empty() {
        text.push_str(" empty");
    } else {
        text.push('\n');
        text.push_str(&numbered(report.book.iter().map(book_line)));
    }
    text.push_str("\ncounts: ");
    text.push_str(&counts_line(&report.counts));
    text
}

pub fn format_reply(reply: &LookaheadReply) -> String {
    let mut text = format_report(reply.now_ms, &reply.report);
    if !reply.lines.is_empty() {
        text.push_str("\nthis call:\n");
        text.push_str(&numbered(reply.lines.iter().map(timed)));
    }
    text
}
