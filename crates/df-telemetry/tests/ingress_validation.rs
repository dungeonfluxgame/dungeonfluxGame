#![cfg(not(target_arch = "wasm32"))]
use df_observe::{Durability, Signal, TelemetryError, TelemetryLimits};
use df_telemetry::{BarrierStage, DiagnosticReader, FailureBarrier, QueryFilter, Store};
use opentelemetry_proto::tonic::{
    collector::{logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest},
    common::v1::{AnyValue, ArrayValue, KeyValue, any_value::Value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
    resource::v1::Resource,
    trace::v1::{
        ResourceSpans, ScopeSpans, Span,
        span::{Event, Link},
    },
};
use prost::Message;
use std::{
    path::PathBuf,
    sync::mpsc,
    time::{Duration, SystemTime},
};

const COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);

fn fixture_root(case_name: &str) -> PathBuf {
    assert!(
        [
            "ingress-normal",
            "ingress-byte-boundaries",
            "ingress-field-boundaries",
            "ingress-depth-boundaries",
            "ingress-identity-boundaries",
            "ingress-malformed-boundaries",
            "ingress-queue-cost",
            "ingress-queue-rollback",
        ]
        .contains(&case_name)
    );
    let namespace = std::env::var("DF_TELEMETRY_FIXTURE_NAMESPACE").unwrap_or_else(|_| {
        format!(
            "i01-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    });
    assert!(!namespace.is_empty());
    assert!(
        namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    );
    let path =
        PathBuf::from("/Users/earlcameron/Desktop/dungeonflux/development/runtime/telemetry-g06")
            .join(format!("ownedfixture-{namespace}-{case_name}"));
    assert!(path.file_name().unwrap().len() <= 100);
    assert!(
        !path.exists(),
        "fixture root must be fresh: {}",
        path.display()
    );
    path
}

fn attribute(name: &str, value: Value) -> KeyValue {
    KeyValue {
        key: name.into(),
        value: Some(AnyValue { value: Some(value) }),
    }
}

fn identities(record: u8, sequence: i64) -> Vec<KeyValue> {
    vec![
        attribute("df.producer_id", Value::StringValue("07".repeat(16))),
        attribute(
            "df.record_id",
            Value::StringValue(format!("{record:02x}").repeat(16)),
        ),
        attribute("df.source_sequence", Value::IntValue(sequence)),
    ]
}

fn record(id: u8) -> LogRecord {
    LogRecord {
        attributes: identities(id, i64::from(id)),
        ..Default::default()
    }
}

fn logs(records: Vec<LogRecord>) -> ExportLogsServiceRequest {
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: records,
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn spans(span: Span) -> ExportTraceServiceRequest {
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans: vec![span],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn accepted(store: &Store, signal: Signal, bytes: &[u8], expected_record_count: usize) {
    let receipt = store
        .ingress()
        .submit(signal, bytes)
        .unwrap()
        .wait(COMPLETION_TIMEOUT)
        .unwrap();
    assert_eq!(receipt.durability, Durability::Committed);
    assert_eq!(receipt.records, expected_record_count);
}

fn refused(store: &Store, signal: Signal, bytes: &[u8], expected_error: TelemetryError) {
    let before = store.health();
    assert!(
        matches!(store.ingress().submit(signal, bytes), Err(actual) if actual == expected_error)
    );
    let after = store.health();
    assert_eq!(after.admitted, before.admitted);
    assert_eq!(after.pending_items, before.pending_items);
    assert_eq!(after.pending_bytes, before.pending_bytes);
    assert_eq!(after.spooled, before.spooled);
    assert_eq!(after.committed, before.committed);
    assert_eq!(after.rejected, before.rejected + 1);
    assert_eq!(after.unconfirmed_gaps, before.unconfirmed_gaps + 1);
    assert_eq!(
        after.capacity,
        before.capacity + u64::from(expected_error == TelemetryError::Capacity)
    );
}

fn varint(mut value: u64, bytes: &mut Vec<u8>) {
    while value >= 128 {
        bytes.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn nested(field_number: u64, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    varint(field_number << 3 | 2, &mut bytes);
    varint(payload.len() as u64, &mut bytes);
    bytes.extend(payload);
    bytes
}

#[test]
fn ordinary_logs_and_spans_preserve_typed_events_links_and_maximum_sequence() {
    let path = fixture_root("ingress-normal");
    let mut store = Store::open(&path, TelemetryLimits::default(), None).unwrap();
    let mut log = record(1);
    log.attributes[2] = attribute("df.source_sequence", Value::IntValue(i64::MAX));
    log.body = Some(AnyValue {
        value: Some(Value::IntValue(i64::MIN)),
    });
    let bytes = logs(vec![log]).encode_to_vec();
    accepted(&store, Signal::Logs, &bytes, 1);
    let span = Span {
        attributes: identities(2, 1),
        trace_id: vec![1; 16],
        span_id: vec![2; 8],
        parent_span_id: vec![3; 8],
        name: "normal".into(),
        events: vec![Event {
            name: "event".into(),
            attributes: vec![attribute("typed", Value::BoolValue(true))],
            ..Default::default()
        }],
        links: vec![Link {
            trace_id: vec![4; 16],
            span_id: vec![5; 8],
            ..Default::default()
        }],
        ..Default::default()
    };
    let span_bytes = spans(span).encode_to_vec();
    accepted(&store, Signal::Spans, &span_bytes, 1);
    let page = DiagnosticReader::open(&path)
        .unwrap()
        .query(&QueryFilter::default(), 0, 10)
        .unwrap();
    assert_eq!(page.records.len(), 2);
    assert_eq!(page.records[0].otlp, bytes);
    assert_eq!(page.records[1].otlp, span_bytes);
    store.shutdown(COMPLETION_TIMEOUT).unwrap();
}

#[test]
fn byte_and_record_caps_accept_the_edge_and_refuse_the_next_byte_or_record() {
    let mut store = Store::open(
        &fixture_root("ingress-byte-boundaries"),
        TelemetryLimits::default(),
        None,
    )
    .unwrap();
    let mut request = logs(vec![record(1)]);
    request.resource_logs[0].scope_logs[0].log_records[0].body = Some(AnyValue {
        value: Some(Value::BytesValue(Vec::new())),
    });
    if let Some(Value::BytesValue(body)) = request.resource_logs[0].scope_logs[0].log_records[0]
        .body
        .as_mut()
        .unwrap()
        .value
        .as_mut()
    {
        *body = vec![0; 8192];
    }
    let overhead = request.encoded_len() - 8192;
    if let Some(Value::BytesValue(body)) = request.resource_logs[0].scope_logs[0].log_records[0]
        .body
        .as_mut()
        .unwrap()
        .value
        .as_mut()
    {
        body.truncate(8192 - overhead);
    }
    assert_eq!(request.encode_to_vec().len(), 8192);
    accepted(&store, Signal::Logs, &request.encode_to_vec(), 1);
    if let Some(Value::BytesValue(body)) = request.resource_logs[0].scope_logs[0].log_records[0]
        .body
        .as_mut()
        .unwrap()
        .value
        .as_mut()
    {
        body.push(0);
    }
    refused(
        &store,
        Signal::Logs,
        &request.encode_to_vec(),
        TelemetryError::Oversized,
    );

    let mut bytes = logs(vec![record(2)]).encode_to_vec();
    let raw = nested(100, &vec![0; 65536 - bytes.len() - 5]);
    bytes.extend(raw);
    assert_eq!(bytes.len(), 65536);
    accepted(&store, Signal::Logs, &bytes, 1);
    bytes.push(0);
    refused(&store, Signal::Logs, &bytes, TelemetryError::Oversized);
    accepted(
        &store,
        Signal::Logs,
        &logs((3..35).map(record).collect()).encode_to_vec(),
        32,
    );
    refused(
        &store,
        Signal::Logs,
        &logs((35..68).map(record).collect()).encode_to_vec(),
        TelemetryError::Oversized,
    );
    store.shutdown(COMPLETION_TIMEOUT).unwrap();
}

#[test]
fn message_event_and_aggregate_fields_share_the_existing_structural_budgets() {
    let mut store = Store::open(
        &fixture_root("ingress-field-boundaries"),
        TelemetryLimits::default(),
        None,
    )
    .unwrap();
    let mut span = Span {
        attributes: identities(1, 1),
        events: vec![Event::default(); 125],
        ..Default::default()
    };
    accepted(
        &store,
        Signal::Spans,
        &spans(span.clone()).encode_to_vec(),
        1,
    );
    span.events.push(Event::default());
    refused(
        &store,
        Signal::Spans,
        &spans(span).encode_to_vec(),
        TelemetryError::Oversized,
    );
    let mut log = record(2);
    log.attributes.extend(vec![KeyValue::default(); 125]);
    accepted(
        &store,
        Signal::Logs,
        &logs(vec![log.clone()]).encode_to_vec(),
        1,
    );
    log.attributes.push(KeyValue::default());
    refused(
        &store,
        Signal::Logs,
        &logs(vec![log]).encode_to_vec(),
        TelemetryError::Oversized,
    );
    let mut scope = Vec::new();
    // 34 enclosing fields + 32 * 12 identity fields + 3678 unknown scalars = 4096.
    for index in 0..32 {
        let mut bytes = record(index + 3).encode_to_vec();
        for _ in 0..(114 + usize::from(index < 30)) {
            bytes.extend([0xa0, 6, 0]);
        }
        scope.extend(nested(2, &bytes));
    }
    let bytes = nested(1, &nested(2, &scope));
    accepted(&store, Signal::Logs, &bytes, 32);
    scope.extend([0xa0, 6, 0]);
    refused(
        &store,
        Signal::Logs,
        &nested(1, &nested(2, &scope)),
        TelemetryError::Oversized,
    );
    store.shutdown(COMPLETION_TIMEOUT).unwrap();
}

#[test]
fn nested_values_accept_depth_twelve_and_refuse_deeper_allocations() {
    let mut store = Store::open(
        &fixture_root("ingress-depth-boundaries"),
        TelemetryLimits::default(),
        None,
    )
    .unwrap();
    let mut value = AnyValue {
        value: Some(Value::BoolValue(true)),
    };
    for _ in 0..4 {
        value = AnyValue {
            value: Some(Value::ArrayValue(ArrayValue {
                values: vec![value],
            })),
        };
    }
    let mut log = record(1);
    log.body = Some(value.clone());
    accepted(
        &store,
        Signal::Logs,
        &logs(vec![log.clone()]).encode_to_vec(),
        1,
    );
    log.body = Some(AnyValue {
        value: Some(Value::ArrayValue(ArrayValue {
            values: vec![value],
        })),
    });
    refused(
        &store,
        Signal::Logs,
        &logs(vec![log]).encode_to_vec(),
        TelemetryError::Oversized,
    );
    store.shutdown(COMPLETION_TIMEOUT).unwrap();
}

#[test]
fn required_capture_identities_refuse_missing_duplicate_mistyped_and_invalid_values() {
    let mut store = Store::open(
        &fixture_root("ingress-identity-boundaries"),
        TelemetryLimits::default(),
        None,
    )
    .unwrap();
    for index in 0..3 {
        let mut log = record(1);
        log.attributes.remove(index);
        refused(
            &store,
            Signal::Logs,
            &logs(vec![log]).encode_to_vec(),
            TelemetryError::InvalidIdentity,
        );
        let mut log = record(1);
        log.attributes.push(log.attributes[index].clone());
        refused(
            &store,
            Signal::Logs,
            &logs(vec![log]).encode_to_vec(),
            TelemetryError::InvalidIdentity,
        );
        let mut log = record(1);
        log.attributes[index].value = Some(AnyValue {
            value: Some(Value::BoolValue(true)),
        });
        refused(
            &store,
            Signal::Logs,
            &logs(vec![log]).encode_to_vec(),
            TelemetryError::InvalidIdentity,
        );
    }
    for index in 0..2 {
        for value in [
            "".into(),
            "a".repeat(31),
            "a".repeat(33),
            "A".repeat(32),
            "g".repeat(32),
            "0".repeat(32),
        ] {
            let mut log = record(1);
            log.attributes[index].value = Some(AnyValue {
                value: Some(Value::StringValue(value)),
            });
            refused(
                &store,
                Signal::Logs,
                &logs(vec![log]).encode_to_vec(),
                TelemetryError::InvalidIdentity,
            );
        }
    }
    for sequence in [0, -1, i64::MIN] {
        let mut log = record(1);
        log.attributes[2] = attribute("df.source_sequence", Value::IntValue(sequence));
        refused(
            &store,
            Signal::Logs,
            &logs(vec![log]).encode_to_vec(),
            TelemetryError::InvalidIdentity,
        );
    }
    let mut span = Span {
        attributes: identities(1, 1),
        ..Default::default()
    };
    span.attributes.push(span.attributes[0].clone());
    refused(
        &store,
        Signal::Spans,
        &spans(span).encode_to_vec(),
        TelemetryError::InvalidIdentity,
    );
    accepted(
        &store,
        Signal::Logs,
        &logs(vec![record(1)]).encode_to_vec(),
        1,
    );
    assert_eq!(store.health().admitted, 1);
    store.shutdown(COMPLETION_TIMEOUT).unwrap();
}

#[test]
fn malformed_wire_and_bad_correlation_lengths_refuse_without_reserving_the_queue() {
    let mut store = Store::open(
        &fixture_root("ingress-malformed-boundaries"),
        TelemetryLimits::default(),
        None,
    )
    .unwrap();
    for bytes in [
        vec![],
        vec![0],
        vec![0x80],
        vec![8, 0x80],
        vec![0x0b],
        vec![0x0f],
        vec![10, 4, 0],
        vec![9, 0],
        vec![13, 0],
        [vec![8], vec![0xff; 9], vec![2]].concat(),
    ] {
        refused(&store, Signal::Logs, &bytes, TelemetryError::Malformed);
    }
    for (trace, span) in [(vec![1; 15], vec![2; 8]), (vec![1; 16], vec![2; 7])] {
        let mut log = record(1);
        log.trace_id = trace;
        log.span_id = span;
        refused(
            &store,
            Signal::Logs,
            &logs(vec![log]).encode_to_vec(),
            TelemetryError::Malformed,
        );
    }
    let mut bytes = logs(vec![record(1)]).encode_to_vec();
    bytes.extend(nested(100, &[0, 255, 128]));
    accepted(&store, Signal::Logs, &bytes, 1);
    store.shutdown(COMPLETION_TIMEOUT).unwrap();
}

fn gated_store(
    case_name: &str,
    limits: TelemetryLimits,
) -> (Store, mpsc::Receiver<BarrierStage>, mpsc::Sender<()>) {
    let (reached, receiver) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::channel();
    let store = Store::open(
        &fixture_root(case_name),
        limits,
        Some(FailureBarrier {
            stage: BarrierStage::BeforeSpool,
            reached,
            release: gate,
        }),
    )
    .unwrap();
    (store, receiver, release)
}

#[test]
fn retained_projection_bytes_are_charged_and_byte_refusal_rolls_back_items() {
    let mut request = logs(vec![record(1)]);
    let log = &mut request.resource_logs[0].scope_logs[0].log_records[0];
    log.trace_id = vec![1; 16];
    log.span_id = vec![2; 8];
    log.attributes.extend([
        attribute("df.session", Value::StringValue("s".repeat(256))),
        attribute("df.operation", Value::StringValue("o".repeat(256))),
    ]);
    request.resource_logs[0].resource = Some(Resource {
        attributes: vec![attribute("df.build", Value::StringValue("b".repeat(256)))],
        ..Default::default()
    });
    let bytes = request.encode_to_vec();
    let cost = bytes.len() * 2 + 256 + 768 + 24;
    let limits = TelemetryLimits {
        queue_items: 2,
        queue_bytes: cost,
        ..Default::default()
    };
    let (mut store, reached, release) = gated_store("ingress-queue-cost", limits);
    let pending = store.ingress().submit(Signal::Logs, &bytes).unwrap();
    reached.recv_timeout(COMPLETION_TIMEOUT).unwrap();
    assert_eq!(store.health().pending_bytes, cost);
    assert_eq!(store.health().pending_items, 1);
    refused(&store, Signal::Logs, &bytes, TelemetryError::Capacity);
    release.send(()).unwrap();
    pending.wait(COMPLETION_TIMEOUT).unwrap();
    store.flush(COMPLETION_TIMEOUT).unwrap();
    assert_eq!(store.health().pending_bytes, 0);
    assert_eq!(store.health().pending_items, 0);
    store.shutdown(COMPLETION_TIMEOUT).unwrap();
}

#[test]
fn capacity_refusal_leaves_space_for_a_smaller_valid_admission() {
    let limits = TelemetryLimits {
        queue_items: 1,
        queue_bytes: 1024,
        ..Default::default()
    };
    let (mut store, reached, release) = gated_store("ingress-queue-rollback", limits);
    let mut large = record(1);
    large.body = Some(AnyValue {
        value: Some(Value::StringValue("x".repeat(600))),
    });
    refused(
        &store,
        Signal::Logs,
        &logs(vec![large]).encode_to_vec(),
        TelemetryError::Capacity,
    );
    assert_eq!(store.health().pending_items, 0);
    let bytes = logs(vec![record(2)]).encode_to_vec();
    let pending = store.ingress().submit(Signal::Logs, &bytes).unwrap();
    reached.recv_timeout(COMPLETION_TIMEOUT).unwrap();
    refused(&store, Signal::Logs, &bytes, TelemetryError::Capacity);
    release.send(()).unwrap();
    pending.wait(COMPLETION_TIMEOUT).unwrap();
    store.flush(COMPLETION_TIMEOUT).unwrap();
    assert_eq!(store.health().pending_items, 0);
    assert_eq!(store.health().pending_bytes, 0);
    store.shutdown(COMPLETION_TIMEOUT).unwrap();
}
