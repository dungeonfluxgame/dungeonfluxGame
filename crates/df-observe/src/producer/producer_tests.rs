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
    assert!(
        resource
            .resource
            .as_ref()
            .unwrap()
            .attributes
            .iter()
            .any(|attribute| attribute.key == "service.name"
                && attribute.value.as_ref().unwrap().value
                    == Some(any_value::Value::StringValue("df-portal-hosted".to_owned())))
    );
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
                && attribute.value.as_ref().unwrap().value == Some(any_value::Value::IntValue(1)))
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
