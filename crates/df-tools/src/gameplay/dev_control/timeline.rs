//! Development-only bounded mirror of the shared audit path, never authoritative game state.
use serde::Serialize;
use serde_json::{Map, Value, json};
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
    "music",
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
        component_filter: Option<&str>,
        summary_only: bool,
    ) -> serde_json::Value {
        let matched = self
            .events
            .iter()
            .filter(|event| {
                resource_filter.is_none_or(|filter| {
                    event.fields.get("resource_type").and_then(Value::as_str) == Some(filter)
                }) && component_filter.is_none_or(|filter| {
                    event.fields.get("component_type").and_then(Value::as_str) == Some(filter)
                })
            })
            .collect::<Vec<_>>();
        let window = &matched[matched.len().saturating_sub(last)..];
        let events = if summary_only {
            Vec::new()
        } else {
            window.to_vec()
        };
        let mut by_type = std::collections::BTreeMap::<String, usize>::new();
        let mut by_stage = std::collections::BTreeMap::<String, usize>::new();
        let mut by_status = std::collections::BTreeMap::<String, usize>::new();
        let mut identities = std::collections::BTreeSet::<(String, String)>::new();
        let mut component_identities =
            std::collections::BTreeSet::<(String, String, String, String)>::new();
        let mut by_component = std::collections::BTreeMap::<String, usize>::new();
        let mut by_component_phase = std::collections::BTreeMap::<String, usize>::new();
        let mut by_visibility = std::collections::BTreeMap::<String, usize>::new();
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
            if let Some(component) = event.fields.get("component_type").and_then(Value::as_str) {
                *by_component.entry(component.to_owned()).or_default() += 1;
                if let Some(id) = event
                    .fields
                    .get("resource_id")
                    .and_then(|value| match value {
                        Value::String(id) => Some(id.clone()),
                        Value::Number(id) => Some(id.to_string()),
                        _ => None,
                    })
                {
                    let producer = event
                        .fields
                        .get("page_producer_hash")
                        .and_then(Value::as_str)
                        .unwrap_or("none");
                    let scene = event
                        .fields
                        .get("scene_id")
                        .and_then(|value| match value {
                            Value::String(id) => Some(id.clone()),
                            Value::Number(id) => Some(id.to_string()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "none".to_owned());
                    component_identities.insert((
                        component.to_owned(),
                        id,
                        producer.to_owned(),
                        scene,
                    ));
                }
            }
            if let Some(value) = event.fields.get("component_phase").and_then(Value::as_str) {
                *by_component_phase.entry(value.to_owned()).or_default() += 1;
            }
            if let Some(value) = event.fields.get("visibility").and_then(Value::as_str) {
                *by_visibility.entry(value.to_owned()).or_default() += 1;
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
                "component_count": component_identities.len(),
                "by_component": by_component,
                "by_component_phase": by_component_phase,
                "by_visibility": by_visibility,
            },
            "filter": {"resource_type": resource_filter, "component": component_filter, "summary_only": summary_only},
            "dropped": self.next - 1 - self.events.len() as u64,
            "capacity": CAPACITY,
            "scope":"current server process; browser events require a connected display"
        })
    }
}

/// The Jev classification events a `dev decisions` view lists, as `(kind, event)`. Each event
/// names its judgement in a `source=` token: `jev` when the model decided, `rules` when it was
/// unavailable and the neutral fallback decided.
pub(super) const DECISION_KINDS: &[(&str, &str)] = &[
    ("line", "gameplay.line_judgement"),
    ("trade", "gameplay.merchant.trade_judgement"),
    ("quote", "gameplay.merchant.quote_candidate"),
    ("attempt", "gameplay.attempt_classified"),
    ("action", "gameplay.physical_action_classified"),
    ("target", "gameplay.investigation_target"),
    ("npc-target", "gameplay.npc_target"),
    ("approach", "gameplay.social.reading"),
    ("freeform", "gameplay.freeform_action"),
    ("scene", "gameplay.scene_interaction"),
    ("npc-event", "gameplay.npc.reply_event"),
];

/// The kind a line starts with: its first word when that word is a plain upper-case token such as
/// `LINE_JUDGEMENT`, `SCENE-FIGHT` or `IMAGERY`; otherwise `other`.
fn line_kind(line: &str) -> String {
    let word = line.split_whitespace().next().unwrap_or("");
    let plain = !word.is_empty()
        && word.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        });
    if plain {
        word.to_owned()
    } else {
        "other".to_owned()
    }
}

