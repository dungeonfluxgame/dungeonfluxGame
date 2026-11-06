//! Bounded, content-free presentation timing capture. Browser clocks are relative to one page.

use std::collections::VecDeque;

use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::{AnyValue, KeyValue, any_value::Value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
};
use prost::Message;

use crate::limits::MAXIMUM_PAGE_MS;
pub use crate::limits::{
    MAXIMUM_TIMING_BATCH_BYTES, MAXIMUM_TIMING_BATCH_EVENTS, MAXIMUM_TIMING_EVENTS,
};

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
    "plate-crossfade",
    "plate-kept",
    "visual-source",
    "component",
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
    "started",
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
    "successor",
    "decode-fail",
    "visibility-lost",
    // Canonical component causes emitted by the HUD, check and ordinary attack presenters.
    "settled",
    "battle-begin",
    "phase-change",
    "dev-pause-capacity-resumed",
    "attack-rolled",
    "attack-settled",
    "face-dwell-complete",
    "attack-breakdown",
    "damage-rolled",
    "damage-settled",
    "attack-clearing",
    "read-complete",
    "damage-clearing",
    "attack-cleared",
    "card-closed",
    "damage-cleared",
    "attack-cancelled",
];
const NUMERIC_FIELDS: &[&str] = &[
    "id",
    "scene",
    "at_ms",
    "slot",
    "table_sequence",
    "chars",
    "read_ms",
    "opened_ms",
    "voice_start_ms",
    "voice_end_ms",
    "closed_ms",
    "crossfade_ms",
    "requested_asset",
    "effective_asset",
    "visible_held_ms",
    "lag_ms",
];

/// Closed catalog of display components whose lifecycle may be reported to the agent timeline.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PresentationComponent {
    Subtitle,
    SpeechBubble,
    ThinkingLoader,
    IntentLoader,
    CheckCard,
    /// A server d20 roll's die and result card on the shared display.
    Dice,
    RollCard,
    Die,
    CombatBanner,
    TurnChange,
    Battlefield,
    ItemCard,
    TitleCard,
    SceneCard,
    PlateIncoming,
    PlateOutgoing,
    PartyHud,
}

impl PresentationComponent {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Subtitle => "subtitle",
            Self::SpeechBubble => "speech_bubble",
            Self::ThinkingLoader => "thinking_loader",
            Self::IntentLoader => "intent_loader",
            Self::CheckCard => "check_card",
            Self::Dice => "dice",
            Self::RollCard => "roll_card",
            Self::Die => "die",
            Self::CombatBanner => "combat_banner",
            Self::TurnChange => "turn_change",
            Self::Battlefield => "battlefield",
            Self::ItemCard => "item_card",
            Self::TitleCard => "title_card",
            Self::SceneCard => "scene_card",
            Self::PlateIncoming => "plate_incoming",
            Self::PlateOutgoing => "plate_outgoing",
            Self::PartyHud => "party_hud",
        }
    }

    pub const fn all() -> &'static [Self] {
        &[
            Self::Subtitle,
            Self::SpeechBubble,
            Self::ThinkingLoader,
            Self::IntentLoader,
            Self::CheckCard,
            Self::Dice,
            Self::RollCard,
            Self::Die,
            Self::CombatBanner,
            Self::TurnChange,
            Self::Battlefield,
            Self::ItemCard,
            Self::TitleCard,
            Self::SceneCard,
            Self::PlateIncoming,
            Self::PlateOutgoing,
            Self::PartyHud,
        ]
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::all()
            .iter()
            .copied()
            .find(|component| component.label() == label)
    }
}

/// Closed lifecycle phases. `readable` is emitted only after the browser confirms effective
/// visibility; it is not inferred from a node merely existing in the DOM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentPhase {
    Requested,
    Queued,
    Opened,
    Readable,
    Closing,
    Activated,
    Hidden,
    Failed,
}

impl ComponentPhase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Queued => "queued",
            Self::Opened => "opened",
            Self::Readable => "readable",
            Self::Closing => "closing",
            Self::Activated => "activated",
            Self::Hidden => "hidden",
            Self::Failed => "failed",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "requested" => Some(Self::Requested),
            "queued" => Some(Self::Queued),
            "opened" => Some(Self::Opened),
            "readable" => Some(Self::Readable),
            "closing" => Some(Self::Closing),
            "activated" => Some(Self::Activated),
            "hidden" => Some(Self::Hidden),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Effective browser visibility after style, viewport and hit-test checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectiveVisibility {
    Visible,
    Hidden,
    Occluded,
    Offscreen,
    Transparent,
    Unknown,
}

