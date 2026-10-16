//! Bounded, content-free presentation timing capture. Browser clocks are relative to one page.

use std::collections::VecDeque;

use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::{AnyValue, KeyValue, any_value::Value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
};
use prost::Message;

pub const MAX_BUFFER_EVENTS: usize = 128;
pub const MAX_BATCH_EVENTS: usize = 32;
// Leave room for authenticated generated-RPC scope/identity fields under the 64 KiB bridge cap.
pub const MAX_BATCH_BYTES: usize = 60 * 1024;
const MAX_PAGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

/// Stable page-local identifier for an already audience-safe scene/asset label. This hash is
/// diagnostic correlation, never anonymization, authentication or a canonical game identity.
pub fn local_identity(label: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in label.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    hash & i64::MAX as u64
}
const KINDS: &[&str] = &[
    "line",
    "line-open",
    "voice-start",
    "voice-end",
    "voice-failed",
    "figure",
    "figure-mounted",
    "figure-shown",
    "figure-hidden",
    "figure-kept",
    "scene",
    "plate-requested",
    "plate-loaded",
    "plate-decoded",
    "plate-shown",
    "plate-kept",
    "visual-source",
    "clip-requested",
    "clip-ready",
    "clip-shown",
    "clip-ended",
    "clip-failed",
    "check-shown",
    "check-hidden",
    "sound",
    "paint",
    "paint-opportunity",
    "usable",
];
const REASONS: &[&str] = &[
    "ready",
    "read",
    "voice-ended",
    "skipped",
    "scene-change",
    "replaced",
    "ended",
    "failed",
    "muted",
    "unlocked",
    "blocked",
    "appeared",
    "left",
    "timeout",
    "disposed",
    "changed",
];
const NUMERIC_FIELDS: &[&str] = &[
    "id",
    "scene",
    "at_ms",
    "slot",
    "chars",
    "read_ms",
    "opened_ms",
    "voice_start_ms",
    "voice_end_ms",
    "closed_ms",
    "crossfade_ms",
    "requested_asset",
    "effective_asset",
];

/// Closed scene image source labels used by the display's visual-source diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualSource {
    Other,
    ScenePlate,
    LiveBackdrop,
    SceneArtwork,
    EstablishingPhoto,
}

impl VisualSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Other => "other",
            Self::ScenePlate => "scene_plate",
            Self::LiveBackdrop => "live_backdrop",
            Self::SceneArtwork => "scene_artwork",
            Self::EstablishingPhoto => "establishing_photo",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "other" => Some(Self::Other),
            "scene_plate" => Some(Self::ScenePlate),
            "live_backdrop" => Some(Self::LiveBackdrop),
            "scene_artwork" => Some(Self::SceneArtwork),
            "establishing_photo" => Some(Self::EstablishingPhoto),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualShot {
    Other,
    Plate,
    Establishing,
    Stage,
}

impl VisualShot {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Other => "other",
            Self::Plate => "plate",
            Self::Establishing => "establishing",
            Self::Stage => "stage",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "other" => Some(Self::Other),
            "plate" => Some(Self::Plate),
            "establishing" => Some(Self::Establishing),
            "stage" => Some(Self::Stage),
            _ => None,
        }
    }
}

/// Closed server plate states included with a browser source observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualPlateState {
    None,
    Pending,
    Ready,
    Failed,
    Unavailable,
}

impl VisualPlateState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "none" => Some(Self::None),
            "pending" => Some(Self::Pending),
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            "unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualSourceDetails {
    pub requested: VisualSource,
    pub effective: VisualSource,
    pub shot: VisualShot,
    pub plate_state: VisualPlateState,
    /// Page-local hashes of validated local asset identifiers, never URLs.
    pub requested_asset: Option<u64>,
    pub effective_asset: Option<u64>,
}

