//! Content-free image lifecycle observations over the shared OTEL capture boundary.
//! Events never carry prompts, reference bytes, paths, supplier prose or money authority.

use crate::execution::{DispatchId, ExecutionStageId, StageKind, StageView};
use crate::listed::listed_enum;
use crate::{AssetId, BackendId, ImageInterestId, ImageJob, ImageJobId, ImageOwner, Purpose};
use df_observe::{AnyValue, DiagnosticRecord, DiagnosticSink, LogInput, Severity};
use df_types::{OperationId, RunId};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

/// An opaque delivery identity, independent of image content and credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryId([u8; 16]);

impl DeliveryId {
    /// The identity for `bytes`, or `None` for the all-zero value, which names no delivery.
    pub fn from_bytes(bytes: [u8; 16]) -> Option<Self> {
        (bytes != [0; 16]).then_some(Self(bytes))
    }
}

/// Immutable provenance captured by the owner before asynchronous work is transferred.
#[derive(Clone, Debug, Default)]
pub struct EventContext {
    pub operation: Option<OperationId>,
    pub request: Option<ImageInterestId>,
    pub job: Option<ImageJobId>,
    pub stage: Option<ExecutionStageId>,
    pub attempt: Option<DispatchId>,
    /// Causal dispatch from the durable stage basis; lineage never adds its cost again.
    pub parent_attempt: Option<DispatchId>,
    pub run: Option<RunId>,
    pub config_revision: Option<u64>,
    /// Current admission gate, distinct from the stage's frozen configuration revision.
    pub gate_revision: Option<u64>,
    pub backend: Option<BackendId>,
    pub kind: Option<StageKind>,
    pub purpose: Option<Purpose>,
    pub analysis_kind: Option<AnalysisKind>,
    pub asset: Option<AssetId>,
    pub catalog_attempt: Option<df_assets::AttemptId>,
    pub delivery: Option<DeliveryId>,
    pub browser_producer: Option<df_observe::ProducerId>,
    pub trace: Option<df_observe::DiagnosticTrace>,
}

impl EventContext {
    /// Only safe identifiers cross this boundary; persisted request/config bodies never do.
    pub fn for_stage(job: &ImageJob, stage: &StageView, dispatch: Option<DispatchId>) -> Self {
        Self {
            job: Some(job.job),
            stage: Some(stage.stage),
            attempt: dispatch,
            catalog_attempt: stage
                .planned_publication
                .and_then(|identity| df_assets::AttemptId::from_bytes(&identity.attempt).ok()),
            run: match job.owner {
                ImageOwner::Run { run, .. } => Some(run),
                ImageOwner::Draft { .. } => None,
            },
            kind: Some(stage.kind),
            ..Self::default()
        }
    }
}

/// Closed lifecycle vocabulary keeps event names finite and private data out of labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Requested,
    Recorded,
    Replayed,
    Refused,
    CacheHit,
    CacheMiss,
    Coalesced,
    Queued,
    Claimed,
    Admitted,
    Sent,
    SupplierCheckpointed,
    SupplierCompleted,
    CandidateCheckpointed,
    DecisionCheckpointed,
    AnalysisCompleted,
    LocalAnalysisCompleted,
    SettlementRecorded,
    UsageProjected,
    UsageRecorded,
    UsageProjectionFailed,
    UsageProjectionPending,
    NormalizationStarted,
    NormalizationCompleted,
    TransformStarted,
    TransformCompleted,
    OutputStored,
    Published,
    RecoveryStarted,
    RecoveryCompleted,
    RecoveryDeferred,
    StageCompleted,
    Cancelled,
    Revoked,
    CodexStarted,
    CodexCompleted,
    Failed,
    Unknown,
    Completed,
    DeliveryAuthorized,
    Displayed,
    StaleCompletion,
    ConfigurationChanged,
    SourceDeferred,
    QuotaOpened,
    QuotaRecovered,
}

