//! The text of the open pages and of one page's timeline.

use df_playground::media::AudioState;
use df_playground::timeline::{ClientTimeline, TimelineDetail, TimelineEntry};

use crate::wire::ClientView;

fn audio_text(audio: AudioState) -> &'static str {
    match audio {
        AudioState::Unlocked => "audio unlocked",
        AudioState::Locked => {
            "audio LOCKED (the browser plays sound only after someone clicks the page)"
        }
    }
}

fn optional(text: Option<&str>) -> &str {
    text.filter(|text| !text.is_empty()).unwrap_or("-")
}

/// The lines of one page: what it shows and plays, where its frame stands against the host's, its
/// link and what it loaded, then its errors.
fn client_lines(client: &ClientView, host_frame: Option<u64>) -> Vec<String> {
    let report = &client.report;
    let music = report.music.as_deref().unwrap_or("none");
    let shown = if report.images_shown.is_empty() {
        "none".to_owned()
    } else {
        report.images_shown.join(", ")
    };
    let frame = report
        .frame
        .map_or_else(|| "-".to_owned(), |frame| frame.to_string());
    let host = host_frame.map_or_else(|| "?".to_owned(), |frame| frame.to_string());
    let fixture_error = report
        .fixture_error
        .map_or_else(String::new, |cause| format!(" ({})", cause.key()));
    let viewport = report.viewport_px.map_or_else(
        || "-".to_owned(),
        |size| format!("{}x{}", size.width, size.height),
    );
    let mut lines = vec![
        format!(
            "client {} ({:.1} s ago): {}; last sound cue {}; music {music}; pictures shown {shown}",
            client.client,
            client.age_ms as f64 / 1000.0,
            audio_text(report.audio),
            report.last_cue
        ),
        format!(
            "  frame {frame}/{host} link={} failures={} rev={} fixture={}{fixture_error} \
             device={} pace={} viewport={viewport} boot={}",
            report.link.key(),
            report.failures,
            optional(Some(report.asset_revision.as_str())),
            optional(report.fixture.as_deref()),
            report.device.key(),
            report.pace.key(),
            optional(report.boot.as_deref())
        ),
    ];
    lines.extend(report.errors.iter().map(|error| {
        format!(
            "  error: {} ({} #{} at {} ms)",
            error.message,
            error.source.key(),
            error.seq,
            error.page_ms
        )
    }));
    lines
}

/// What each open page says it is showing and playing. An agent cannot see or hear a screen, so
/// this is how it checks that a picture or a sound arrived. `host_frame` is the host's frame count,
/// when it could be read.
pub fn format_clients(clients: &[ClientView], host_frame: Option<u64>) -> String {
    if clients.is_empty() {
        return "clients: none (open the playground page)".to_owned();
    }
    clients
        .iter()
        .flat_map(|client| client_lines(client, host_frame))
        .collect::<Vec<_>>()
        .join("\n")
}

fn span_text(entry: &TimelineEntry) -> String {
    match (entry.end_ms, entry.shown_ms) {
        (Some(end), Some(shown)) => format!("{}..{} shown={shown}", entry.start_ms, end),
        _ => format!("{}.. open", entry.start_ms),
    }
}

fn detail_text(detail: &TimelineDetail) -> String {
    match detail {
        TimelineDetail::Draw {
            frame,
            devices,
            parts,
            transition,
            moved,
        } => {
            let frame = frame.map_or_else(|| "-".to_owned(), |frame| frame.to_string());
            let transition = transition.map_or("none", |style| style.key());
            format!(
                "frame={frame} devices={devices} parts={} transition={transition} moved={moved}",
                if parts.is_empty() {
                    "-".to_owned()
                } else {
                    parts.join(",")
                }
            )
        }
        TimelineDetail::Camera {
            zoom_percent,
            allowed_percent,
        } => {
            let allowed = allowed_percent.map_or_else(|| "-".to_owned(), |zoom| zoom.to_string());
            format!("zoom={zoom_percent} allowed={allowed}")
        }
        TimelineDetail::Bubble { side, moved } => {
            format!("side={} moved={moved}", side.key())
        }
        TimelineDetail::Scene { from, transition } => format!(
            "from={} transition={}",
            optional(from.as_deref()),
            transition.map_or("none", |style| style.key())
        ),
    }
}

fn entry_line(entry: &TimelineEntry) -> String {
    let planned = entry
        .planned_ms
        .map_or_else(|| "-".to_owned(), |planned| planned.to_string());
    let reason = entry.end_reason.map_or("-", |reason| reason.key());
    let short = if entry.short { " short=true" } else { "" };
    let mut line = format!(
        "{:>5}  {:<6} {}  {} planned={planned} reason={reason}{short}",
        entry.seq,
        entry.kind.key(),
        entry.id,
        span_text(entry)
    );
    if let Some(detail) = &entry.detail {
        line.push_str("  ");
        line.push_str(&detail_text(detail));
    }
    if let Some(message) = &entry.message {
        line.push_str(&format!("  message: {message}"));
    }
    line
}

/// One page's timeline, oldest first, on the host's playground clock (milliseconds).
pub fn format_timeline(timeline: &ClientTimeline) -> String {
    if timeline.entries.is_empty() {
        return format!("client {}: no timeline entries", timeline.client);
    }
    let mut lines = vec![format!(
        "client {}: {} entries (start..end in host ms)",
        timeline.client,
        timeline.entries.len()
    )];
    lines.extend(timeline.entries.iter().map(entry_line));
    lines.join("\n")
}
