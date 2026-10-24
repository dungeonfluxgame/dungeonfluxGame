//! Shared OTEL conventions, experimental transport spans, and native bounded capture.
use opentelemetry::trace::{Span, SpanContext, SpanId, TraceFlags, TraceId, TraceState, Tracer};
use opentelemetry::{Context, KeyValue, global};

/// Shared content-free operation identity, also used by browser PHONE diagnostics.
pub fn operation_hash(bytes: &[u8]) -> u64 {
    let label = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    presentation::local_identity(&label)
}

/// Seat identifiers use one diagnostic hash across native and browser records.
pub fn seat_hash(bytes: &[u8]) -> u64 {
    operation_hash(bytes)
}

/// Synchronous owner correlation. Async callers pass explicit fields at their boundary.
#[derive(Clone, Default)]
pub struct AuditContext {
    pub operation: Option<u64>,
    pub seat: Option<u64>,
    pub room: Option<u64>,
    pub revision: Option<String>,
}
thread_local! {
    static AUDIT_CONTEXT: std::cell::RefCell<AuditContext> = std::cell::RefCell::new(AuditContext::default());
    static AUDIT_FORWARDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static AUDIT_SINK: std::sync::OnceLock<fn(&'static str, String)> = std::sync::OnceLock::new();

/// Restore correlation even when an owner returns early or unwinds.
pub struct AuditScope(AuditContext, std::marker::PhantomData<std::rc::Rc<()>>);
impl Drop for AuditScope {
    fn drop(&mut self) {
        AUDIT_CONTEXT.with_borrow_mut(|context| *context = std::mem::take(&mut self.0));
    }
}
/// Set correlation for synchronous work on the current thread only.
/// Never hold this guard across `.await`, including local browser futures: another task
/// may run on the same thread. Capture owned AuditContext for asynchronous dispatch instead.
pub fn audit_scope(context: AuditContext) -> AuditScope {
    AuditScope(
        AUDIT_CONTEXT.with_borrow_mut(|current| std::mem::replace(current, context)),
        std::marker::PhantomData,
    )
}

/// Install the application audit/timeline bridge once, before admission.
pub fn install_audit_sink(sink: fn(&'static str, String)) {
    let _installed = AUDIT_SINK.set(sink);
}

/// Add explicit missing-context markers rather than silently attaching stale identities.
pub fn audit_line(mut line: String) -> String {
    AUDIT_CONTEXT.with_borrow(|context| {
        let numeric_alias = |key: &str| {
            line.split_ascii_whitespace()
                .find_map(|field| {
                    field
                        .strip_prefix(key)
                        .and_then(|value| value.parse::<u64>().ok())
                })
                .map(|value| value.to_string())
        };
        let operation = numeric_alias("operation_hash=")
            .or_else(|| context.operation.map(|value| value.to_string()));
        let seat =
            numeric_alias("seat_hash=").or_else(|| context.seat.map(|value| value.to_string()));
        let elapsed = numeric_alias("elapsed_ms=")
            .or_else(|| numeric_alias("duration_ms="))
            .or_else(|| numeric_alias("commit_ms="));
        for (key, value) in [
            ("op", operation),
            ("seat", seat),
            ("room", context.room.map(|value| value.to_string())),
            ("rev", context.revision.clone()),
            ("ms", elapsed.or_else(|| Some("unmeasured".to_owned()))),
        ] {
            if !line
                .split_ascii_whitespace()
                .any(|field| field.starts_with(&format!("{key}=")))
            {
                line.push_str(&format!(" {key}={}", value.as_deref().unwrap_or("none")));
            }
        }
    });
    line
}

/// Audit delivery is independent of whether OTEL has a provider or samples the span.
pub fn record_audit_line(event: &'static str, line: String) {
    let line = audit_line(line);
    if let Some(sink) = AUDIT_SINK.get() {
        AUDIT_FORWARDING.with(|forwarding| {
            if forwarding.replace(true) {
                return;
            }
            struct ForwardingReset<'a>(&'a std::cell::Cell<bool>);
            impl Drop for ForwardingReset<'_> {
                fn drop(&mut self) {
                    self.0.set(false);
                }
            }
            let _reset = ForwardingReset(forwarding);
            sink(event, line);
        });
    } else {
        println!("{line}");
    }
}

/// Correlation is diagnostic provenance, never authorization.
#[derive(Clone)]
pub struct OperationContext {
    pub trace_parent: String,
    pub build: String,
}

fn parent(context: &OperationContext) -> Context {
    use opentelemetry::trace::TraceContextExt;
    // Version 00 has one fixed shape. Reject before traversing untrusted metadata.
    if context.trace_parent.len() != 55 {
        return Context::new();
    }
    let mut parts = context.trace_parent.split('-');
    if let (Some("00"), Some(trace), Some(span), Some(flags), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) && trace.len() == 32
        && span.len() == 16
        && flags.len() == 2
        && [trace, span, flags].iter().all(|part| {
            part.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        && let (Ok(trace), Ok(span), Ok(flags)) = (
            TraceId::from_hex(trace),
            SpanId::from_hex(span),
            u8::from_str_radix(flags, 16),
        )
        && trace != TraceId::INVALID
        && span != SpanId::INVALID
    {
        return Context::new().with_remote_span_context(SpanContext::new(
            trace,
            span,
            TraceFlags::new(flags),
            true,
            TraceState::default(),
        ));
    }
    Context::new()
}
/// Owned OTEL operation span. Drop records abandonment unless its owner finishes it.
pub struct OperationSpan {
    span: global::BoxedSpan,
    finished: bool,
    build: String,
    method: &'static str,
    audit: AuditContext,
    #[cfg(not(target_arch = "wasm32"))]
    started: std::time::Instant,
}
/// Begin an operation using explicit correlation across asynchronous dispatch.
pub fn begin(context: &OperationContext, method: &'static str) -> OperationSpan {
    begin_with_tracer(
        &global::tracer("df-observe.transport-fixture"),
        context,
        method,
    )
}
fn begin_with_tracer(
    tracer: &global::BoxedTracer,
    context: &OperationContext,
    method: &'static str,
) -> OperationSpan {
    let mut span = tracer.start_with_context(method, &parent(context));
    span.set_attribute(KeyValue::new("fixture.build", context.build.clone()));
    OperationSpan {
        span,
        finished: false,
        build: context.build.clone(),
        method,
        audit: crate::AUDIT_CONTEXT.with_borrow(Clone::clone),
        #[cfg(not(target_arch = "wasm32"))]
        started: std::time::Instant::now(),
    }
}
impl OperationSpan {
    /// Attach a structured event with caller-selected safe provenance, never private payloads.
    pub fn event(&mut self, name: &'static str, attributes: &[(&'static str, String)]) {
        self.span.add_event(
            name,
            attributes
                .iter()
                .map(|(key, value)| KeyValue::new(*key, value.clone()))
                .collect(),
        );
    }
    /// Finish exactly once with a safe classification and actual observed byte count.
    pub fn finish(&mut self, status: &str, bytes: usize) {
        self.end(status, Some(bytes), true);
    }
    /// Finish without inventing a byte count when measurement is unavailable.
    pub fn finish_unmeasured(&mut self, status: &str) {
        self.end(status, None, true);
    }
    /// Finish a span only after its fact was already emitted through the shared audit sink.
    /// This explicit path avoids recursion without discarding ordinary `recorded` outcomes.
    pub fn finish_audit_record(&mut self) {
        self.end("recorded", None, false);
    }
    fn end(&mut self, status: &str, bytes: Option<usize>, emit_audit: bool) {
        if self.finished {
            return;
        }
        // OTEL integer attributes are signed. An unrepresentable observation is
        // unmeasured rather than a wrapped or fabricated byte count.
        let bytes = bytes.and_then(|bytes| i64::try_from(bytes).ok());
        self.span
            .set_attribute(KeyValue::new("rpc.status", status.to_owned()));
        self.span
            .set_attribute(KeyValue::new("rpc.bytes.measured", bytes.is_some()));
        if let Some(bytes) = bytes {
            self.span.set_attribute(KeyValue::new("rpc.bytes", bytes));
        }
        self.span.end();
        self.finished = true;
        // Audit-line spans explicitly opt out; ordinary classifications, including
        // "recorded", must remain visible even with no SDK or an unsampled trace.
        if emit_audit && !AUDIT_FORWARDING.with(std::cell::Cell::get) {
            #[cfg(not(target_arch = "wasm32"))]
            let elapsed = self.started.elapsed().as_millis().to_string();
            #[cfg(target_arch = "wasm32")]
            let elapsed = "unmeasured";
            let _scope = audit_scope(self.audit.clone());
            record_audit_line(
                self.method,
                format!(
                    "OUTCOME operation={} outcome={status} ms={} bytes={}",
                    self.method,
                    elapsed,
                    bytes.map_or_else(|| "unmeasured".to_owned(), |bytes| bytes.to_string()),
                ),
            );
        }
    }
}

impl Drop for OperationSpan {
    fn drop(&mut self) {
        self.finish_unmeasured("abandoned");
    }
}
/// Record a bounded lifecycle event without recording payload bytes.
/// Browser sends trace context in generated RPC metadata; this fixture installs
/// an exporting SDK on the native side only.
pub fn record(context: &OperationContext, method: &'static str, status: &str, bytes: usize) {
    begin(context, method).finish(status, bytes);
}
#[cfg(not(target_arch = "wasm32"))]
pub use native::FixtureTelemetry;
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use opentelemetry::global;
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::{
        error::{OTelSdkError, OTelSdkResult},
        trace::{SdkTracerProvider, SpanData, SpanExporter},
    };
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };
    const MAX_SPANS: usize = 512;
    #[derive(Clone, Debug, Default)]
    struct BoundedExporter {
        spans: Arc<Mutex<VecDeque<SpanData>>>,
        dropped: Arc<AtomicUsize>,
    }
    impl SpanExporter for BoundedExporter {
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            let mut spans = self.spans.lock().map_err(|_| {
                OTelSdkError::InternalFailure("fixture export lock poisoned".to_owned())
            })?;
            for span in batch {
                if spans.len() == MAX_SPANS {
                    spans.pop_front();
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
                spans.push_back(span);
            }
            Ok(())
        }
    }
    /// Fixture-only bounded in-memory export. Not durable telemetry storage.
    pub struct FixtureTelemetry {
        pub(super) provider: SdkTracerProvider,
        exporter: BoundedExporter,
    }
    impl Default for FixtureTelemetry {
        fn default() -> Self {
            let exporter = BoundedExporter::default();
            let provider = SdkTracerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            Self { provider, exporter }
        }
    }
    impl FixtureTelemetry {
        /// Begin a span with this fixture's bounded, in-memory exporter.
        pub fn begin(
            &self,
            context: &crate::OperationContext,
            method: &'static str,
        ) -> crate::OperationSpan {
            let tracer = global::BoxedTracer::new(Box::new(
                self.provider.tracer("df-observe.transport-fixture"),
            ));
            crate::begin_with_tracer(&tracer, context, method)
        }
        /// Record a lifecycle event in this fixture's bounded, in-memory exporter.
        pub fn record(
            &self,
            context: &crate::OperationContext,
            method: &'static str,
            status: &str,
            bytes: usize,
        ) {
            self.begin(context, method).finish(status, bytes);
        }
        /// Return safe emitted span count; exporter failures remain visible.
        pub fn count(&self) -> Result<usize, String> {
            self.exporter
                .spans
                .lock()
                .map(|spans| spans.len())
                .map_err(|error| error.to_string())
        }
        /// Old diagnostic spans are discarded only at this declared fixture bound.
        pub fn dropped(&self) -> usize {
            self.exporter.dropped.load(Ordering::Relaxed)
        }
        /// Read-only safe OTEL records retain parent/trace IDs and typed attributes.
        pub fn snapshot(&self) -> Result<String, String> {
            let spans = self
                .exporter
                .spans
                .lock()
                .map_err(|error| error.to_string())?;
            Ok(spans
                .iter()
                .map(|span| {
                    format!(
                        "{} trace={} span={} parent={} duration_us={:?} attributes={:?} events={:?}\n",
                        span.name,
                        span.span_context.trace_id(),
                        span.span_context.span_id(),
                        span.parent_span_id,
                        span.end_time
                            .duration_since(span.start_time)
                            .map(|duration| duration.as_micros()),
                        span.attributes,
                        span.events
                            .iter()
                            .map(|event| (&event.name, &event.attributes))
                            .collect::<Vec<_>>()
                    )
                })
                .collect())
        }
        /// Flush the actual SDK before preview shutdown.
        pub fn shutdown(&self) -> Result<(), String> {
            self.provider.shutdown().map_err(|error| error.to_string())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::OperationContext;
        use opentelemetry::{Value, trace::SpanId};

        const TRACE_PARENT: &str = "00-11111111111111111111111111111111-2222222222222222-01";

        fn context(trace_parent: &str) -> OperationContext {
            OperationContext {
                trace_parent: trace_parent.to_owned(),
                build: "admitted-build".to_owned(),
            }
        }

        fn attribute<'a>(span: &'a SpanData, name: &str) -> Option<&'a Value> {
            span.attributes
                .iter()
                .find(|attribute| attribute.key.as_str() == name)
                .map(|attribute| &attribute.value)
        }

        #[test]
        fn structured_revision_event_retains_exact_provenance_through_shared_export() {
            let telemetry = FixtureTelemetry::default();
            let mut span = telemetry.begin(&context(TRACE_PARENT), "gameplay.startup");
            let attributes = [
                ("game.run", "run-42".to_owned()),
                ("gameplay.pack_revision", "pack-sha".to_owned()),
                ("gameplay.tuning_revision", "tuning-sha".to_owned()),
                ("gameplay.departures", "hero.armor_class".to_owned()),
            ];
            span.event("gameplay.pack.loaded", &attributes);
            span.finish_unmeasured("loaded");
            drop(span);
            let spans = telemetry.exporter.spans.lock().unwrap();
            let event = &spans[0].events.events[0];
            assert_eq!(event.name, "gameplay.pack.loaded");
            for ((key, value), attribute) in attributes.iter().zip(&event.attributes) {
                assert_eq!(attribute.key.as_str(), *key);
                assert_eq!(attribute.value, Value::String(value.clone().into()));
            }
            assert_eq!(event.attributes.len(), 4);
        }

        #[test]
        fn exported_operations_keep_admitted_context_and_terminal_measurements() {
            let telemetry = FixtureTelemetry::default();
            let mut admitted = context(TRACE_PARENT);
            let mut complete = telemetry.begin(&admitted, "operation.complete");
            admitted.build = "later-build".to_owned();
            admitted.trace_parent.clear();
            complete.finish("complete", 0);
            complete.finish("duplicate", 99);
            drop(complete);
            let mut cancelled = telemetry.begin(&context(TRACE_PARENT), "operation.cancelled");
            cancelled.finish_unmeasured("cancelled");
            drop(cancelled);
            drop(telemetry.begin(&context(TRACE_PARENT), "operation.abandoned"));

            let spans = telemetry.exporter.spans.lock().unwrap();
            assert_eq!(spans.len(), 3);
            for (span, (name, status, measured)) in spans.iter().zip([
                ("operation.complete", "complete", true),
                ("operation.cancelled", "cancelled", false),
                ("operation.abandoned", "abandoned", false),
            ]) {
                assert_eq!(span.name, name);
                assert_eq!(
                    span.instrumentation_scope.name(),
                    "df-observe.transport-fixture"
                );
                assert_eq!(
                    span.span_context.trace_id().to_string(),
                    "11111111111111111111111111111111"
                );
                assert_eq!(span.parent_span_id.to_string(), "2222222222222222");
                assert!(span.span_context.is_valid());
                assert!(span.span_context.is_sampled());
                assert_eq!(
                    attribute(span, "fixture.build"),
                    Some(&Value::String("admitted-build".into()))
                );
                assert_eq!(
                    attribute(span, "rpc.status"),
                    Some(&Value::String(status.into()))
                );
                assert_eq!(
                    attribute(span, "rpc.bytes.measured"),
                    Some(&Value::Bool(measured))
                );
                assert_eq!(
                    attribute(span, "rpc.bytes"),
                    measured.then_some(&Value::I64(0))
                );
                assert_eq!(span.attributes.len(), if measured { 4 } else { 3 });
            }
        }

        #[test]
        fn malformed_parent_exports_a_root_without_copying_the_input() {
            let telemetry = FixtureTelemetry::default();
            for malformed in [
                "",
                " 00-11111111111111111111111111111111-2222222222222222-01",
                "00-11111111111111111111111111111111-2222222222222222-01 ",
                "00-1111111111111111111111111111111é-2222222222222222-01",
                "00-111111111111111111111111111111é-2222222222222222-01",
                "00-1111111111111111111111111111111-2222222222222222-01",
                "00-11111111111111111111111111111111_2222222222222222-01",
                "01-11111111111111111111111111111111-2222222222222222-01",
                "ff-11111111111111111111111111111111-2222222222222222-01",
                "00-1111111111111111111111111111111g-2222222222222222-01",
                "00-1111111111111111111111111111111A-2222222222222222-01",
                "00-11111111111111111111111111111111-222222222222222g-01",
                "00-11111111111111111111111111111111-2222222222222222-0g",
                "00-11111111111111111111111111111111-2222222222222222-1",
                "00-00000000000000000000000000000000-2222222222222222-01",
                "00-11111111111111111111111111111111-0000000000000000-01",
                "00-11111111111111111111111111111111-2222222222222222-01-extra",
            ] {
                telemetry.record(&context(malformed), "operation.root", "complete", 1);
            }
            telemetry.record(
                &context(&"-".repeat(65536)),
                "operation.long-parent",
                "complete",
                1,
            );
            let spans = telemetry.exporter.spans.lock().unwrap();
            assert_eq!(spans.len(), 18);
            for span in spans.iter() {
                assert_eq!(span.parent_span_id, SpanId::INVALID);
                assert!(span.span_context.is_valid());
                assert_ne!(
                    span.span_context.trace_id().to_string(),
                    "11111111111111111111111111111111"
                );
                assert_eq!(span.attributes.len(), 4);
            }
        }

        #[test]
        fn unsampled_parent_preserves_causality_without_forcing_export() {
            use opentelemetry::trace::{Span, TraceContextExt};
            let context = context("00-11111111111111111111111111111111-2222222222222222-00");
            let parent = crate::parent(&context);
            let remote = parent.span().span_context().clone();
            assert!(remote.is_remote());
            assert!(remote.is_valid());
            assert!(!remote.is_sampled());
            assert_eq!(remote.span_id().to_string(), "2222222222222222");
            let telemetry = FixtureTelemetry::default();
            let mut operation = telemetry.begin(&context, "operation.unsampled");
            assert_eq!(operation.span.span_context().trace_id(), remote.trace_id());
            assert!(!operation.span.span_context().is_sampled());
            operation.finish("complete", 1);
            assert_eq!(telemetry.count().unwrap(), 0);
        }

        #[test]
        fn byte_count_boundaries_never_export_wrapped_measurements() {
            let telemetry = FixtureTelemetry::default();
            let maximum = usize::try_from(i64::MAX).unwrap();
            for (name, bytes) in [
                ("operation.zero", 0),
                ("operation.maximum", maximum),
                ("operation.overflow", maximum + 1),
                ("operation.usize-maximum", usize::MAX),
            ] {
                telemetry.record(&context(TRACE_PARENT), name, "complete", bytes);
            }
            let spans = telemetry.exporter.spans.lock().unwrap();
            assert_eq!(spans.len(), 4);
            for (span, expected) in spans.iter().zip([Some(0), Some(i64::MAX), None, None]) {
                assert_eq!(
                    attribute(span, "rpc.bytes.measured"),
                    Some(&Value::Bool(expected.is_some()))
                );
                assert_eq!(
                    attribute(span, "rpc.bytes"),
                    expected.map(Value::I64).as_ref()
                );
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod ingress;
#[cfg(not(target_arch = "wasm32"))]
pub use ingress::*;
#[cfg(not(target_arch = "wasm32"))]
mod producer;
#[cfg(not(target_arch = "wasm32"))]
pub use producer::{AnyValue, CapturedBatch, LogInput, NativeProducer, Severity, SpanInput};

mod dispatch_context;
pub use dispatch_context::{begin_dispatched, begin_dispatched_with_audit};

mod capture;
pub use capture::{CaptureKey, ProducerId, RecordId, Signal, SourceSequence, TelemetryError};
mod browser;
pub mod presentation;
pub use browser::{
    BrowserBuffer, BrowserLog, BufferError, BufferSnapshot, UploadFailure, UploadLease,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod audit_tests {
    use super::*;
    static LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    fn sink(event: &'static str, line: String) {
        if event == "test.audit-terminal" {
            LINES.lock().unwrap().push(line);
        }
        let context = OperationContext {
            trace_parent: String::new(),
            build: "test".to_owned(),
        };
        begin(&context, "test.audit-record").finish_unmeasured("recorded");
    }
    #[test]
    fn terminal_audit_survives_no_sdk_and_keeps_dispatch_correlation_without_recursion() {
        install_audit_sink(sink);
        let context = OperationContext {
            trace_parent: String::new(),
            build: "test".to_owned(),
        };
        let owner = audit_scope(AuditContext {
            operation: Some(71),
            seat: Some(72),
            room: Some(73),
            revision: Some("1:4".to_owned()),
        });
        let mut operation = begin(&context, "test.audit-terminal");
        drop(owner);
        let _other = audit_scope(AuditContext {
            operation: Some(99),
            ..AuditContext::default()
        });
        operation.finish_unmeasured("retained");
        operation.finish_unmeasured("duplicate");
        let lines = LINES.lock().unwrap();
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!(line.contains("outcome=retained"));
        assert!(line.contains("op=71 seat=72 room=73 rev=1:4"));
        assert!(!line.contains("op=99"));
        drop(lines);
        let parent_scope = audit_scope(AuditContext {
            operation: Some(81),
            seat: Some(82),
            room: Some(83),
            revision: Some("2:5".to_owned()),
        });
        let parent = begin(&context, "test.dispatch-parent");
        let dispatch = parent.capture_context();
        let audit = parent.capture_audit_context();
        drop(parent_scope);
        std::thread::spawn(move || {
            begin_dispatched_with_audit(&dispatch, &audit, "test.audit-terminal")
                .finish_unmeasured("worker-finished");
        })
        .join()
        .unwrap();
        let lines = LINES.lock().unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("outcome=worker-finished"));
        assert!(lines[1].contains("op=81 seat=82 room=83 rev=2:5"));
        drop(lines);
        begin(&context, "test.audit-terminal").finish_unmeasured("recorded");
        let lines = LINES.lock().unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("outcome=recorded"));
    }
    #[test]
    fn audit_scope_restores_context_and_explicit_fields_win() {
        let original = audit_line("TEST outcome=before".to_owned());
        {
            let _owner = audit_scope(AuditContext {
                operation: Some(7),
                ..AuditContext::default()
            });
            assert!(audit_line("TEST op=8 ms=4".to_owned()).contains("op=8 ms=4"));
            assert!(audit_line("TEST".to_owned()).contains("op=7"));
        }
        assert_eq!(audit_line("TEST outcome=before".to_owned()), original);
    }
}