impl EventKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Requested => "image.requested",
            Self::Recorded => "image.recorded",
            Self::Replayed => "image.replayed",
            Self::Refused => "image.refused",
            Self::CacheHit => "image.cache_hit",
            Self::CacheMiss => "image.cache_miss",
            Self::Coalesced => "image.coalesced",
            Self::Queued => "image.queued",
            Self::Claimed => "image.claimed",
            Self::Admitted => "image.admitted",
            Self::Sent => "image.sent",
            Self::SupplierCheckpointed => "image.supplier_checkpointed",
            Self::SupplierCompleted => "image.supplier_completed",
            Self::CandidateCheckpointed => "image.candidate_checkpointed",
            Self::DecisionCheckpointed => "image.decision_checkpointed",
            Self::AnalysisCompleted => "image.analysis_completed",
            Self::LocalAnalysisCompleted => "image.local_analysis_completed",
            Self::SettlementRecorded => "image.settlement_recorded",
            Self::UsageProjected => "image.usage_projected",
            Self::UsageRecorded => "image.usage_recorded",
            Self::UsageProjectionFailed => "image.usage_projection_failed",
            Self::UsageProjectionPending => "image.usage_projection_pending",
            Self::NormalizationStarted => "image.normalization_started",
            Self::NormalizationCompleted => "image.normalization_completed",
            Self::TransformStarted => "image.transform_started",
            Self::TransformCompleted => "image.transform_completed",
            Self::OutputStored => "image.output_stored",
            Self::Published => "image.published",
            Self::RecoveryStarted => "image.recovery_started",
            Self::RecoveryCompleted => "image.recovery_completed",
            Self::RecoveryDeferred => "image.recovery_deferred",
            Self::StageCompleted => "image.stage_completed",
            Self::Cancelled => "image.cancelled",
            Self::Revoked => "image.revoked",
            Self::CodexStarted => "image.codex_started",
            Self::CodexCompleted => "image.codex_completed",
            Self::Failed => "image.failed",
            Self::Unknown => "image.unknown",
            Self::Completed => "image.completed",
            Self::DeliveryAuthorized => "image.delivery_authorized",
            Self::Displayed => "image.displayed",
            Self::StaleCompletion => "image.stale_completion",
            Self::ConfigurationChanged => "image.configuration_changed",
            Self::SourceDeferred => "image.source_deferred",
            Self::QuotaOpened => "image.quota_opened",
            Self::QuotaRecovered => "image.quota_recovered",
        }
    }

    fn level(self) -> ImageLogLevel {
        match self {
            Self::Failed | Self::Unknown | Self::UsageProjectionFailed => ImageLogLevel::Error,
            Self::Refused
            | Self::StaleCompletion
            | Self::SourceDeferred
            | Self::QuotaOpened
            | Self::RecoveryDeferred => ImageLogLevel::Warn,
            Self::Requested
            | Self::CacheMiss
            | Self::NormalizationStarted
            | Self::TransformStarted
            | Self::CodexStarted => ImageLogLevel::Debug,
            _ => ImageLogLevel::Info,
        }
    }
}

listed_enum! {
    keyed
    /// Diagnostic classification only; none of these codes authorizes retry or money movement.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum FailureKind {
        Authorization,
        Validation,
        Conflict,
        Capacity,
        Disabled,
        Expired,
        Cancelled,
        ProviderRefused,
        ProviderUnknown,
        Decode,
        Transform,
        Publication,
        Storage,
        Journal,
        UsageProjection,
        Worker,
    }
}

impl FailureKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Authorization => "authorization",
            Self::Validation => "validation",
            Self::Conflict => "conflict",
            Self::Capacity => "capacity",
            Self::Disabled => "disabled",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
            Self::ProviderRefused => "provider_refused",
            Self::ProviderUnknown => "provider_unknown",
            Self::Decode => "decode",
            Self::Transform => "transform",
            Self::Publication => "publication",
            Self::Storage => "storage",
            Self::Journal => "journal",
            Self::UsageProjection => "usage_projection",
            Self::Worker => "worker",
        }
    }
}

listed_enum! {
    keyed
    /// Closed supplier protocol position; neither supplier URLs nor raw errors are accepted.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ProviderStep {
        Submit,
        Poll,
        Delivery,
        Download,
        Validate,
        Unknown,
    }
}

impl ProviderStep {
    fn key(self) -> &'static str {
        match self {
            Self::Submit => "submit",
            Self::Poll => "poll",
            Self::Delivery => "delivery",
            Self::Download => "download",
            Self::Validate => "validate",
            Self::Unknown => "unknown",
        }
    }
}

