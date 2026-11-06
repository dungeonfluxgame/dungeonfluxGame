//! Turns the records of a run into the text report: one table per scenario.
//!
//! It owns the layout and the verdict lines against the H.5 budgets, and the split of a long run
//! into minutes so that thermal throttling shows (a fanless laptop slows under sustained load; the
//! first and last minute are reported separately). Everything it prints is computed from
//! `InputRecord`s with the integer statistics of `statistics`; only the final millisecond text uses
//! floating point.

use super::error::HarnessError;
use super::producers::Arrival;
use super::run::{InputRecord, RunResult, Stop};
use super::state::SEAT_COUNT;
use super::statistics::{BusyInterval, Summary, busy_permille_per_window, summarise};

/// H.5: projection compute budget at p95, in nanoseconds.
pub const PROJECTION_BUDGET_NS: u64 = 5_000_000;
/// H.5: the rules-only path budget at p95, in nanoseconds.
pub const RULES_ONLY_BUDGET_NS: u64 = 400_000_000;

const WINDOW_NS: u64 = 1_000_000_000;
const MINUTE_NS: u64 = 60 * WINDOW_NS;

fn milliseconds(ns: u64) -> String {
    format!("{:.3}", ns as f64 / 1_000_000.0)
}

fn summary_of(
    records: &[InputRecord],
    pick: impl Fn(&InputRecord) -> u64,
) -> Result<Summary, HarnessError> {
    let mut values: Vec<u64> = records.iter().map(pick).collect();
    Ok(summarise(&mut values)?)
}

fn summary_of_many(
    records: &[InputRecord],
    pick: impl Fn(&InputRecord) -> Vec<u64>,
) -> Result<Summary, HarnessError> {
    let mut values: Vec<u64> = records.iter().flat_map(pick).collect();
    Ok(summarise(&mut values)?)
}

fn row(name: &str, summary: Summary) -> String {
    format!(
        "{name:<22}{:>9}{:>10}{:>10}{:>10}{:>10}{:>10}",
        summary.count,
        milliseconds(summary.mean_ns),
        milliseconds(summary.p50_ns),
        milliseconds(summary.p95_ns),
        milliseconds(summary.p99_ns),
        milliseconds(summary.max_ns),
    )
}

fn arrival_text(arrival: Arrival) -> String {
    match arrival {
        Arrival::Steady { inputs_per_s } => format!("steady {inputs_per_s}/s"),
        Arrival::Burst { inputs, period_ms } => format!("burst of {inputs} every {period_ms} ms"),
        Arrival::Saturated { window } => format!("saturated, {window} outstanding per seat"),
    }
}

fn stop_text(stop: Stop) -> String {
    match stop {
        Stop::AfterInputs(count) => format!("{count} measured inputs"),
        Stop::AfterSeconds(seconds) => format!("{seconds} s"),
    }
}

fn phase_table(records: &[InputRecord]) -> Result<Vec<String>, HarnessError> {
    let phones = |record: &InputRecord| {
        record
            .projection_per_audience_ns
            .iter()
            .take(SEAT_COUNT)
            .copied()
            .collect::<Vec<u64>>()
    };
    let tv = |record: &InputRecord| {
        record
            .projection_per_audience_ns
            .last()
            .copied()
            .unwrap_or(0)
    };
    Ok(vec![
        format!(
            "{:<22}{:>9}{:>10}{:>10}{:>10}{:>10}{:>10}",
            "phase (ms)", "count", "mean", "p50", "p95", "p99", "max"
        ),
        row(
            "queue_wait",
            summary_of(records, InputRecord::queue_wait_ns)?,
        ),
        row("serve_next", summary_of(records, |r| r.serve_ns)?),
        row("step", summary_of(records, |r| r.step_ns)?),
        row("projection_phone (1)", summary_of_many(records, phones)?),
        row("projection_tv (1)", summary_of(records, tv)?),
        row(
            "projection_total (7)",
            summary_of(records, |r| r.projection_ns)?,
        ),
        row("commit_serialise", summary_of(records, |r| r.serialise_ns)?),
        row("commit_hash", summary_of(records, |r| r.hash_ns)?),
        row("commit_store", summary_of(records, |r| r.store_ns)?),
        row("commit_total", summary_of(records, |r| r.commit_ns)?),
        row("finish", summary_of(records, |r| r.finish_ns)?),
        row("reduce", summary_of(records, InputRecord::reduce_ns)?),
        row(
            "end_to_end",
            summary_of(records, InputRecord::end_to_end_ns)?,
        ),
    ])
}

fn verdict(name: &str, p95_ns: u64, budget_ns: u64) -> String {
    let outcome = if p95_ns <= budget_ns {
        format!("within, {} ms to spare", milliseconds(budget_ns - p95_ns))
    } else {
        format!("OVER by {} ms", milliseconds(p95_ns - budget_ns))
    };
    format!(
        "{name}: p95 {} ms against {} ms: {outcome}",
        milliseconds(p95_ns),
        milliseconds(budget_ns)
    )
}

