//! What `history`, `trace`, `config`, `replay` and the verbs `describe` says are not built yet
//! answer with: the typed reports of the event and settings verbs.
//!
//! Like every report of the engine routes, they hold ids, closed keys, counts and the one-line
//! audit rendering of events, and never player text. `EventView` is one retained record as a
//! reader sees it: the audit line and what joins it to an input.

use serde::{Deserialize, Serialize};

use crate::observe::{ObservedRecord, SinkHealth};

use super::{SubmissionReport, TurnReport};

/// One retained event: the sink's number for it, when it was seen, its class and its line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventView {
    pub audit_id: u64,
    /// Milliseconds since the Unix epoch, from the sink's clock.
    pub observed_ms: u64,
    /// `A` (always on, never sampled) or `B` (verbose detail).
    pub class: char,
    pub severity: String,
    pub area: String,
    /// The one-line `AREA verb key=value` rendering of the event.
    pub line: String,
    /// The operation the event belongs to, when it has one.
    pub operation: Option<String>,
    pub turn_seq: Option<u64>,
}

impl From<&ObservedRecord> for EventView {
    fn from(record: &ObservedRecord) -> Self {
        Self {
            audit_id: record.audit_id,
            observed_ms: record.observed_ms,
            class: record.class.letter(),
            severity: record.severity.key().to_owned(),
            area: record.area.to_owned(),
            line: record.audit_line.clone(),
            operation: record
                .correlation
                .as_ref()
                .map(|correlation| correlation.operation.clone()),
            turn_seq: record
                .correlation
                .as_ref()
                .and_then(|correlation| correlation.turn_seq),
        }
    }
}

/// How much of the story the history still holds, so an answer says what it cannot see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionView {
    pub retained: usize,
    /// Events forgotten since the run began to stay within the memory bound.
    pub evicted: u64,
    pub oldest_audit_id: Option<u64>,
    pub newest_audit_id: Option<u64>,
}

/// `history`: the newest events the host kept, oldest first, and the stream's health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryReport {
    pub retention: RetentionView,
    /// How many retained events matched; `events` is the newest part of them.
    pub matched: usize,
    pub events: Vec<EventView>,
    pub stream: SinkHealth,
}

/// `trace <operation>`: everything the host kept about one input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceReport {
    /// The operation id, 32 hexadecimal digits.
    pub operation: String,
    pub retention: RetentionView,
    /// The events of the operation in the order they were recorded: the table's own events and
    /// the host's lines about it (the input as a dev call, a refusal while backing off).
    pub events: Vec<EventView>,
    /// What the queue did with it, if the host still remembers.
    pub submissions: Vec<SubmissionReport>,
    /// The turns that ran for it, if the host still remembers them.
    pub turns: Vec<TurnReport>,
}

/// When a changed setting takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applies {
    /// The pipeline reads the settings for every batch, so the next input uses the new value.
    NextInput,
    /// The owner reads it once when it starts, so a change needs a restart and is refused.
    Restart,
}

impl Applies {
    pub fn key(self) -> &'static str {
        match self {
            Self::NextInput => "next_input",
            Self::Restart => "restart",
        }
    }
}

/// One setting with its spec: the value in force and everything the spec says about it (J.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingView {
    pub name: String,
    pub value: u32,
    pub default: u32,
    pub minimum: u32,
    pub maximum: u32,
    pub unit: String,
    pub owner: String,
    pub cost_effect: String,
    pub applies: Applies,
}

/// A setting that `config set` changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingChange {
    pub name: String,
    pub from: u32,
    pub to: u32,
}

/// `config show` and `config set`: the settings in force, and the change just made if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigReport {
    pub settings: Vec<SettingView>,
    pub changed: Option<SettingChange>,
}

/// What a replay did, as C7's replay will fill it: the shape the seam is written to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayReport {
    pub log: String,
    pub turns_replayed: u64,
    /// The state hash after the last replayed turn, as 16 hexadecimal digits.
    pub final_state_hash: String,
    /// Whether every replayed turn reproduced its recorded outcome and state hash.
    pub matches_recorded: bool,
    /// The turn where replay first differed from the record, when it did.
    pub first_difference_turn: Option<u64>,
    /// With `--diff`: one line per difference.
    pub differences: Vec<String>,
}

/// An H.16 verb the host does not have yet, and what it waits for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotBuiltVerb {
    pub verb: String,
    pub needs: String,
}
