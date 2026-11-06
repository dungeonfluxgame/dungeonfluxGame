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
    fn begin_dispatched(&self, context: &OperationContext, method: &'static str) -> OperationSpan {
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