/// Identifiers are page-local hashes/slots, never names, text, asset URLs or player payloads.
/// Absent optional times mean unobserved; a missing voice end never implies successful playback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimingEvent {
    pub kind: &'static str,
    pub reason: &'static str,
    pub id: u64,
    pub scene: u64,
    pub at_ms: u64,
    pub slot: Option<u32>,
    pub chars: u32,
    pub read_ms: u32,
    pub opened_ms: Option<u64>,
    pub voice_start_ms: Option<u64>,
    pub voice_end_ms: Option<u64>,
    pub closed_ms: Option<u64>,
    pub crossfade_ms: u32,
    pub visual_source: Option<VisualSourceDetails>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimingError {
    Empty,
    Invalid,
    Capacity,
    Oversized,
    SequenceExhausted,
}

impl TimingEvent {
    pub fn lifecycle(
        kind: &'static str,
        reason: &'static str,
        id: u64,
        scene: u64,
        at_ms: u64,
    ) -> Self {
        Self {
            kind,
            reason,
            id,
            scene,
            at_ms,
            slot: None,
            chars: 0,
            read_ms: 0,
            opened_ms: None,
            voice_start_ms: None,
            voice_end_ms: None,
            closed_ms: None,
            crossfade_ms: 0,
            visual_source: None,
        }
    }

    pub fn visual_source(id: u64, scene: u64, at_ms: u64, details: VisualSourceDetails) -> Self {
        let mut event = Self::lifecycle("visual-source", "changed", id, scene, at_ms);
        event.visual_source = Some(details);
        event
    }

    fn validate(&self) -> Result<(), TimingError> {
        if !KINDS.contains(&self.kind)
            || !REASONS.contains(&self.reason)
            || self.id > i64::MAX as u64
            || self.scene > i64::MAX as u64
            || self.at_ms > MAX_PAGE_MS
            || self.slot.is_some_and(|slot| slot > 31)
            || self.chars > 16_384
            || self.read_ms > 120_000
            || self.crossfade_ms > 30_000
            || [
                self.opened_ms,
                self.voice_start_ms,
                self.voice_end_ms,
                self.closed_ms,
            ]
            .into_iter()
            .flatten()
            .any(|time| time > self.at_ms)
            || self
                .opened_ms
                .zip(self.closed_ms)
                .is_some_and(|(open, close)| close < open)
            || self
                .voice_start_ms
                .zip(self.voice_end_ms)
                .is_some_and(|(start, end)| end < start)
            || (self.kind == "visual-source") != self.visual_source.is_some()
            || self.visual_source.as_ref().is_some_and(|details| {
                details
                    .requested_asset
                    .is_some_and(|id| id > i64::MAX as u64)
                    || details
                        .effective_asset
                        .is_some_and(|id| id > i64::MAX as u64)
            })
        {
            return Err(TimingError::Invalid);
        }
        Ok(())
    }

    pub fn log_line(
        &self,
        audience: &str,
        receive_unix_ms: u64,
        producer: &str,
        batch: u64,
    ) -> String {
        let optional =
            |time: Option<u64>| time.map_or_else(|| "none".into(), |time| time.to_string());
        let held = self
            .opened_ms
            .zip(self.closed_ms)
            .map(|(open, close)| close.saturating_sub(open));
        let mut line = format!(
            "{audience} {} producer={producer} batch={batch} receive_unix_ms={receive_unix_ms} id={} scene={} slot={} at_ms={} chars={} read_ms={} opened_ms={} voice_start_ms={} voice_end_ms={} closed_ms={} held_ms={} crossfade_ms={} reason={}",
            self.kind,
            self.id,
            self.scene,
            optional(self.slot.map(u64::from)),
            self.at_ms,
            self.chars,
            self.read_ms,
            optional(self.opened_ms),
            optional(self.voice_start_ms),
            optional(self.voice_end_ms),
            optional(self.closed_ms),
            optional(held),
            self.crossfade_ms,
            self.reason,
        );
        if let Some(details) = &self.visual_source {
            line.push_str(&format!(
                " requested_source={} effective_source={} shot={} plate_state={} requested_asset_id={} effective_asset_id={}",
                details.requested.label(),
                details.effective.label(),
                details.shot.label(),
                details.plate_state.label(),
                optional(details.requested_asset),
                optional(details.effective_asset),
            ));
        }
        line
    }
}