impl EffectiveVisibility {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::Occluded => "occluded",
            Self::Offscreen => "offscreen",
            Self::Transparent => "transparent",
            Self::Unknown => "unknown",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "visible" => Some(Self::Visible),
            "hidden" => Some(Self::Hidden),
            "occluded" => Some(Self::Occluded),
            "offscreen" => Some(Self::Offscreen),
            "transparent" => Some(Self::Transparent),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// Page-local accounting for the interval a component was actually readable. This helper has
/// no clock or logging side effects; callers supply timestamps and computed visibility.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ComponentClock {
    opened_ms: Option<u64>,
    visible_since_ms: Option<u64>,
    visible_held_ms: u64,
}

impl ComponentClock {
    pub fn observe(
        &mut self,
        at_ms: u64,
        phase: ComponentPhase,
        visibility: EffectiveVisibility,
    ) -> u64 {
        if self.opened_ms.is_none()
            && matches!(phase, ComponentPhase::Opened | ComponentPhase::Readable)
        {
            self.opened_ms = Some(at_ms);
        }
        let can_be_visible = matches!(
            phase,
            ComponentPhase::Opened
                | ComponentPhase::Readable
                | ComponentPhase::Closing
                | ComponentPhase::Activated
        );
        if let Some(visible_since) = self.visible_since_ms
            && (!can_be_visible || visibility != EffectiveVisibility::Visible)
        {
            self.visible_held_ms = self
                .visible_held_ms
                .saturating_add(at_ms.saturating_sub(visible_since));
            self.visible_since_ms = None;
        }
        if can_be_visible
            && visibility == EffectiveVisibility::Visible
            && self.visible_since_ms.is_none()
        {
            self.visible_since_ms = Some(at_ms);
        }
        self.visible_held_ms.saturating_add(
            self.visible_since_ms
                .map_or(0, |start| at_ms.saturating_sub(start)),
        )
    }

    pub const fn opened_ms(&self) -> Option<u64> {
        self.opened_ms
    }

    pub fn visible_held_ms(&self, at_ms: u64) -> u64 {
        self.visible_held_ms.saturating_add(
            self.visible_since_ms
                .map_or(0, |start| at_ms.saturating_sub(start)),
        )
    }
}

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

/// Closed framing labels of the displayed scene image.
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

/// Which scene image the display asked for and which one it actually shows.
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
    /// Public server table-line sequence; absent for scene-only and combat captions.
    pub table_sequence: Option<u64>,
    pub chars: u32,
    pub read_ms: u32,
    pub opened_ms: Option<u64>,
    pub voice_start_ms: Option<u64>,
    pub voice_end_ms: Option<u64>,
    pub closed_ms: Option<u64>,
    pub crossfade_ms: u32,
    pub visual_source: Option<VisualSourceDetails>,
    pub component: Option<PresentationComponent>,
    pub component_phase: Option<ComponentPhase>,
    pub visibility: Option<EffectiveVisibility>,
    pub visible_held_ms: Option<u64>,
    /// How long a combat beat waited between the display receiving its view and the beat
    /// starting to play (the display's queue lag); set on a component record only.
    pub lag_ms: Option<u64>,
}

