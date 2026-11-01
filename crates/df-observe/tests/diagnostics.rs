#![cfg(not(target_arch = "wasm32"))]

use df_observe::{
    AnyValue, DiagnosticRecord, DiagnosticSink, DiagnosticTrace, Durability, IngestReceipt,
    LogInput, NativeDiagnosticSink, NativeProducer, PendingReceipt, ProducerId, Severity, Signal,
    SourceSequence, TelemetryError, TelemetryIngress, TelemetryLimits,
};
use df_types::BuildIdentity;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use prost::Message;
use std::{
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, SystemTime},
};

type Reply = Sender<Result<IngestReceipt, TelemetryError>>;

#[derive(Debug, Default)]
struct Delayed {
    bytes: Mutex<Vec<Vec<u8>>>,
    replies: Mutex<Vec<Reply>>,
}

impl TelemetryIngress for Delayed {
    fn submit(&self, signal: Signal, bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        assert_eq!(signal, Signal::Logs);
        self.bytes.lock().unwrap().push(bytes.to_vec());
        let (sender, receipt) = PendingReceipt::channel();
        self.replies.lock().unwrap().push(sender);
        Ok(receipt)
    }
}

impl Delayed {
    fn confirm(&self, index: usize) {
        self.replies.lock().unwrap()[index]
            .send(Ok(IngestReceipt {
                durability: Durability::Committed,
                spool_position: index as u64 + 1,
                committed_watermark: index as u64 + 1,
                records: 1,
            }))
            .unwrap();
    }
}

fn next_record() -> [u8; 16] {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    u128::from(NEXT.fetch_add(1, Ordering::Relaxed)).to_be_bytes()
}

fn sink(ingress: Arc<dyn TelemetryIngress>) -> NativeDiagnosticSink {
    sink_with_capacity(ingress, 1)
}

fn sink_with_capacity(
    ingress: Arc<dyn TelemetryIngress>,
    maximum_pending: usize,
) -> NativeDiagnosticSink {
    let build = BuildIdentity::new(
        Some("diagnostic-source"),
        Some("fixture-native"),
        Some("not-applicable"),
        Some("fixture-config"),
        Some("fixture-content"),
    )
    .unwrap();
    let producer = NativeProducer::for_service(
        ProducerId::new([1; 16]).unwrap(),
        SourceSequence::new(1).unwrap(),
        &build,
        ingress,
        TelemetryLimits::default(),
        "diagnostic-fixture",
    )
    .unwrap();
    NativeDiagnosticSink::new(producer, maximum_pending, next_record).unwrap()
}

fn record() -> DiagnosticRecord {
    DiagnosticRecord {
        event_name: "subsystem.lifecycle",
        input: LogInput {
            timestamp: SystemTime::now(),
            observed_timestamp: SystemTime::now(),
            severity: Severity::Info,
            body: AnyValue::String("safe".into()),
            attributes: Vec::new(),
            trace: None,
        },
    }
}

#[test]
fn bounded_receipts_expose_refusal_and_confirm_only_matched_admissions() {
    let ingress = Arc::new(Delayed::default());
    let sink = sink(ingress.clone());
    sink.emit(record()).unwrap();
    assert_eq!(sink.health().pending, 1);
    assert_eq!(sink.health().confirmed, 0);
    assert_eq!(sink.emit(record()), Err(TelemetryError::Capacity));
    assert_eq!(sink.health().last_reserved, 2);
    assert_eq!(sink.health().refused, 1);
    ingress.confirm(0);
    sink.poll_receipts();
    assert_eq!(sink.health().last_confirmed, 1);
    sink.emit(record()).unwrap();
    ingress.confirm(1);
    sink.poll_receipts();
    let health = sink.health();
    assert_eq!(
        (health.attempted, health.admitted, health.confirmed),
        (3, 2, 2)
    );
    assert_eq!(
        (health.last_reserved, health.last_confirmed, health.pending),
        (3, 3, 0)
    );
    let batches = ingress.bytes.lock().unwrap();
    assert_eq!(batches.len(), 2);
    for (bytes, sequence) in batches.iter().zip([1, 3]) {
        let request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
        let record = &request.resource_logs[0].scope_logs[0].log_records[0];
        assert_eq!(record.event_name, "subsystem.lifecycle");
        let source_sequence = record
            .attributes
            .iter()
            .find(|entry| entry.key == "df.source_sequence")
            .unwrap();
        assert_eq!(
            source_sequence.value.as_ref().unwrap().value,
            Some(opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(sequence))
        );
    }
    sink.shutdown(Duration::from_secs(1)).unwrap();
    assert_eq!(sink.emit(record()), Err(TelemetryError::Closed));
    assert_eq!(sink.health().refused, 2);
}

