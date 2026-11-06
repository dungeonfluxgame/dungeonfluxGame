#![cfg(not(target_arch = "wasm32"))]
//! Current native boundary witness; no collector, browser uploader, or service authority.
use df_observe::{
    AnyValue, BrowserBuffer, BrowserLog, BufferError, CaptureKey, Durability, LogInput,
    NativeProducer, ProducerId, RecordId, Severity, Signal, SourceSequence, SpanInput,
    TelemetryError, TelemetryLimits, UploadCause,
};
use df_telemetry::{DiagnosticReader, QueryFilter, Store};
use df_types::{BuildIdentity, OperationId, SessionId};
use opentelemetry::{
    Context, KeyValue,
    trace::{Link, SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState},
};
use opentelemetry_proto::tonic::{
    collector::{logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest},
    common::v1::any_value::Value,
    logs::v1::SeverityNumber,
};
use prost::Message;
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

const WAIT: Duration = Duration::from_secs(5);
/// Finest `SystemTime` step: Windows counts 100 ns intervals, other platforms nanoseconds.
#[cfg(windows)]
const SYSTEM_TIME_STEP_NANOS: u64 = 100;
#[cfg(not(windows))]
const SYSTEM_TIME_STEP_NANOS: u64 = 1;
/// Delay from the log's source time to its observed time: 9 clock steps.
const OBSERVED_DELAY_NANOS: u64 = 9 * SYSTEM_TIME_STEP_NANOS;
/// Span duration of 11 clock steps.
const SPAN_DURATION_NANOS: u64 = 11 * SYSTEM_TIME_STEP_NANOS;

