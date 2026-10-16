//! Development-only bounded mirror of the shared audit path, never authoritative game state.
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const CAPACITY: usize = 4096;
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
    fn push(&mut self, event: &str, line: &str, fields: Map<String, Value>) {
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
    fn snapshot(&self, last: usize) -> serde_json::Value {
        serde_json::json!({"events":self.events.iter().skip(self.events.len().saturating_sub(last)).collect::<Vec<_>>(),
            "dropped":self.next - 1 - self.events.len() as u64, "capacity":CAPACITY,
            "scope":"current server process; display events require a connected display"})
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

pub(super) fn snapshot(last: usize) -> serde_json::Value {
    TIMELINE
        .get_or_init(|| Mutex::new(Timeline::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot(last)
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
        let snapshot = timeline.snapshot(3);
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

        let event = &timeline.snapshot(1)["events"][0];
        assert_eq!(event["fields"]["prompt_char_count"], 3978);
        assert_eq!(
            event["fields"]["reference_byte_lengths"],
            serde_json::json!([1024, 2048, 4096])
        );
        assert_eq!(
            event["line"],
            "IMAGERY request kind=scene_plate prompt_chars=3978 references=3"
        );
    }
}
