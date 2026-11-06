use crate::{
    CaptureKey, PendingReceipt, ProducerId, RecordId, Signal, SourceSequence, TelemetryError,
    TelemetryIngress, TelemetryLimits,
    capture::{
        BUILD_REVISION_ATTRIBUTES, PRODUCER_ID_ATTRIBUTE, RECORD_ID_ATTRIBUTE, RESOURCE_SCHEMA_URL,
        SCOPE_SCHEMA_URL, SCOPE_VERSION, SOURCE_SEQUENCE_ATTRIBUTE, is_capture_identity_attribute,
    },
    limits::{
        MAXIMUM_ATTRIBUTES, MAXIMUM_INPUT_NODES, MAXIMUM_NAME_BYTES, MAXIMUM_SPAN_EVENTS,
        MAXIMUM_SPAN_LINKS, MAXIMUM_SPAN_NAME_BYTES, MAXIMUM_VALUE_DEPTH, MAXIMUM_VALUE_ELEMENTS,
    },
};
use df_types::BuildIdentity;
use opentelemetry::{
    Context, InstrumentationScope, KeyValue,
    logs::{LogRecord, Logger, LoggerProvider},
    trace::{Span, Tracer, TracerProvider},
};
use opentelemetry_proto::{
    tonic::{
        collector::{logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest},
        logs::v1::ResourceLogs,
        trace::v1::ResourceSpans,
    },
    transform::common::tonic::ResourceAttributesWithSchema,
};
use opentelemetry_sdk::{
    Resource,
    error::{OTelSdkError, OTelSdkResult},
    logs::{LogBatch, LogExporter, SdkLoggerProvider},
    trace::{Sampler, SdkTracerProvider, SpanData, SpanExporter},
};
use prost::Message;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

pub use opentelemetry::logs::{AnyValue, Severity};
pub use opentelemetry_sdk::trace::SpanData as NativeSpanData;

/// Budget charge for a link's trace and span identity.
const LINK_IDENTITY_BYTES: usize = 64;
/// Budget charge for one scalar value.
const SCALAR_BYTES: usize = 8;
/// Event name of a log captured without a subsystem-specific name.
const DEFAULT_LOG_EVENT_NAME: &str = "native.capture";
/// Service name of a producer built for synthetic fixtures.
const SYNTHETIC_SERVICE_NAME: &str = "df-telemetry.synthetic-native";
/// Instrumentation scope of every record a producer captures.
const PRODUCER_SCOPE: &str = "df-observe.native";
/// Safe synthetic/native capture input. The owner supplies explicit timestamps and correlation.
pub struct LogInput {
    pub timestamp: SystemTime,
    pub observed_timestamp: SystemTime,
    pub severity: Severity,
    pub body: AnyValue,
    pub attributes: Vec<(String, AnyValue)>,
    pub trace: Option<opentelemetry::trace::SpanContext>,
}

