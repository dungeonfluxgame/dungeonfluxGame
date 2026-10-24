use crate::{OperationContext, OperationSpan, parent};
use opentelemetry::{
    KeyValue, global,
    trace::{Link, Span, TraceContextExt, Tracer},
};

/// Begin owned asynchronous work with a causal link to its dispatch snapshot.
/// Capture with `OperationSpan::capture_context` before transferring work to its owner.
/// Request completion or cancellation does not invalidate the snapshot or cancel the child.
pub fn begin_dispatched(context: &OperationContext, method: &'static str) -> OperationSpan {
    begin_with_tracer(
        &global::tracer("df-observe.transport-fixture"),
        context,
        method,
    )
}

/// Begin dispatched work with an owned diagnostic snapshot, including when no SDK is installed.
/// The snapshot is provenance only and must be captured by the dispatch owner.
pub fn begin_dispatched_with_audit(
    context: &OperationContext,
    audit: &crate::AuditContext,
    method: &'static str,
) -> OperationSpan {
    let mut span = begin_dispatched(context, method);
    span.audit = audit.clone();
    span
}

fn begin_with_tracer(
    tracer: &global::BoxedTracer,
    context: &OperationContext,
    method: &'static str,
) -> OperationSpan {
    let parent = parent(context);
    let mut builder = tracer.span_builder(method);
    let source = parent.span().span_context().clone();
    if source.is_valid() {
        builder = builder.with_links(vec![Link::new(source, Vec::new(), 0)]);
    }
    let mut span = tracer.build_with_context(builder, &parent);
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
    /// Own the safe audit correlation at dispatch, independently of SDK trace sampling.
    pub fn capture_audit_context(&self) -> crate::AuditContext {
        self.audit.clone()
    }

    /// Capture diagnostic correlation and admitted build before dispatch. The returned
    /// owned context contains this span's SDK identity, never raw incoming metadata.
    /// It carries no authorization, job lifetime, deadline, or cancellation ownership;
    /// those remain with the native dispatch owner independently of the request wait.
    pub fn capture_context(&self) -> OperationContext {
        let source = self.span.span_context();
        let trace_parent = if source.is_valid() {
            format!(
                "00-{}-{}-{:02x}",
                source.trace_id(),
                source.span_id(),
                source.trace_flags().to_u8()
            )
        } else {
            String::new()
        };
        OperationContext {
            trace_parent,
            build: self.build.clone(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl crate::FixtureTelemetry {
    /// Begin dispatched work using this fixture's independent bounded exporter.
    pub fn begin_dispatched(
        &self,
        context: &OperationContext,
        method: &'static str,
    ) -> OperationSpan {
        use opentelemetry::trace::TracerProvider;
        let tracer = global::BoxedTracer::new(Box::new(
            self.provider.tracer("df-observe.transport-fixture"),
        ));
        begin_with_tracer(&tracer, context, method)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use opentelemetry::{
        Value,
        trace::{SpanId, TracerProvider},
    };
    use opentelemetry_sdk::{
        error::OTelSdkResult,
        trace::{SdkTracerProvider, SpanData, SpanExporter},
    };
    use std::sync::{Arc, Mutex};

    const TRACE_PARENT: &str = "00-11111111111111111111111111111111-2222222222222222-01";

    #[derive(Clone, Debug, Default)]
    struct TestExporter {
        spans: Arc<Mutex<Vec<SpanData>>>,
    }
    impl SpanExporter for TestExporter {
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            self.spans.lock().unwrap().extend(batch);
            Ok(())
        }
    }
    struct TestTelemetry {
        provider: SdkTracerProvider,
        exporter: TestExporter,
    }
    impl Default for TestTelemetry {
        fn default() -> Self {
            let exporter = TestExporter::default();
            let provider = SdkTracerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            Self { provider, exporter }
        }
    }
    impl TestTelemetry {
        fn tracer(&self) -> global::BoxedTracer {
            global::BoxedTracer::new(Box::new(self.provider.tracer("df-observe.dispatch-test")))
        }
        fn begin(&self, context: &OperationContext, method: &'static str) -> OperationSpan {
            crate::begin_with_tracer(&self.tracer(), context, method)
        }
        fn begin_dispatched(
            &self,
            context: &OperationContext,
            method: &'static str,
        ) -> OperationSpan {
            super::begin_with_tracer(&self.tracer(), context, method)
        }
        fn snapshot(&self) -> Result<String, String> {
            self.exporter
                .spans
                .lock()
                .map(|spans| format!("{spans:?}"))
                .map_err(|error| error.to_string())
        }
        fn count(&self) -> Result<usize, String> {
            self.exporter
                .spans
                .lock()
                .map(|spans| spans.len())
                .map_err(|error| error.to_string())
        }
    }
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
    fn dispatch_links_reference_the_request_span_after_it_ends() {
        let telemetry = TestTelemetry::default();
        let mut request = telemetry.begin(&context(TRACE_PARENT), "request");
        let captured = request.capture_context();
        let source = request.span.span_context().clone();
        request.finish_unmeasured("complete");
        drop(request);
        telemetry
            .begin_dispatched(&captured, "child")
            .finish_unmeasured("complete");

        let spans = telemetry.exporter.spans.lock().unwrap();
        assert_eq!(spans.len(), 2);
        let child = &spans[1];
        assert_eq!(child.parent_span_id, source.span_id());
        assert_eq!(child.span_context.trace_id(), source.trace_id());
        assert_ne!(child.span_context.span_id(), source.span_id());
        assert_eq!(child.links.links.len(), 1);
        assert_eq!(
            child.links.links[0].span_context.trace_id(),
            source.trace_id()
        );
        assert_eq!(
            child.links.links[0].span_context.span_id(),
            source.span_id()
        );
        assert!(child.links.links[0].attributes.is_empty());
        assert_eq!(child.attributes.len(), 3);
        assert_eq!(
            attribute(child, "fixture.build"),
            Some(&Value::String("admitted-build".into()))
        );
    }

    #[test]
    fn dispatch_capture_excludes_invalid_metadata_and_keeps_unsampled_identity() {
        let telemetry = TestTelemetry::default();
        let private = "authorization=secret-token;player.message=private-transcript";
        let request = telemetry.begin(&context(private), "request.root");
        let captured = request.capture_context();
        let source = request.span.span_context().clone();
        assert!(!captured.trace_parent.contains(private));
        assert_eq!(captured.trace_parent.len(), 55);
        drop(request);
        telemetry
            .begin_dispatched(&captured, "child.root")
            .finish_unmeasured("complete");
        telemetry
            .begin_dispatched(&context(private), "child.invalid")
            .finish_unmeasured("complete");
        let spans = telemetry.exporter.spans.lock().unwrap();
        assert_eq!(
            spans[1].links.links[0].span_context.span_id(),
            source.span_id()
        );
        assert!(spans[2].links.links.is_empty());
        assert_eq!(spans[2].parent_span_id, SpanId::INVALID);
        drop(spans);
        assert!(!telemetry.snapshot().unwrap().contains(private));

        let request = telemetry.begin(
            &context("00-11111111111111111111111111111111-2222222222222222-00"),
            "request.unsampled",
        );
        let captured = request.capture_context();
        assert!(captured.trace_parent.ends_with("-00"));
        let source = request.span.span_context().clone();
        drop(request);
        let mut child = telemetry.begin_dispatched(&captured, "child.unsampled");
        assert_eq!(child.span.span_context().trace_id(), source.trace_id());
        assert!(!child.span.span_context().is_sampled());
        child.finish_unmeasured("complete");
        assert_eq!(telemetry.count().unwrap(), 3);
    }
}