fn attribute(key: &str, value: Value) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue { value: Some(value) }),
    }
}

/// Encode only the timing catalog as standard OTLP logs; no free-form body or resource metadata.
pub fn encode(events: &[TimingEvent]) -> Result<Vec<u8>, TimingError> {
    if events.is_empty() || events.len() > MAX_BATCH_EVENTS {
        return Err(TimingError::Capacity);
    }
    let mut records = Vec::with_capacity(events.len());
    for event in events {
        event.validate()?;
        let mut attributes = vec![attribute("reason", Value::StringValue(event.reason.into()))];
        for (name, value) in [
            ("id", Some(event.id)),
            ("scene", Some(event.scene)),
            ("at_ms", Some(event.at_ms)),
            ("slot", event.slot.map(u64::from)),
            ("chars", Some(u64::from(event.chars))),
            ("read_ms", Some(u64::from(event.read_ms))),
            ("opened_ms", event.opened_ms),
            ("voice_start_ms", event.voice_start_ms),
            ("voice_end_ms", event.voice_end_ms),
            ("closed_ms", event.closed_ms),
            ("crossfade_ms", Some(u64::from(event.crossfade_ms))),
        ] {
            if let Some(value) = value {
                attributes.push(attribute(name, Value::IntValue(value as i64)));
            }
        }
        if let Some(details) = &event.visual_source {
            attributes.extend([
                attribute(
                    "requested_source",
                    Value::StringValue(details.requested.label().into()),
                ),
                attribute(
                    "effective_source",
                    Value::StringValue(details.effective.label().into()),
                ),
                attribute("shot", Value::StringValue(details.shot.label().into())),
                attribute(
                    "plate_state",
                    Value::StringValue(details.plate_state.label().into()),
                ),
            ]);
            for (name, value) in [
                ("requested_asset", details.requested_asset),
                ("effective_asset", details.effective_asset),
            ] {
                if let Some(value) = value {
                    attributes.push(attribute(name, Value::IntValue(value as i64)));
                }
            }
        }
        records.push(LogRecord {
            event_name: event.kind.into(),
            attributes,
            ..Default::default()
        });
    }
    let bytes = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: records,
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec();
    if bytes.len() > MAX_BATCH_BYTES {
        return Err(TimingError::Oversized);
    }
    Ok(bytes)
}

