//! One record of what the table did, in both renderings (O.6: "two renderings from one record").
//!
//! A record is made from one typed `TableEvent` (or one `EngineAudit` line, which has the same
//! shape): its audit line, `AREA verb key=value`, is the event's own `render_audit_line`, and its
//! attributes are the event's typed fields as serde writes them, flattened to `name = value` with
//! text, whole numbers and flags only. Both come from the same value in the same call, so they
//! cannot disagree, and neither is parsed from the other: the line is never read back (RES41), and
//! a field is never found by looking in text.
//!
//! The correlation (`OperationId`, `turn_seq`, `SessionRevision`) and the table scope are lifted
//! out of the event into the `df.*` attributes the shared telemetry path indexes (`df.operation`
//! among them), so a record joins back to the input that caused it. An event that has none (a
//! canvas change) says `df.correlated=false` and never borrows a correlation from whatever input
//! happens to be current (observability: "captured at dispatch, not reconstructed").
//!
//! It owns the record types, the exhaustive event name and severity of every outer event
//! variant, and the flattening. It never decides what is recorded or where a record goes: the
//! sink does.

use std::fmt;

use df_table::event::{EventClass, TableEvent};
use serde::Serialize;
use serde_json::{Map, Number, Value};

use super::limits::{MAXIMUM_ATTRIBUTE_DEPTH, RECORD_OVERHEAD_BYTES};

/// How often a record is wanted: class A is always on and never sampled, class B is a verbose
/// level (H.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordClass {
    Always,
    Verbose,
}

impl RecordClass {
    pub fn key(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Verbose => "verbose",
        }
    }

    /// The letter the plan uses for the class.
    pub fn letter(self) -> char {
        match self {
            Self::Always => 'A',
            Self::Verbose => 'B',
        }
    }
}

impl From<EventClass> for RecordClass {
    fn from(class: EventClass) -> Self {
        match class {
            EventClass::Always => Self::Always,
            EventClass::Verbose => Self::Verbose,
        }
    }
}

/// How serious a record is: refusals and handled degradations are warnings, failed owned work is
/// an error, and verbose detail is debug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordSeverity {
    Debug,
    Info,
    Warn,
    Error,
}

impl RecordSeverity {
    pub fn key(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// An attribute value: text, a whole number or a flag, the three shapes every path accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum AttributeValue {
    Text(String),
    Whole(i64),
    Flag(bool),
}

/// What joins a record to the input that caused it. Only the operation is always present; an
/// engine line (a commit, a dev call) has no turn or revision of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordCorrelation {
    /// The operation id, 32 lowercase hexadecimal digits.
    pub operation: String,
    pub turn_seq: Option<u64>,
    pub epoch: Option<u64>,
    /// The sequence of the session revision within its epoch.
    pub revision: Option<u64>,
}

/// Where in the table a record happened.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RecordScope {
    pub table: Option<u64>,
    pub scene: Option<u64>,
    pub machine: Option<String>,
}

/// Why an event could not be made into a record. The audit line was still written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// The event could not be written as typed fields.
    Unserializable { reason: String },
    /// The event's typed fields are not a map of named fields.
    NotAnObject,
    /// The event has no variant tag, so it cannot be named.
    MissingTag,
    /// A field every engine event shares (`scope`, `correlation`) is not what it should be.
    BadField { field: &'static str },
    /// The event nests deeper than `MAXIMUM_ATTRIBUTE_DEPTH`.
    TooDeep,
}

impl fmt::Display for RecordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unserializable { reason } => {
                write!(formatter, "the event could not be written: {reason}")
            }
            Self::NotAnObject => formatter.write_str("the event is not a map of fields"),
            Self::MissingTag => formatter.write_str("the event has no variant tag"),
            Self::BadField { field } => write!(formatter, "the event's {field} is not valid"),
            Self::TooDeep => formatter.write_str("the event nests too deeply to flatten"),
        }
    }
}

impl std::error::Error for RecordError {}

/// A record before it has an id and a time: everything the event itself says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordDraft {
    pub class: RecordClass,
    pub severity: RecordSeverity,
    pub area: &'static str,
    /// The OTEL event name: a fixed dotted lowercase word, one per outer event variant.
    pub event_name: &'static str,
    /// The variant as serde names it, `input_received` or `pipeline.dice_drawn`.
    pub kind: String,
    pub audit_line: String,
    pub scope: RecordScope,
    pub correlation: Option<RecordCorrelation>,
    /// The event's own fields, flattened.
    pub fields: Vec<(String, AttributeValue)>,
}

