//! The text the dev command line prints for each answer of the owner.
//!
//! Every list is numbered from 1 (`1: ...`), like the answers of the canvas commands, so an answer
//! can be read aloud and a line can be named. It formats and never decides: it reads the typed
//! reports of `report` and prints their fields. Player text never reaches it, because no report
//! holds any.

pub(super) mod observe;

use crate::broker::ledger::{LedgerEnd, LedgerEntry};
use crate::broker::purpose::Price;
use crate::broker::report::{LedgerReport, QueuesReport};

use self::observe::stream_lines;
use super::report::{
    BrokerRequested, ChaosReport, ClockReport, CommitDisposition, CommitReport, Description,
    DiceReport, ForceReport, HealthReport, QueueReport, ServedReply, SubmissionReport, SubmitReply,
    TurnReport, ViableReport,
};

pub(super) fn numbered(lines: impl IntoIterator<Item = String>) -> String {
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| format!("{}: {line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn list_or_none(lines: impl IntoIterator<Item = String>, none: &str) -> String {
    let text = numbered(lines);
    if text.is_empty() {
        none.to_owned()
    } else {
        text
    }
}

fn commit_text(commit: &CommitDisposition) -> String {
    match commit {
        CommitDisposition::Committed { id } => format!("committed as {id}"),
        CommitDisposition::NothingToCommit => "nothing to save".to_owned(),
        CommitDisposition::Failed(error) => format!("not saved ({})", error.key()),
    }
}

pub(super) fn submission_text(submission: &SubmissionReport) -> String {
    let head = format!(
        "{} {} ({})",
        submission.operation, submission.sender, submission.kind
    );
    match (&submission.refusal, submission.lane) {
        (Some(refusal), _) => {
            let mut text = format!(
                "{head}: refused ({}): {}",
                refusal.cause.key(),
                refusal.detail
            );
            if let Some(retry) = refusal.retry_after_ms {
                text.push_str(&format!("; retry in {retry} ms"));
            }
            if let Some(first) = &refusal.first_status {
                text.push_str(&format!("; the first copy is {first}"));
            }
            text
        }
        (None, Some(lane)) => format!("{head}: admitted to the {} lane", lane.key()),
        (None, None) => format!("{head}: admitted"),
    }
}

pub(super) fn turn_text(turn: &TurnReport) -> String {
    let outcome = match turn.decline {
        Some(code) => format!("{} ({})", turn.outcome.key(), code.key()),
        None => turn.outcome.key().to_owned(),
    };
    let action = turn
        .action
        .as_ref()
        .map_or(String::new(), |action| format!(" action={action}"));
    let narrated = turn
        .narration
        .map_or(String::new(), |class| format!(" narration={}", class.key()));
    let replayed = if turn.replayed { ", replayed" } else { "" };
    format!(
        "turn {} {} {} from the {} lane after {} ms: {outcome}{action}, {}:{} -> {}:{}, \
         {} effects, {}{narrated}{replayed}, \"{}\"",
        turn.turn_seq,
        turn.sender,
        turn.kind,
        turn.lane.key(),
        turn.waited_ms,
        turn.machine_before,
        turn.state_before,
        turn.machine_after,
        turn.state_after,
        turn.effects,
        commit_text(&turn.commit),
        turn.readout,
    )
}

pub fn format_submit(reply: &SubmitReply) -> String {
    numbered(
        std::iter::once(submission_text(&reply.submission))
            .chain(reply.turns.iter().map(turn_text)),
    )
}

pub fn format_served(reply: &ServedReply) -> String {
    list_or_none(reply.turns.iter().map(turn_text), "no input was served")
}

pub fn format_viable(report: &ViableReport) -> String {
    if report.actions.is_empty() {
        return format!(
            "seat {} can do nothing now in {} ({})",
            report.seat, report.machine, report.state
        );
    }
    let actions = numbered(report.actions.iter().cloned());
    format!(
        "seat {} can do, in {} ({}):\n{actions}",
        report.seat, report.machine, report.state
    )
}

pub fn format_force(report: &ForceReport) -> String {
    format!(
        "1: the table is now in {} ({}) at revision {}, {}",
        report.machine,
        report.state,
        report.revision,
        commit_text(&report.commit)
    )
}

pub fn format_view(view: &serde_json::Value) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(view)
}

