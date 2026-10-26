use crate::{
    CaptureKey, PendingReceipt, ProducerId, RecordId, Signal, SourceSequence, TelemetryError,
    TelemetryIngress, TelemetryLimits,
};
use df_types::{BuildIdentity, BuildRevision};
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
/// Safe synthetic/native capture input. The owner supplies explicit timestamps and correlation.
#[derive(Debug)]
pub struct LogInput {
    pub timestamp: SystemTime,
    pub observed_timestamp: SystemTime,
    pub severity: Severity,
    pub body: AnyValue,
    pub attributes: Vec<(String, AnyValue)>,
    pub trace: Option<opentelemetry::trace::SpanContext>,
}
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
        if service.is_empty() || service.len() > 128 {
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
        let resource = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(
                    "service.name",
                    service.unwrap_or("df-telemetry.synthetic-native"),
                ),
                KeyValue::new(
                    "df.build",
                    build.revision(BuildRevision::Source).as_str().to_owned(),
                ),
                KeyValue::new(
                    "df.native",
                    build.revision(BuildRevision::Native).as_str().to_owned(),
                ),
                KeyValue::new(
                    "df.wasm",
                    build.revision(BuildRevision::Wasm).as_str().to_owned(),
                ),
                KeyValue::new(
                    "df.configuration",
                    build
                        .revision(BuildRevision::Configuration)
                        .as_str()
                        .to_owned(),
                ),
                KeyValue::new(
                    "df.content",
                    build.revision(BuildRevision::Content).as_str().to_owned(),
                ),
            ])
            .with_schema_url([], "https://dungeonflux.local/telemetry/native-v1")
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
        let scope = InstrumentationScope::builder("df-observe.native")
            .with_version("0.1.0")
            .with_schema_url("https://dungeonflux.local/telemetry/scope-v1")
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
        if key.producer != self.source || input.attributes.len() > 64 {
            return Err(TelemetryError::InvalidIdentity);
        }
        let mut budget = InputBudget::new(self.limits.record_bytes);
        budget.log_value(&input.body, 0)?;
        for (name, value) in &input.attributes {
            if name.starts_with("df.producer_id")
                || name == "df.record_id"
                || name == "df.source_sequence"
            {
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
        record.set_event_name("native.capture");
        record.set_body(input.body);
        record.add_attributes(input.attributes);
        record.add_attribute("df.producer_id", key.producer.hex());
        record.add_attribute("df.record_id", key.record.hex());
        record.add_attribute("df.source_sequence", key.sequence.get());
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
            || span.attributes.len() > 64
            || span.events.events.len() > 128
            || span.links.links.len() > 16
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
            if matches!(
                attribute.key.as_str(),
                "df.producer_id" | "df.record_id" | "df.source_sequence"
            ) {
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
            budget.node(64)?;
            for attribute in &link.attributes {
                budget.node(attribute.key.as_str().len())?;
                budget.span_value(&attribute.value)?;
            }
        }
        span.attributes.extend([
            KeyValue::new("df.producer_id", key.producer.hex()),
            KeyValue::new("df.record_id", key.record.hex()),
            KeyValue::new("df.source_sequence", key.sequence.get()),
        ]);
        let resource_spans = vec![ResourceSpans::new(span, &self.completed_spans.resource)];
        // The exact admission result is retained in the shared capture slot, including refusal.
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
            || input.attributes.len() > 64
            || input.links.len() > 16
            || input.name.len() > 256
        {
            return Err(TelemetryError::Oversized);
        }
        if input.attributes.iter().any(|attribute| {
            matches!(
                attribute.key.as_str(),
                "df.producer_id" | "df.record_id" | "df.source_sequence"
            )
        }) {
            return Err(TelemetryError::InvalidIdentity);
        }
        let mut budget = InputBudget::new(self.limits.record_bytes);
        budget.node(input.name.len())?;
        for attribute in &input.attributes {
            budget.node(attribute.key.as_str().len())?;
            budget.span_value(&attribute.value)?;
        }
        for link in &input.links {
            if link.attributes.len() > 64 {
                return Err(TelemetryError::Oversized);
            }
            budget.node(64)?;
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
        input.attributes.extend([
            KeyValue::new("df.producer_id", key.producer.hex()),
            KeyValue::new("df.record_id", key.record.hex()),
            KeyValue::new("df.source_sequence", key.sequence.get()),
        ]);
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
        self.logs
            .shutdown_with_timeout(timeout)
            .map_err(|_| TelemetryError::Internal)?;
        self.traces
            .shutdown_with_timeout(timeout)
            .map_err(|_| TelemetryError::Internal)
    }
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
const MAXIMUM_INPUT_NODES: usize = 512;
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
        if depth > 8 {
            return Err(TelemetryError::Oversized);
        }
        match value {
            AnyValue::String(value) => self.node(value.as_str().len()),
            AnyValue::Bytes(value) => self.node(value.len()),
            AnyValue::Int(_) | AnyValue::Double(_) | AnyValue::Boolean(_) => self.node(8),
            AnyValue::ListAny(values) => {
                self.node(0)?;
                if values.len() > 64 {
                    return Err(TelemetryError::Oversized);
                }
                for value in values.iter() {
                    self.log_value(value, depth + 1)?;
                }
                Ok(())
            }
            AnyValue::Map(values) => {
                self.node(0)?;
                if values.len() > 64 {
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
            Value::Bool(_) | Value::I64(_) | Value::F64(_) => self.node(8),
            Value::String(value) => self.node(value.as_str().len()),
            Value::Array(array) => {
                self.node(0)?;
                match array {
                    Array::Bool(values) => self.scalar_array(values.len(), 1),
                    Array::I64(values) => self.scalar_array(values.len(), 8),
                    Array::F64(values) => self.scalar_array(values.len(), 8),
                    Array::String(values) => {
                        if values.len() > 64 {
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
        if length > 64 {
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
mod tests {
    use super::*;

    static COMPLETED_SPAN: Mutex<Option<NativeSpanData>> = Mutex::new(None);
    fn capture_completed_span(span: NativeSpanData) {
        *COMPLETED_SPAN.lock().unwrap() = Some(span);
    }

    #[test]
    fn completed_sdk_span_keeps_events_status_kind_scope_and_enforces_input_budget() {
        use opentelemetry::trace::{SpanKind, Status};
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(ForwardSpans(capture_completed_span))
            .build();
        let tracer = provider.tracer("test.original-scope");
        let mut span = tracer.build_with_context(
            tracer
                .span_builder("test.completed")
                .with_kind(SpanKind::Client),
            &Context::new(),
        );
        span.add_event("test.event", vec![KeyValue::new("count", 7i64)]);
        span.set_status(Status::error("test-error-class"));
        span.end();
        let completed = COMPLETED_SPAN.lock().unwrap().take().unwrap();
        let context = completed.span_context.clone();
        let event_time = completed.events.events[0].timestamp;
        let build = BuildIdentity::new(
            Some("build"),
            Some("native"),
            Some("none"),
            Some("config"),
            Some("none"),
        )
        .unwrap();
        let mut producer = NativeProducer::for_service(
            ProducerId::new([1; 16]).unwrap(),
            SourceSequence::new(1).unwrap(),
            &build,
            Arc::new(Accepted),
            TelemetryLimits::default(),
            "test.service",
        )
        .unwrap();
        let mut oversized = completed.clone();
        oversized.events.events[0].name = "x".repeat(8192).into();
        let key = producer.reserve(RecordId::new([2; 16]).unwrap()).unwrap();
        assert!(matches!(
            producer.emit_finished_span(key, oversized),
            Err(TelemetryError::Oversized)
        ));
        let key = producer.reserve(RecordId::new([3; 16]).unwrap()).unwrap();
        let captured = producer.emit_finished_span(key, completed).unwrap();
        captured.pending.wait(Duration::from_secs(1)).unwrap();
        let request = ExportTraceServiceRequest::decode(captured.bytes.as_slice()).unwrap();
        let scope = &request.resource_spans[0].scope_spans[0];
        assert_eq!(scope.scope.as_ref().unwrap().name, "test.original-scope");
        let span = &scope.spans[0];
        assert_eq!(span.trace_id, context.trace_id().to_bytes());
        assert_eq!(span.span_id, context.span_id().to_bytes());
        assert_eq!(span.kind, 3);
        assert_eq!(span.status.as_ref().unwrap().code, 2);
        assert_eq!(span.status.as_ref().unwrap().message, "test-error-class");
        assert_eq!(span.events.len(), 1);
        assert_eq!(span.events[0].name, "test.event");
        assert_eq!(
            u128::from(span.events[0].time_unix_nano),
            event_time
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        producer.shutdown(Duration::from_secs(1)).unwrap();
        provider.shutdown().unwrap();
    }

    #[derive(Debug)]
    struct Accepted;
    impl TelemetryIngress for Accepted {
        fn submit(&self, _: Signal, _: &[u8]) -> Result<PendingReceipt, TelemetryError> {
            let (sender, receipt) = PendingReceipt::channel();
            sender
                .send(Ok(crate::IngestReceipt {
                    durability: crate::Durability::Committed,
                    spool_position: 1,
                    committed_watermark: 1,
                    records: 1,
                }))
                .unwrap();
            Ok(receipt)
        }
    }

    #[test]
    fn service_capture_preserves_severity_resource_and_dispatched_span_identity() {
        use opentelemetry::trace::{
            SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
        };
        use opentelemetry_proto::tonic::common::v1::any_value;
        let build = BuildIdentity::new(
            Some("build"),
            Some("native"),
            Some("none"),
            Some("config"),
            Some("none"),
        )
        .unwrap();
        let mut producer = NativeProducer::for_service(
            ProducerId::new([1; 16]).unwrap(),
            SourceSequence::new(1).unwrap(),
            &build,
            Arc::new(Accepted),
            TelemetryLimits::default(),
            "df-portal-hosted",
        )
        .unwrap();
        let span = SpanContext::new(
            TraceId::from_bytes([3; 16]),
            SpanId::from_bytes([4; 8]),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        );
        let key = producer.reserve(RecordId::new([2; 16]).unwrap()).unwrap();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let captured = producer
            .emit_log(
                key,
                LogInput {
                    timestamp: now,
                    observed_timestamp: now,
                    severity: Severity::Error,
                    body: AnyValue::String("failed".into()),
                    attributes: vec![("count".to_owned(), AnyValue::Int(1))],
                    trace: Some(span.clone()),
                },
            )
            .unwrap();
        captured.pending.wait(Duration::from_secs(1)).unwrap();
        let request = ExportLogsServiceRequest::decode(captured.bytes.as_slice()).unwrap();
        let resource = &request.resource_logs[0];
        assert!(resource.resource.as_ref().unwrap().attributes.iter().any(
            |attribute| attribute.key == "service.name"
                && attribute.value.as_ref().unwrap().value
                    == Some(any_value::Value::StringValue("df-portal-hosted".to_owned()))
        ));
        let scope = &resource.scope_logs[0];
        assert!(
            scope
                .scope
                .as_ref()
                .unwrap()
                .attributes
                .iter()
                .any(|attribute| attribute.key == "fixture.synthetic"
                    && attribute.value.as_ref().unwrap().value
                        == Some(any_value::Value::BoolValue(false)))
        );
        let record = &scope.log_records[0];
        assert_eq!(record.severity_number, 17);
        assert_eq!(record.severity_text, "ERROR");
        assert_eq!(record.trace_id, [3; 16]);
        assert_eq!(record.span_id, [4; 8]);
        assert!(
            record
                .attributes
                .iter()
                .any(|attribute| attribute.key == "count"
                    && attribute.value.as_ref().unwrap().value
                        == Some(any_value::Value::IntValue(1)))
        );
        let key = producer.reserve(RecordId::new([5; 16]).unwrap()).unwrap();
        let parent = SpanContext::new(
            span.trace_id(),
            SpanId::from_bytes([6; 8]),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        );
        let captured = producer
            .emit_correlated_span(
                key,
                SpanInput {
                    name: "request".to_owned(),
                    start: now,
                    end: now + Duration::from_secs(1),
                    parent: Context::new().with_remote_span_context(parent),
                    links: Vec::new(),
                    attributes: Vec::new(),
                },
                span,
            )
            .unwrap();
        captured.pending.wait(Duration::from_secs(1)).unwrap();
        let request = ExportTraceServiceRequest::decode(captured.bytes.as_slice()).unwrap();
        let record = &request.resource_spans[0].scope_spans[0].spans[0];
        assert_eq!(record.trace_id, [3; 16]);
        assert_eq!(record.span_id, [4; 8]);
        assert_eq!(record.parent_span_id, [6; 8]);
        producer.shutdown(Duration::from_secs(1)).unwrap();
    }

    fn tree(depth: usize) -> AnyValue {
        if depth == 0 {
            AnyValue::String("".into())
        } else {
            AnyValue::ListAny(Box::new((0..16).map(|_| tree(depth - 1)).collect()))
        }
    }

    #[test]
    fn empty_tree_preflight_exhausts_one_shared_budget_before_sdk_work() {
        let mut budget = InputBudget::new(8192);
        budget.log_value(&tree(2), 0).unwrap();
        assert_eq!(budget.nodes_left, MAXIMUM_INPUT_NODES - 273);
        assert!(matches!(
            budget.log_value(&tree(2), 0),
            Err(TelemetryError::Oversized)
        ));
        // Successful charges can never exceed 512. Each rejected traversal stops
        // at its first failed charge; no subsequent sibling is visited.
        assert_eq!(budget.nodes_left, 0);
        for depth in [3, 4] {
            let mut budget = InputBudget::new(8192);
            assert_eq!(
                budget.log_value(&tree(depth), 0),
                Err(TelemetryError::Oversized)
            );
            assert_eq!(budget.nodes_left, 0);
        }
    }

    #[test]
    fn empty_maps_and_span_arrays_charge_structure_across_attributes_and_links() {
        let mut budget = InputBudget::new(8192);
        let empty_map = AnyValue::Map(Box::default());
        budget.log_value(&empty_map, 0).unwrap();
        assert_eq!(budget.bytes_left, 8192 - STRUCTURAL_BYTES);
        let values = opentelemetry::Value::Array(opentelemetry::Array::String(vec!["".into(); 64]));
        for _ in 0..7 {
            budget.node(0).unwrap();
            budget.span_value(&values).unwrap();
        }
        assert_eq!(budget.span_value(&values), Err(TelemetryError::Oversized));
        let mut small = InputBudget::new(31);
        small.log_value(&AnyValue::String("".into()), 0).unwrap();
        assert_eq!(
            small.log_value(&AnyValue::ListAny(Box::default()), 0),
            Err(TelemetryError::Oversized)
        );
    }
}