fn verdict_lines(records: &[InputRecord], arrival: Arrival) -> Result<Vec<String>, HarnessError> {
    let phones = summary_of_many(records, |r| {
        r.projection_per_audience_ns
            .iter()
            .take(SEAT_COUNT)
            .copied()
            .collect()
    })?;
    let total = summary_of(records, |r| r.projection_ns)?;
    let (path, path_p95_ns) = match arrival {
        Arrival::Saturated { .. } => (
            "H.5 rules-only path (reduce alone: a saturated run keeps the queue full by design, so \
             its wait says nothing; rules cost zero)",
            summary_of(records, InputRecord::reduce_ns)?.p95_ns,
        ),
        Arrival::Steady { .. } | Arrival::Burst { .. } => (
            "H.5 rules-only path (queue wait + reduce, rules cost zero)",
            summary_of(records, InputRecord::end_to_end_ns)?.p95_ns,
        ),
    };
    Ok(vec![
        verdict(
            "H.5 projection, one phone",
            phones.p95_ns,
            PROJECTION_BUDGET_NS,
        ),
        verdict(
            "H.5 projection, all seven views",
            total.p95_ns,
            PROJECTION_BUDGET_NS,
        ),
        verdict(path, path_p95_ns, RULES_ONLY_BUDGET_NS),
    ])
}

fn busy_lines(result: &RunResult, first_serve_ns: u64) -> Result<Vec<String>, HarnessError> {
    let intervals: Vec<BusyInterval> = result
        .records
        .iter()
        .map(|record| BusyInterval {
            start_ns: record.serve_started_ns.saturating_sub(first_serve_ns),
            end_ns: record.finished_ns.saturating_sub(first_serve_ns),
        })
        .collect();
    let run_ns = result.duration_ns.saturating_sub(first_serve_ns);
    let windows = busy_permille_per_window(&intervals, WINDOW_NS, run_ns)?;
    if windows.is_empty() {
        return Ok(vec![
            "owner busy per 1 s window: run shorter than one window".to_owned(),
        ]);
    }
    let mut permille: Vec<u64> = windows.iter().map(|&value| u64::from(value)).collect();
    let summary = summarise(&mut permille)?;
    Ok(vec![format!(
        "owner busy per 1 s window ({} windows): mean {:.1} %, p95 {:.1} %, max {:.1} %",
        summary.count,
        summary.mean_ns as f64 / 10.0,
        summary.p95_ns as f64 / 10.0,
        summary.max_ns as f64 / 10.0
    )])
}

fn minute_lines(records: &[InputRecord], first_serve_ns: u64) -> Result<Vec<String>, HarnessError> {
    let last_ns = records.last().map_or(0, |record| record.finished_ns);
    let minutes = last_ns.saturating_sub(first_serve_ns) / MINUTE_NS + 1;
    let mut lines = vec![format!(
        "{:<8}{:>8}{:>14}{:>14}{:>16}{:>16}",
        "minute", "inputs", "reduce p50", "reduce p95", "projection p50", "projection p95"
    )];
    for minute in 0..minutes {
        let from = first_serve_ns + minute * MINUTE_NS;
        let inside: Vec<InputRecord> = records
            .iter()
            .filter(|record| record.finished_ns >= from && record.finished_ns < from + MINUTE_NS)
            .copied()
            .collect();
        if inside.len() < 2 {
            continue;
        }
        let reduce = summary_of(&inside, InputRecord::reduce_ns)?;
        let projection = summary_of(&inside, |r| r.projection_ns)?;
        lines.push(format!(
            "{:<8}{:>8}{:>14}{:>14}{:>16}{:>16}",
            minute + 1,
            inside.len(),
            milliseconds(reduce.p50_ns),
            milliseconds(reduce.p95_ns),
            milliseconds(projection.p50_ns),
            milliseconds(projection.p95_ns),
        ));
    }
    Ok(lines)
}

/// The report of one run as text.
pub fn render(result: &RunResult) -> Result<String, HarnessError> {
    let config = &result.config;
    let first_serve_ns = result
        .records
        .first()
        .map_or(0, |record| record.serve_started_ns);
    let mut lines = vec![
        format!(
            "scenario: {} | profile {} | commit {} | snapshot {} | stop after {}",
            arrival_text(config.arrival),
            config.profile.name,
            config.commit.key(),
            config.snapshot.key(),
            stop_text(config.stop),
        ),
        format!(
            "measured {} inputs after a warm-up of {}; refused {}; run {:.1} s",
            result.records.len(),
            result.warmup_served,
            result.refused,
            result.duration_ns as f64 / 1e9,
        ),
        format!(
            "bytes: phone view {} (last), tv view {} (last), checkpoint {}",
            result.phone_view_bytes, result.tv_view_bytes, result.checkpoint_bytes
        ),
    ];
    lines.extend(phase_table(&result.records)?);
    lines.extend(verdict_lines(&result.records, config.arrival)?);
    lines.extend(busy_lines(result, first_serve_ns)?);
    let span_ns = result.duration_ns.saturating_sub(first_serve_ns);
    if span_ns >= 2 * MINUTE_NS {
        lines.extend(minute_lines(&result.records, first_serve_ns)?);
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}