listed_enum! {
    keyed
    /// Closed semantic operation names; private targets, instructions and decisions stay out.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum AnalysisKind {
        CreatorClassification,
        Inspection,
        PhotoInspection,
        HeadLocations,
        Segmentation,
        StagingIdentity,
        StagingFacing,
    }
}

impl AnalysisKind {
    fn key(self) -> &'static str {
        match self {
            Self::CreatorClassification => "creator_classification",
            Self::Inspection => "inspection",
            Self::PhotoInspection => "photo_inspection",
            Self::HeadLocations => "head_locations",
            Self::Segmentation => "segmentation",
            Self::StagingIdentity => "staging_identity",
            Self::StagingFacing => "staging_facing",
        }
    }
}

listed_enum! {
    keyed
    /// Diagnostic verdict only; it neither stores private decision payloads nor qualifies billing.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum AnalysisVerdict {
        Accepted,
        Rejected,
        Unjudged,
        Empty,
    }
}

impl AnalysisVerdict {
    fn key(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Unjudged => "unjudged",
            Self::Empty => "empty",
        }
    }
}

listed_enum! {
    keyed
    /// An acknowledged durable dispatch outcome, separate from whole-job publication or settlement.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum StageOutcome {
        OutputStored,
        Decision,
        Candidate,
        Unusable,
        Failed,
        Unknown,
    }
}

