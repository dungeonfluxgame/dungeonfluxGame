//! The text the dev command line prints for each answer of the scene builder's workbench.
//!
//! Every list is numbered from 1 (`1: ...`), like the answers of the engine verbs, so an answer can
//! be read aloud and a line can be named. It formats and never decides: it reads the typed replies
//! of `request` and prints their fields. Times are printed as seconds and tenths, from whole
//! milliseconds. Player text never reaches it, because no reply holds any.

use df_table::plate::buffer::BufferReport;
use df_table::plate::cost::{CostBasis, CostView, StagePrice};
use df_table::plate::scheduler::SchedulerReport;

use super::report::{AnchorReport, CardReport, JobReport, LocateReport, SceneReport};
use super::request::{
    BufferShow, BuildOutcome, BuildReply, PlateTimingReport, PlatesReport, RunReply, SceneShow,
    TimedLine, TimingReply,
};
use crate::broker::report::QueuesReport;

const MICRO_DOLLARS_PER_DOLLAR: u64 = 1_000_000;
const MICRO_DOLLARS_PER_CENT: u64 = 10_000;

pub(super) fn numbered(lines: impl IntoIterator<Item = String>) -> String {
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| format!("{}: {line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `1.5 s` from 1,500 milliseconds.
pub(super) fn seconds(milliseconds: u64) -> String {
    format!("{}.{} s", milliseconds / 1_000, milliseconds % 1_000 / 100)
}

fn dollars(micro_dollars: u64) -> String {
    format!(
        "${}.{:02}",
        micro_dollars / MICRO_DOLLARS_PER_DOLLAR,
        micro_dollars % MICRO_DOLLARS_PER_DOLLAR / MICRO_DOLLARS_PER_CENT
    )
}

pub(super) fn timed(line: &TimedLine) -> String {
    format!("[{}] {}", seconds(line.at_ms), line.line)
}

fn outcome_text(outcome: &BuildOutcome) -> String {
    match outcome {
        BuildOutcome::PlateOnScreen { at_ms } => {
            format!("the first plate was on screen after {}", seconds(*at_ms))
        }
        BuildOutcome::Ended { end, stage, cause } => match (stage, cause) {
            (Some(stage), Some(cause)) => format!(
                "the job ended {}: the {} stage failed ({})",
                end.key(),
                stage.key(),
                cause.key()
            ),
            _ => format!("the job ended {}", end.key()),
        },
        BuildOutcome::NoPlate => "no plate came".to_owned(),
    }
}

pub fn format_build(reply: &BuildReply) -> String {
    let card = reply.card_due_after_ms.map_or_else(String::new, |after| {
        format!(
            "\nplay went on to the establishing card after {}",
            seconds(after)
        )
    });
    let ahead = reply.lookahead.map_or_else(String::new, |take| {
        format!("\nthe look-ahead found it: {}", take.key())
    });
    format!(
        "scene {} (generation {}), {}: {}, {} in all{card}{ahead}\n{}",
        reply.scene,
        reply.generation,
        reply.place.as_str(),
        outcome_text(&reply.outcome),
        seconds(reply.elapsed_ms),
        numbered(reply.lines.iter().map(timed))
    )
}

fn anchor_line(kind: &str, anchor: &AnchorReport) -> String {
    let at = format!("{:.1},{:.1}", anchor.at.x, anchor.at.y);
    let how = match (&anchor.reason, anchor.found_by_phrase) {
        (Some(reason), _) => format!(" ({reason})"),
        (None, Some(phrase)) => format!(" (phrase {phrase})"),
        (None, None) => String::new(),
    };
    let tried = anchor
        .phrases_tried
        .map_or_else(String::new, |tried| format!(", {tried} tried"));
    format!(
        "{kind} {}: {} at {at}{how}{tried}",
        anchor.id, anchor.anchor
    )
}

fn locate_lines(locate: &LocateReport) -> Vec<String> {
    locate
        .figures
        .iter()
        .map(|anchor| anchor_line("person", anchor))
        .chain(
            locate
                .items
                .iter()
                .map(|anchor| anchor_line("item", anchor)),
        )
        .collect()
}

fn card_line(card: &CardReport) -> String {
    let late = card
        .late_by_ms
        .map_or_else(String::new, |late| format!(", late by {}", seconds(late)));
    let party = if card.party.is_empty() {
        "nobody".to_owned()
    } else {
        card.party.join(", ")
    };
    format!(
        "card {} (due at {}{late}): narrator line {}, party {party}, music {}, background {}, tag {}",
        card.state,
        seconds(card.due_at_ms),
        if card.has_narrator_line { "yes" } else { "no" },
        card.place_music.as_deref().unwrap_or("none"),
        if card.background_stored { "yes" } else { "no" },
        card.tag
    )
}

fn job_line(job: &JobReport) -> String {
    let stored = if job.stored.is_empty() {
        "none".to_owned()
    } else {
        let keys: Vec<&str> = job.stored.iter().map(|stage| stage.key()).collect();
        keys.join(", ")
    };
    let running = job
        .running
        .map_or_else(String::new, |stage| format!(", running {}", stage.key()));
    let failed = job.failure.as_ref().map_or_else(String::new, |failure| {
        format!(
            ", failed at {} ({})",
            failure.stage.key(),
            failure.cause.key()
        )
    });
    format!(
        "job {} ({}, tick at {}): {} figures, stored {stored}{running}{failed}",
        job.job,
        job.reason.key(),
        seconds(job.tick_ms),
        job.figures
    )
}

fn scene_lines(scene: &SceneReport) -> Vec<String> {
    let mut lines = vec![
        format!(
            "scene {} {} (generation {}, {}, {}): started {}, description {}, background {}, first plate {}",
            scene.scene,
            scene.place,
            scene.generation,
            scene.kind,
            scene.priority,
            seconds(scene.started_ms),
            if scene.description_stored {
                "stored"
            } else {
                "not stored"
            },
            if scene.background_stored {
                "stored"
            } else {
                "not stored"
            },
            if scene.first_plate_ready {
                "ready"
            } else {
                "not ready"
            },
        ),
        card_line(&scene.card),
    ];
    lines.extend(scene.jobs.iter().map(job_line));
    lines
}

pub fn format_show(show: &SceneShow) -> String {
    if show.scenes.is_empty() {
        return "no scene is open: try scene build <place>".to_owned();
    }
    let mut lines: Vec<String> = show.scenes.iter().flat_map(scene_lines).collect();
    match &show.on_screen {
        Some(locate) => {
            lines.push("the plate on screen was located as follows".to_owned());
            lines.extend(
                locate_lines(locate)
                    .into_iter()
                    .map(|line| format!("  {line}")),
            );
        }
        None => lines.push("no plate is on screen".to_owned()),
    }
    numbered(lines)
}

fn cost_lines(cost: &CostView) -> Vec<String> {
    let mut lines = vec![format!(
        "interval {} s: at most {} plates an hour",
        cost.interval_s, cost.plates_per_hour_at_most
    )];
    for stage in &cost.stages {
        let price = match stage.price {
            StagePrice::Priced { micro_dollars } => {
                format!("{} a call", dollars(micro_dollars))
            }
            StagePrice::Unpriced => "unpriced".to_owned(),
        };
        let money = match (
            stage.micro_dollars_least_per_hour,
            stage.micro_dollars_most_per_hour,
        ) {
            (Some(least), Some(most)) if least == most => format!(", {} an hour", dollars(least)),
            (Some(least), Some(most)) => {
                format!(", {} to {} an hour", dollars(least), dollars(most))
            }
            _ => String::new(),
        };
        let calls = if stage.calls_least_per_hour == stage.calls_most_per_hour {
            format!("{} calls", stage.calls_least_per_hour)
        } else {
            format!(
                "{} to {} calls",
                stage.calls_least_per_hour, stage.calls_most_per_hour
            )
        };
        lines.push(format!(
            "stage {} ({price}): {calls} an hour{money}",
            stage.purpose
        ));
    }
    lines.push(match cost.basis {
        CostBasis::AllStagesPriced => format!(
            "cost an hour: {} to {}, an upper bound (a quiet table pays for no plate)",
            dollars(cost.priced_micro_dollars_least_per_hour),
            dollars(cost.priced_micro_dollars_most_per_hour)
        ),
        CostBasis::SomeStagesUnpriced => format!(
            "estimate: the priced stages come to {} to {} an hour, and a stage with no price is left out; \
             its calls an hour are above",
            dollars(cost.priced_micro_dollars_least_per_hour),
            dollars(cost.priced_micro_dollars_most_per_hour)
        ),
    });
    lines
}

fn scheduler_lines(scheduler: &SchedulerReport) -> Vec<String> {
    let stats = scheduler.stats;
    let mut lines = vec![format!(
        "clock: next tick {}, {} queued; {} ticks, {} queued in all",
        scheduler
            .next_tick_ms
            .map_or_else(|| "not started".to_owned(), seconds),
        scheduler.queued.len(),
        stats.ticks,
        stats.queued
    )];
    lines.extend(scheduler.queued.iter().map(|job| {
        format!(
            "queued job {} ({}, tick at {})",
            job.job,
            job.reason,
            seconds(job.tick_ms)
        )
    }));
    lines.push(format!(
        "skipped: {} nothing changed, {} table not active, {} no viewer, {} scene closed; \
         {} dropped as stale, {} discarded for a new generation, {} beats missed",
        stats.skipped_nothing_changed,
        stats.skipped_table_not_active,
        stats.skipped_no_viewer,
        stats.skipped_scene_closed,
        stats.dropped_stale,
        stats.discarded_for_generation,
        stats.missed_beats
    ));
    lines
}

pub fn format_plates(report: &PlatesReport) -> String {
    let mut lines = Vec::new();
    match &report.scheduler {
        Some(scheduler) => lines.extend(scheduler_lines(scheduler)),
        None => lines.push("no scene is open: try scene build <place>".to_owned()),
    }
    lines.extend(report.in_flight.iter().map(job_line));
    lines.push(format!(
        "{} plates finished{}",
        report.plates_finished,
        report
            .mean_build_ms
            .map_or_else(String::new, |mean| format!(
                ", {} each on average",
                seconds(mean)
            ))
    ));
    if let Some(cost) = &report.cost {
        lines.extend(cost_lines(cost));
    }
    lines.push(timing_line(&report.timing));
    lines.push(broker_line(&report.broker));
    numbered(lines)
}

fn timing_line(timing: &PlateTimingReport) -> String {
    let on_screen = match timing.on_screen_interval_s {
        Some(on_screen) if on_screen != timing.interval_s => {
            format!(" (the scene on screen swaps every {on_screen} s until the next scene)")
        }
        _ => String::new(),
    };
    format!(
        concat!(
            "timing: interval {} s{}, a plate up at least {}, dissolve {},",
            " catching up after {}"
        ),
        timing.interval_s,
        on_screen,
        seconds(timing.minimum_on_screen_ms),
        timing
            .dissolve_ms
            .map_or_else(|| "not seen yet".to_owned(), seconds),
        seconds(timing.catch_up_ms),
    )
}

fn broker_line(broker: &QueuesReport) -> String {
    let classes = broker
        .classes
        .iter()
        .map(|class| {
            format!(
                "{} {} waiting ({} dropped)",
                class.class.key(),
                class.depth,
                class.dropped
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "workbench broker: epoch {}{}; {classes}; {} running",
        broker.epoch,
        if broker.closed { " (closed)" } else { "" },
        broker.running.len()
    )
}

/// `scene timing set`: the change and the timings now.
pub fn format_timing(reply: &TimingReply) -> String {
    let mut lines = vec![format!(
        "{} was {}; {}",
        reply.setting.key(),
        reply.previous,
        timing_line(&reply.timing)
    )];
    lines.extend(reply.lines.iter().map(timed));
    numbered(lines)
}

fn buffer_lines(buffer: &BufferReport) -> Vec<String> {
    let mut lines = vec![format!(
        "buffer of scene {} (generation {})",
        buffer.scene, buffer.generation
    )];
    lines.push(match &buffer.current {
        Some(shown) => format!(
            "on screen: plate of job {} (tick at {}, shown at {}, up {}); people found {} and lost {}; items found {}, own picture {}, lost {}",
            shown.job,
            seconds(shown.tick_ms),
            seconds(shown.shown_ms),
            seconds(shown.on_screen_ms),
            shown.located_figures,
            shown.unlocated_figures,
            shown.located_items,
            shown.own_picture_items,
            shown.unlocated_items
        ),
        None => "on screen: no plate (the establishing card holds the screen)".to_owned(),
    });
    lines.push(format!(
        "ready: {}; next slot {}",
        if buffer.ready.is_empty() {
            "none".to_owned()
        } else {
            buffer
                .ready
                .iter()
                .map(|job| format!("job {job}"))
                .collect::<Vec<_>>()
                .join(", ")
        },
        buffer
            .next_slot_ms
            .map_or_else(|| "none".to_owned(), seconds)
    ));
    lines.extend(buffer.pending.iter().map(|pending| {
        format!(
            "pending: job {} (tick at {}, {} old)",
            pending.job,
            seconds(pending.tick_ms),
            seconds(pending.age_ms)
        )
    }));
    lines.push(format!(
        "lateness: slot {} late, oldest outstanding job {} old; picture catching up {}",
        seconds(buffer.lateness.slot_late_ms),
        buffer
            .lateness
            .oldest_outstanding_age_ms
            .map_or_else(|| "none".to_owned(), seconds),
        if buffer.catching_up { "yes" } else { "no" }
    ));
    let stats = buffer.stats;
    lines.push(format!(
        "{} plates accepted, {} swaps ({} overdue); refused: {} of another scene, {} of an earlier generation, {} older than the one shown",
        stats.accepted,
        stats.swaps,
        stats.swaps_overdue,
        stats.rejected_wrong_scene,
        stats.rejected_stale_generation,
        stats.rejected_older_than_shown
    ));
    lines
}

pub fn format_buffer(show: &BufferShow) -> String {
    let mut lines = match &show.buffer {
        Some(buffer) => buffer_lines(buffer),
        None => vec!["no scene is open: try scene build <place>".to_owned()],
    };
    if let Some(card) = &show.card {
        lines.push(card_line(card));
    }
    numbered(lines)
}

pub fn format_run(reply: &RunReply) -> String {
    let head = format!(
        "ran {} s{}, screen moment {}",
        reply.seconds,
        if reply.quiet {
            " with nothing changing"
        } else {
            ""
        },
        reply.moment.key()
    );
    let lines = numbered(reply.lines.iter().map(timed));
    let buffer = numbered(buffer_lines(&reply.buffer));
    if lines.is_empty() {
        format!("{head}\nno events\n{buffer}")
    } else {
        format!("{head}\n{lines}\n{buffer}")
    }
}
