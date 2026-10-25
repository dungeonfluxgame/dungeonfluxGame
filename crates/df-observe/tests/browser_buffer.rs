use df_observe::{
    BrowserBuffer, BrowserLog, BufferError, CaptureKey, ProducerId, RecordId, SourceSequence,
    UploadFailure,
};
use df_types::{BuildIdentity, OperationId, SessionId};
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::any_value::Value,
    logs::v1::{LogRecord, SeverityNumber},
};
use prost::Message;
use std::time::Duration;

fn build_identity() -> BuildIdentity {
    BuildIdentity::new(
        Some("source"),
        Some("native"),
        Some("wasm"),
        Some("config"),
        Some("content"),
    )
    .unwrap()
}
fn capture_key(sequence: i64) -> CaptureKey {
    let mut id = [0; 16];
    id[..8].copy_from_slice(&sequence.to_be_bytes());
    CaptureKey {
        producer: ProducerId::new([1; 16]).unwrap(),
        record: RecordId::new(id).unwrap(),
        sequence: SourceSequence::new(sequence).unwrap(),
    }
}
fn browser_log() -> BrowserLog {
    BrowserLog {
        timestamp_unix_nanos: 1,
        observed_unix_nanos: 2,
        severity: SeverityNumber::Info,
        event_name: "browser.fixture",
        status: "complete",
        measured_bytes: Some(0),
        session: Some(SessionId::from_bytes(&[2; 16]).unwrap()),
        operation: Some(OperationId::from_bytes(&[3; 16]).unwrap()),
        trace: Some(SpanContext::new(
            TraceId::from_bytes([4; 16]),
            SpanId::from_bytes([5; 8]),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        )),
    }
}
fn decode_log_record(bytes: &[u8]) -> LogRecord {
    let request = ExportLogsServiceRequest::decode(bytes).unwrap();
    assert_eq!(request.resource_logs.len(), 1);
    let resource = &request.resource_logs[0];
    assert_eq!(resource.scope_logs.len(), 1);
    assert_eq!(resource.resource.as_ref().unwrap().attributes.len(), 6);
    assert_eq!(resource.scope_logs[0].log_records.len(), 1);
    resource.scope_logs[0].log_records[0].clone()
}
#[test]
fn standard_otlp_capture_contains_only_approved_typed_fields_and_identity() {
    let mut buffer = BrowserBuffer::new(1);
    let admitted_key = capture_key(7);
    buffer
        .retain_log(&build_identity(), admitted_key, browser_log())
        .unwrap();
    let lease = buffer.begin_upload(Duration::ZERO).unwrap();
    let bytes = buffer.upload_records(&lease).unwrap().next().unwrap();
    let log = decode_log_record(bytes);
    assert!(log.body.is_none());
    assert_eq!(log.time_unix_nano, 1);
    assert_eq!(log.observed_time_unix_nano, 2);
    assert_eq!(log.trace_id, [4; 16]);
    assert_eq!(log.span_id, [5; 8]);
    assert_eq!(log.flags, 1);
    assert_eq!(log.event_name, "browser.fixture");
    let attributes: std::collections::BTreeMap<_, _> = log
        .attributes
        .iter()
        .map(|entry| {
            (
                entry.key.as_str(),
                entry.value.as_ref().unwrap().value.as_ref().unwrap(),
            )
        })
        .collect();
    assert_eq!(attributes.len(), 9);
    assert_eq!(
        attributes["df.producer_id"],
        &Value::StringValue(admitted_key.producer.hex())
    );
    assert_eq!(
        attributes["df.record_id"],
        &Value::StringValue(admitted_key.record.hex())
    );
    assert_eq!(attributes["df.source_sequence"], &Value::IntValue(7));
    assert_eq!(attributes["rpc.bytes"], &Value::IntValue(0));
    assert_eq!(attributes["rpc.bytes.measured"], &Value::BoolValue(true));
    assert_eq!(
        attributes["df.session"],
        &Value::StringValue("02".repeat(16))
    );
    assert_eq!(
        attributes["df.operation"],
        &Value::StringValue("03".repeat(16))
    );
    assert_eq!(
        attributes["fixture.build"],
        &Value::StringValue("source".into())
    );
    assert_eq!(
        attributes["rpc.status"],
        &Value::StringValue("complete".into())
    );
}
#[test]
fn item_overflow_is_visible_and_never_evicts_existing_records() {
    let mut buffer = BrowserBuffer::new(1);
    for sequence in 1..=128 {
        buffer
            .retain_log(&build_identity(), capture_key(sequence), browser_log())
            .unwrap();
    }
    let before = buffer.snapshot();
    assert_eq!(
        buffer.retain_log(&build_identity(), capture_key(129), browser_log()),
        Err(BufferError::Capacity)
    );
    let after = buffer.snapshot();
    assert_eq!(after.retained_records, 128);
    assert_eq!(after.retained_bytes, before.retained_bytes);
    assert_eq!(after.refused_records, 1);
    let lease = buffer.begin_upload(Duration::ZERO).unwrap();
    assert_eq!(lease.records(), 32);
    assert!(lease.bytes() <= 65536);
    let sequences: Vec<_> = buffer
        .upload_records(&lease)
        .unwrap()
        .map(|bytes| {
            let log = decode_log_record(bytes);
            log.attributes
                .into_iter()
                .find(|entry| entry.key == "df.source_sequence")
                .unwrap()
                .value
                .unwrap()
                .value
                .unwrap()
        })
        .collect();
    assert_eq!(sequences.first(), Some(&Value::IntValue(1)));
    assert_eq!(sequences.last(), Some(&Value::IntValue(32)));
}
#[test]
fn deadline_failure_and_retry_preserve_original_bytes() {
    let mut buffer = BrowserBuffer::new(4);
    buffer
        .retain_log(&build_identity(), capture_key(1), browser_log())
        .unwrap();
    let first = buffer.begin_upload(Duration::from_secs(2)).unwrap();
    let original = buffer
        .upload_records(&first)
        .unwrap()
        .next()
        .unwrap()
        .to_vec();
    assert_eq!(first.deadline(), Duration::from_secs(12));
    assert_eq!(
        buffer.expire_upload(Duration::from_secs(11)),
        Err(BufferError::NotExpired)
    );
    assert_eq!(
        buffer.acknowledge_durable(&first, Duration::from_secs(12)),
        Err(BufferError::Deadline)
    );
    buffer.expire_upload(Duration::from_secs(12)).unwrap();
    for outcome in [
        UploadFailure::Transport,
        UploadFailure::Cancelled,
        UploadFailure::PartialOrRejected,
    ] {
        let lease = buffer.begin_upload(Duration::from_secs(12)).unwrap();
        assert_eq!(
            buffer.upload_records(&lease).unwrap().next().unwrap(),
            original
        );
        assert_eq!(buffer.fail_upload(&lease, outcome), Ok(outcome));
        assert_eq!(buffer.snapshot().retained_records, 1);
        assert_eq!(
            buffer.acknowledge_durable(&lease, Duration::from_secs(13)),
            Err(BufferError::StaleLease)
        );
    }
    let retry = buffer.begin_upload(Duration::from_secs(14)).unwrap();
    assert_eq!(
        buffer.acknowledge_durable(&first, Duration::from_secs(15)),
        Err(BufferError::StaleLease)
    );
    buffer
        .acknowledge_durable(&retry, Duration::from_secs(15))
        .unwrap();
    assert_eq!(buffer.snapshot().retained_bytes, 0);
}
#[test]
fn prefix_acknowledgment_preserves_later_append_and_fifo_wrap() {
    let mut buffer = BrowserBuffer::new(1);
    for sequence in 1..=128 {
        buffer
            .retain_log(&build_identity(), capture_key(sequence), browser_log())
            .unwrap();
    }
    for sequence in 129..=256 {
        let lease = buffer.begin_upload(Duration::ZERO).unwrap();
        buffer
            .acknowledge_durable(&lease, Duration::from_secs(1))
            .unwrap();
        buffer
            .retain_log(&build_identity(), capture_key(sequence), browser_log())
            .unwrap();
    }
    let lease = buffer.begin_upload(Duration::ZERO).unwrap();
    buffer
        .retain_log(&build_identity(), capture_key(257), browser_log())
        .unwrap();
    let before = buffer.snapshot().retained_records;
    buffer
        .acknowledge_durable(&lease, Duration::from_secs(1))
        .unwrap();
    assert_eq!(buffer.snapshot().retained_records, before - lease.records());
}
#[test]
fn reconstructed_same_generation_owner_rejects_old_callback_but_moving_preserves_it() {
    let mut original = BrowserBuffer::new(20);
    original
        .retain_log(&build_identity(), capture_key(1), browser_log())
        .unwrap();
    let old = original.begin_upload(Duration::ZERO).unwrap();
    drop(original);
    let mut replacement = BrowserBuffer::new(20);
    replacement
        .retain_log(&build_identity(), capture_key(2), browser_log())
        .unwrap();
    let new = replacement.begin_upload(Duration::ZERO).unwrap();
    assert_eq!(
        replacement.acknowledge_durable(&old, Duration::from_secs(1)),
        Err(BufferError::StaleLease)
    );
    assert_eq!(replacement.snapshot().retained_records, 1);
    let mut moved = replacement;
    moved
        .acknowledge_durable(&new, Duration::from_secs(1))
        .unwrap();
    assert_eq!(moved.snapshot().retained_records, 0);
}
#[test]
fn rebind_reports_gap_and_same_generation_does_not_reuse_lease_identity() {
    let mut buffer = BrowserBuffer::new(1);
    buffer
        .retain_log(&build_identity(), capture_key(1), browser_log())
        .unwrap();
    let old = buffer.begin_upload(Duration::ZERO).unwrap();
    assert_eq!(buffer.invalidate(1), 1);
    buffer
        .retain_log(&build_identity(), capture_key(2), browser_log())
        .unwrap();
    let new = buffer.begin_upload(Duration::ZERO).unwrap();
    assert_eq!(
        buffer.fail_upload(&old, UploadFailure::Transport),
        Err(BufferError::StaleLease)
    );
    buffer
        .acknowledge_durable(&new, Duration::from_secs(1))
        .unwrap();
}
#[test]
fn capture_refusals_do_not_retain_input_or_fabricate_measurements() {
    let mut buffer = BrowserBuffer::new(1);
    let mut input = browser_log();
    input.event_name = "";
    assert_eq!(
        buffer.retain_log(&build_identity(), capture_key(1), input),
        Err(BufferError::Empty)
    );
    let mut input = browser_log();
    input.status = concat!(
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        "x"
    );
    assert_eq!(
        buffer.retain_log(&build_identity(), capture_key(2), input),
        Err(BufferError::Oversized)
    );
    let mut input = browser_log();
    input.measured_bytes = Some(-1);
    assert_eq!(
        buffer.retain_log(&build_identity(), capture_key(3), input),
        Err(BufferError::InvalidMeasurement)
    );
    assert_eq!(buffer.snapshot().retained_bytes, 0);
    assert_eq!(buffer.snapshot().refused_records, 3);
    assert!(matches!(
        buffer.begin_upload(Duration::ZERO),
        Err(BufferError::Empty)
    ));
    buffer
        .retain_log(&build_identity(), capture_key(4), browser_log())
        .unwrap();
    assert!(matches!(
        buffer.begin_upload(Duration::MAX),
        Err(BufferError::ClockOverflow)
    ));
    assert!(!buffer.snapshot().upload_active);
}

#[test]
fn largest_bounded_public_log_exposes_exact_retained_payload_accounting() {
    const NAME: &str = concat!(
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    );
    let label = "x".repeat(128);
    let build = BuildIdentity::new(
        Some(&label),
        Some(&label),
        Some(&label),
        Some(&label),
        Some(&label),
    )
    .unwrap();
    let mut buffer = BrowserBuffer::new(1);
    let mut input = browser_log();
    input.event_name = NAME;
    input.status = NAME;
    input.timestamp_unix_nanos = u64::MAX;
    input.observed_unix_nanos = u64::MAX;
    input.measured_bytes = Some(i64::MAX);
    buffer
        .retain_log(&build, capture_key(i64::MAX), input)
        .unwrap();
    let lease = buffer.begin_upload(Duration::ZERO).unwrap();
    let length = buffer.upload_records(&lease).unwrap().next().unwrap().len();
    assert_eq!(lease.bytes(), length);
    assert_eq!(buffer.snapshot().retained_bytes, length);
    assert!(length <= 8192);
    use std::io::Write;
    writeln!(
        std::io::stdout().lock(),
        "largest fixed-catalog public log encoded bytes: {length}"
    )
    .unwrap();
}
