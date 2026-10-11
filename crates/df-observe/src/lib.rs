//! Shared OTEL conventions, experimental transport spans, and native bounded capture.
use opentelemetry::trace::{Span, SpanContext, SpanId, TraceFlags, TraceId, TraceState, Tracer};
use opentelemetry::{Context, KeyValue, global};

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
        self.end(status, Some(bytes));
    }
    /// Finish without inventing a byte count when measurement is unavailable.
    pub fn finish_unmeasured(&mut self, status: &str) {
        self.end(status, None);
    }
    fn end(&mut self, status: &str, bytes: Option<usize>) {
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
                        "{} trace={} span={} parent={} duration_us={:?} attributes={:?}\n",
                        span.name,
                        span.span_context.trace_id(),
                        span.span_context.span_id(),
                        span.parent_span_id,
                        span.end_time
                            .duration_since(span.start_time)
                            .map(|duration| duration.as_micros()),
                        span.attributes
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
pub use dispatch_context::begin_dispatched;

mod capture;
pub use capture::{CaptureKey, ProducerId, RecordId, Signal, SourceSequence, TelemetryError};
mod browser;
pub use browser::{
    BrowserBuffer, BrowserLog, BufferError, BufferSnapshot, UploadFailure, UploadLease,
};
