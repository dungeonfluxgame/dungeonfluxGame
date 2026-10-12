#![cfg(not(target_arch = "wasm32"))]
use df_observe::{
    AnyValue, Durability, LogInput, NativeProducer, ProducerId, RecordId, Severity, Signal,
    SourceSequence, SpanInput, TelemetryError, TelemetryLimits,
};
use df_telemetry::{BarrierStage, DiagnosticReader, FailureBarrier, QueryFilter, Readiness, Store};
use df_types::BuildIdentity;
use opentelemetry::{
    Context, KeyValue,
    logs::AnyValue as SdkValue,
    trace::{Link, SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState},
};
use opentelemetry_proto::tonic::{
    collector::{logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest},
    common::v1::{AnyValue as WireValue, any_value::Value},
};
use prost::Message;
use rusqlite::Connection;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime},
};
const WAIT: Duration = Duration::from_secs(5);
fn root(name: &str) -> PathBuf {
    if let Ok(namespace) = std::env::var("DF_TELEMETRY_FIXTURE_NAMESPACE") {
        assert!(!namespace.is_empty());
        assert!(
            namespace
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        );
        assert!(
            [
                "typed",
                "dedupe",
                "outage",
                "queue-bytes",
                "queue-items",
                "wire-boundary",
                "spool-capacity",
                "sqlite-capacity",
                "truncated",
                "corrupt",
                "foreign",
                "exclusive",
                "symlink",
                "query",
                "sequence",
                "enabled-levels",
                "structural-budget",
                "sql-content-corrupt",
                "checkpoint-missing-record",
            ]
            .contains(&name)
        );
        let path = PathBuf::from(
            "/Users/earlcameron/Desktop/dungeonflux/development/runtime/telemetry-g06",
        )
        .join(format!("ownedfixture-{namespace}-{name}"));
        assert!(path.file_name().unwrap().len() <= 100);
        assert!(
            !path.exists(),
            "managed fixture root must be fresh: {}",
            path.display()
        );
        return path;
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base =
        PathBuf::from("/Users/earlcameron/Desktop/dungeonflux/development/runtime/telemetry-g06");
    fs::create_dir_all(&base).unwrap();
    base.join(format!(
        "ownedfixture-a2-{}-{nonce}-{}-{}",
        name,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
fn build() -> BuildIdentity {
    BuildIdentity::new(
        Some("sdk-roundtrip-v1"),
        Some("native-fixture"),
        Some("native-only-unqualified"),
        Some("synthetic-v1"),
        Some("synthetic-safe"),
    )
    .unwrap()
}
fn make_producer(store: &Store, limits: TelemetryLimits) -> NativeProducer {
    NativeProducer::new(
        ProducerId::new([7; 16]).unwrap(),
        SourceSequence::new(1).unwrap(),
        &build(),
        store.ingress(),
        limits,
    )
    .unwrap()
}
fn log(body: &str) -> LogInput {
    let time = SystemTime::UNIX_EPOCH + Duration::from_nanos(1700000000000000000);
    LogInput {
        timestamp: time,
        observed_timestamp: time + Duration::from_nanos(9),
        severity: Severity::Info,
        body: AnyValue::String(body.to_owned().into()),
        attributes: vec![
            ("df.session".into(), AnyValue::String("session-one".into())),
            (
                "df.operation".into(),
                AnyValue::String("operation-one".into()),
            ),
        ],
        trace: None,
    }
}
fn barrier(
    stage: BarrierStage,
) -> (
    FailureBarrier,
    mpsc::Receiver<BarrierStage>,
    mpsc::Sender<()>,
) {
    let (reached, receiver) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::channel();
    (
        FailureBarrier {
            stage,
            reached,
            release: gate,
        },
        receiver,
        release,
    )
}
#[test]
fn actual_sdk_typed_log_unsampled_correlation_and_span_causality_roundtrip() {
    let path = root("typed");
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = make_producer(&store, limits);
    let unsampled = SpanContext::new(
        TraceId::from_bytes([1; 16]),
        SpanId::from_bytes([2; 8]),
        TraceFlags::NOT_SAMPLED,
        true,
        TraceState::default(),
    );
    let mut input = log("unused");
    input.timestamp = SystemTime::UNIX_EPOCH + Duration::from_nanos(u64::MAX - 1);
    input.observed_timestamp = SystemTime::UNIX_EPOCH + Duration::from_nanos(u64::MAX);
    input.severity = Severity::Fatal;
    input.trace = Some(unsampled.clone());
    input.body = SdkValue::Map(Box::new(HashMap::from([
        ("integer".into(), AnyValue::Int(i64::MIN)),
        (
            "list".into(),
            AnyValue::ListAny(Box::new(vec![
                AnyValue::Boolean(true),
                AnyValue::Double(1.25),
                AnyValue::Bytes(Box::new(vec![0, 255])),
                AnyValue::String("typed".into()),
            ])),
        ),
    ])));
    input.attributes.extend([
        ("min".into(), AnyValue::Int(i64::MIN)),
        ("max".into(), AnyValue::Int(i64::MAX)),
        ("bool".into(), AnyValue::Boolean(true)),
        ("bytes".into(), AnyValue::Bytes(Box::new(vec![0, 1, 255]))),
    ]);
    let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
    let captured = producer.emit_log(key, input).unwrap();
    assert_eq!(
        captured.pending.wait(WAIT).unwrap().durability,
        Durability::Committed
    );
    let parent = SpanContext::new(
        TraceId::from_bytes([3; 16]),
        SpanId::from_bytes([4; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    let link = Link::new(unsampled, vec![KeyValue::new("link.integer", i64::MAX)], 0);
    let start = SystemTime::UNIX_EPOCH + Duration::from_nanos(1700000000000000100);
    let key = producer.reserve(RecordId::new([9; 16]).unwrap()).unwrap();
    let span = producer
        .emit_span(
            key,
            SpanInput {
                name: "native-parent-link".into(),
                start,
                end: start + Duration::from_nanos(101),
                parent: Context::new().with_remote_span_context(parent),
                links: vec![link],
                attributes: vec![
                    KeyValue::new("df.session", "session-one"),
                    KeyValue::new("df.operation", "operation-one"),
                    KeyValue::new("negative", i64::MIN),
                ],
            },
        )
        .unwrap();
    assert_eq!(
        span.pending.wait(WAIT).unwrap().durability,
        Durability::Committed
    );
    let mut reader = DiagnosticReader::open(&path).unwrap();
    assert!(reader.verify_write_denied().unwrap());
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 2);
    assert!(page.unconfirmed_capture_gaps);
    let view = &page.records[0];
    assert_eq!(view.source_time_unix_nanos, u64::MAX - 1);
    assert_eq!(view.observed_time_unix_nanos, Some(u64::MAX));
    let wire = ExportLogsServiceRequest::decode(view.otlp.as_slice()).unwrap();
    let resource = &wire.resource_logs[0];
    let scope = &resource.scope_logs[0];
    let record = &scope.log_records[0];
    assert_eq!(
        resource.schema_url,
        "https://dungeonflux.local/telemetry/native-v1"
    );
    assert_eq!(
        scope.schema_url,
        "https://dungeonflux.local/telemetry/scope-v1"
    );
    assert_eq!(scope.scope.as_ref().unwrap().name, "df-observe.native");
    assert_eq!(scope.scope.as_ref().unwrap().version, "0.1.0");
    assert!(!scope.scope.as_ref().unwrap().attributes.is_empty());
    assert_eq!(record.trace_id, [1; 16]);
    assert_eq!(record.span_id, [2; 8]);
    assert_eq!(record.flags & 1, 0);
    assert_eq!(record.severity_number, 21);
    assert!(
        record
            .attributes
            .iter()
            .any(|attribute| attribute.key == "min"
                && attribute.value
                    == Some(WireValue {
                        value: Some(Value::IntValue(i64::MIN))
                    }))
    );
    assert!(
        record
            .attributes
            .iter()
            .any(|attribute| attribute.key == "max"
                && attribute.value
                    == Some(WireValue {
                        value: Some(Value::IntValue(i64::MAX))
                    }))
    );
    assert!(
        record
            .attributes
            .iter()
            .any(|attribute| attribute.key == "bytes"
                && attribute.value
                    == Some(WireValue {
                        value: Some(Value::BytesValue(vec![0, 1, 255]))
                    }))
    );
    let Some(Value::KvlistValue(body)) = record.body.as_ref().unwrap().value.as_ref() else {
        panic!("typed map lost")
    };
    assert_eq!(body.values.len(), 2);
    let Some(Value::ArrayValue(list)) = body
        .values
        .iter()
        .find(|attribute| attribute.key == "list")
        .unwrap()
        .value
        .as_ref()
        .unwrap()
        .value
        .as_ref()
    else {
        panic!("typed list lost")
    };
    assert_eq!(list.values.len(), 4);
    assert_eq!(page.records[1].observed_time_unix_nanos, None);
    let wire = ExportTraceServiceRequest::decode(page.records[1].otlp.as_slice()).unwrap();
    let resource = &wire.resource_spans[0];
    let scope = &resource.scope_spans[0];
    let span = &scope.spans[0];
    assert_eq!(
        resource.schema_url,
        "https://dungeonflux.local/telemetry/native-v1"
    );
    assert_eq!(
        scope.schema_url,
        "https://dungeonflux.local/telemetry/scope-v1"
    );
    assert_eq!(span.trace_id, [3; 16]);
    assert_eq!(span.parent_span_id, [4; 8]);
    assert_eq!(span.links[0].trace_id, [1; 16]);
    assert_eq!(span.links[0].span_id, [2; 8]);
    assert_eq!(span.end_time_unix_nano - span.start_time_unix_nano, 101);
    let filtered = reader
        .query(
            &QueryFilter {
                from_unix_nanos: Some(u64::MAX - 1),
                through_unix_nanos: Some(u64::MAX),
                ..Default::default()
            },
            0,
            100,
        )
        .unwrap();
    assert_eq!(filtered.records.len(), 1);
    println!(
        "typed_sdk_roundtrip root={} log_source={} log_observed={} resource_schema={} scope_schema={} records=2 unsampled_log=true span_parent=0404040404040404 link=0202020202020202",
        path.display(),
        view.source_time_unix_nanos,
        view.observed_time_unix_nanos.unwrap(),
        resource.schema_url,
        scope.schema_url
    );
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
}
#[test]
fn retry_deduplicates_original_position_and_conflicting_record_or_sequence_is_refused() {
    let path = root("dedupe");
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = make_producer(&store, limits);
    let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
    let captured = producer.emit_log(key, log("stable")).unwrap();
    let bytes = captured.bytes;
    captured.pending.wait(WAIT).unwrap();
    let ingress = store.ingress();
    ingress
        .submit(Signal::Logs, &bytes)
        .unwrap()
        .wait(WAIT)
        .unwrap();
    let mut reader = DiagnosticReader::open(&path).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 1);
    let position = page.records[0].position;
    let mut request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
    request.resource_logs[0].scope_logs[0].log_records[0].severity_text = "conflict".into();
    assert_eq!(
        ingress
            .submit(Signal::Logs, &request.encode_to_vec())
            .unwrap()
            .wait(WAIT),
        Err(TelemetryError::Conflict)
    );
    let mut request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
    let record = &mut request.resource_logs[0].scope_logs[0].log_records[0];
    record
        .attributes
        .iter_mut()
        .find(|attribute| attribute.key == "df.record_id")
        .unwrap()
        .value = Some(WireValue {
        value: Some(Value::StringValue(RecordId::new([9; 16]).unwrap().hex())),
    });
    assert_eq!(
        ingress
            .submit(Signal::Logs, &request.encode_to_vec())
            .unwrap()
            .wait(WAIT),
        Err(TelemetryError::Conflict)
    );
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records[0].position, position);
    assert_eq!(page.records.len(), 1);
    assert_eq!(store.health().duplicates, 1);
    println!(
        "retry_conflict root={} stable_position={} duplicates=1 conflict=2",
        path.display(),
        position
    );
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
}
#[test]
fn sink_lock_spooled_receipt_lost_ack_gap_health_and_recovery_are_independent() {
    let path = root("outage");
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = make_producer(&store, limits);
    let lock = Connection::open(path.join("telemetry.sqlite3")).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
    let captured = producer.emit_log(key, log("outage")).unwrap();
    let bytes = captured.bytes;
    let receipt = captured.pending.wait(WAIT).unwrap();
    assert_eq!(receipt.durability, Durability::Spooled);
    assert_eq!(receipt.committed_watermark, 0);
    assert_eq!(store.health().checkpoint, 0);
    store.set_time(Duration::from_secs(31)).unwrap();
    assert_eq!(store.health().readiness, Readiness::Warning);
    store.set_time(Duration::from_secs(121)).unwrap();
    assert_eq!(store.health().readiness, Readiness::Degraded);
    let mut reader = DiagnosticReader::open(&path).unwrap();
    assert_eq!(
        reader
            .query(&QueryFilter::default(), 0, 100)
            .unwrap()
            .records
            .len(),
        0
    );
    lock.execute_batch("COMMIT").unwrap();
    store.flush(WAIT).unwrap();
    assert_eq!(store.health().readiness, Readiness::Healthy);
    drop(store.ingress().submit(Signal::Logs, &bytes).unwrap());
    store.flush(WAIT).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 1);
    assert!(page.emergency_source.is_some());
    assert!(!page.emergency_bytes.is_empty());
    assert_eq!(store.health().duplicates, 1);
    println!(
        "outage_recovery root={} spooled={} checkpoint={} emergency_bytes={} duplicate=1",
        path.display(),
        receipt.spool_position,
        store.health().checkpoint,
        page.emergency_bytes.len()
    );
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
}
#[test]
fn queue_item_and_byte_exhaustion_and_shutdown_deadline_use_controlled_barriers() {
    for byte_bound in [false, true] {
        let path = root(if byte_bound {
            "queue-bytes"
        } else {
            "queue-items"
        });
        let limits = TelemetryLimits {
            queue_items: if byte_bound { 8 } else { 1 },
            queue_bytes: if byte_bound { 4096 } else { 1048576 },
            ..Default::default()
        };
        let text = if byte_bound {
            "q".repeat(512)
        } else {
            "small".into()
        };
        let (gate, reached, release) = barrier(BarrierStage::BeforeSpool);
        let mut store = Store::open(&path, limits, Some(gate)).unwrap();
        let mut producer = make_producer(&store, limits);
        let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
        let first = producer.emit_log(key, log(&text)).unwrap();
        reached.recv_timeout(WAIT).unwrap();
        let key = producer.reserve(RecordId::new([9; 16]).unwrap()).unwrap();
        assert!(matches!(
            producer.emit_log(key, log(&text)),
            Err(TelemetryError::Capacity)
        ));
        assert!(store.health().capacity > 0);
        assert!(store.health().pending_items <= limits.queue_items);
        assert!(store.health().pending_bytes <= limits.queue_bytes);
        assert_eq!(
            store.shutdown(Duration::ZERO),
            Err(TelemetryError::Deadline)
        );
        release.send(()).unwrap();
        first.pending.wait(WAIT).unwrap();
        producer.shutdown(WAIT).unwrap();
        store.shutdown(WAIT).unwrap();
        println!(
            "queue_boundary root={} byte_boundary={} capacity={} shutdown_deadline=true",
            path.display(),
            byte_bound,
            store.health().capacity
        );
    }
}
#[test]
fn malformed_oversized_and_adversarial_nested_wire_are_refused_before_admission() {
    let path = root("wire-boundary");
    let mut store = Store::open(&path, TelemetryLimits::default(), None).unwrap();
    let ingress = store.ingress();
    assert!(matches!(
        ingress.submit(Signal::Logs, &vec![0; 65537]),
        Err(TelemetryError::Oversized)
    ));
    assert!(matches!(
        ingress.submit(Signal::Logs, &[10, 255, 255, 255, 255, 15]),
        Err(TelemetryError::Malformed)
    ));
    assert!(matches!(
        ingress.submit(Signal::Logs, &[0]),
        Err(TelemetryError::Malformed)
    ));
    // 33 zero-length records are refused by the allocation-free structural pass.
    let scopes = [18u8, 0].repeat(33);
    let resource = [vec![18, scopes.len() as u8], scopes].concat();
    let request = [vec![10, resource.len() as u8], resource].concat();
    assert!(matches!(
        ingress.submit(Signal::Logs, &request),
        Err(TelemetryError::Oversized)
    ));
    let mut deep = WireValue {
        value: Some(Value::BoolValue(true)),
    };
    for _ in 0..16 {
        deep = WireValue {
            value: Some(Value::ArrayValue(
                opentelemetry_proto::tonic::common::v1::ArrayValue { values: vec![deep] },
            )),
        };
    }
    let request = ExportLogsServiceRequest {
        resource_logs: vec![opentelemetry_proto::tonic::logs::v1::ResourceLogs {
            scope_logs: vec![opentelemetry_proto::tonic::logs::v1::ScopeLogs {
                log_records: vec![opentelemetry_proto::tonic::logs::v1::LogRecord {
                    body: Some(deep),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    assert!(matches!(
        ingress.submit(Signal::Logs, &request.encode_to_vec()),
        Err(TelemetryError::Oversized)
    ));
    let request = ExportLogsServiceRequest {
        resource_logs: vec![opentelemetry_proto::tonic::logs::v1::ResourceLogs {
            scope_logs: vec![opentelemetry_proto::tonic::logs::v1::ScopeLogs {
                log_records: vec![opentelemetry_proto::tonic::logs::v1::LogRecord {
                    body: Some(WireValue {
                        value: Some(Value::StringValue("s".repeat(8193))),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    assert!(matches!(
        ingress.submit(Signal::Logs, &request.encode_to_vec()),
        Err(TelemetryError::Oversized)
    ));
    assert_eq!(store.health().admitted, 0);
    assert_eq!(store.health().rejected, 6);
    store.shutdown(WAIT).unwrap();
    println!(
        "wire_boundary root={} admitted=0 rejected=6 maximum_supplied_bytes=65537 malicious_declared_length=4294967295 deep_value_levels=16",
        path.display()
    );
}
#[test]
fn spool_sqlite_capacity_corruption_truncation_and_foreign_ownership_are_explicit() {
    let path = root("spool-capacity");
    let limits = TelemetryLimits {
        spool_bytes: 128,
        ..Default::default()
    };
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = make_producer(&store, limits);
    let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
    assert_eq!(
        producer
            .emit_log(key, log("small"))
            .unwrap()
            .pending
            .wait(WAIT),
        Err(TelemetryError::Capacity)
    );
    assert_eq!(store.health().committed, 0);
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
    let path = root("sqlite-capacity");
    let limits = TelemetryLimits {
        sqlite_bytes: 262144,
        ..Default::default()
    };
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = make_producer(&store, limits);
    let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
    assert_eq!(
        producer
            .emit_log(key, log("small"))
            .unwrap()
            .pending
            .wait(WAIT)
            .unwrap()
            .durability,
        Durability::Spooled
    );
    assert_eq!(store.flush(WAIT), Err(TelemetryError::Capacity));
    producer.shutdown(WAIT).unwrap();
    assert_eq!(store.shutdown(WAIT), Err(TelemetryError::Capacity));
    for truncate in [false, true] {
        let path = root(if truncate { "truncated" } else { "corrupt" });
        let limits = TelemetryLimits::default();
        let mut store = Store::open(&path, limits, None).unwrap();
        let mut producer = make_producer(&store, limits);
        let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
        producer
            .emit_log(key, log("small"))
            .unwrap()
            .pending
            .wait(WAIT)
            .unwrap();
        producer.shutdown(WAIT).unwrap();
        store.shutdown(WAIT).unwrap();
        let frame = fs::read_dir(path.join("spool"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut bytes = fs::read(&frame).unwrap();
        if truncate {
            bytes.pop();
        } else {
            bytes[48] ^= 1;
        }
        fs::write(frame, bytes).unwrap();
        assert!(matches!(
            Store::open(&path, limits, None),
            Err(TelemetryError::Corrupt)
        ));
        println!("spool_refusal root={} truncated={truncate}", path.display());
    }
    let foreign = root("foreign");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("foreign-evidence"), b"preserve").unwrap();
    assert!(matches!(
        Store::open(&foreign, TelemetryLimits::default(), None),
        Err(TelemetryError::ForeignRoot)
    ));
    assert_eq!(
        fs::read(foreign.join("foreign-evidence")).unwrap(),
        b"preserve"
    );
    let owned = root("exclusive");
    let mut store = Store::open(&owned, TelemetryLimits::default(), None).unwrap();
    assert!(matches!(
        Store::open(&owned, TelemetryLimits::default(), None),
        Err(TelemetryError::Ownership)
    ));
    store.shutdown(WAIT).unwrap();
    let alias = foreign.join("foreign-database-alias");
    fs::hard_link(owned.join("telemetry.sqlite3"), &alias).unwrap();
    let before = fs::read(&alias).unwrap();
    assert!(matches!(
        Store::open(&owned, TelemetryLimits::default(), None),
        Err(TelemetryError::ForeignRoot)
    ));
    assert!(matches!(
        DiagnosticReader::open(&owned),
        Err(TelemetryError::ForeignRoot)
    ));
    assert_eq!(fs::read(&alias).unwrap(), before);
    // Creating a symlink is a Unix-only case; link-count and foreign-root
    // coverage above keeps running on Windows.
    #[cfg(unix)]
    {
        let symlink = root("symlink");
        std::os::unix::fs::symlink(&foreign, &symlink).unwrap();
        assert!(matches!(
            Store::open(&symlink, TelemetryLimits::default(), None),
            Err(TelemetryError::ForeignRoot)
        ));
    }
}
#[test]
fn read_only_filters_pagination_and_source_gaps_are_stable_during_ingestion() {
    let path = root("query");
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = make_producer(&store, limits);
    let mut reader = DiagnosticReader::open(&path).unwrap();
    for index in 1..=6u128 {
        let key = producer
            .reserve(RecordId::new(index.to_be_bytes()).unwrap())
            .unwrap();
        if index == 3 {
            continue;
        }
        producer
            .emit_log(key, log("query"))
            .unwrap()
            .pending
            .wait(WAIT)
            .unwrap();
        let page = reader
            .query(
                &QueryFilter {
                    producer: Some(key.producer),
                    session: Some("session-one".into()),
                    operation: Some("operation-one".into()),
                    minimum_severity: Some(9),
                    build: Some("sdk-roundtrip-v1".into()),
                    ..Default::default()
                },
                0,
                2,
            )
            .unwrap();
        assert!(page.records.len() <= 2);
        assert!(reader.verify_write_denied().unwrap());
    }
    let first = reader.query(&QueryFilter::default(), 0, 2).unwrap();
    let second = reader
        .query(&QueryFilter::default(), first.next_cursor, 2)
        .unwrap();
    let third = reader
        .query(&QueryFilter::default(), second.next_cursor, 2)
        .unwrap();
    assert_eq!(
        first.records.len() + second.records.len() + third.records.len(),
        5
    );
    assert!(first.records.last().unwrap().position < second.records[0].position);
    assert_eq!(third.source_watermarks[0].missing_through_highest, 1);
    assert!(reader.query(&QueryFilter::default(), 0, 101).is_err());
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
    println!(
        "readonly_query root={} records=5 page_sizes=2,2,1 gap_sequence=3 write_denied=true",
        path.display()
    );
}
#[test]
fn source_sequence_nonzero_and_checked_exhaustion_preserve_distinct_id_kinds() {
    assert!(ProducerId::new([0; 16]).is_err());
    assert!(RecordId::new([0; 16]).is_err());
    assert!(SourceSequence::new(0).is_err());
    assert!(SourceSequence::new(-1).is_err());
    assert!(ProducerId::from_hex("A0000000000000000000000000000000").is_err());
    let path = root("sequence");
    let mut store = Store::open(&path, TelemetryLimits::default(), None).unwrap();
    let mut producer = NativeProducer::new(
        ProducerId::new([1; 16]).unwrap(),
        SourceSequence::new(i64::MAX).unwrap(),
        &build(),
        store.ingress(),
        TelemetryLimits::default(),
    )
    .unwrap();
    let key = producer.reserve(RecordId::new([1; 16]).unwrap()).unwrap();
    assert_eq!(key.sequence.get(), i64::MAX);
    assert_eq!(
        producer.reserve(RecordId::new([2; 16]).unwrap()),
        Err(TelemetryError::SequenceExhausted)
    );
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
}

#[test]
fn every_enabled_sdk_log_level_is_captured_with_an_unsampled_trace() {
    let path = root("enabled-levels");
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = make_producer(&store, limits);
    for (index, severity) in [
        Severity::Trace,
        Severity::Debug,
        Severity::Info,
        Severity::Warn,
        Severity::Error,
        Severity::Fatal,
    ]
    .into_iter()
    .enumerate()
    {
        let key = producer
            .reserve(RecordId::new((index as u128 + 1).to_be_bytes()).unwrap())
            .unwrap();
        let mut input = log("enabled");
        input.severity = severity;
        input.trace = Some(SpanContext::new(
            TraceId::from_bytes([1; 16]),
            SpanId::from_bytes([2; 8]),
            TraceFlags::NOT_SAMPLED,
            true,
            TraceState::default(),
        ));
        producer
            .emit_log(key, input)
            .unwrap()
            .pending
            .wait(WAIT)
            .unwrap();
    }
    let mut reader = DiagnosticReader::open(&path).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 6);
    for (index, view) in page.records.iter().enumerate() {
        let request = ExportLogsServiceRequest::decode(view.otlp.as_slice()).unwrap();
        let record = &request.resource_logs[0].scope_logs[0].log_records[0];
        assert_eq!(record.severity_number, index as i32 * 4 + 1);
        assert_eq!(record.flags & 1, 0);
    }
    println!(
        "enabled_levels root={} trace_sampled=false logs=6 levels=trace,debug,info,warn,error,fatal",
        path.display()
    );
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
}

#[test]
fn sdk_rejects_aggregate_empty_log_and_span_structure_without_admission() {
    fn tree(depth: usize) -> AnyValue {
        if depth == 0 {
            AnyValue::String("".into())
        } else {
            AnyValue::ListAny(Box::new((0..16).map(|_| tree(depth - 1)).collect()))
        }
    }
    let path = root("structural-budget");
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = make_producer(&store, limits);
    let key = producer.reserve(RecordId::new([1; 16]).unwrap()).unwrap();
    let mut input = log("");
    input.attributes.clear();
    input.body = tree(2);
    producer
        .emit_log(key, input)
        .unwrap()
        .pending
        .wait(WAIT)
        .unwrap();
    for (index, depth) in [3, 4, 2].into_iter().enumerate() {
        let key = producer
            .reserve(RecordId::new([index as u8 + 2; 16]).unwrap())
            .unwrap();
        let mut input = log("");
        input.attributes.clear();
        input.body = tree(depth);
        if depth == 2 {
            input.attributes.push(("additional".into(), tree(2)));
        }
        assert!(matches!(
            producer.emit_log(key, input),
            Err(TelemetryError::Oversized)
        ));
    }
    let key = producer.reserve(RecordId::new([5; 16]).unwrap()).unwrap();
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
    let attributes: Vec<KeyValue> = (0..4)
        .map(|index| {
            KeyValue::new(
                format!("array-{index}"),
                opentelemetry::Value::Array(opentelemetry::Array::String(vec!["".into(); 64])),
            )
        })
        .collect();
    let context = SpanContext::new(
        TraceId::from_bytes([1; 16]),
        SpanId::from_bytes([2; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    let input = SpanInput {
        name: "aggregate-span".into(),
        start: time,
        end: time,
        parent: Context::new(),
        links: vec![Link::new(context, attributes.clone(), 0)],
        attributes,
    };
    assert!(matches!(
        producer.emit_span(key, input),
        Err(TelemetryError::Oversized)
    ));
    assert_eq!(store.health().admitted, 1);
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
    assert_eq!(
        DiagnosticReader::open(&path)
            .unwrap()
            .query(&QueryFilter::default(), 0, 100)
            .unwrap()
            .records
            .len(),
        1
    );
}

#[test]
fn checkpoint_coverage_and_actual_sqlite_content_are_validated_on_reopen() {
    for content_corruption in [false, true] {
        let path = root(if content_corruption {
            "sql-content-corrupt"
        } else {
            "checkpoint-missing-record"
        });
        let limits = TelemetryLimits::default();
        let mut store = Store::open(&path, limits, None).unwrap();
        let mut producer = make_producer(&store, limits);
        for id in [1, 2] {
            let key = producer.reserve(RecordId::new([id; 16]).unwrap()).unwrap();
            producer
                .emit_log(key, log("accepted"))
                .unwrap()
                .pending
                .wait(WAIT)
                .unwrap();
        }
        producer.shutdown(WAIT).unwrap();
        store.shutdown(WAIT).unwrap();
        assert_eq!(
            u64::from_be_bytes(
                fs::read(path.join("checkpoint"))
                    .unwrap()
                    .try_into()
                    .unwrap()
            ),
            2
        );
        let database = Connection::open(path.join("telemetry.sqlite3")).unwrap();
        if content_corruption {
            database
                .execute(
                    "UPDATE records SET otlp=zeroblob(length(otlp)) WHERE sequence=2",
                    [],
                )
                .unwrap();
        } else {
            database
                .execute("DELETE FROM records WHERE sequence=2", [])
                .unwrap();
        }
        database.close().unwrap();
        assert!(matches!(
            Store::open(&path, limits, None),
            Err(TelemetryError::Corrupt)
        ));
        assert_eq!(fs::read_dir(path.join("spool")).unwrap().count(), 2);
        assert_eq!(
            u64::from_be_bytes(
                fs::read(path.join("checkpoint"))
                    .unwrap()
                    .try_into()
                    .unwrap()
            ),
            2
        );
    }
}