/// One record as the sink stamped it and every outlet receives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObservedRecord {
    /// The sink's number for this record, from one counter for the whole run. Both renderings
    /// carry it: the history prints it beside the line and `df.audit_id` holds it.
    pub audit_id: u64,
    /// When the sink saw it, in milliseconds since the Unix epoch, from the sink's clock.
    pub observed_ms: u64,
    pub class: RecordClass,
    pub severity: RecordSeverity,
    pub area: &'static str,
    pub event_name: &'static str,
    pub kind: String,
    /// The one-line rendering. It is also the body of the structured record.
    pub audit_line: String,
    pub correlation: Option<RecordCorrelation>,
    /// The structured rendering: the `df.*` attributes, then the event's own fields.
    pub attributes: Vec<(String, AttributeValue)>,
}

impl RecordDraft {
    /// A draft for something that is not a `TableEvent`, such as a line of the engine's own
    /// audit. `fields` and the operation are added with the builders below.
    pub fn new(
        class: RecordClass,
        severity: RecordSeverity,
        area: &'static str,
        event_name: &'static str,
        kind: &str,
        audit_line: String,
    ) -> Self {
        Self {
            class,
            severity,
            area,
            event_name,
            kind: kind.to_owned(),
            audit_line,
            scope: RecordScope::default(),
            correlation: None,
            fields: Vec::new(),
        }
    }

    /// Names the input this record is about, for a line that has an operation and no more.
    pub fn about_operation(mut self, operation: String) -> Self {
        self.correlation = Some(RecordCorrelation {
            operation,
            turn_seq: None,
            epoch: None,
            revision: None,
        });
        self
    }

    pub fn with_field(mut self, name: &str, value: AttributeValue) -> Self {
        self.fields.push((name.to_owned(), value));
        self
    }

    /// The record of one typed event: its line from `render_audit_line`, its fields from serde.
    pub fn from_event(event: &TableEvent) -> Result<Self, RecordError> {
        let audit_line = event.render_audit_line();
        let value = serde_json::to_value(event).map_err(|error| RecordError::Unserializable {
            reason: error.to_string(),
        })?;
        let Value::Object(mut map) = value else {
            return Err(RecordError::NotAnObject);
        };
        let tag = take_text(&mut map, "event").ok_or(RecordError::MissingTag)?;
        let kind = match take_text(&mut map, "what") {
            Some(what) => format!("{tag}.{what}"),
            None => tag,
        };
        let scope = map
            .remove("scope")
            .map(|value| scope_of(&value))
            .transpose()?;
        let correlation = map
            .remove("correlation")
            .map(|value| correlation_of(&value))
            .transpose()?;
        let mut fields = Vec::new();
        for (name, value) in &map {
            flatten(name, value, 0, &mut fields)?;
        }
        let class = RecordClass::from(event.class());
        Ok(Self {
            class,
            severity: severity_of(event, class),
            area: event.area(),
            event_name: event_name(event),
            kind,
            audit_line,
            scope: scope.unwrap_or_default(),
            correlation,
            fields,
        })
    }
}

impl ObservedRecord {
    /// Gives a draft its id and time, and writes the `df.*` attributes the shared path indexes.
    pub fn stamp(draft: RecordDraft, audit_id: u64, observed_ms: u64) -> Self {
        let mut attributes = Vec::with_capacity(draft.fields.len() + 12);
        let mut text = |name: &str, value: &str| {
            attributes.push((name.to_owned(), AttributeValue::Text(value.to_owned())));
        };
        text("df.area", draft.area);
        text("df.class", draft.class.key());
        text("df.event", &draft.kind);
        if let Some(machine) = &draft.scope.machine {
            text("df.machine", machine);
        }
        if let Some(correlation) = &draft.correlation {
            text("df.operation", &correlation.operation);
        }
        let mut whole = |name: &str, value: Option<u64>| {
            if let Some(value) = value {
                attributes.push((
                    name.to_owned(),
                    AttributeValue::Whole(i64::try_from(value).unwrap_or(i64::MAX)),
                ));
            }
        };
        whole("df.audit_id", Some(audit_id));
        whole("df.table", draft.scope.table);
        whole("df.scene", draft.scope.scene);
        let correlation = draft.correlation.as_ref();
        whole("df.turn_seq", correlation.and_then(|c| c.turn_seq));
        whole("df.revision.epoch", correlation.and_then(|c| c.epoch));
        whole("df.revision.sequence", correlation.and_then(|c| c.revision));
        attributes.push((
            "df.correlated".to_owned(),
            AttributeValue::Flag(draft.correlation.is_some()),
        ));
        attributes.extend(draft.fields);
        Self {
            audit_id,
            observed_ms,
            class: draft.class,
            severity: draft.severity,
            area: draft.area,
            event_name: draft.event_name,
            kind: draft.kind,
            audit_line: draft.audit_line,
            correlation: draft.correlation,
            attributes,
        }
    }