impl StageOutcome {
    fn key(self) -> &'static str {
        match self {
            Self::OutputStored => "output_stored",
            Self::Decision => "decision",
            Self::Candidate => "candidate",
            Self::Unusable => "unusable",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

/// Scalar measurements use explicit units and never contain private image content.
#[derive(Clone, Debug, Default)]
pub struct Measurements {
    pub queue_depth: Option<u64>,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub transform_steps: Option<u32>,
    pub algorithm_revision: Option<u32>,
    pub usage_revision: Option<crate::cost::UsageRevision>,
    pub usage_evidence: Option<crate::cost::UsageEvidence>,
    pub provider_step: Option<ProviderStep>,
    pub provider_status: Option<u16>,
    pub delivery_identified: Option<bool>,
    pub admission_refused: Option<bool>,
    pub browser_asset_identity: Option<u64>,
    pub browser_scene_identity: Option<u64>,
    pub browser_at_ms: Option<u64>,
    /// Durable raw-response retention only; neither bytes nor supplier prose are carried.
    pub raw_checkpointed: Option<bool>,
    /// Identifies a supplier send attempt; true proves neither supplier acceptance nor a bill.
    pub supplier_dispatched: Option<bool>,
    pub analysis_verdict: Option<AnalysisVerdict>,
    /// Copied after authority acknowledgement; this observation never moves money.
    pub settlement: Option<crate::execution::DispatchSettlement>,
    pub retry_after_ms: Option<u64>,
    pub stage_outcome: Option<StageOutcome>,
    pub quota_deferral: Option<crate::execution::QuotaDeferralReason>,
    pub preflight_rejection: Option<crate::runtime::PreflightCause>,
}

/// Owned event with a local monotonic duration, never a subtraction of different wall clocks.
#[derive(Clone, Debug)]
pub struct ImageEvent {
    pub kind: EventKind,
    pub context: EventContext,
    pub failure: Option<FailureKind>,
    pub measurements: Measurements,
    occurred_at: SystemTime,
    elapsed: Option<Duration>,
}

impl ImageEvent {
    pub fn new(kind: EventKind, context: EventContext) -> Self {
        Self {
            kind,
            context,
            failure: None,
            measurements: Measurements::default(),
            occurred_at: SystemTime::now(),
            elapsed: None,
        }
    }

    pub fn elapsed(mut self, elapsed: Duration) -> Self {
        self.elapsed = Some(elapsed);
        self
    }

    pub fn failed(mut self, failure: FailureKind) -> Self {
        self.failure = Some(failure);
        self
    }

    fn into_record(self) -> DiagnosticRecord {
        let mut attributes = self
            .context
            .attributes(self.measurements.supplier_dispatched != Some(false));
        self.measurements.append_attributes(&mut attributes);
        if let Some(failure) = self.failure {
            attributes.push(("image.failure".into(), text(failure.key())));
        }
        if let Some(elapsed) = self.elapsed {
            attributes.push(("image.clock_domain".into(), text("native_monotonic")));
            attributes.push((
                "image.duration_ns".into(),
                i64::try_from(elapsed.as_nanos())
                    .map_or_else(|_| text(&elapsed.as_nanos().to_string()), AnyValue::Int),
            ));
        }
        DiagnosticRecord {
            event_name: self.kind.name(),
            input: LogInput {
                timestamp: self.occurred_at,
                observed_timestamp: SystemTime::now(),
                severity: self.kind.level().severity(),
                body: text(self.kind.name()),
                attributes,
                trace: self
                    .context
                    .trace
                    .map(df_observe::DiagnosticTrace::span_context),
            },
        }
    }
}

impl EventContext {
    fn attributes(&self, supplier_link: bool) -> Vec<(String, AnyValue)> {
        let mut attributes = vec![("image.schema".into(), text("df.image/1"))];
        for (name, bytes) in [
            ("df.operation", self.operation.map(|id| *id.as_bytes())),
            ("image.request_id", self.request.map(|id| *id.as_bytes())),
            ("image.job_id", self.job.map(|id| *id.as_bytes())),
            ("image.stage_id", self.stage.map(|id| *id.as_bytes())),
            ("image.attempt_id", self.attempt.map(|id| *id.as_bytes())),
            (
                "image.parent_attempt_id",
                self.parent_attempt.map(|id| *id.as_bytes()),
            ),
            ("df.run_id", self.run.map(|id| *id.as_bytes())),
            ("image.asset_id", self.asset.map(|id| *id.as_bytes())),
            (
                "image.catalog_attempt_id",
                self.catalog_attempt.map(|id| *id.as_bytes()),
            ),
            ("image.delivery_id", self.delivery.map(|id| id.0)),
            (
                "image.browser_producer_id",
                self.browser_producer.map(|id| id.bytes()),
            ),
        ] {
            if let Some(bytes) = bytes {
                attributes.push((name.into(), text(&crate::lower_hex(&bytes))));
            }
        }
        for (name, value) in [
            ("image.backend", self.backend.map(BackendId::key)),
            ("image.stage_kind", self.kind.map(StageKind::key)),
            ("image.purpose", self.purpose.map(Purpose::key)),
            (
                "image.analysis_kind",
                self.analysis_kind.map(AnalysisKind::key),
            ),
        ] {
            if let Some(value) = value {
                attributes.push((name.into(), text(value)));
            }
        }
        if self.kind == Some(StageKind::LocalTransform) {
            attributes.push(("image.processing".into(), text("local")));
            if let Some(stage) = self.stage {
                attributes.push(("image.transform_id".into(), text(&stage.to_hex())));
            }
        } else if supplier_link && let Some(attempt) = self.attempt {
            attributes.push((
                "image.cost_call_id".into(),
                text(&crate::cost::pinned_call_id(attempt.as_bytes())),
            ));
        }
        append_unsigned(
            &mut attributes,
            "image.config_revision",
            self.config_revision,
        );
        append_unsigned(&mut attributes, "image.gate_revision", self.gate_revision);
        attributes
    }
}

impl Measurements {
    fn append_attributes(&self, attributes: &mut Vec<(String, AnyValue)>) {
        if let Some(evidence) = self.usage_evidence {
            attributes.push(("image.usage_evidence".into(), text(evidence.key())));
        }
        if let Some(step) = self.provider_step {
            attributes.push(("image.provider_step".into(), text(step.key())));
        }
        if let Some(verdict) = self.analysis_verdict {
            attributes.push(("image.analysis_verdict".into(), text(verdict.key())));
        }
        if let Some(settlement) = self.settlement {
            attributes.push(("image.settlement".into(), text(settlement.key())));
        }
        if let Some(outcome) = self.stage_outcome {
            attributes.push(("image.stage_outcome".into(), text(outcome.key())));
        }
        if let Some(reason) = self.quota_deferral {
            attributes.push(("image.quota_deferral".into(), text(reason.key())));
        }
        if let Some(reason) = self.preflight_rejection {
            attributes.push(("image.preflight_reason".into(), text(reason.key())));
        }
        for (name, value) in [
            ("image.delivery_identified", self.delivery_identified),
            ("image.admission_refused", self.admission_refused),
            ("image.raw_checkpointed", self.raw_checkpointed),
            ("image.supplier_dispatched", self.supplier_dispatched),
        ] {
            if let Some(value) = value {
                attributes.push((name.into(), AnyValue::Boolean(value)));
            }
        }
        for (name, value) in [
            ("image.queue_depth", self.queue_depth),
            ("image.input_bytes", self.input_bytes),
            ("image.output_bytes", self.output_bytes),
            ("image.width_px", self.width_px.map(u64::from)),
            ("image.height_px", self.height_px.map(u64::from)),
            ("image.provider_status", self.provider_status.map(u64::from)),
            ("image.browser_asset_identity", self.browser_asset_identity),
            ("image.browser_scene_identity", self.browser_scene_identity),
            ("image.browser_at_ms", self.browser_at_ms),
            ("image.retry_after_ms", self.retry_after_ms),
            ("image.transform_steps", self.transform_steps.map(u64::from)),
            (
                "image.algorithm_revision",
                self.algorithm_revision.map(u64::from),
            ),
            (
                "image.usage_revision",
                self.usage_revision.map(|revision| revision.0),
            ),
        ] {
            append_unsigned(attributes, name, value);
        }
    }
}

fn append_unsigned(attributes: &mut Vec<(String, AnyValue)>, name: &str, value: Option<u64>) {
    if let Some(value) = value {
        // OTEL integers are signed; decimal text preserves an unsigned overflow exactly.
        attributes.push((
            name.into(),
            i64::try_from(value).map_or_else(|_| text(&value.to_string()), AnyValue::Int),
        ));
    }
}

fn text(value: &str) -> AnyValue {
    AnyValue::String(value.to_owned().into())
}

/// Enabled levels are captured independently of whether any trace is sampled or installed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ImageLogLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl ImageLogLevel {
    fn severity(self) -> Severity {
        match self {
            Self::Off | Self::Info => Severity::Info,
            Self::Error => Severity::Error,
            Self::Warn => Severity::Warn,
            Self::Debug => Severity::Debug,
            Self::Trace => Severity::Trace,
        }
    }
}

/// A synchronous bounded observation can never refuse image work or trigger another attempt.
pub trait ImageObserver: Send + Sync {
    fn observe(&self, event: ImageEvent);
}

/// Selection and admission health remain readable even during a disabled or failed interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObserverHealth {
    pub level: ImageLogLevel,
    pub filtered: u64,
    pub admitted: u64,
    pub failures: u64,
}