#[test]
fn failed_receipts_and_shutdown_timeout_never_invent_confirmation() {
    let ingress = Arc::new(Delayed::default());
    let sink = sink(ingress.clone());
    sink.emit(record()).unwrap();
    ingress.replies.lock().unwrap()[0]
        .send(Err(TelemetryError::Io))
        .unwrap();
    sink.poll_receipts();
    assert_eq!(sink.health().receipt_failures, 1);
    assert_eq!(sink.health().confirmed, 0);
    sink.emit(record()).unwrap();
    assert_eq!(sink.shutdown(Duration::ZERO), Err(TelemetryError::Deadline));
    assert_eq!(sink.health().receipt_failures, 2);
    assert_eq!(sink.health().confirmed, 0);
}

#[test]
fn concurrent_enabled_writers_retain_every_record_below_capacity() {
    let ingress = Arc::new(Delayed::default());
    let sink = Arc::new(sink_with_capacity(ingress.clone(), 64));
    let barrier = Arc::new(Barrier::new(8));
    let writers = (0..8)
        .map(|_| {
            let sink = sink.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..8 {
                    sink.emit(record()).unwrap();
                }
            })
        })
        .collect::<Vec<_>>();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(sink.health().attempted, 64);
    assert_eq!(sink.health().admitted, 64);
    assert_eq!(sink.health().refused, 0);
    assert_eq!(ingress.bytes.lock().unwrap().len(), 64);
    for index in 0..64 {
        ingress.confirm(index);
    }
    sink.poll_receipts();
    assert_eq!(sink.health().confirmed, 64);
    assert_eq!(sink.health().pending, 0);
    sink.shutdown(Duration::from_secs(1)).unwrap();
}

#[test]
fn trace_state_is_excluded_from_diagnostic_debug_and_actual_otlp_bytes() {
    let private = "private-trace-state-sentinel";
    let raw = SpanContext::new(
        TraceId::from_bytes([2; 16]),
        SpanId::from_bytes([3; 8]),
        TraceFlags::NOT_SAMPLED,
        true,
        TraceState::from_key_value([("vendor", private)]).unwrap(),
    );
    assert!(raw.trace_state().header().contains(private));
    let trace = DiagnosticTrace::from(&raw);
    assert!(!format!("{trace:?}").contains(private));
    assert!(trace.span_context().trace_state().header().is_empty());
    let ingress = Arc::new(Delayed::default());
    let sink = sink(ingress.clone());
    let mut event = record();
    event.input.trace = Some(raw);
    assert!(!format!("{:?}", event.input).contains(private));
    assert!(!format!("{event:?}").contains(private));
    sink.emit(event).unwrap();
    let bytes = &ingress.bytes.lock().unwrap()[0];
    assert!(
        !bytes
            .windows(private.len())
            .any(|value| value == private.as_bytes())
    );
    let request = ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap();
    let log = &request.resource_logs[0].scope_logs[0].log_records[0];
    assert_eq!(log.trace_id, [2; 16]);
    assert_eq!(log.span_id, [3; 8]);
    assert_eq!(log.flags & 1, 0);
    ingress.confirm(0);
    sink.shutdown(Duration::from_secs(1)).unwrap();
}

#[derive(Debug)]
struct StalledIngress {
    entered: Sender<()>,
    release: Mutex<Receiver<()>>,
}

impl TelemetryIngress for StalledIngress {
    fn submit(&self, _signal: Signal, _bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        self.entered.send(()).map_err(|_| TelemetryError::Closed)?;
        self.release
            .lock()
            .unwrap()
            .recv()
            .map_err(|_| TelemetryError::Closed)?;
        let (sender, receipt) = PendingReceipt::channel();
        sender
            .send(Ok(IngestReceipt {
                durability: Durability::Committed,
                spool_position: 1,
                committed_watermark: 1,
                records: 1,
            }))
            .map_err(|_| TelemetryError::Closed)?;
        Ok(receipt)
    }
}

#[test]
fn shutdown_deadline_during_stalled_capture_preserves_ownership_for_later_drain() {
    let (entered, arrived) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let sink = Arc::new(sink(Arc::new(StalledIngress {
        entered,
        release: Mutex::new(blocked),
    })));
    let producer = sink.clone();
    let writer = std::thread::spawn(move || producer.emit(record()));
    arrived.recv_timeout(Duration::from_secs(1)).unwrap();
    let (reply, outcome) = mpsc::channel();
    let closing = sink.clone();
    let shutdown =
        std::thread::spawn(move || reply.send(closing.shutdown(Duration::ZERO)).unwrap());
    let before_release = outcome.recv_timeout(Duration::from_secs(1));
    // Always release and join the owned workers, even if the shutdown deadline regresses.
    release.send(()).unwrap();
    writer.join().unwrap().unwrap();
    shutdown.join().unwrap();
    assert_eq!(before_release, Ok(Err(TelemetryError::Deadline)));
    assert_eq!(sink.health().admitted, 1);
    sink.shutdown(Duration::from_secs(1)).unwrap();
    assert_eq!(sink.health().confirmed, 1);
}