    /// What the record costs to keep, for the byte bound of the history and the spool queue.
    pub fn approximate_bytes(&self) -> usize {
        let attributes: usize = self
            .attributes
            .iter()
            .map(|(name, value)| {
                name.len()
                    + match value {
                        AttributeValue::Text(text) => text.len(),
                        AttributeValue::Whole(_) | AttributeValue::Flag(_) => {
                            std::mem::size_of::<i64>()
                        }
                    }
            })
            .sum();
        RECORD_OVERHEAD_BYTES + self.audit_line.len() + self.kind.len() + attributes
    }

    /// Whether the record belongs to the input with this operation id (32 hexadecimal digits).
    pub fn is_about(&self, operation: &str) -> bool {
        self.correlation
            .as_ref()
            .is_some_and(|correlation| correlation.operation == operation)
    }
}

fn take_text(map: &mut Map<String, Value>, key: &str) -> Option<String> {
    match map.remove(key)? {
        Value::String(text) => Some(text),
        _ => None,
    }
}

fn whole_field(
    map: &Map<String, Value>,
    key: &str,
    field: &'static str,
) -> Result<u64, RecordError> {
    map.get(key)
        .and_then(Value::as_u64)
        .ok_or(RecordError::BadField { field })
}

fn optional_whole(
    map: &Map<String, Value>,
    key: &str,
    field: &'static str,
) -> Result<Option<u64>, RecordError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or(RecordError::BadField { field }),
    }
}

fn scope_of(value: &Value) -> Result<RecordScope, RecordError> {
    let bad = RecordError::BadField { field: "scope" };
    let Value::Object(map) = value else {
        return Err(bad);
    };
    let machine = match map.get("machine") {
        None | Some(Value::Null) => None,
        Some(Value::String(machine)) => Some(machine.clone()),
        Some(_) => return Err(bad),
    };
    Ok(RecordScope {
        table: Some(whole_field(map, "table", "scope")?),
        scene: optional_whole(map, "scene", "scope")?,
        machine,
    })
}

fn correlation_of(value: &Value) -> Result<RecordCorrelation, RecordError> {
    let bad = || RecordError::BadField {
        field: "correlation",
    };
    let Value::Object(map) = value else {
        return Err(bad());
    };
    let Some(Value::String(operation)) = map.get("operation_id") else {
        return Err(bad());
    };
    let Some(Value::Object(revision)) = map.get("revision") else {
        return Err(bad());
    };
    Ok(RecordCorrelation {
        operation: operation.clone(),
        turn_seq: Some(whole_field(map, "turn_seq", "correlation")?),
        epoch: Some(whole_field(revision, "epoch", "correlation")?),
        revision: Some(whole_field(revision, "sequence", "correlation")?),
    })
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

fn number(number: &Number) -> AttributeValue {
    if let Some(whole) = number.as_i64() {
        AttributeValue::Whole(whole)
    } else if let Some(whole) = number.as_u64() {
        i64::try_from(whole).map_or_else(
            |_| AttributeValue::Text(whole.to_string()),
            AttributeValue::Whole,
        )
    } else {
        AttributeValue::Text(number.to_string())
    }
}

/// Writes `value` as attributes named by their path (`effect.kind`). A null is an absent field
/// and writes nothing; a list names each item by its place.
fn flatten(
    path: &str,
    value: &Value,
    depth: usize,
    out: &mut Vec<(String, AttributeValue)>,
) -> Result<(), RecordError> {
    if depth > MAXIMUM_ATTRIBUTE_DEPTH {
        return Err(RecordError::TooDeep);
    }
    match value {
        Value::Null => {}
        Value::Bool(flag) => out.push((path.to_owned(), AttributeValue::Flag(*flag))),
        Value::Number(value) => out.push((path.to_owned(), number(value))),
        Value::String(text) => out.push((path.to_owned(), AttributeValue::Text(text.clone()))),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                flatten(&join(path, &index.to_string()), item, depth + 1, out)?;
            }
        }
        Value::Object(map) => {
            for (name, item) in map {
                flatten(&join(path, name), item, depth + 1, out)?;
            }
        }
    }
    Ok(())
}