impl std::fmt::Debug for LogInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LogInput")
            .field("timestamp", &self.timestamp)
            .field("observed_timestamp", &self.observed_timestamp)
            .field("severity", &self.severity)
            .field("body", &self.body)
            .field("attributes", &self.attributes)
            .field(
                "trace",
                &self.trace.as_ref().map(crate::DiagnosticTrace::from),
            )
            .finish()
    }
}
/// Safe synthetic/native span input with explicit times and parent context.
#[derive(Debug)]
pub struct SpanInput {
    pub name: String,
    pub start: SystemTime,
    pub end: SystemTime,
    pub parent: Context,
    pub links: Vec<opentelemetry::trace::Link>,
    pub attributes: Vec<KeyValue>,
}
/// Retry exactly these standard OTLP bytes; this identity never changes on a lost receipt.
#[derive(Debug)]
pub struct CapturedBatch {
    pub signal: Signal,
    pub bytes: Vec<u8>,
    pub pending: PendingReceipt,
}
type Submission = Arc<Mutex<Option<Result<CapturedBatch, TelemetryError>>>>;
#[derive(Debug)]
struct NativeExporter {
    ingress: Arc<dyn TelemetryIngress>,
    resource: ResourceAttributesWithSchema,
    result: Submission,
    limits: TelemetryLimits,
}
impl NativeExporter {
    fn submit(&self, signal: Signal, bytes: Vec<u8>) -> OTelSdkResult {
        let result = if bytes.len() > self.limits.batch_bytes {
            Err(TelemetryError::Oversized)
        } else {
            self.ingress
                .submit(signal, &bytes)
                .map(|pending| CapturedBatch {
                    signal,
                    bytes,
                    pending,
                })
        };
        let failed = result.is_err();
        let mut slot = self
            .result
            .lock()
            .map_err(|_| OTelSdkError::InternalFailure("native capture slot poisoned".into()))?;
        *slot = Some(result);
        if failed {
            Err(OTelSdkError::InternalFailure(
                "native bounded admission refused".into(),
            ))
        } else {
            Ok(())
        }
    }
}
impl LogExporter for NativeExporter {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        // Single-resource conversion preserves instrumentation scope schema (the upstream
        // grouping helper instead uses resource schema for logs in SDK 0.31).
        let resource_logs = batch
            .iter()
            .map(|pair| ResourceLogs::from((pair, &self.resource)))
            .collect();
        self.submit(
            Signal::Logs,
            ExportLogsServiceRequest { resource_logs }.encode_to_vec(),
        )
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.resource = ordered_resource(resource);
    }
}
impl SpanExporter for NativeExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let resource_spans = batch
            .into_iter()
            .map(|span| ResourceSpans::new(span, &self.resource))
            .collect();
        self.submit(
            Signal::Spans,
            ExportTraceServiceRequest { resource_spans }.encode_to_vec(),
        )
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.resource = ordered_resource(resource);
    }
}
/// Owner of actual SDK providers and sequential native capture. SDK callbacks do bounded
/// encoding/queue admission only; the independent ingress worker owns all disk work.
pub struct NativeProducer {
    source: ProducerId,
    next: Option<i64>,
    logs: SdkLoggerProvider,
    traces: SdkTracerProvider,
    result: Submission,
    scope: InstrumentationScope,
    limits: TelemetryLimits,
    completed_spans: NativeExporter,
}

/// Owns the process-wide OTEL provider; its sink must perform bounded, nonblocking admission.
pub struct NativeTraceBridge(SdkTracerProvider);

#[derive(Debug)]
struct ForwardSpans(fn(NativeSpanData));

impl SpanExporter for ForwardSpans {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        for span in batch {
            (self.0)(span);
        }
        Ok(())
    }
}

impl NativeTraceBridge {
    /// Install before serving. Application logs have their own unsampled audit bridge.
    pub fn install(sink: fn(NativeSpanData)) -> Self {
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_simple_exporter(ForwardSpans(sink))
            .build();
        opentelemetry::global::set_tracer_provider(provider.clone());
        Self(provider)
    }

    pub fn shutdown(self, timeout: Duration) -> Result<(), TelemetryError> {
        self.0
            .shutdown_with_timeout(timeout)
            .map_err(|_| TelemetryError::Internal)
    }
}