/// Reject every uncatalogued field, including OTLP bodies, arbitrary attributes and trace IDs.
/// Call before native logging; client timestamps and IDs grant no identity or authority.
pub fn decode(bytes: &[u8]) -> Result<Vec<TimingEvent>, TimingError> {
    if bytes.len() > MAX_BATCH_BYTES {
        return Err(TimingError::Oversized);
    }
    let request = ExportLogsServiceRequest::decode(bytes).map_err(|_| TimingError::Invalid)?;
    let [resource] = request.resource_logs.as_slice() else {
        return Err(TimingError::Invalid);
    };
    let [scope] = resource.scope_logs.as_slice() else {
        return Err(TimingError::Invalid);
    };
    if resource.resource.is_some()
        || !resource.schema_url.is_empty()
        || scope.scope.is_some()
        || !scope.schema_url.is_empty()
        || scope.log_records.is_empty()
        || scope.log_records.len() > MAX_BATCH_EVENTS
    {
        return Err(TimingError::Invalid);
    }
    let mut events = Vec::with_capacity(scope.log_records.len());
    for record in &scope.log_records {
        let kind = KINDS
            .iter()
            .copied()
            .find(|kind| *kind == record.event_name)
            .ok_or(TimingError::Invalid)?;
        let mut values = std::collections::BTreeMap::new();
        let mut reason = None;
        let mut requested_source = None;
        let mut effective_source = None;
        let mut shot = None;
        let mut plate_state = None;
        for field in &record.attributes {
            let value = field
                .value
                .as_ref()
                .and_then(|value| value.value.as_ref())
                .ok_or(TimingError::Invalid)?;
            if field.key == "reason" {
                let Value::StringValue(text) = value else {
                    return Err(TimingError::Invalid);
                };
                if reason.is_some() {
                    return Err(TimingError::Invalid);
                }
                reason = REASONS.iter().copied().find(|reason| *reason == text);
                if reason.is_none() {
                    return Err(TimingError::Invalid);
                }
            } else if field.key == "requested_source" {
                let Value::StringValue(text) = value else {
                    return Err(TimingError::Invalid);
                };
                if requested_source.is_some() {
                    return Err(TimingError::Invalid);
                }
                requested_source = VisualSource::from_label(text);
                if requested_source.is_none() {
                    return Err(TimingError::Invalid);
                }
            } else if field.key == "effective_source" {
                let Value::StringValue(text) = value else {
                    return Err(TimingError::Invalid);
                };
                if effective_source.is_some() {
                    return Err(TimingError::Invalid);
                }
                effective_source = VisualSource::from_label(text);
                if effective_source.is_none() {
                    return Err(TimingError::Invalid);
                }
            } else if field.key == "plate_state" {
                let Value::StringValue(text) = value else {
                    return Err(TimingError::Invalid);
                };
                if plate_state.is_some() {
                    return Err(TimingError::Invalid);
                }
                plate_state = VisualPlateState::from_label(text);
                if plate_state.is_none() {
                    return Err(TimingError::Invalid);
                }
            } else if field.key == "shot" {
                let Value::StringValue(text) = value else {
                    return Err(TimingError::Invalid);
                };
                if shot.is_some() {
                    return Err(TimingError::Invalid);
                }
                shot = VisualShot::from_label(text);
                if shot.is_none() {
                    return Err(TimingError::Invalid);
                }
            } else {
                let Value::IntValue(value) = value else {
                    return Err(TimingError::Invalid);
                };
                if !NUMERIC_FIELDS.contains(&field.key.as_str())
                    || *value < 0
                    || values.insert(field.key.as_str(), *value as u64).is_some()
                {
                    return Err(TimingError::Invalid);
                }
            }
        }
        let visual_source = match (requested_source, effective_source, shot, plate_state) {
            (Some(requested), Some(effective), Some(shot), Some(plate_state)) => {
                Some(VisualSourceDetails {
                    requested,
                    effective,
                    shot,
                    plate_state,
                    requested_asset: values.get("requested_asset").copied(),
                    effective_asset: values.get("effective_asset").copied(),
                })
            }
            (None, None, None, None) => None,
            _ => return Err(TimingError::Invalid),
        };
        let required = |name| values.get(name).copied().ok_or(TimingError::Invalid);
        let event = TimingEvent {
            kind,
            reason: reason.ok_or(TimingError::Invalid)?,
            id: required("id")?,
            scene: required("scene")?,
            at_ms: required("at_ms")?,
            slot: values
                .get("slot")
                .copied()
                .map(u32::try_from)
                .transpose()
                .map_err(|_| TimingError::Invalid)?,
            chars: u32::try_from(required("chars")?).map_err(|_| TimingError::Invalid)?,
            read_ms: u32::try_from(required("read_ms")?).map_err(|_| TimingError::Invalid)?,
            crossfade_ms: u32::try_from(required("crossfade_ms")?)
                .map_err(|_| TimingError::Invalid)?,
            opened_ms: values.get("opened_ms").copied(),
            voice_start_ms: values.get("voice_start_ms").copied(),
            voice_end_ms: values.get("voice_end_ms").copied(),
            closed_ms: values.get("closed_ms").copied(),
            visual_source,
        };
        event.validate()?;
        // Comparing a rebuilt record also rejects severity, body, trace, flags, timestamps,
        // dropped-attribute markers and other uncatalogued OTLP fields.
        let canonical = encode(std::slice::from_ref(&event))?;
        let canonical = ExportLogsServiceRequest::decode(canonical.as_slice())
            .map_err(|_| TimingError::Invalid)?;
        let mut original = record.clone();
        original
            .attributes
            .sort_by(|left, right| left.key.cmp(&right.key));
        let mut expected = canonical.resource_logs[0].scope_logs[0].log_records[0].clone();
        expected
            .attributes
            .sort_by(|left, right| left.key.cmp(&right.key));
        if original != expected {
            return Err(TimingError::Invalid);
        }
        events.push(event);
    }
    Ok(events)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimingBatch {
    pub sequence: u64,
    pub bytes: Vec<u8>,
    pub records: usize,
    pub dropped: u64,
}

/// Memory-only page capture: one immutable prefix is retried until acknowledged. Overflow drops
/// newest events and reports the cumulative gap. Page close loses the remainder explicitly.
pub struct TimingBuffer {
    events: VecDeque<TimingEvent>,
    pending: Option<TimingBatch>,
    next: u64,
    dropped: u64,
    reported_dropped: u64,
}
impl Default for TimingBuffer {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            pending: None,
            next: 1,
            dropped: 0,
            reported_dropped: 0,
        }
    }
}
impl TimingBuffer {
    /// Carry a gap reported by a bounded capture source before upload admission.
    pub fn report_loss(&mut self, records: u64) {
        self.dropped = self.dropped.saturating_add(records);
    }
    pub fn push(&mut self, event: TimingEvent) -> Result<(), TimingError> {
        if let Err(error) = event.validate() {
            self.dropped = self.dropped.saturating_add(1);
            return Err(error);
        }
        if self.events.len() == MAX_BUFFER_EVENTS {
            self.dropped = self.dropped.saturating_add(1);
            return Err(TimingError::Capacity);
        }
        self.events.push_back(event);
        Ok(())
    }
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
    pub fn batch(&mut self) -> Result<TimingBatch, TimingError> {
        if let Some(batch) = &self.pending {
            return Ok(batch.clone());
        }
        if self.next > i64::MAX as u64 {
            return Err(TimingError::SequenceExhausted);
        }
        let events = self
            .events
            .iter()
            .take(MAX_BATCH_EVENTS)
            .cloned()
            .collect::<Vec<_>>();
        if events.is_empty() && self.dropped == self.reported_dropped {
            return Err(TimingError::Empty);
        }
        let batch = TimingBatch {
            sequence: self.next,
            records: events.len(),
            bytes: if events.is_empty() {
                Vec::new()
            } else {
                encode(&events)?
            },
            dropped: self.dropped,
        };
        self.pending = Some(batch.clone());
        Ok(batch)
    }
    pub fn acknowledge(&mut self, sequence: u64) -> bool {
        let Some(batch) = self
            .pending
            .as_ref()
            .filter(|batch| batch.sequence == sequence)
        else {
            return false;
        };
        self.events.drain(..batch.records);
        self.reported_dropped = batch.dropped;
        self.pending = None;
        self.next += 1;
        true
    }
}

