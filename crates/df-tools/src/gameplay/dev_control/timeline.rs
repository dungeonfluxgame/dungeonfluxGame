//! Development-only bounded mirror of the shared audit path, never authoritative game state.
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const CAPACITY: usize = 4096;
pub(super) const RESOURCE_TYPES: &[&str] = &[
    "text_model",
    "speech_input",
    "voice",
    "image",
    "plate",
    "video",
    "browser_media",
    "state",
];
static TIMELINE: OnceLock<Mutex<Timeline>> = OnceLock::new();

#[derive(Clone, Serialize)]
struct Event {
    sequence: u64,
    unix_ms: i128,
    elapsed_ms: u128,
    event: String,
    line: String,
    fields: Map<String, Value>,
}

struct Timeline {
    start: Instant,
    next: u64,
    events: VecDeque<Event>,
}

impl Timeline {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            next: 1,
            events: VecDeque::new(),
        }
    }
    fn push(&mut self, event: &str, line: &str, mut fields: Map<String, Value>) {
        if let Some(resource_type) = resource_type(event, line, &fields) {
            fields
                .entry("resource_type")
                .or_insert_with(|| Value::String(resource_type.to_owned()));
        }
        if let Some(stage) = lifecycle_stage(event, line, &fields) {
            fields
                .entry("stage")
                .or_insert_with(|| Value::String(stage.to_owned()));
        }
        if let Some(status) = lifecycle_status(event, line, &fields) {
            fields
                .entry("status")
                .or_insert_with(|| Value::String(status.to_owned()));
        }
        let unix_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(time) => time.as_millis() as i128,
            Err(time) => -(time.duration().as_millis() as i128),
        };
        self.events.push_back(Event {
            sequence: self.next,
            unix_ms,
            elapsed_ms: self.start.elapsed().as_millis(),
            event: event.into(),
            line: line.chars().take(2048).collect(),
            fields,
        });
        self.next += 1;
        if self.events.len() > CAPACITY {
            self.events.pop_front();
        }
    }
    fn snapshot(
        &self,
        last: usize,
        resource_filter: Option<&str>,
        summary_only: bool,
    ) -> serde_json::Value {
        let matched = self
            .events
            .iter()
            .filter(|event| {
                resource_filter.is_none_or(|filter| {
                    event.fields.get("resource_type").and_then(Value::as_str) == Some(filter)
                })
            })
            .collect::<Vec<_>>();
        let window = &matched[matched.len().saturating_sub(last)..];
        let events = if summary_only {
            Vec::new()
        } else {
            window.iter().copied().collect::<Vec<_>>()
        };
        let mut by_type = std::collections::BTreeMap::<String, usize>::new();
        let mut by_stage = std::collections::BTreeMap::<String, usize>::new();
        let mut by_status = std::collections::BTreeMap::<String, usize>::new();
        let mut identities = std::collections::BTreeSet::<(String, String)>::new();
        for event in window {
            if let Some(value) = event.fields.get("resource_type").and_then(Value::as_str) {
                *by_type.entry(value.to_owned()).or_default() += 1;
                let id = event
                    .fields
                    .get("resource_id")
                    .or_else(|| event.fields.get("image_key"))
                    .or_else(|| event.fields.get("plate_id"))
                    .or_else(|| event.fields.get("asset_id"))
                    .and_then(|value| match value {
                        Value::String(id) => Some(id.clone()),
                        Value::Number(id) => Some(id.to_string()),
                        _ => None,
                    });
                if let Some(id) = id {
                    identities.insert((value.to_owned(), id));
                }
            }
            if let Some(value) = event.fields.get("stage").and_then(Value::as_str) {
                *by_stage.entry(value.to_owned()).or_default() += 1;
            }
            if let Some(value) = event.fields.get("status").and_then(Value::as_str) {
                *by_status.entry(value.to_owned()).or_default() += 1;
            }
        }
        serde_json::json!({
            "events": events,
            "matched": matched.len(),
            "summary": {
                "window_events": window.len(),
                "resource_count": identities.len(),
                "by_resource_type": by_type,
                "by_stage": by_stage,
                "by_status": by_status,
            },
            "filter": {"resource_type": resource_filter, "summary_only": summary_only},
            "dropped": self.next - 1 - self.events.len() as u64,
            "capacity": CAPACITY,
            "scope":"current server process; browser events require a connected display"
        })
    }
}