/// Compares two kind names the way an agent types them: case-insensitive, `_` and `-` alike.
fn same_kind(left: &str, right: &str) -> bool {
    let normal = |text: &str| text.to_ascii_uppercase().replace('_', "-");
    normal(left) == normal(right)
}

/// The `source=` token of an audit line, when the line carries one.
fn source_of(line: &str) -> Option<&str> {
    line.split_whitespace()
        .find_map(|word| word.strip_prefix("source="))
}

impl Timeline {
    /// The newest `last` audit lines, optionally only those of one kind, oldest first.
    fn audit(&self, last: usize, line_kind_filter: Option<&str>) -> Value {
        let wanted = |event: &&Event| {
            line_kind_filter.is_none_or(|kind| same_kind(&line_kind(&event.line), kind))
        };
        let total = self.events.iter().filter(wanted).count();
        let mut events: Vec<Value> = self
            .events
            .iter()
            .rev()
            .filter(wanted)
            .take(last)
            .map(|event| {
                json!({
                    "sequence": event.sequence,
                    "unix_ms": event.unix_ms,
                    "event": event.event,
                    "type": line_kind(&event.line),
                    "line": event.line,
                })
            })
            .collect();
        events.reverse();
        json!({
            "events": events,
            "matched": total,
            "returned": events.len(),
            "filter": {"type": line_kind_filter},
            "dropped": self.next - 1 - self.events.len() as u64,
            "capacity": CAPACITY,
            "scope": "current server process; the same audit lines the server prints, mirrored in a bounded ring",
        })
    }