/// One privacy adapter over the host's existing shared capture path; never a second log store.
pub struct SharedImageObserver {
    sink: Arc<dyn DiagnosticSink>,
    level: AtomicU8,
    filtered: AtomicU64,
    admitted: AtomicU64,
    failures: AtomicU64,
    recent: Mutex<RecentEvents>,
}

/// How many enabled events the observer keeps for operator reads, oldest dropped first: room for
/// a long development session, at a few hundred bytes per event.
pub const RECENT_EVENT_CAPACITY: usize = 4_096;

/// A safe, bounded view of one enabled event for operators and agents: names, labels, safe
/// identifiers and measurements. It never carries prompts, paths, credentials or supplier bodies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentImageEvent {
    /// Monotonic per observer; a reader resumes after the last sequence it saw.
    pub sequence: u64,
    pub at_unix_ms: u64,
    pub name: &'static str,
    pub level: ImageLogLevel,
    pub failure: Option<&'static str>,
    pub job: Option<String>,
    pub stage: Option<String>,
    pub attempt: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub output_bytes: Option<u64>,
}

#[derive(Default)]
struct RecentEvents {
    next: u64,
    events: std::collections::VecDeque<RecentImageEvent>,
}

impl SharedImageObserver {
    pub fn new(sink: Arc<dyn DiagnosticSink>) -> Self {
        Self {
            sink,
            level: AtomicU8::new(ImageLogLevel::Info as u8),
            filtered: AtomicU64::new(0),
            admitted: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            recent: Mutex::new(RecentEvents::default()),
        }
    }

