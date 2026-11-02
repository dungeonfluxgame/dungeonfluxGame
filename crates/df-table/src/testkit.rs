//! Scripted fakes and sample data for this crate's tests and for hosts that test against it.

use std::sync::Mutex;

use crate::event::TableEvent;
use crate::kind::KindRegistry;
use crate::kind_file::KindFile;
use crate::ports::EventSink;

/// A kind with one of each field type, a label field and a color field.
pub const CHARACTER_KIND: &str = r##"{
    "name": "character", "shape": "circle", "width": 6, "height": 6,
    "label_field": "name", "color_field": "color",
    "fields": [
        {"name": "name", "type": "text", "maximum_length": 20, "required": true},
        {"name": "color", "type": "color", "default": "#4f8cff"},
        {"name": "hit_points", "type": "number", "minimum": 0, "maximum": 999, "default": 10},
        {"name": "role", "type": "choice", "choices": ["hero", "npc", "foe"], "default": "hero"},
        {"name": "hidden", "type": "flag", "default": false}
    ]
}"##;

/// A kind with no fields at all.
pub const MARKER_KIND: &str = r#"{"name": "marker", "shape": "circle", "width": 2, "height": 2}"#;

/// Reads kind-file texts into a registry, for tests whose kinds are known to be valid.
pub fn registry_from(sources: &[&str]) -> KindRegistry {
    let files: Vec<KindFile> = sources
        .iter()
        .map(|source| serde_json::from_str(source).expect("the test kind file parses"))
        .collect();
    KindRegistry::from_files(&files).expect("the test kinds are valid")
}

/// An event sink that keeps everything it is given, in order.
#[derive(Debug, Default)]
pub struct RecordingSink {
    events: Mutex<Vec<TableEvent>>,
}

impl RecordingSink {
    /// A copy of the recorded events, oldest first.
    pub fn events(&self) -> Vec<TableEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl EventSink for RecordingSink {
    fn record(&self, event: &TableEvent) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event.clone());
    }
}