fn resource_type(event: &str, line: &str, fields: &Map<String, Value>) -> Option<&'static str> {
    if let Some(value) = fields.get("resource_type").and_then(Value::as_str) {
        return RESOURCE_TYPES.iter().copied().find(|kind| *kind == value);
    }
    if event.starts_with("gameplay.storyteller.") {
        Some("text_model")
    } else if event.starts_with("gameplay.speech_input")
        || event.starts_with("gameplay.speech_cancel")
    {
        Some("speech_input")
    } else if event.starts_with("gameplay.voice") {
        Some("voice")
    } else if event.starts_with("gameplay.imagery.") {
        Some("image")
    } else if event.starts_with("gameplay.living_scene.plate_")
        || event.starts_with("gameplay.plate.")
    {
        Some("plate")
    } else if event.starts_with("gameplay.scene_video") || event.starts_with("gameplay.home_video")
    {
        Some("video")
    } else if event.starts_with("gameplay.session_machine_") {
        Some("state")
    } else if event == "gameplay.presentation" {
        if line.contains(" visual-source ")
            || line.contains(" scene ")
            || line.contains(" figure")
            || line.contains(" check-")
            || line.contains(" paint")
            || line.contains(" usable")
        {
            Some("browser_media")
        } else if line.contains(" voice-") {
            Some("voice")
        } else if line.contains(" plate-") {
            Some("plate")
        } else if line.contains(" clip-") {
            Some("video")
        } else {
            None
        }
    } else {
        None
    }
}

fn lifecycle_stage(event: &str, line: &str, fields: &Map<String, Value>) -> Option<&'static str> {
    if let Some(value) = fields.get("stage").and_then(Value::as_str) {
        return Some(match value {
            "request" | "requested" => "requested",
            "queued" => "queued",
            "ready" => "ready",
            "failed" | "failure" => "failed",
            "played" => "played",
            "projected" => "projected",
            "delivered" | "delivery" => "delivered",
            _ => return None,
        });
    }
    if event.contains("requested") || event.ends_with(".request") || line.contains(" request ") {
        return Some("requested");
    }
    if event.contains("failed") || event.contains("refused") || line.contains(" failed ") {
        return Some("failed");
    }
    if event.contains("ready") || line.contains(" ready ") {
        return Some("ready");
    }
    if event.contains("projected") || line.contains(" projected ") {
        return Some("projected");
    }
    if event.contains("shown") || event.contains("played") || line.contains(" shown ") {
        return Some("played");
    }
    if event.contains("voice-start") || event.contains("clip-ended") || line.contains(" started ") {
        return Some("played");
    }
    if event.contains("queued") || line.contains(" queued ") {
        return Some("queued");
    }
    None
}

fn lifecycle_status(event: &str, line: &str, fields: &Map<String, Value>) -> Option<&'static str> {
    if let Some(value) = fields.get("status").and_then(Value::as_str) {
        return match value {
            "requested" => Some("requested"),
            "queued" => Some("queued"),
            "ready" => Some("ready"),
            "failed" => Some("failed"),
            "played" => Some("played"),
            "projected" => Some("projected"),
            "delivered" => Some("delivered"),
            "cached" => Some("cached"),
            "fallback" => Some("fallback"),
            "partial" => Some("partial"),
            "cancelled" => Some("cancelled"),
            _ => None,
        };
    }
    if event.contains("failed") || event.contains("refused") || line.contains("failed") {
        Some("failed")
    } else if event.contains("ready") || line.contains("ready") {
        Some("ready")
    } else if event.contains("requested") || event.ends_with(".request") {
        Some("requested")
    } else if event.contains("projected") {
        Some("projected")
    } else if event.contains("shown") || event.contains("played") {
        Some("played")
    } else {
        None
    }
}

