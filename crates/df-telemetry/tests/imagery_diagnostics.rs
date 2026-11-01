#![cfg(not(target_arch = "wasm32"))]

use df_imagery::execution::{DispatchId, ExecutionStageId, StageKind};
use df_imagery::telemetry::{
    DeliveryId, EventContext, EventKind, ImageEvent, ImageLogLevel, ImageObserver,
    SharedImageObserver,
};
use df_imagery::{BackendId, ImageInterestId, ImageJobId, Purpose};
use df_observe::{
    DiagnosticRecord, DiagnosticSink, NativeDiagnosticSink, NativeProducer, PendingReceipt,
    ProducerId, Signal, SourceSequence, TelemetryError, TelemetryIngress, TelemetryLimits,
};
use df_telemetry::{DiagnosticReader, QueryFilter, Store};
use df_types::{BuildIdentity, OperationId};
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest, common::v1::any_value::Value,
};
use prost::Message;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::{Duration, SystemTime},
};

const WAIT: Duration = Duration::from_secs(5);
const PRIVATE_TRACE_STATE: &str = "private-trace-state-sentinel";

struct DebugCheckingSink(Arc<NativeDiagnosticSink>);

impl DiagnosticSink for DebugCheckingSink {
    fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
        assert!(!format!("{record:?}").contains(PRIVATE_TRACE_STATE));
        assert!(!format!("{:?}", record.input).contains(PRIVATE_TRACE_STATE));
        assert!(
            record
                .input
                .trace
                .as_ref()
                .is_none_or(|trace| trace.trace_state().header().is_empty())
        );
        self.0.emit(record)
    }
}

fn next_record() -> [u8; 16] {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    u128::from(NEXT.fetch_add(1, Ordering::Relaxed)).to_be_bytes()
}

type HeldReceipt = (
    PendingReceipt,
    Sender<Result<df_observe::IngestReceipt, TelemetryError>>,
);

#[derive(Debug)]
struct HeldIngress {
    target: Arc<dyn TelemetryIngress>,
    receipts: Mutex<Vec<HeldReceipt>>,
}

impl TelemetryIngress for HeldIngress {
    fn submit(&self, signal: Signal, bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        let durable = self.target.submit(signal, bytes)?;
        let (sender, pending) = PendingReceipt::channel();
        self.receipts.lock().unwrap().push((durable, sender));
        Ok(pending)
    }
}

impl HeldIngress {
    fn confirm(&self) {
        let (durable, reply) = self.receipts.lock().unwrap().remove(0);
        reply.send(durable.wait(WAIT)).unwrap();
    }
}

fn root() -> PathBuf {
    let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("artifacts/tmp/imagery-finish-20261029/logging/telemetry-fixtures");
    std::fs::create_dir_all(&parent).unwrap();
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    parent.join(format!("imagery-{}-{nonce}", std::process::id()))
}

fn value<'a>(
    record: &'a opentelemetry_proto::tonic::logs::v1::LogRecord,
    name: &str,
) -> Option<&'a Value> {
    record.attributes.iter().find_map(|attribute| {
        (attribute.key == name).then_some(attribute.value.as_ref()?.value.as_ref()?)
    })
}