/// The OTEL event name of an event. Exhaustive over the outer variants, so a variant added without
/// its name does not compile; the nested families (`pipeline`, `table_state`, `projection`) share
/// one name each and say which of theirs it is in `df.event`.
fn event_name(event: &TableEvent) -> &'static str {
    match event {
        TableEvent::ElementSpawned { .. } => "table.canvas.element_spawned",
        TableEvent::ElementDespawned { .. } => "table.canvas.element_despawned",
        TableEvent::ElementChanged { .. } => "table.canvas.element_changed",
        TableEvent::ElementMoved { .. } => "table.canvas.element_moved",
        TableEvent::ElementTravelling { .. } => "table.canvas.element_travelling",
        TableEvent::ElementArrived { .. } => "table.canvas.element_arrived",
        TableEvent::ElementStopped { .. } => "table.canvas.element_stopped",
        TableEvent::CanvasCleared { .. } => "table.canvas.cleared",
        TableEvent::EffectRejected { .. } => "table.canvas.effect_rejected",
        TableEvent::KindsReloaded { .. } => "table.canvas.kinds_reloaded",
        TableEvent::KindsRejected { .. } => "table.canvas.kinds_rejected",
        TableEvent::ElementTransformed { .. } => "table.canvas.element_transformed",
        TableEvent::InputReceived { .. } => "table.input.received",
        TableEvent::InputDeclined { .. } => "table.input.declined",
        TableEvent::InputRefused { .. } => "table.input.refused",
        TableEvent::InputServed { .. } => "table.input.served",
        TableEvent::InputAbandoned { .. } => "table.input.abandoned",
        TableEvent::InputFinishedLate { .. } => "table.input.finished_late",
        TableEvent::StepFinished { .. } => "table.step.finished",
        TableEvent::TurnApplied { .. } => "table.turn.applied",
        TableEvent::BoundaryFailed { .. } => "table.boundary.failed",
        TableEvent::SettingRejected { .. } => "table.setting.rejected",
        TableEvent::SettingKeysIgnored { .. } => "table.setting.keys_ignored",
        TableEvent::MachineEntered { .. } => "table.machine.entered",
        TableEvent::MachineTransitioned { .. } => "table.machine.transitioned",
        TableEvent::MachineExited { .. } => "table.machine.exited",
        TableEvent::MachineFailed { .. } => "table.machine.failed",
        TableEvent::EffectApplied { .. } => "table.effect.applied",
        TableEvent::EffectRefused { .. } => "table.effect.refused",
        TableEvent::LoadRefused { .. } => "table.load.refused",
        TableEvent::Pipeline(_) => "table.pipeline",
        TableEvent::TableState(_) => "table.state",
        TableEvent::Projection(_) => "table.projection",
    }
}

/// Failed owned work is an error, a refusal or a handled degradation is a warning, verbose detail
/// is debug and everything else is information.
fn severity_of(event: &TableEvent, class: RecordClass) -> RecordSeverity {
    if class == RecordClass::Verbose {
        return RecordSeverity::Debug;
    }
    match event {
        TableEvent::BoundaryFailed { .. }
        | TableEvent::MachineFailed { .. }
        | TableEvent::LoadRefused { .. } => RecordSeverity::Error,
        TableEvent::EffectRejected { .. }
        | TableEvent::KindsRejected { .. }
        | TableEvent::InputDeclined { .. }
        | TableEvent::InputRefused { .. }
        | TableEvent::InputAbandoned { .. }
        | TableEvent::InputFinishedLate { .. }
        | TableEvent::EffectRefused { .. }
        | TableEvent::SettingRejected { .. }
        | TableEvent::SettingKeysIgnored { .. } => RecordSeverity::Warn,
        TableEvent::ElementSpawned { .. }
        | TableEvent::ElementDespawned { .. }
        | TableEvent::ElementChanged { .. }
        | TableEvent::ElementMoved { .. }
        | TableEvent::ElementTravelling { .. }
        | TableEvent::ElementArrived { .. }
        | TableEvent::ElementStopped { .. }
        | TableEvent::CanvasCleared { .. }
        | TableEvent::KindsReloaded { .. }
        | TableEvent::ElementTransformed { .. }
        | TableEvent::InputReceived { .. }
        | TableEvent::InputServed { .. }
        | TableEvent::StepFinished { .. }
        | TableEvent::TurnApplied { .. }
        | TableEvent::MachineEntered { .. }
        | TableEvent::MachineTransitioned { .. }
        | TableEvent::MachineExited { .. }
        | TableEvent::EffectApplied { .. }
        | TableEvent::Pipeline(_)
        | TableEvent::TableState(_)
        | TableEvent::Projection(_) => RecordSeverity::Info,
    }
}
