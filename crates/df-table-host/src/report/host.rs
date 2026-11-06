//! The text of the host's own reads: `host status`, `frame` and `media head`.

use crate::client::HeadAnswer;
use crate::observe::{OutletStatus, SinkHealth};
use crate::server::request::HostStatus;

fn sink_lines(label: &str, health: &SinkHealth) -> Vec<String> {
    let mut lines = vec![format!(
        "{label}: level={} seen={} always={} verbose={} filtered_verbose={} unrenderable={} \
         retained={} evicted={} lag_ms={} lost_always={} lost_verbose={} degraded={}",
        health.level,
        health.seen,
        health.always,
        health.verbose,
        health.filtered_verbose,
        health.unrenderable,
        health.retained,
        health.evicted,
        health.lag_ms,
        health.lost_always,
        health.lost_verbose,
        health.degraded
    )];
    lines.extend(health.outlets.iter().map(outlet_line));
    lines.push(match &health.shared {
        Some(shared) => format!(
            "  shared path: spooled={} exported={} dropped={} lag_ms={}",
            shared.spooled, shared.exported, shared.dropped, shared.lag_ms
        ),
        None => "  shared path: not composed".to_owned(),
    });
    lines
}

fn outlet_line(outlet: &OutletStatus) -> String {
    format!(
        "  outlet {}: accepted={} confirmed={} pending={} lag_ms={} lost_always={} \
         lost_verbose={} evicted={} unconfirmed={}",
        outlet.name,
        outlet.accepted,
        outlet.confirmed,
        outlet.pending,
        outlet.lag_ms,
        outlet.lost_always,
        outlet.lost_verbose,
        outlet.evicted,
        outlet.unconfirmed
    )
}

/// What the host's log, event sinks and clock are doing, one group per line.
pub fn format_host_status(status: &HostStatus) -> String {
    let log = &status.log;
    let clock = &status.clock;
    let mut lines = vec![
        format!(
            "log: latest={} kept={} bytes={} forgotten={} echo_pending={} echo_dropped={} \
             echo_writer_gone={} undelivered_audit_lines={}",
            log.latest,
            log.kept,
            log.bytes,
            log.forgotten,
            log.echo_pending,
            log.echo_dropped,
            log.echo_writer_gone,
            log.undelivered_audit_lines
        ),
        format!(
            "clock: ticks={} moving={} tick_ms={}",
            clock.ticks, clock.moving, clock.tick_ms
        ),
    ];
    lines.extend(sink_lines("sink", &status.sink));
    match &status.engine_sink {
        Some(engine) => lines.extend(sink_lines("engine sink", engine)),
        None => lines.push("engine sink: not running".to_owned()),
    }
    lines.join("\n")
}

/// Why a frame body could not be summarised.
#[derive(Debug)]
pub enum FrameShapeError {
    /// The body is not JSON.
    NotJson(serde_json::Error),
    /// The body is JSON but not an object with a frame count.
    NotAFrame,
}

impl std::fmt::Display for FrameShapeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson(error) => write!(formatter, "the frame is not JSON: {error}"),
            Self::NotAFrame => formatter.write_str("the frame has no frame count"),
        }
    }
}

impl std::error::Error for FrameShapeError {}

/// The frame count of a frame body: the number the page keeps and asks `after=` with.
pub fn frame_count(body: &str) -> Result<u64, FrameShapeError> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(FrameShapeError::NotJson)?;
    value
        .get("frame")
        .and_then(serde_json::Value::as_u64)
        .ok_or(FrameShapeError::NotAFrame)
}

/// A frame body in a few lines: its count, the run of the host it came from, its size and its
/// top-level keys.
pub fn format_frame(body: &str) -> Result<String, FrameShapeError> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(FrameShapeError::NotJson)?;
    let object = value.as_object().ok_or(FrameShapeError::NotAFrame)?;
    let frame = object
        .get("frame")
        .and_then(serde_json::Value::as_u64)
        .ok_or(FrameShapeError::NotAFrame)?;
    let boot = object
        .get("controls")
        .and_then(|controls| controls.get("boot_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|boot| !boot.is_empty())
        .unwrap_or("-");
    let keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    Ok(format!(
        "frame: {frame}\nboot: {boot}\nbytes: {}\nkeys: {}",
        body.len(),
        keys.join(", ")
    ))
}

/// What a frame wait said when nothing changed before it ran out.
pub fn format_frame_unchanged(after: u64) -> String {
    format!("frame: unchanged after {after} (no change before the wait ran out)")
}

/// What a HEAD of a media file answered.
pub fn format_media_head(name: &str, answer: &HeadAnswer) -> String {
    let content_type = answer.content_type.as_deref().unwrap_or("-");
    let length = answer
        .content_length
        .map_or_else(|| "-".to_owned(), |length| length.to_string());
    let entity_tag = answer.entity_tag.as_deref().unwrap_or("-");
    format!(
        "{name}: status {} type {content_type} length {length} etag {entity_tag}",
        answer.status.as_u16()
    )
}