pub(in crate::gameplay) fn record(event: &str, line: &str) {
    if std::env::var("DF_GAMEPLAY_DEV_CONTROL").as_deref() != Ok("on") {
        return;
    }
    TIMELINE
        .get_or_init(|| Mutex::new(Timeline::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(event, line, Map::new());
}

pub(in crate::gameplay) fn record_structured(event: &str, line: &str, fields: Map<String, Value>) {
    if std::env::var("DF_GAMEPLAY_DEV_CONTROL").as_deref() != Ok("on") {
        return;
    }
    TIMELINE
        .get_or_init(|| Mutex::new(Timeline::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(event, line, fields);
}

pub(super) fn snapshot(
    last: usize,
    resource_filter: Option<&str>,
    summary_only: bool,
) -> serde_json::Value {
    TIMELINE
        .get_or_init(|| Mutex::new(Timeline::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot(last, resource_filter, summary_only)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeline_keeps_order_times_and_reports_eviction() {
        let mut timeline = Timeline::new();
        for _ in 0..CAPACITY + 2 {
            timeline.push("line.open", "synthetic public line", Map::new());
        }
        let snapshot = timeline.snapshot(3, None, false);
        let events = snapshot["events"].as_array().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(snapshot["dropped"], 2);
        assert_eq!(events[2]["sequence"], CAPACITY + 2);
        assert!(events[0]["unix_ms"].as_i64().unwrap() > 0);
        assert!(
            events
                .windows(2)
                .all(|pair| pair[0]["elapsed_ms"].as_u64() <= pair[1]["elapsed_ms"].as_u64())
        );
    }

    #[test]
    fn structured_fields_remain_separate_from_the_display_line() {
        let mut timeline = Timeline::new();
        let mut fields = Map::new();
        fields.insert("prompt_char_count".to_owned(), Value::from(3978));
        fields.insert("reference_count".to_owned(), Value::from(3));
        fields.insert(
            "reference_byte_lengths".to_owned(),
            serde_json::json!([1024, 2048, 4096]),
        );
        timeline.push(
            "gameplay.imagery.request",
            "IMAGERY request kind=scene_plate prompt_chars=3978 references=3",
            fields,
        );

        let event = &timeline.snapshot(1, None, false)["events"][0];
        assert_eq!(event["fields"]["prompt_char_count"], 3978);
        assert_eq!(
            event["fields"]["reference_byte_lengths"],
            serde_json::json!([1024, 2048, 4096])
        );
        assert_eq!(
            event["line"],
            "IMAGERY request kind=scene_plate prompt_chars=3978 references=3"
        );
        assert_eq!(event["fields"]["resource_type"], "image");
        assert_eq!(event["fields"]["stage"], "requested");
    }

    #[test]
    fn resource_filter_keeps_correlated_failure_and_ready_and_summarizes_them() {
        let mut timeline = Timeline::new();
        for (event, status, failure) in [
            ("gameplay.imagery.render_failed", "failed", Some("provider")),
            ("gameplay.imagery.render_ready", "ready", None),
        ] {
            let fields = Map::from_iter([
                (
                    "resource_id".to_owned(),
                    Value::String("image-key-42".to_owned()),
                ),
                ("revision".to_owned(), Value::from(7)),
                ("status".to_owned(), Value::String(status.to_owned())),
                ("failure_category".to_owned(), serde_json::json!(failure)),
            ]);
            timeline.push(event, "content-free image lifecycle", fields);
        }
        timeline.push(
            "gameplay.voice_clip",
            "content-free voice lifecycle",
            Map::from_iter([(
                "resource_type".to_owned(),
                Value::String("voice".to_owned()),
            )]),
        );

        let filtered = timeline.snapshot(20, Some("image"), false);
        let events = filtered["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["fields"]["resource_id"], "image-key-42");
        assert_eq!(events[1]["fields"]["resource_id"], "image-key-42");
        assert_eq!(filtered["summary"]["resource_count"], 1);
        assert_eq!(filtered["summary"]["by_status"]["failed"], 1);
        assert_eq!(filtered["summary"]["by_status"]["ready"], 1);

        let summary = timeline.snapshot(20, Some("image"), true);
        assert_eq!(summary["events"].as_array().unwrap().len(), 0);
        assert_eq!(summary["matched"], 2);
    }
}