fn commit_report_text(commit: &CommitReport) -> String {
    let mut text = format!(
        "commit: {} policy, deadline {} ms, {} commits, {} failures",
        commit.policy, commit.deadline_ms, commit.commits, commit.failures
    );
    if let (Some(last), Some(bytes)) = (commit.last_commit, commit.last_checkpoint_bytes) {
        text.push_str(&format!(", last commit {last} of {bytes} bytes"));
    }
    if let Some(remaining) = commit.backing_off_for_ms {
        text.push_str(&format!(", refusing input for {remaining} ms more"));
    }
    text
}

pub fn format_queue(report: &QueueReport) -> String {
    let lanes = numbered(
        report
            .lanes
            .iter()
            .map(|lane| format!("{} {}", lane.lane.key(), lane.depth)),
    );
    let in_progress = list_or_none(
        report.in_progress.iter().map(|operation| {
            format!(
                "{} {} turn {} for {} ms",
                operation.sender, operation.operation, operation.turn_seq, operation.age_ms
            )
        }),
        "none",
    );
    let seats = list_or_none(
        report
            .seats
            .iter()
            .filter(|seat| seat.pending > 0)
            .map(|seat| format!("seat {} has {} pending", seat.seat, seat.pending)),
        "none",
    );
    let submissions = list_or_none(report.submissions.iter().map(submission_text), "none");
    let turns = list_or_none(report.turns.iter().map(turn_text), "none");
    format!(
        "serving: {}, at {} ms\nlanes, in the order they are served:\n{lanes}\nin \
         progress:\n{in_progress}\n\
         pending by seat:\n{seats}\noperations remembered: {}, last turn: {}\n{}\n\
         submissions, oldest first:\n{submissions}\nturns, oldest first:\n{turns}",
        report.serving,
        report.now_ms,
        report.remembered_operations,
        report.last_turn_seq,
        commit_report_text(&report.commit),
    )
}

pub fn format_description(description: &Description) -> String {
    let machines = numbered(description.machines.iter().map(|machine| {
        format!(
            "{} ({}, {}{}) states: {}; hands off to: {}; rules actions: {}; actions: {}",
            machine.machine,
            machine.scope,
            if machine.enabled {
                "enabled"
            } else {
                "disabled"
            },
            if machine.active { ", active" } else { "" },
            machine.states.join(", "),
            machine.hand_off_targets.join(", "),
            machine.rules_actions.join(", "),
            machine.actions.join(", "),
        )
    }));
    let lanes: Vec<&str> = description.lanes.iter().map(|lane| lane.key()).collect();
    let verbs = numbered(description.verbs.iter().cloned());
    let verb_set = numbered(description.verb_set.iter().map(|line| {
        format!(
            "{} ({}, {}): {}",
            line.words,
            line.effect,
            line.access,
            line.host.describe(),
        )
    }));
    let not_built = list_or_none(
        description
            .not_built
            .iter()
            .map(|verb| format!("{} (waits for: {})", verb.verb, verb.needs)),
        "none",
    );
    format!(
        "machines:\n{machines}\nlanes, in the order they are served: {}\nports wired: {}\n\
         commit policy: {}\nverbs:\n{verbs}\n\
         the verb set of C7's inspect, and what this host has of it:\n{verb_set}\n\
         H.16 verbs not built yet:\n{not_built}",
        lanes.join(", "),
        description.ports.join(", "),
        description.commit_policy,
    )
}

pub fn format_requested(requested: &BrokerRequested) -> String {
    format!(
        "1: requested {}: handle {}, {}",
        requested.purpose, requested.handle, requested.readiness
    )
}