// DOM components do not own a connection or credentials. This finite page-local staging queue
// hands their content-free events to the display's uploader; it never sends or logs by itself.
#[cfg(any(target_arch = "wasm32", test))]
thread_local! {
    static DISPLAY_CAPTURE: std::cell::RefCell<(VecDeque<TimingEvent>, u64)> =
        const { std::cell::RefCell::new((VecDeque::new(), 0)) };
}

#[cfg(any(target_arch = "wasm32", test))]
pub fn capture_display(event: TimingEvent) -> Result<(), TimingError> {
    DISPLAY_CAPTURE.with_borrow_mut(|(events, lost)| {
        if let Err(error) = event.validate() {
            *lost = lost.saturating_add(1);
            return Err(error);
        }
        if events.len() == MAX_BUFFER_EVENTS {
            *lost = lost.saturating_add(1);
            return Err(TimingError::Capacity);
        }
        events.push_back(event);
        Ok(())
    })
}

/// Transfer one finite chunk and its loss counter to the authenticated display owner.
#[cfg(any(target_arch = "wasm32", test))]
pub fn drain_display_capture() -> (Vec<TimingEvent>, u64) {
    DISPLAY_CAPTURE.with_borrow_mut(|(events, lost)| {
        let count = events.len().min(MAX_BATCH_EVENTS);
        (events.drain(..count).collect(), std::mem::take(lost))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> TimingEvent {
        TimingEvent {
            kind: "line",
            reason: "read",
            id: 29,
            scene: 7,
            at_ms: 10_000,
            slot: Some(2),
            chars: 89,
            read_ms: 8_285,
            opened_ms: Some(1_000),
            voice_start_ms: None,
            voice_end_ms: None,
            closed_ms: Some(10_000),
            crossfade_ms: 0,
            visual_source: None,
        }
    }

    #[test]
    fn display_staging_preserves_older_events_and_transfers_its_loss_counter() {
        DISPLAY_CAPTURE.with_borrow_mut(|(events, lost)| {
            events.clear();
            *lost = 0;
        });
        for id in 0..MAX_BUFFER_EVENTS as u64 {
            capture_display(TimingEvent { id, ..line() }).unwrap();
        }
        assert_eq!(capture_display(line()), Err(TimingError::Capacity));
        let (events, lost) = drain_display_capture();
        assert_eq!(events.len(), MAX_BATCH_EVENTS);
        assert_eq!(events[0].id, 0);
        assert_eq!(lost, 1);
        assert_eq!(drain_display_capture().1, 0);
        DISPLAY_CAPTURE.with_borrow_mut(|(events, _)| events.clear());
    }

    #[test]
    fn timing_batch_keeps_its_identity_and_bytes_until_matching_receipt() {
        let mut buffer = TimingBuffer::default();
        buffer.push(line()).unwrap();
        let first = buffer.batch().unwrap();
        buffer.push(TimingEvent { id: 30, ..line() }).unwrap();
        assert_eq!(buffer.batch().unwrap(), first);
        assert!(!buffer.acknowledge(first.sequence + 1));
        assert_eq!(buffer.batch().unwrap(), first);
        assert!(buffer.acknowledge(first.sequence));
        let second = buffer.batch().unwrap();
        assert_eq!(second.sequence, first.sequence + 1);
        assert_eq!(decode(&second.bytes).unwrap()[0].id, 30);
    }

    #[test]
    fn a_gap_without_any_retained_event_is_uploaded_once_after_acknowledgment() {
        let mut buffer = TimingBuffer::default();
        assert_eq!(buffer.batch(), Err(TimingError::Empty));
        buffer.report_loss(2);
        let gap = buffer.batch().unwrap();
        assert_eq!(gap.records, 0);
        assert!(gap.bytes.is_empty());
        assert_eq!(gap.dropped, 2);
        assert_eq!(buffer.batch().unwrap(), gap);
        assert!(buffer.acknowledge(gap.sequence));
        assert_eq!(buffer.batch(), Err(TimingError::Empty));
    }

    #[test]
    fn timing_buffer_bounds_batches_and_reports_loss_without_evicting_pending() {
        let mut buffer = TimingBuffer::default();
        for _ in 0..MAX_BUFFER_EVENTS {
            buffer.push(line()).unwrap();
        }
        let first = buffer.batch().unwrap();
        assert_eq!(first.records, MAX_BATCH_EVENTS);
        assert!(first.bytes.len() <= MAX_BATCH_BYTES);
        assert_eq!(buffer.push(line()), Err(TimingError::Capacity));
        assert_eq!(buffer.dropped(), 1);
        assert_eq!(buffer.batch().unwrap(), first);
    }

    #[test]
    fn timing_allowlist_rejects_text_unknown_keys_and_invalid_measurements() {
        assert_eq!(
            encode(&[TimingEvent {
                reason: "private words",
                ..line()
            }]),
            Err(TimingError::Invalid)
        );
        assert_eq!(
            encode(&[TimingEvent {
                opened_ms: Some(11_000),
                ..line()
            }]),
            Err(TimingError::Invalid)
        );
        let bytes = encode(&[line()]).unwrap();
        assert_eq!(decode(&bytes).unwrap(), vec![line()]);
        let mut request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
        request.resource_logs[0].scope_logs[0].log_records[0].body = Some(AnyValue {
            value: Some(Value::StringValue("private".into())),
        });
        assert_eq!(decode(&request.encode_to_vec()), Err(TimingError::Invalid));
    }

    #[test]
    fn visual_source_round_trips_closed_sources_state_and_asset_hashes() {
        let event = TimingEvent::visual_source(
            17,
            23,
            100,
            VisualSourceDetails {
                requested: VisualSource::ScenePlate,
                effective: VisualSource::SceneArtwork,
                shot: VisualShot::Establishing,
                plate_state: VisualPlateState::Failed,
                requested_asset: Some(31),
                effective_asset: Some(37),
            },
        );
        let bytes = encode(std::slice::from_ref(&event)).unwrap();
        let [decoded] = decode(&bytes).unwrap().try_into().unwrap();
        assert_eq!(decoded, event);
        let line = decoded.log_line("DISPLAY", 200, "producer", 3);
        assert!(line.contains("requested_source=scene_plate"));
        assert!(line.contains("effective_source=scene_artwork"));
        assert!(line.contains("plate_state=failed"));
        assert!(line.contains("requested_asset_id=31"));
        assert!(line.contains("effective_asset_id=37"));
    }

    #[test]
    fn visual_source_rejects_missing_or_uncatalogued_details() {
        assert_eq!(
            encode(&[TimingEvent::lifecycle("visual-source", "changed", 1, 2, 3,)]),
            Err(TimingError::Invalid)
        );
        let event = TimingEvent::visual_source(
            17,
            23,
            100,
            VisualSourceDetails {
                requested: VisualSource::ScenePlate,
                effective: VisualSource::SceneArtwork,
                shot: VisualShot::Establishing,
                plate_state: VisualPlateState::Failed,
                requested_asset: Some(31),
                effective_asset: Some(37),
            },
        );
        let bytes = encode(&[event]).unwrap();
        let mut request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
        let record = &mut request.resource_logs[0].scope_logs[0].log_records[0];
        let source = record
            .attributes
            .iter_mut()
            .find(|field| field.key == "effective_source")
            .unwrap();
        source.value.as_mut().unwrap().value = Some(Value::StringValue("private-url".into()));
        assert_eq!(decode(&request.encode_to_vec()), Err(TimingError::Invalid));
    }

    #[test]
    fn timing_ingress_rejects_unknown_attributes_duplicates_resources_and_oversize() {
        let bytes = encode(&[line()]).unwrap();
        let request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
        for field in [
            attribute("prompt", Value::StringValue("private".into())),
            attribute("at_ms", Value::IntValue(1)),
            attribute("slot", Value::IntValue(-1)),
        ] {
            let mut invalid = request.clone();
            invalid.resource_logs[0].scope_logs[0].log_records[0]
                .attributes
                .push(field);
            assert_eq!(decode(&invalid.encode_to_vec()), Err(TimingError::Invalid));
        }
        let mut invalid = request.clone();
        invalid.resource_logs.push(request.resource_logs[0].clone());
        assert_eq!(decode(&invalid.encode_to_vec()), Err(TimingError::Invalid));
        assert_eq!(
            decode(&vec![0; MAX_BATCH_BYTES + 1]),
            Err(TimingError::Oversized)
        );
        assert_eq!(
            encode(&vec![line(); MAX_BATCH_EVENTS + 1]),
            Err(TimingError::Capacity)
        );
    }

    #[test]
    fn timing_log_distinguishes_no_voice_from_cut_voice_and_keeps_receive_time() {
        let text = line().log_line("DISPLAY", 123_456, "page", 1);
        assert!(text.contains("receive_unix_ms=123456"));
        assert!(text.contains("held_ms=9000"));
        assert!(text.contains("voice_start_ms=none"));
        assert!(text.contains("chars=89 read_ms=8285"));
        assert!(text.contains("reason=read"));
    }
}