    /// The newest `limit` enabled events, oldest first. An empty buffer or a poisoned lock
    /// reads as empty; diagnostics never block or fail a generation.
    pub fn recent(&self, limit: usize) -> Vec<RecentImageEvent> {
        let Ok(recent) = self.recent.lock() else {
            return Vec::new();
        };
        let skip = recent.events.len().saturating_sub(limit);
        recent.events.iter().skip(skip).cloned().collect()
    }

    fn remember(&self, event: &ImageEvent, level: ImageLogLevel) {
        let Ok(mut recent) = self.recent.lock() else {
            return;
        };
        recent.next += 1;
        let at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
            });
        let entry = RecentImageEvent {
            sequence: recent.next,
            at_unix_ms,
            name: event.kind.name(),
            level,
            failure: event.failure.map(FailureKind::key),
            job: event
                .context
                .job
                .map(|job| crate::lower_hex(job.as_bytes())),
            stage: event
                .context
                .stage
                .map(|stage| crate::lower_hex(stage.as_bytes())),
            attempt: event
                .context
                .attempt
                .map(|attempt| crate::lower_hex(attempt.as_bytes())),
            elapsed_ms: event
                .elapsed
                .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)),
            output_bytes: event.measurements.output_bytes,
        };
        if recent.events.len() == RECENT_EVENT_CAPACITY {
            recent.events.pop_front();
        }
        recent.events.push_back(entry);
    }

    /// Changing diagnostics never changes model, execution policy, content or financial state.
    pub fn set_level(&self, level: ImageLogLevel) {
        self.level.store(level as u8, Ordering::Relaxed);
    }

    pub fn health(&self) -> ObserverHealth {
        let level = match self.level.load(Ordering::Relaxed) {
            0 => ImageLogLevel::Off,
            1 => ImageLogLevel::Error,
            2 => ImageLogLevel::Warn,
            3 => ImageLogLevel::Info,
            4 => ImageLogLevel::Debug,
            _ => ImageLogLevel::Trace,
        };
        ObserverHealth {
            level,
            filtered: self.filtered.load(Ordering::Relaxed),
            admitted: self.admitted.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
        }
    }
}

impl ImageObserver for SharedImageObserver {
    fn observe(&self, event: ImageEvent) {
        if event.kind.level() as u8 > self.level.load(Ordering::Relaxed) {
            self.filtered.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.remember(&event, event.kind.level());
        if self.sink.emit(event.into_record()).is_ok() {
            self.admitted.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl crate::runtime::ImageRuntimeTelemetry for SharedImageObserver {
    fn record(
        &self,
        job: &ImageJob,
        stage: &StageView,
        dispatch: Option<DispatchId>,
        phase: crate::runtime::RuntimePhase,
        _at_ms: i64,
    ) -> Result<(), crate::runtime::RuntimeError> {
        use crate::runtime::RuntimePhase;
        let kind = match phase {
            RuntimePhase::Claimed => EventKind::Claimed,
            RuntimePhase::Admitted => EventKind::Admitted,
            RuntimePhase::Sent => EventKind::Sent,
            RuntimePhase::SupplierCheckpointed => EventKind::SupplierCheckpointed,
            RuntimePhase::OutputStored => EventKind::OutputStored,
            RuntimePhase::Published => EventKind::Published,
            RuntimePhase::Unknown => EventKind::Unknown,
        };
        ImageObserver::observe(
            self,
            ImageEvent::new(kind, EventContext::for_stage(job, stage, dispatch)),
        );
        Ok(())
    }

    fn observe(&self, event: ImageEvent) {
        ImageObserver::observe(self, event);
    }
}

impl crate::runtime::ImageRuntimeTelemetry for Arc<SharedImageObserver> {
    fn record(
        &self,
        job: &ImageJob,
        stage: &StageView,
        dispatch: Option<DispatchId>,
        phase: crate::runtime::RuntimePhase,
        at_ms: i64,
    ) -> Result<(), crate::runtime::RuntimeError> {
        crate::runtime::ImageRuntimeTelemetry::record(
            self.as_ref(),
            job,
            stage,
            dispatch,
            phase,
            at_ms,
        )
    }

    fn observe(&self, event: ImageEvent) {
        ImageObserver::observe(self.as_ref(), event);
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