#[test]
fn unsampled_image_lifecycle_is_queryable_after_shared_ingress_commit_and_restart() {
    let root = root();
    let limits = TelemetryLimits::default();
    let mut store = Store::open_configured(&root, limits).unwrap();
    let build = BuildIdentity::new(
        Some("imagery-diagnostic-source"),
        Some("fixture-native"),
        Some("not-applicable"),
        Some("fixture-image-config"),
        Some("fixture-content"),
    )
    .unwrap();
    let producer = NativeProducer::for_service(
        ProducerId::new([1; 16]).unwrap(),
        SourceSequence::new(1).unwrap(),
        &build,
        store.ingress(),
        limits,
        "imagery-boundary-fixture",
    )
    .unwrap();
    let sink = Arc::new(NativeDiagnosticSink::new(producer, 64, next_record).unwrap());
    let observer = SharedImageObserver::new(Arc::new(DebugCheckingSink(sink.clone())));
    observer.set_level(ImageLogLevel::Debug);
    let operation = OperationId::from_bytes(&[2; 16]).unwrap();
    let context = EventContext {
        operation: Some(operation),
        request: Some(ImageInterestId::from_bytes(&[3; 16]).unwrap()),
        job: Some(ImageJobId::from_bytes(&[4; 16]).unwrap()),
        stage: Some(ExecutionStageId::from_bytes(&[5; 16]).unwrap()),
        attempt: Some(DispatchId::from_bytes(&[6; 16]).unwrap()),
        backend: Some(BackendId::CodexCli),
        kind: Some(StageKind::Generation),
        config_revision: Some(7),
        purpose: Some(Purpose::CreatorPortrait),
        trace: Some(
            SpanContext::new(
                TraceId::from_bytes([8; 16]),
                SpanId::from_bytes([9; 8]),
                TraceFlags::NOT_SAMPLED,
                true,
                TraceState::from_key_value([("vendor", PRIVATE_TRACE_STATE)]).unwrap(),
            )
            .into(),
        ),
        ..EventContext::default()
    };
    assert!(!format!("{:?}", context.trace).contains(PRIVATE_TRACE_STATE));
    assert!(!format!("{context:?}").contains(PRIVATE_TRACE_STATE));
    let kinds = [
        EventKind::Requested,
        EventKind::Queued,
        EventKind::Claimed,
        EventKind::Admitted,
        EventKind::Sent,
        EventKind::SupplierCheckpointed,
        EventKind::UsageRecorded,
        EventKind::NormalizationCompleted,
        EventKind::OutputStored,
        EventKind::Published,
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        let mut event =
            ImageEvent::new(kind, context.clone()).elapsed(Duration::from_nanos(index as u64 + 1));
        assert!(!format!("{event:?}").contains(PRIVATE_TRACE_STATE));
        if kind == EventKind::UsageRecorded {
            event.measurements.usage_revision = Some(df_imagery::cost::UsageRevision(1));
            event.measurements.usage_evidence = Some(df_imagery::cost::UsageEvidence::Unknown);
        }
        observer.observe(event);
    }
    let mut local = context.clone();
    local.kind = Some(StageKind::LocalTransform);
    local.backend = None;
    local.trace = None;
    observer.observe(
        ImageEvent::new(EventKind::TransformCompleted, local).elapsed(Duration::from_nanos(42)),
    );
    let mut delivery = context.clone();
    delivery.delivery = DeliveryId::from_bytes([10; 16]);
    observer.observe(ImageEvent::new(EventKind::DeliveryAuthorized, delivery));
    observer.set_level(ImageLogLevel::Off);
    observer.observe(ImageEvent::new(EventKind::Sent, context));
    assert_eq!(observer.health().filtered, 1);
    assert_eq!(observer.health().failures, 0);
    assert_eq!(observer.health().admitted, 12);
    sink.shutdown(WAIT).unwrap();
    let health = sink.health();
    assert_eq!(
        (health.admitted, health.confirmed, health.refused),
        (12, 12, 0)
    );
    assert_eq!(health.pending, 0);
    assert_eq!(health.receipt_failures, 0);
    store.shutdown(WAIT).unwrap();
    drop(store);

    let mut reopened = Store::open_configured(&root, limits).unwrap();
    let mut reader = DiagnosticReader::open(&root).unwrap();
    assert!(reader.verify_write_denied().unwrap());
    let page = reader
        .query(
            &QueryFilter {
                operation: Some("02".repeat(16)),
                ..QueryFilter::default()
            },
            0,
            64,
        )
        .unwrap();
    assert_eq!(page.records.len(), 12);
    for (index, record) in page.records.iter().enumerate() {
        assert_eq!(record.source_sequence, index as i64 + 1);
        let request = ExportLogsServiceRequest::decode(record.otlp.as_slice()).unwrap();
        assert!(
            !record
                .otlp
                .windows(b"private-trace-state-sentinel".len())
                .any(|bytes| bytes == b"private-trace-state-sentinel")
        );
        let log = &request.resource_logs[0].scope_logs[0].log_records[0];
        assert_eq!(
            value(log, "image.job_id"),
            Some(&Value::StringValue("04".repeat(16)))
        );
        assert_eq!(
            value(log, "image.config_revision"),
            Some(&Value::IntValue(7))
        );
        if index < kinds.len() {
            assert_eq!(log.event_name, kinds[index].name());
            assert_eq!(log.trace_id, [8; 16]);
            assert_eq!(log.flags & 1, 0);
            assert_eq!(
                value(log, "image.duration_ns"),
                Some(&Value::IntValue(index as i64 + 1))
            );
        } else if index == kinds.len() {
            assert!(log.trace_id.is_empty());
            assert!(value(log, "image.cost_call_id").is_none());
            assert_eq!(
                value(log, "image.processing"),
                Some(&Value::StringValue("local".into()))
            );
            assert_eq!(value(log, "image.duration_ns"), Some(&Value::IntValue(42)));
        } else {
            assert_eq!(
                value(log, "image.delivery_id"),
                Some(&Value::StringValue("0a".repeat(16)))
            );
        }
    }
    assert_eq!(page.source_watermarks[0].missing_through_highest, 0);
    drop(reader);
    reopened.shutdown(WAIT).unwrap();
}

#[test]
fn real_ingress_exposes_the_reserved_sequence_gap_after_bounded_capture_refusal() {
    let root = root();
    let limits = TelemetryLimits::default();
    let mut store = Store::open_configured(&root, limits).unwrap();
    let ingress = Arc::new(HeldIngress {
        target: store.ingress(),
        receipts: Mutex::default(),
    });
    let build = BuildIdentity::new(
        Some("imagery-gap-source"),
        Some("fixture-native"),
        Some("not-applicable"),
        Some("fixture-image-config"),
        Some("fixture-content"),
    )
    .unwrap();
    let producer = NativeProducer::for_service(
        ProducerId::new([2; 16]).unwrap(),
        SourceSequence::new(1).unwrap(),
        &build,
        ingress.clone(),
        limits,
        "imagery-gap-fixture",
    )
    .unwrap();
    let sink = Arc::new(NativeDiagnosticSink::new(producer, 1, next_record).unwrap());
    let observer = SharedImageObserver::new(sink.clone());
    for _ in 0..2 {
        observer.observe(ImageEvent::new(EventKind::Queued, EventContext::default()));
    }
    assert_eq!(observer.health().admitted, 1);
    assert_eq!(observer.health().failures, 1);
    assert_eq!(sink.health().last_reserved, 2);
    assert_eq!(sink.health().confirmed, 0);
    ingress.confirm();
    sink.poll_receipts();
    observer.observe(ImageEvent::new(
        EventKind::Completed,
        EventContext::default(),
    ));
    ingress.confirm();
    sink.shutdown(WAIT).unwrap();
    assert_eq!(sink.health().confirmed, 2);
    assert_eq!(sink.health().refused, 1);
    let mut reader = DiagnosticReader::open(&root).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 64).unwrap();
    assert_eq!(
        page.records
            .iter()
            .map(|record| record.source_sequence)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(page.source_watermarks[0].missing_through_highest, 1);
    assert!(page.unconfirmed_capture_gaps);
    drop(reader);
    store.shutdown(WAIT).unwrap();
}