impl NativeProducer {
    pub fn new(
        source: ProducerId,
        first_sequence: SourceSequence,
        build: &BuildIdentity,
        ingress: Arc<dyn TelemetryIngress>,
        limits: TelemetryLimits,
    ) -> Result<Self, TelemetryError> {
        Self::build(source, first_sequence, build, ingress, limits, None)
    }
    /// Production composition identity; capture and durability use the same bounded SDK path.
    pub fn for_service(
        source: ProducerId,
        first_sequence: SourceSequence,
        build: &BuildIdentity,
        ingress: Arc<dyn TelemetryIngress>,
        limits: TelemetryLimits,
        service: &'static str,
    ) -> Result<Self, TelemetryError> {
        if service.is_empty() || service.len() > MAXIMUM_NAME_BYTES {
            return Err(TelemetryError::InvalidIdentity);
        }
        Self::build(
            source,
            first_sequence,
            build,
            ingress,
            limits,
            Some(service),
        )
    }
    fn build(
        source: ProducerId,
        first_sequence: SourceSequence,
        build: &BuildIdentity,
        ingress: Arc<dyn TelemetryIngress>,
        limits: TelemetryLimits,
        service: Option<&'static str>,
    ) -> Result<Self, TelemetryError> {
        let limits = limits.validate()?;
        let result = Arc::new(Mutex::new(None));
        let service_name = KeyValue::new("service.name", service.unwrap_or(SYNTHETIC_SERVICE_NAME));
        let revisions = BUILD_REVISION_ATTRIBUTES
            .into_iter()
            .map(|(name, revision)| {
                KeyValue::new(name, build.revision(revision).as_str().to_owned())
            });
        let resource = Resource::builder_empty()
            .with_attributes(std::iter::once(service_name).chain(revisions))
            .with_schema_url([], RESOURCE_SCHEMA_URL)
            .build();
        let exporter = || NativeExporter {
            ingress: Arc::clone(&ingress),
            resource: ordered_resource(&resource),
            result: Arc::clone(&result),
            limits,
        };
        let logs = SdkLoggerProvider::builder()
            .with_resource(resource.clone())
            .with_simple_exporter(exporter())
            .build();
        let span_exporter = exporter();
        let completed_spans = exporter();
        let traces = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_resource(resource)
            .with_simple_exporter(span_exporter)
            .build();
        let scope = InstrumentationScope::builder(PRODUCER_SCOPE)
            .with_version(SCOPE_VERSION)
            .with_schema_url(SCOPE_SCHEMA_URL)
            .with_attributes([KeyValue::new("fixture.synthetic", service.is_none())])
            .build();
        Ok(Self {
            source,
            next: Some(first_sequence.get()),
            logs,
            traces,
            result,
            scope,
            limits,
            completed_spans,
        })
    }
    /// Reserve identity before dispatch. Failed capture still consumes sequence and creates a
    /// possible source gap. Caller never restarts a source lifecycle at an already used sequence.
    pub fn reserve(&mut self, record: RecordId) -> Result<CaptureKey, TelemetryError> {
        let sequence = self.next.ok_or(TelemetryError::SequenceExhausted)?;
        self.next = sequence.checked_add(1);
        Ok(CaptureKey {
            producer: self.source,
            record,
            sequence: SourceSequence::new(sequence)?,
        })
    }
    fn take_submission(&self) -> Result<CapturedBatch, TelemetryError> {
        self.result
            .lock()
            .map_err(|_| TelemetryError::Internal)?
            .take()
            .ok_or(TelemetryError::Internal)?
    }
    pub fn emit_log(
        &mut self,
        key: CaptureKey,
        input: LogInput,
    ) -> Result<CapturedBatch, TelemetryError> {
        self.emit_named_log(key, DEFAULT_LOG_EVENT_NAME, input)
    }
    /// Preserve the emitting subsystem's event name through the shared SDK capture path.
    pub fn emit_named_log(
        &mut self,
        key: CaptureKey,
        event_name: &'static str,
        input: LogInput,
    ) -> Result<CapturedBatch, TelemetryError> {
        if event_name.is_empty()
            || event_name.len() > MAXIMUM_NAME_BYTES
            || !event_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(TelemetryError::InvalidIdentity);
        }
        if key.producer != self.source || input.attributes.len() > MAXIMUM_ATTRIBUTES {
            return Err(TelemetryError::InvalidIdentity);
        }
        let mut budget = InputBudget::new(self.limits.record_bytes);
        budget.log_value(&input.body, 0)?;
        for (name, value) in &input.attributes {
            // Logs also refuse any name that merely starts with the producer attribute.
            if name.starts_with(PRODUCER_ID_ATTRIBUTE) || is_capture_identity_attribute(name) {
                return Err(TelemetryError::InvalidIdentity);
            }
            budget.node(name.len())?;
            budget.log_value(value, 0)?;
        }
        validate_time(input.timestamp)?;
        validate_time(input.observed_timestamp)?;
        let logger = self.logs.logger_with_scope(self.scope.clone());
        let mut record = logger.create_log_record();
        record.set_timestamp(input.timestamp);
        record.set_observed_timestamp(input.observed_timestamp);
        record.set_severity_number(input.severity);
        record.set_severity_text(input.severity.name());
        record.set_event_name(event_name);
        record.set_body(input.body);
        record.add_attributes(input.attributes);
        record.add_attribute(PRODUCER_ID_ATTRIBUTE, key.producer.hex());
        record.add_attribute(RECORD_ID_ATTRIBUTE, key.record.hex());
        record.add_attribute(SOURCE_SEQUENCE_ATTRIBUTE, key.sequence.get());
        if let Some(trace) = input.trace {
            record.set_trace_context(trace.trace_id(), trace.span_id(), Some(trace.trace_flags()));
        }
        logger.emit(record);
        self.take_submission()
    }
    pub fn emit_span(
        &mut self,
        key: CaptureKey,
        input: SpanInput,
    ) -> Result<CapturedBatch, TelemetryError> {
        self.emit_span_with_id(key, input, None)
    }
    /// Capture a completed SDK span without reconstructing away events, status, kind or scope.
    /// The composition sanitizes payloads first; this shared boundary bounds them before encoding.
    pub fn emit_finished_span(
        &mut self,
        key: CaptureKey,
        mut span: NativeSpanData,
    ) -> Result<CapturedBatch, TelemetryError> {
        if key.producer != self.source
            || !span.span_context.is_valid()
            || span.attributes.len() > MAXIMUM_ATTRIBUTES
            || span.events.events.len() > MAXIMUM_SPAN_EVENTS
            || span.links.links.len() > MAXIMUM_SPAN_LINKS
        {
            return Err(TelemetryError::Oversized);
        }
        validate_time(span.start_time)?;
        validate_time(span.end_time)?;
        if span.end_time < span.start_time {
            return Err(TelemetryError::Malformed);
        }
        let mut budget = InputBudget::new(self.limits.record_bytes);
        budget.node(span.name.len())?;
        budget.node(span.instrumentation_scope.name().len())?;
        budget.node(span.instrumentation_scope.version().map_or(0, str::len))?;
        budget.node(span.instrumentation_scope.schema_url().map_or(0, str::len))?;
        for attribute in span.instrumentation_scope.attributes() {
            budget.node(attribute.key.as_str().len())?;
            budget.span_value(&attribute.value)?;
        }
        if let opentelemetry::trace::Status::Error { description } = &span.status {
            budget.node(description.len())?;
        }
        for attribute in &span.attributes {
            if is_capture_identity_attribute(attribute.key.as_str()) {
                return Err(TelemetryError::InvalidIdentity);
            }
            budget.node(attribute.key.as_str().len())?;
            budget.span_value(&attribute.value)?;
        }
        for event in &span.events.events {
            validate_time(event.timestamp)?;
            budget.node(event.name.len())?;
            for attribute in &event.attributes {
                budget.node(attribute.key.as_str().len())?;
                budget.span_value(&attribute.value)?;
            }
        }
        for link in &span.links.links {
            budget.node(LINK_IDENTITY_BYTES)?;
            for attribute in &link.attributes {
                budget.node(attribute.key.as_str().len())?;
                budget.span_value(&attribute.value)?;
            }
        }
        span.attributes.extend(capture_identity_attributes(key));
        let resource_spans = vec![ResourceSpans::new(span, &self.completed_spans.resource)];
        // The exact admission result, including refusal, is retained in the shared capture slot
        // and returned by `take_submission`; this SDK-shaped copy carries nothing more.
        let _sdk_outcome = self.completed_spans.submit(
            Signal::Spans,
            ExportTraceServiceRequest { resource_spans }.encode_to_vec(),
        );
        self.take_submission()
    }
    /// Preserve an already dispatched span identity in correlated logs and exported span data.
    pub fn emit_correlated_span(
        &mut self,
        key: CaptureKey,
        input: SpanInput,
        span: opentelemetry::trace::SpanContext,
    ) -> Result<CapturedBatch, TelemetryError> {
        if !span.is_valid() {
            return Err(TelemetryError::InvalidIdentity);
        }
        self.emit_span_with_id(key, input, Some(span))
    }
    fn emit_span_with_id(
        &mut self,
        key: CaptureKey,
        mut input: SpanInput,
        span: Option<opentelemetry::trace::SpanContext>,
    ) -> Result<CapturedBatch, TelemetryError> {
        if key.producer != self.source
            || input.attributes.len() > MAXIMUM_ATTRIBUTES
            || input.links.len() > MAXIMUM_SPAN_LINKS
            || input.name.len() > MAXIMUM_SPAN_NAME_BYTES
        {
            return Err(TelemetryError::Oversized);
        }
        if input
            .attributes
            .iter()
            .any(|attribute| is_capture_identity_attribute(attribute.key.as_str()))
        {
            return Err(TelemetryError::InvalidIdentity);
        }
        let mut budget = InputBudget::new(self.limits.record_bytes);
        budget.node(input.name.len())?;
        for attribute in &input.attributes {
            budget.node(attribute.key.as_str().len())?;
            budget.span_value(&attribute.value)?;
        }
        for link in &input.links {
            if link.attributes.len() > MAXIMUM_ATTRIBUTES {
                return Err(TelemetryError::Oversized);
            }
            budget.node(LINK_IDENTITY_BYTES)?;
            for attribute in &link.attributes {
                budget.node(attribute.key.as_str().len())?;
                budget.span_value(&attribute.value)?;
            }
        }
        validate_time(input.start)?;
        validate_time(input.end)?;
        if input.end < input.start {
            return Err(TelemetryError::Malformed);
        }
        input.attributes.extend(capture_identity_attributes(key));
        let tracer = self.traces.tracer_with_scope(self.scope.clone());
        let mut builder = tracer
            .span_builder(input.name)
            .with_start_time(input.start)
            .with_attributes(input.attributes)
            .with_links(input.links);
        if let Some(span) = span {
            builder.span_id = Some(span.span_id());
            builder.trace_id = Some(span.trace_id());
        }
        let mut span = tracer.build_with_context(builder, &input.parent);
        span.end_with_timestamp(input.end);
        self.take_submission()
    }
    pub fn shutdown(self, timeout: Duration) -> Result<(), TelemetryError> {
        let started = std::time::Instant::now();
        self.logs
            .shutdown_with_timeout(timeout)
            .map_err(|_| TelemetryError::Internal)?;
        self.traces
            .shutdown_with_timeout(timeout.saturating_sub(started.elapsed()))
            .map_err(|_| TelemetryError::Internal)
    }
}
/// The capture identity attributes the shared path appends to every span.
fn capture_identity_attributes(key: CaptureKey) -> [KeyValue; 3] {
    [
        KeyValue::new(PRODUCER_ID_ATTRIBUTE, key.producer.hex()),
        KeyValue::new(RECORD_ID_ATTRIBUTE, key.record.hex()),
        KeyValue::new(SOURCE_SEQUENCE_ATTRIBUTE, key.sequence.get()),
    ]
}
fn validate_time(time: SystemTime) -> Result<(), TelemetryError> {
    let nanos = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| TelemetryError::Malformed)?
        .as_nanos();
    if nanos > u128::from(u64::MAX) {
        return Err(TelemetryError::Oversized);
    }
    Ok(())
}
// One budget spans the complete caller input, before SDK conversion or allocation.
// Nonzero structural charges bound empty values too; the node cap bounds traversal.
const STRUCTURAL_BYTES: usize = 16;
struct InputBudget {
    bytes_left: usize,
    nodes_left: usize,
}
impl InputBudget {
    fn new(bytes: usize) -> Self {
        Self {
            bytes_left: bytes,
            nodes_left: MAXIMUM_INPUT_NODES,
        }
    }
    fn node(&mut self, payload_bytes: usize) -> Result<(), TelemetryError> {
        let bytes = payload_bytes
            .checked_add(STRUCTURAL_BYTES)
            .ok_or(TelemetryError::Oversized)?;
        self.bytes_left = self
            .bytes_left
            .checked_sub(bytes)
            .ok_or(TelemetryError::Oversized)?;
        self.nodes_left = self
            .nodes_left
            .checked_sub(1)
            .ok_or(TelemetryError::Oversized)?;
        Ok(())
    }
    fn log_value(&mut self, value: &AnyValue, depth: usize) -> Result<(), TelemetryError> {
        if depth > MAXIMUM_VALUE_DEPTH {
            return Err(TelemetryError::Oversized);
        }
        match value {
            AnyValue::String(value) => self.node(value.as_str().len()),
            AnyValue::Bytes(value) => self.node(value.len()),
            AnyValue::Int(_) | AnyValue::Double(_) | AnyValue::Boolean(_) => {
                self.node(SCALAR_BYTES)
            }
            AnyValue::ListAny(values) => {
                self.node(0)?;
                if values.len() > MAXIMUM_VALUE_ELEMENTS {
                    return Err(TelemetryError::Oversized);
                }
                for value in values.iter() {
                    self.log_value(value, depth + 1)?;
                }
                Ok(())
            }
            AnyValue::Map(values) => {
                self.node(0)?;
                if values.len() > MAXIMUM_VALUE_ELEMENTS {
                    return Err(TelemetryError::Oversized);
                }
                for (name, value) in values.iter() {
                    self.node(name.as_str().len())?;
                    self.log_value(value, depth + 1)?;
                }
                Ok(())
            }
            _ => Err(TelemetryError::Malformed),
        }
    }
    fn span_value(&mut self, value: &opentelemetry::Value) -> Result<(), TelemetryError> {
        use opentelemetry::{Array, Value};
        match value {
            Value::Bool(_) | Value::I64(_) | Value::F64(_) => self.node(SCALAR_BYTES),
            Value::String(value) => self.node(value.as_str().len()),
            Value::Array(array) => {
                self.node(0)?;
                match array {
                    Array::Bool(values) => self.scalar_array(values.len(), 1),
                    Array::I64(values) => self.scalar_array(values.len(), SCALAR_BYTES),
                    Array::F64(values) => self.scalar_array(values.len(), SCALAR_BYTES),
                    Array::String(values) => {
                        if values.len() > MAXIMUM_VALUE_ELEMENTS {
                            return Err(TelemetryError::Oversized);
                        }
                        for value in values {
                            self.node(value.as_str().len())?;
                        }
                        Ok(())
                    }
                    _ => Err(TelemetryError::Malformed),
                }
            }
            _ => Err(TelemetryError::Malformed),
        }
    }
    fn scalar_array(&mut self, length: usize, element_bytes: usize) -> Result<(), TelemetryError> {
        if length > MAXIMUM_VALUE_ELEMENTS {
            return Err(TelemetryError::Oversized);
        }
        for _ in 0..length {
            self.node(element_bytes)?;
        }
        Ok(())
    }
}

fn ordered_resource(resource: &Resource) -> ResourceAttributesWithSchema {
    let mut converted: ResourceAttributesWithSchema = resource.into();
    // SDK Resource uses a hash map. Stable encoding prevents fresh SDK construction
    // from changing bytes solely because hash iteration order differs after restart.
    converted
        .attributes
        .0
        .sort_by(|left, right| left.key.cmp(&right.key));
    converted
}

#[cfg(test)]
mod producer_tests;