pub fn format_broker_queues(report: &QueuesReport) -> String {
    let classes = numbered(report.classes.iter().map(|class| {
        let oldest = class
            .oldest_age_ms
            .map_or("nothing waiting".to_owned(), |age| {
                format!("oldest waited {age} ms")
            });
        format!(
            "{}: {} waiting, {oldest}, {} dropped",
            class.class.key(),
            class.depth,
            class.dropped
        )
    }));
    let running = list_or_none(
        report.running.iter().map(|call| {
            format!(
                "attempt {} of request {} for {} ({}), {}, lease to {} ms",
                call.attempt,
                call.handle,
                call.purpose,
                call.class.key(),
                if call.sent { "sent" } else { "not sent yet" },
                call.lease_expires_ms
            )
        }),
        "none",
    );
    format!(
        "epoch {}{}; {} text slots, {} image slots\nclasses, in the order they are \
         served:\n{classes}\nrunning:\n{running}",
        report.epoch,
        if report.closed { " (closed)" } else { "" },
        report.text_slots,
        report.image_slots,
    )
}

fn price_text(price: Price) -> String {
    match price {
        Price::Priced { micro_dollars } => format!("{micro_dollars} micro-dollars"),
        Price::Unpriced => "unpriced".to_owned(),
    }
}

fn end_text(end: LedgerEnd) -> String {
    match end {
        LedgerEnd::Open => "open".to_owned(),
        LedgerEnd::Ready => "ready".to_owned(),
        LedgerEnd::Failed { cause } => format!("failed ({})", cause.key()),
        LedgerEnd::Cancelled => "cancelled".to_owned(),
        LedgerEnd::Superseded => "superseded".to_owned(),
        LedgerEnd::Dropped => "dropped".to_owned(),
    }
}

fn entry_text(entry: &LedgerEntry) -> String {
    format!(
        "request {} for {} ({}), {}, {} calls sent, {}",
        entry.handle,
        entry.purpose,
        entry.class.key(),
        price_text(entry.price),
        entry.calls_sent,
        end_text(entry.end),
    )
}

pub fn format_broker_ledger(report: &LedgerReport) -> String {
    let totals = list_or_none(
        report.totals.iter().map(|line| {
            format!(
                "{}: {} requests, {} calls sent, {} micro-dollars spent, {} unpriced calls",
                line.purpose,
                line.totals.requests,
                line.totals.calls_sent,
                line.totals.spent_micro_dollars,
                line.totals.unpriced_calls
            )
        }),
        "nothing requested yet",
    );
    let entries = list_or_none(report.entries.iter().map(entry_text), "none");
    format!(
        "spent so far, by purpose (recorded, never limited):\n{totals}\nnewest requests ({} older \
         forgotten):\n{entries}",
        report.forgotten_entries
    )
}

pub fn format_health(report: &HealthReport) -> String {
    let mut text = format!(
        "1: owner {}, serving {}, {} inputs waiting, {} commits, {} failures",
        report.owner, report.serving, report.waiting_inputs, report.commits, report.failures
    );
    if let Some(age) = report.oldest_in_progress_ms {
        text.push_str(&format!(", the oldest input has run {age} ms"));
    }
    if let Some(remaining) = report.backing_off_for_ms {
        text.push_str(&format!(", refusing input for {remaining} ms more"));
    }
    text.push_str(&format!(
        "\n2: broker at epoch {}{}\n3: the owner's clock reads {} ms\n{}",
        report.broker_epoch,
        if report.broker_closed {
            " (closed)"
        } else {
            ""
        },
        report.now_ms,
        stream_lines(&report.events, 4),
    ));
    text
}

pub fn format_clock(report: &ClockReport) -> String {
    format!(
        "1: the clock reads {} ms and is {}",
        report.now_ms,
        if report.frozen { "frozen" } else { "running" }
    )
}

pub fn format_dice(report: &DiceReport) -> String {
    let seed = report
        .seed
        .map_or(String::new(), |seed| format!(", seeded with {seed}"));
    format!(
        "1: {} forced values wait before the seeded sequence continues{seed}",
        report.forced_waiting
    )
}

pub fn format_chaos(report: &ChaosReport) -> String {
    format!("1: {} fault in force: {}", report.fault, report.in_force)
}