/// Why a timing event, batch or upload was refused.
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
            table_sequence: None,
            chars: 0,
            read_ms: 0,
            opened_ms: None,
            voice_start_ms: None,
            voice_end_ms: None,
            closed_ms: None,
            crossfade_ms: 0,
            visual_source: None,
            component: None,
            component_phase: None,
            visibility: None,
            visible_held_ms: None,
            lag_ms: None,
        }
    }

    pub fn visual_source(id: u64, scene: u64, at_ms: u64, details: VisualSourceDetails) -> Self {
        let mut event = Self::lifecycle("visual-source", "changed", id, scene, at_ms);
        event.visual_source = Some(details);
        event
    }

    pub fn component_lifecycle(
        id: u64,
        scene: u64,
        at_ms: u64,
        component: PresentationComponent,
        phase: ComponentPhase,
        visibility: EffectiveVisibility,
        reason: &'static str,
    ) -> Self {
        let mut event = Self::lifecycle("component", reason, id, scene, at_ms);
        event.component = Some(component);
        event.component_phase = Some(phase);
        event.visibility = Some(visibility);
        event
    }

    fn validate(&self) -> Result<(), TimingError> {
        if !KINDS.contains(&self.kind)
            || !REASONS.contains(&self.reason)
            || self.id > i64::MAX as u64
            || self.scene > i64::MAX as u64
            || self.at_ms > MAXIMUM_PAGE_MS
            || self.slot.is_some_and(|slot| slot > 31)
            || self
                .table_sequence
                .is_some_and(|sequence| sequence == 0 || sequence > i64::MAX as u64)
            || self.chars > 16_384
            || self.read_ms > 120_000
            || self.crossfade_ms > 30_000
            || self
                .visible_held_ms
                .is_some_and(|time| time > MAXIMUM_PAGE_MS)
            || self.lag_ms.is_some_and(|time| time > MAXIMUM_PAGE_MS)
            || (self.kind != "component" && self.lag_ms.is_some())
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
            || (self.kind == "component")
                != (self.component.is_some()
                    && self.component_phase.is_some()
                    && self.visibility.is_some())
            || (self.kind != "component"
                && (self.component.is_some()
                    || self.component_phase.is_some()
                    || self.visibility.is_some()
                    || self.visible_held_ms.is_some()))
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
            concat!(
                "{audience} {} producer={producer} batch={batch} ",
                "receive_unix_ms={receive_unix_ms} id={} scene={} slot={} at_ms={} chars={} ",
                "read_ms={} opened_ms={} voice_start_ms={} voice_end_ms={} closed_ms={} ",
                "held_ms={} crossfade_ms={} reason={}",
            ),
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
            audience = audience,
            producer = producer,
            batch = batch,
            receive_unix_ms = receive_unix_ms,
        );
        if let Some(sequence) = self.table_sequence {
            line.push_str(&format!(" table_sequence={sequence}"));
        }
        if let Some(details) = &self.visual_source {
            line.push_str(&format!(
                concat!(
                    " requested_source={} effective_source={} shot={} plate_state={} ",
                    "requested_asset_id={} effective_asset_id={}",
                ),
                details.requested.label(),
                details.effective.label(),
                details.shot.label(),
                details.plate_state.label(),
                optional(details.requested_asset),
                optional(details.effective_asset),
            ));
        }
        if let (Some(component), Some(phase), Some(visibility)) =
            (self.component, self.component_phase, self.visibility)
        {
            line.push_str(&format!(
                " component={} component_phase={} visibility={} visible_held_ms={} lag_ms={}",
                component.label(),
                phase.label(),
                visibility.label(),
                optional(self.visible_held_ms),
                optional(self.lag_ms),
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

/// OTLP integers are signed; `validate` already bounds every measurement to `i64::MAX`.
fn measurement(value: u64) -> Result<i64, TimingError> {
    i64::try_from(value).map_err(|_| TimingError::Invalid)
}

/// Encode only the timing catalog as standard OTLP logs; no free-form body or resource metadata.
pub fn encode(events: &[TimingEvent]) -> Result<Vec<u8>, TimingError> {
    if events.is_empty() || events.len() > MAXIMUM_TIMING_BATCH_EVENTS {
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
            ("table_sequence", event.table_sequence),
            ("chars", Some(u64::from(event.chars))),
            ("read_ms", Some(u64::from(event.read_ms))),
            ("opened_ms", event.opened_ms),
            ("voice_start_ms", event.voice_start_ms),
            ("voice_end_ms", event.voice_end_ms),
            ("closed_ms", event.closed_ms),
            ("crossfade_ms", Some(u64::from(event.crossfade_ms))),
        ] {
            if let Some(value) = value {
                attributes.push(attribute(name, Value::IntValue(measurement(value)?)));
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
                    attributes.push(attribute(name, Value::IntValue(measurement(value)?)));
                }
            }
        }
        if let (Some(component), Some(phase), Some(visibility)) =
            (event.component, event.component_phase, event.visibility)
        {
            attributes.extend([
                attribute("component", Value::StringValue(component.label().into())),
                attribute("component_phase", Value::StringValue(phase.label().into())),
                attribute("visibility", Value::StringValue(visibility.label().into())),
            ]);
        }
        if let Some(visible_held_ms) = event.visible_held_ms {
            attributes.push(attribute(
                "visible_held_ms",
                Value::IntValue(measurement(visible_held_ms)?),
            ));
        }
        if let Some(lag_ms) = event.lag_ms {
            attributes.push(attribute("lag_ms", Value::IntValue(measurement(lag_ms)?)));
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
    if bytes.len() > MAXIMUM_TIMING_BATCH_BYTES {
        return Err(TimingError::Oversized);
    }
    Ok(bytes)
}

/// Fill a closed-label slot from one string attribute; a repeat, a non-string or an
/// uncatalogued label is invalid.
fn set_label<T>(
    slot: &mut Option<T>,
    value: &Value,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<(), TimingError> {
    let Value::StringValue(text) = value else {
        return Err(TimingError::Invalid);
    };
    if slot.is_some() {
        return Err(TimingError::Invalid);
    }
    *slot = Some(parse(text).ok_or(TimingError::Invalid)?);
    Ok(())
}

/// Reject every uncatalogued field, including OTLP bodies, arbitrary attributes and trace IDs.
/// Call before native logging; client timestamps and IDs grant no identity or authority.
pub fn decode(bytes: &[u8]) -> Result<Vec<TimingEvent>, TimingError> {
    if bytes.len() > MAXIMUM_TIMING_BATCH_BYTES {
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
        || scope.log_records.len() > MAXIMUM_TIMING_BATCH_EVENTS
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
        let mut component = None;
        let mut component_phase = None;
        let mut visibility = None;
        for field in &record.attributes {
            let attribute_value = field
                .value
                .as_ref()
                .and_then(|attribute_value| attribute_value.value.as_ref())
                .ok_or(TimingError::Invalid)?;
            match field.key.as_str() {
                "reason" => set_label(&mut reason, attribute_value, |text| {
                    REASONS.iter().copied().find(|reason| *reason == text)
                })?,
                "requested_source" => set_label(
                    &mut requested_source,
                    attribute_value,
                    VisualSource::from_label,
                )?,
                "effective_source" => set_label(
                    &mut effective_source,
                    attribute_value,
                    VisualSource::from_label,
                )?,
                "plate_state" => set_label(
                    &mut plate_state,
                    attribute_value,
                    VisualPlateState::from_label,
                )?,
                "shot" => set_label(&mut shot, attribute_value, VisualShot::from_label)?,
                "component" => set_label(
                    &mut component,
                    attribute_value,
                    PresentationComponent::from_label,
                )?,
                "component_phase" => set_label(
                    &mut component_phase,
                    attribute_value,
                    ComponentPhase::from_label,
                )?,
                "visibility" => set_label(
                    &mut visibility,
                    attribute_value,
                    EffectiveVisibility::from_label,
                )?,
                numeric => {
                    let Value::IntValue(measurement) = attribute_value else {
                        return Err(TimingError::Invalid);
                    };
                    let measurement =
                        u64::try_from(*measurement).map_err(|_| TimingError::Invalid)?;
                    if !NUMERIC_FIELDS.contains(&numeric)
                        || values.insert(numeric, measurement).is_some()
                    {
                        return Err(TimingError::Invalid);
                    }
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
            table_sequence: values.get("table_sequence").copied(),
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
            component,
            component_phase,
            visibility,
            visible_held_ms: values.get("visible_held_ms").copied(),
            lag_ms: values.get("lag_ms").copied(),
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

/// One immutable upload prefix and the cumulative loss count it reports.
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
        if self.events.len() == MAXIMUM_TIMING_EVENTS {
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
            .take(MAXIMUM_TIMING_BATCH_EVENTS)
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
        if events.len() == MAXIMUM_TIMING_EVENTS {
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
        let count = events.len().min(MAXIMUM_TIMING_BATCH_EVENTS);
        (events.drain(..count).collect(), std::mem::take(lost))
    })
}

#[cfg(test)]
mod presentation_tests;