    /// The newest `last` Jev classification events, optionally only those of one kind, oldest
    /// first. An event with `source=rules` (or `plan=unavailable`) is a neutral outcome: the model
    /// did not decide it.
    fn decisions(&self, last: usize, kind_filter: Option<&str>) -> Value {
        let matched = self.events.iter().filter_map(|event| {
            let kind = DECISION_KINDS
                .iter()
                .find(|(_, name)| *name == event.event)?
                .0;
            if kind_filter.is_some_and(|wanted| wanted != kind) {
                return None;
            }
            Some((event, kind))
        });
        let total = matched.clone().count();
        let mut decisions: Vec<Value> = matched
            .rev()
            .take(last)
            .map(|(event, kind)| {
                let source = source_of(&event.line);
                json!({
                    "sequence": event.sequence,
                    "unix_ms": event.unix_ms,
                    "kind": kind,
                    "event": event.event,
                    "source": source,
                    "neutral": source == Some("rules") || event.line.contains("plan=unavailable"),
                    "line": event.line,
                })
            })
            .collect();
        decisions.reverse();
        json!({
            "decisions": decisions,
            "matched": total,
            "returned": decisions.len(),
            "filter": {"kind": kind_filter},
            "dropped": self.next - 1 - self.events.len() as u64,
            "capacity": CAPACITY,
            "scope": "current server process; Jev classifications and their neutral fallbacks from the audit ring",
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
    } else if event.starts_with("gameplay.music") || event.starts_with("gameplay.tempo") {
        Some("music")
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
        if line.contains(" music-") || line.contains(" tempo-") {
            Some("music")
        } else if line.contains(" visual-source ")
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
    component_filter: Option<&str>,
    summary_only: bool,
) -> serde_json::Value {
    TIMELINE
        .get_or_init(|| Mutex::new(Timeline::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot(last, resource_filter, component_filter, summary_only)
}

/// `dev audit`: the newest audit lines from the shared ring, optionally of one line kind.
pub(super) fn audit(last: usize, line_kind: Option<&str>) -> Value {
    TIMELINE
        .get_or_init(|| Mutex::new(Timeline::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .audit(last, line_kind)
}

fn matches_npc(event: &Event, wanted: &str) -> bool {
    let same = |value: &str| {
        value.eq_ignore_ascii_case(wanted)
            || value
                .rsplit_once(':')
                .is_some_and(|(_, id)| id.eq_ignore_ascii_case(wanted))
    };
    for key in ["npc", "npc_id", "actor_id", "scene.actor"] {
        if event
            .fields
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(same)
        {
            return true;
        }
        let token = format!("{key}=");
        for (index, _) in event.line.match_indices(&token) {
            if index > 0 && !event.line.as_bytes()[index - 1].is_ascii_whitespace() {
                continue;
            }
            let value = &event.line[index + token.len()..];
            let value = if value.starts_with('"') {
                serde_json::Deserializer::from_str(value)
                    .into_iter::<String>()
                    .next()
                    .and_then(Result::ok)
            } else {
                value.split_whitespace().next().map(str::to_owned)
            };
            if value.as_deref().is_some_and(same) {
                return true;
            }
        }
    }
    false
}

/// Cursor queries filter before paging. With a cursor, return the next page, never the tail.
pub(super) fn query(
    params: &std::collections::HashMap<String, String>,
    mode: &str,
    last: usize,
    kind: Option<&str>,
    component: Option<&str>,
    summary: bool,
) -> Result<Value, String> {
    let since = params
        .get("since")
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| "since must be an unsigned sequence".to_owned())?;
    let timeline = TIMELINE
        .get_or_init(|| Mutex::new(Timeline::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(timeline.query(
        since,
        params.get("npc").map(String::as_str),
        mode,
        last,
        kind,
        component,
        summary,
    ))
}
impl Timeline {
    // The three read routes share independent cursor, classification and presentation filters.
    #[allow(clippy::too_many_arguments)]
    fn query(
        &self,
        since: Option<u64>,
        npc: Option<&str>,
        mode: &str,
        last: usize,
        kind: Option<&str>,
        component: Option<&str>,
        summary: bool,
    ) -> Value {
        let mut selected = Self {
            start: self.start,
            next: self.next,
            events: self
                .events
                .iter()
                .filter(|event| since.is_none_or(|since| event.sequence > since))
                .filter(|event| npc.is_none_or(|npc| matches_npc(event, npc)))
                .cloned()
                .collect(),
        };
        selected.events.retain(|event| match mode {
            "audit" => kind.is_none_or(|kind| same_kind(&line_kind(&event.line), kind)),
            "decisions" => DECISION_KINDS.iter().any(|(decision, name)| {
                *name == event.event && kind.is_none_or(|kind| kind == *decision)
            }),
            _ => {
                kind.is_none_or(|kind| {
                    event.fields.get("resource_type").and_then(Value::as_str) == Some(kind)
                }) && component.is_none_or(|component| {
                    event.fields.get("component_type").and_then(Value::as_str) == Some(component)
                })
            }
        });
        let matched = selected.events.len();
        if since.is_some() {
            selected.events.truncate(last);
        }
        let page_size = last;
        let mut document = match mode {
            "audit" => selected.audit(page_size, kind),
            "decisions" => selected.decisions(page_size, kind),
            _ => selected.snapshot(page_size, kind, component, false),
        };
        let key = if mode == "decisions" {
            "decisions"
        } else {
            "events"
        };
        let mut next = since.unwrap_or(0);
        if let Some(events) = document[key].as_array_mut() {
            if since.is_some() {
                events.truncate(last);
            }
            if let Some(sequence) = events.last().and_then(|event| event["sequence"].as_u64()) {
                next = sequence;
            }
            if summary {
                events.clear();
            }
            document["returned"] = json!(events.len());
        }
        document["matched"] = json!(matched);
        document["next_since"] = json!(next);
        document["oldest_available"] = json!(self.events.front().map(|event| event.sequence));
        document["cursor_gap"] = json!(since.is_some_and(|since| {
            self.events
                .front()
                .is_some_and(|event| since.saturating_add(1) < event.sequence)
        }));
        document["dropped"] = json!(self.next - 1 - self.events.len() as u64);
        document["filter"]["since"] = json!(since);
        document["filter"]["npc"] = json!(npc);
        document
    }
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
        let snapshot = timeline.snapshot(3, None, None, false);
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

        let event = &timeline.snapshot(1, None, None, false)["events"][0];
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

        let filtered = timeline.snapshot(20, Some("image"), None, false);
        let events = filtered["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["fields"]["resource_id"], "image-key-42");
        assert_eq!(events[1]["fields"]["resource_id"], "image-key-42");
        assert_eq!(filtered["summary"]["resource_count"], 1);
        assert_eq!(filtered["summary"]["by_status"]["failed"], 1);
        assert_eq!(filtered["summary"]["by_status"]["ready"], 1);

        let summary = timeline.snapshot(20, Some("image"), None, true);
        assert_eq!(summary["events"].as_array().unwrap().len(), 0);
        assert_eq!(summary["matched"], 2);
    }

    #[test]
    fn music_plan_is_filterable_without_mixing_into_visual_events() {
        let mut timeline = Timeline::new();
        timeline.push(
            "gameplay.music.plan",
            "MUSIC plan phase=anticipation intensity=700 suite=main bed=amb-harbour colour=hearth-warm",
            Map::from_iter([
                ("resource_id".to_owned(), Value::String("tempo:4".to_owned())),
                ("status".to_owned(), Value::String("projected".to_owned())),
            ]),
        );
        let snapshot = timeline.snapshot(10, Some("music"), None, false);
        assert_eq!(snapshot["matched"], 1);
        assert_eq!(snapshot["events"][0]["fields"]["resource_type"], "music");
        assert_eq!(snapshot["events"][0]["fields"]["status"], "projected");
    }

    #[test]
    fn component_filter_correlates_lifetimes_and_reports_effective_visibility() {
        let mut timeline = Timeline::new();
        for (phase, visibility, at_ms, held, reason) in [
            ("opened", "visible", 100, 0, "ready"),
            ("readable", "visible", 250, 0, "ready"),
            ("readable", "occluded", 900, 650, "visibility-lost"),
            ("hidden", "hidden", 1_100, 650, "successor"),
        ] {
            timeline.push(
                "gameplay.presentation",
                "DISPLAY component content-free lifecycle",
                Map::from_iter([
                    (
                        "resource_type".to_owned(),
                        Value::String("browser_media".to_owned()),
                    ),
                    ("resource_id".to_owned(), Value::from(42)),
                    ("scene_id".to_owned(), Value::from(7)),
                    (
                        "page_producer_hash".to_owned(),
                        Value::String("sha256:page".to_owned()),
                    ),
                    (
                        "component_type".to_owned(),
                        Value::String("speech_bubble".to_owned()),
                    ),
                    (
                        "component_phase".to_owned(),
                        Value::String(phase.to_owned()),
                    ),
                    (
                        "visibility".to_owned(),
                        Value::String(visibility.to_owned()),
                    ),
                    ("visible_held_ms".to_owned(), Value::from(held)),
                    ("at_ms".to_owned(), Value::from(at_ms)),
                    ("reason".to_owned(), Value::String(reason.to_owned())),
                ]),
            );
        }
        timeline.push(
            "gameplay.presentation",
            "DISPLAY component content-free lifecycle",
            Map::from_iter([
                (
                    "resource_type".to_owned(),
                    Value::String("browser_media".to_owned()),
                ),
                (
                    "component_type".to_owned(),
                    Value::String("check_card".to_owned()),
                ),
            ]),
        );

        let filtered = timeline.snapshot(20, Some("browser_media"), Some("speech_bubble"), false);
        assert_eq!(filtered["matched"], 4);
        assert_eq!(filtered["summary"]["component_count"], 1);
        assert_eq!(filtered["summary"]["by_component"]["speech_bubble"], 4);
        assert_eq!(filtered["summary"]["by_component_phase"]["hidden"], 1);
        assert_eq!(filtered["summary"]["by_visibility"]["occluded"], 1);
        assert_eq!(filtered["events"][3]["fields"]["reason"], "successor");
        assert_eq!(filtered["events"][2]["fields"]["visible_held_ms"], 650);
        assert!(filtered["events"].to_string().find("transcript").is_none());

        let summary = timeline.snapshot(20, Some("browser_media"), Some("speech_bubble"), true);
        assert_eq!(summary["events"].as_array().unwrap().len(), 0);
        assert_eq!(summary["matched"], 4);
    }

    #[test]
    fn audit_filters_by_line_kind_in_any_spelling_and_keeps_order() {
        let mut timeline = Timeline::new();
        for (event, line) in [
            (
                "gameplay.scene_fight",
                "SCENE-FIGHT aggressor=\"Brynn\" target=\"corin\"",
            ),
            ("gameplay.imagery.request", "IMAGERY kind=item reason=coin"),
            (
                "gameplay.line_judgement",
                "LINE_JUDGEMENT stage=scene source=jev",
            ),
            (
                "gameplay.scene_fight",
                "SCENE-FIGHT unavailable reason=\"busy\"",
            ),
        ] {
            timeline.push(event, line, Map::new());
        }

        let fights = timeline.audit(10, Some("scene_fight"));
        let events = fights["events"].as_array().unwrap();
        assert_eq!(fights["matched"], 2);
        assert_eq!(events[0]["type"], "SCENE-FIGHT");
        assert!(events[0]["line"].as_str().unwrap().contains("Brynn"));
        assert!(events[1]["line"].as_str().unwrap().contains("unavailable"));
        assert!(events[0]["sequence"].as_u64() < events[1]["sequence"].as_u64());

        let newest = timeline.audit(1, None);
        assert_eq!(newest["matched"], 4);
        assert_eq!(newest["returned"], 1);
        assert_eq!(newest["events"][0]["type"], "SCENE-FIGHT");
        assert_eq!(timeline.audit(10, Some("INFLUENCE"))["matched"], 0);
    }

    #[test]
    fn decisions_mark_the_rules_fallback_as_neutral_and_filter_by_kind() {
        let mut timeline = Timeline::new();
        for (event, line) in [
            (
                "gameplay.line_judgement",
                "LINE_JUDGEMENT stage=scene source=jev meaning=examine reason=none",
            ),
            (
                "gameplay.merchant.trade_judgement",
                "MERCHANT_TRADE_JUDGEMENT stage=offer source=rules meaning=unavailable reason=provider",
            ),
            (
                "gameplay.freeform_action",
                "FREEFORM plan=unavailable reason=provider",
            ),
            ("gameplay.imagery.request", "IMAGERY kind=item"),
        ] {
            timeline.push(event, line, Map::new());
        }

        let all = timeline.decisions(10, None);
        let decisions = all["decisions"].as_array().unwrap();
        assert_eq!(
            all["matched"], 3,
            "only Jev classification events are listed"
        );
        assert_eq!(decisions[0]["kind"], "line");
        assert_eq!(decisions[0]["source"], "jev");
        assert_eq!(decisions[0]["neutral"], false);
        assert_eq!(decisions[1]["kind"], "trade");
        assert_eq!(decisions[1]["source"], "rules");
        assert_eq!(decisions[1]["neutral"], true);
        assert_eq!(decisions[2]["kind"], "freeform");
        assert!(decisions[2]["source"].is_null());
        assert_eq!(decisions[2]["neutral"], true);

        let trades = timeline.decisions(10, Some("trade"));
        assert_eq!(trades["matched"], 1);
        assert_eq!(trades["decisions"][0]["kind"], "trade");
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    #[test]
    fn cursor_pages_filter_before_limit_and_expose_gaps() {
        let mut timeline = Timeline::new();
        for sequence in 1..=12 {
            timeline.push(
                "gameplay.line_judgement",
                &format!("LINE_JUDGEMENT source=jev step={sequence}"),
                Map::from_iter([(
                    "npc_id".into(),
                    json!(if sequence % 2 == 0 {
                        "merchant"
                    } else {
                        "guard"
                    }),
                )]),
            );
        }
        let first = timeline.query(Some(0), Some("merchant"), "decisions", 2, None, None, false);
        assert_eq!(first["decisions"][0]["sequence"], 2);
        assert_eq!(first["decisions"][1]["sequence"], 4);
        assert_eq!(first["next_since"], 4);
        let second = timeline.query(Some(4), Some("merchant"), "audit", 2, None, None, false);
        assert_eq!(second["events"][0]["sequence"], 6);
        assert_eq!(second["events"][1]["sequence"], 8);
        assert_eq!(second["dropped"], 0);
        assert_eq!(second["cursor_gap"], false);
        timeline.events.pop_front();
        timeline.events.pop_front();
        assert_eq!(
            timeline.query(Some(0), None, "timeline", 2, None, None, false)["cursor_gap"],
            true
        );
    }
}

#[cfg(test)]
mod npc_filter_tests {
    use super::*;
    #[test]
    fn npc_filter_accepts_canonical_ids_and_exact_quoted_audit_fields() {
        let mut timeline = Timeline::new();
        timeline.push(
            "gameplay.line_judgement",
            r#"LINE_JUDGEMENT npc="patron:merchant""#,
            Map::new(),
        );
        timeline.push(
            "gameplay.line_judgement",
            r#"LINE_JUDGEMENT npc="Marek and Co""#,
            Map::new(),
        );
        timeline.push(
            "gameplay.line_judgement",
            r#"LINE_JUDGEMENT npc="merchant-old""#,
            Map::new(),
        );
        assert_eq!(
            timeline.query(
                Some(0),
                Some("merchant"),
                "decisions",
                10,
                None,
                None,
                false
            )["returned"],
            1
        );
        assert_eq!(
            timeline.query(
                Some(0),
                Some("Marek and Co"),
                "audit",
                10,
                None,
                None,
                false
            )["returned"],
            1
        );
    }
}
