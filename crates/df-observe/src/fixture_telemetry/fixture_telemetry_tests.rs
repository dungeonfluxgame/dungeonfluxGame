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
        "00-1111111111111111111111111111111Ã©-2222222222222222-01",
        "00-111111111111111111111111111111Ã©-2222222222222222-01",
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