#[test]
fn current_producers_store_reader_preserve_identity_gaps_retry_and_owned_shutdown() {
    // No fallback root: the caller must supply the exact registered fresh Source/reviewer leaf.
    let path = PathBuf::from(
        std::env::var_os("DF_OBSERVE_BOUNDARY_FIXTURE_ROOT")
            .expect("the qualification caller must grant a protected fixture root"),
    );
    assert!(path.is_absolute());
    assert!(matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(
            "ownedfixture-ob-boundary-a1-worker-01-boundary"
                | "ownedfixture-ob-boundary-a1-worker-02-boundary"
                | "ownedfixture-ob-boundary-a1-review-01-boundary"
        )
    ));
    assert!(
        !path.exists(),
        "retain earlier evidence and grant a fresh fixture leaf"
    );
    let build = BuildIdentity::new(
        Some("observe-boundary-source"),
        Some("synthetic-native"),
        Some("WASM-compile-only"),
        Some("synthetic-config"),
        Some("synthetic-content"),
    )
    .unwrap();
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let ingress = store.ingress();
    let producer_id = ProducerId::new([1; 16]).unwrap();
    let mut producer = NativeProducer::new(
        producer_id,
        SourceSequence::new(1).unwrap(),
        &build,
        ingress.clone(),
        limits,
    )
    .unwrap();
    let unsampled = SpanContext::new(
        TraceId::from_bytes([2; 16]),
        SpanId::from_bytes([3; 8]),
        TraceFlags::NOT_SAMPLED,
        true,
        TraceState::default(),
    );
    let log_key = producer.reserve(RecordId::new([4; 16]).unwrap()).unwrap();
    let time = SystemTime::UNIX_EPOCH + Duration::from_nanos(1000);
    let log = producer
        .emit_log(
            log_key,
            LogInput {
                timestamp: time,
                observed_timestamp: time + Duration::from_nanos(OBSERVED_DELAY_NANOS),
                severity: Severity::Info,
                body: AnyValue::Int(i64::MIN),
                attributes: vec![("rpc.bytes".into(), AnyValue::Int(0))],
                trace: Some(unsampled.clone()),
            },
        )
        .unwrap();
    let original = log.bytes;
    let log_receipt = log.pending.wait(WAIT).unwrap();
    assert_eq!(log_receipt.durability, Durability::Committed);
    assert_eq!(log_receipt.records, 1);

    // An actually reserved but uncaptured sequence creates a visible source gap.
    let skipped = producer.reserve(RecordId::new([5; 16]).unwrap()).unwrap();
    assert_eq!(skipped.sequence.get(), 2);
    let span_key = producer.reserve(RecordId::new([6; 16]).unwrap()).unwrap();
    let parent = SpanContext::new(
        TraceId::from_bytes([7; 16]),
        SpanId::from_bytes([8; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    let span = producer
        .emit_span(
            span_key,
            SpanInput {
                name: "boundary.fixture".into(),
                start: time,
                end: time + Duration::from_nanos(SPAN_DURATION_NANOS),
                parent: Context::new().with_remote_span_context(parent),
                links: vec![Link::new(
                    unsampled,
                    vec![KeyValue::new("rpc.bytes", 0i64)],
                    0,
                )],
                attributes: vec![KeyValue::new("rpc.status", "complete")],
            },
        )
        .unwrap();
    let span_receipt = span.pending.wait(WAIT).unwrap();
    assert_eq!(span_receipt.durability, Durability::Committed);
    assert!(span_receipt.committed_watermark >= log_receipt.committed_watermark);

    let browser_key = CaptureKey {
        producer: ProducerId::new([9; 16]).unwrap(),
        record: RecordId::new([10; 16]).unwrap(),
        sequence: SourceSequence::new(1).unwrap(),
    };
    let session = SessionId::from_bytes(&[11; 16]).unwrap();
    let operation = OperationId::from_bytes(&[12; 16]).unwrap();
    let mut browser = BrowserBuffer::new(1);
    browser
        .retain_log(
            &build,
            browser_key,
            BrowserLog {
                timestamp_unix_nanos: 1000,
                observed_unix_nanos: 1009,
                severity: SeverityNumber::Info,
                event_name: "browser.fixture",
                status: "complete",
                measured_bytes: Some(0),
                session: Some(session),
                operation: Some(operation),
                trace: None,
            },
        )
        .unwrap();
    let first_lease = browser.begin_upload(Duration::ZERO).unwrap();
    let browser_bytes = browser
        .upload_records(&first_lease)
        .unwrap()
        .next()
        .unwrap()
        .to_vec();
    assert!(browser_bytes.len() <= 8192);
    browser
        .fail_upload(&first_lease, UploadCause::Transport)
        .unwrap();
    assert_eq!(browser.snapshot().retained_records, 1);
    let retry_lease = browser.begin_upload(Duration::from_secs(1)).unwrap();
    assert_eq!(
        browser
            .upload_records(&retry_lease)
            .unwrap()
            .next()
            .unwrap(),
        browser_bytes
    );
    assert_eq!(
        browser.acknowledge_durable(&first_lease, Duration::from_secs(2)),
        Err(BufferError::StaleLease)
    );
    let browser_receipt = ingress
        .submit(Signal::Logs, &browser_bytes)
        .unwrap()
        .wait(WAIT)
        .unwrap();
    assert_eq!(browser_receipt.durability, Durability::Committed);
    assert_eq!(browser_receipt.records, retry_lease.records());
    // Matched full terminal local receipt only; this does not model an RPC upload protocol.
    browser
        .acknowledge_durable(&retry_lease, Duration::from_secs(2))
        .unwrap();
    assert_eq!(browser.snapshot().retained_records, 0);
    assert_eq!(browser.snapshot().retained_bytes, 0);

    let replay = ingress
        .submit(Signal::Logs, &original)
        .unwrap()
        .wait(WAIT)
        .unwrap();
    // A replay gets a new spool frame; the stored capture position remains unchanged.
    assert!(replay.spool_position > browser_receipt.spool_position);
    assert_eq!(replay.records, 1);
    assert_eq!(store.health().duplicates, 1);
    let mut changed = ExportLogsServiceRequest::decode(original.as_slice()).unwrap();
    changed.resource_logs[0].scope_logs[0].log_records[0].severity_text = "fixture conflict".into();
    assert_eq!(
        ingress
            .submit(Signal::Logs, &changed.encode_to_vec())
            .unwrap()
            .wait(WAIT),
        Err(TelemetryError::Conflict)
    );
    assert!(matches!(
        ingress.submit(Signal::Logs, &[0]),
        Err(TelemetryError::Malformed)
    ));

    let mut reader = DiagnosticReader::open(&path).unwrap();
    assert!(reader.verify_write_denied().unwrap());
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 3);
    assert_eq!(
        page.committed_watermark,
        browser_receipt.committed_watermark
    );
    assert!(page.spool_checkpoint >= log_receipt.spool_position);
    assert!(page.unconfirmed_capture_gaps);
    assert!(
        page.retention
            .contains("producer lifetime completeness unknown")
    );
    let native_source = page
        .source_watermarks
        .iter()
        .find(|s| s.producer == producer_id)
        .unwrap();
    assert_eq!(
        (
            native_source.accepted_records,
            native_source.highest_sequence,
            native_source.missing_through_highest
        ),
        (2, 3, 1)
    );
    let view = &page.records[0];
    assert_eq!(view.position, log_receipt.committed_watermark);
    assert_eq!(
        (view.producer, view.record, view.source_sequence),
        (log_key.producer, log_key.record, log_key.sequence.get())
    );
    assert_eq!(
        (view.source_time_unix_nanos, view.observed_time_unix_nanos),
        (1000, Some(1000 + OBSERVED_DELAY_NANOS))
    );
    let logs = ExportLogsServiceRequest::decode(view.otlp.as_slice()).unwrap();
    let resource = &logs.resource_logs[0];
    assert_eq!(
        resource.schema_url,
        "https://dungeonflux.local/telemetry/native-v1"
    );
    assert!(
        resource
            .resource
            .as_ref()
            .unwrap()
            .attributes
            .iter()
            .any(|a| a.key == "df.build"
                && a.value.as_ref().unwrap().value
                    == Some(Value::StringValue("observe-boundary-source".into())))
    );
    let scope = &resource.scope_logs[0];
    assert_eq!(scope.scope.as_ref().unwrap().name, "df-observe.native");
    assert_eq!(
        scope.schema_url,
        "https://dungeonflux.local/telemetry/scope-v1"
    );
    let record = &scope.log_records[0];
    assert_eq!(
        record.body.as_ref().unwrap().value,
        Some(Value::IntValue(i64::MIN))
    );
    assert_eq!(record.trace_id, [2; 16]);
    assert_eq!(record.span_id, [3; 8]);
    assert_eq!(record.flags & 1, 0);

    let spans = ExportTraceServiceRequest::decode(page.records[1].otlp.as_slice()).unwrap();
    let span = &spans.resource_spans[0].scope_spans[0].spans[0];
    assert_eq!(span.trace_id, [7; 16]);
    assert_eq!(span.parent_span_id, [8; 8]);
    assert_eq!(span.links[0].trace_id, [2; 16]);
    assert_eq!(span.links[0].span_id, [3; 8]);
    assert_eq!(
        span.end_time_unix_nano - span.start_time_unix_nano,
        SPAN_DURATION_NANOS
    );
    let filtered = reader
        .query(
            &QueryFilter {
                producer: Some(browser_key.producer),
                session: Some("0b".repeat(16)),
                operation: Some("0c".repeat(16)),
                ..Default::default()
            },
            0,
            100,
        )
        .unwrap();
    assert_eq!(filtered.records.len(), 1);
    assert_eq!(filtered.records[0].record, browser_key.record);
    let request = ExportLogsServiceRequest::decode(filtered.records[0].otlp.as_slice()).unwrap();
    let record = &request.resource_logs[0].scope_logs[0].log_records[0];
    assert!(record.body.is_none());
    assert_eq!(record.event_name, "browser.fixture");
    assert_eq!(record.attributes.len(), 9);

    drop(reader);
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
    assert!(matches!(
        ingress.submit(Signal::Logs, &original),
        Err(TelemetryError::Closed)
    ));
    drop(ingress);
    drop(store);
    // Reopen proves the first owned writer finished and released root custody, with data retained.
    let mut reopened = Store::open(&path, limits, None).unwrap();
    let mut reader = DiagnosticReader::open(&path).unwrap();
    assert_eq!(
        reader
            .query(&QueryFilter::default(), 0, 100)
            .unwrap()
            .records
            .len(),
        3
    );
    drop(reader);
    reopened.shutdown(WAIT).unwrap();
    println!(
        "boundary root={} native_records=2 browser_records=1 source_gap=1 replay=1 typed_conflict=true read_only=true owned_shutdown=true",
        path.display()
    );
}
