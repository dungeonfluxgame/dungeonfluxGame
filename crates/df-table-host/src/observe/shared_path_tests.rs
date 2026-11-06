//! The shared path, through the real df-observe producer: once against the real df-telemetry
//! store (spool, SQLite, read-only reader), and twice against an ingress that answers as a test
//! says, so lag and refusal are deterministic.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use df_observe::{
    IngestReceipt, NativeDiagnosticSink, NativeProducer, PendingReceipt, ProducerId, Signal,
    SourceSequence, TelemetryError, TelemetryIngress, TelemetryLimits,
};
use df_table::ports::EventSink;
use df_table::testkit::ManualClock;
use df_telemetry::{DiagnosticReader, QueryFilter, Store};
use df_types::BuildIdentity;
use tempfile::TempDir;

use crate::event_log::EventLog;

use super::fixtures::{dice_drawn, operation_of, received, spawned};
use super::{
    HostEventSink, SharedPathCause, SharedPathHealth, SharedPathOutlet, compose_shared_path,
    shared_path_line,
};

const SETTLE: Duration = Duration::from_secs(10);

fn build() -> BuildIdentity {
    BuildIdentity::new(
        Some("hosttest"),
        Some("native"),
        Some("none"),
        Some("test"),
        Some("none"),
    )
    .expect("a build identity")
}

/// Fresh opaque entropy for each record, as the sink's contract asks.
fn next_record() -> [u8; 16] {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut bytes = [0; 16];
    bytes[8..].copy_from_slice(&(NEXT.fetch_add(1, Ordering::Relaxed) + 1).to_be_bytes());
    bytes
}

fn diagnostics(ingress: Arc<dyn TelemetryIngress>) -> Arc<NativeDiagnosticSink> {
    let producer = NativeProducer::for_service(
        ProducerId::new([5; 16]).expect("an identity"),
        SourceSequence::new(1).expect("a sequence"),
        &build(),
        ingress,
        TelemetryLimits::default(),
        "df-table-host",
    )
    .expect("a producer");
    Arc::new(NativeDiagnosticSink::new(producer, 1_024, next_record).expect("a sink"))
}

fn sink_over(diagnostics: &Arc<NativeDiagnosticSink>) -> (HostEventSink, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::starting_at(10_000));
    let sink = HostEventSink::new(Arc::new(EventLog::default()), clock.clone())
        .with_outlet(Arc::new(SharedPathOutlet::new(Arc::clone(diagnostics))));
    (sink, clock)
}

/// An ingress that holds each receipt until the test answers it.
#[derive(Debug, Default)]
struct HoldingIngress {
    held: Mutex<Vec<std::sync::mpsc::Sender<Result<IngestReceipt, TelemetryError>>>>,
}

impl HoldingIngress {
    fn answer_all(&self) {
        let held = std::mem::take(&mut *self.held.lock().expect("the lock"));
        for (index, sender) in held.into_iter().enumerate() {
            let receipt = IngestReceipt {
                durability: df_observe::Durability::Spooled,
                spool_position: u64::try_from(index).expect("a small index") + 1,
                committed_watermark: 0,
                records: 1,
            };
            sender
                .send(Ok(receipt))
                .expect("the receipt is still wanted");
        }
    }
}

impl TelemetryIngress for HoldingIngress {
    fn submit(&self, _signal: Signal, _bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        let (sender, pending) = PendingReceipt::channel();
        self.held.lock().expect("the lock").push(sender);
        Ok(pending)
    }
}

/// An ingress that is at capacity.
#[derive(Debug)]
struct FullIngress;

impl TelemetryIngress for FullIngress {
    fn submit(&self, _signal: Signal, _bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        Err(TelemetryError::Capacity)
    }
}

/// An ingress that admits each record and then drops its receipt, so the receipt fails.
#[derive(Debug)]
struct DroppingIngress;

impl TelemetryIngress for DroppingIngress {
    fn submit(&self, _signal: Signal, _bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        let (_sender, pending) = PendingReceipt::channel();
        Ok(pending)
    }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn events_reach_the_real_telemetry_store_with_their_line_name_and_operation() {
    let directory = TempDir::new().expect("a directory");
    let root = directory.path().join("telemetry");
    let mut store = Store::open_configured(&root, TelemetryLimits::default()).expect("a store");
    let diagnostics = diagnostics(store.ingress());
    let (sink, _clock) = sink_over(&diagnostics);
    let events = [received(1), received(2), dice_drawn(2)];

    for event in &events {
        sink.record(event);
    }
    store
        .flush(SETTLE)
        .expect("the store commits what it was given");

    let mut reader = DiagnosticReader::open(&root).expect("a read-only reader");
    let everything = reader
        .query(&QueryFilter::default(), 0, 100)
        .expect("a page");
    assert_eq!(everything.records.len(), 3);
    let one_input = reader
        .query(
            &QueryFilter {
                operation: Some(operation_of(2)),
                ..QueryFilter::default()
            },
            0,
            100,
        )
        .expect("a page");
    assert_eq!(
        one_input.records.len(),
        2,
        "the store indexes df.operation, so one input's records are found by it"
    );
    let otlp = &one_input.records[0].otlp;
    assert!(contains(otlp, "table.input.received"));
    assert!(
        contains(otlp, &events[1].render_audit_line()),
        "the line is the body"
    );
    assert!(contains(otlp, "df.audit_id"));
    assert!(contains(&one_input.records[1].otlp, "table.pipeline"));
    let health = sink.health();
    let shared = &health.outlets[1];
    assert_eq!(shared.name, "shared_path");
    assert_eq!(
        (shared.accepted, shared.confirmed, shared.pending),
        (3, 3, 0)
    );
    assert_eq!(health.lag_ms, 0);
    assert!(!health.degraded);
    store.shutdown(SETTLE).expect("the store shuts down");
}

#[test]
fn ingestion_lag_is_the_age_of_the_oldest_record_still_waiting_for_its_receipt() {
    let ingress = Arc::new(HoldingIngress::default());
    let diagnostics = diagnostics(ingress.clone());
    let (sink, clock) = sink_over(&diagnostics);

    sink.record(&received(1));
    clock.advance_ms(250);
    sink.record(&received(2));
    clock.advance_ms(250);

    let waiting = sink.health();
    assert_eq!(waiting.outlets[1].pending, 2);
    assert_eq!(waiting.outlets[1].confirmed, 0);
    assert_eq!(waiting.lag_ms, 500, "the first record is 500 ms old");

    ingress.answer_all();

    let settled = sink.health();
    assert_eq!(settled.outlets[1].confirmed, 2);
    assert_eq!(settled.lag_ms, 0);
}

#[test]
fn a_refusal_of_the_path_is_counted_by_class_and_never_reaches_the_caller() {
    let diagnostics = diagnostics(Arc::new(FullIngress));
    let (sink, _clock) = sink_over(&diagnostics);

    sink.record(&received(1));

    let health = sink.health();
    assert_eq!(health.lost_always, 1);
    assert!(health.degraded);
    assert_eq!(
        health.retained, 1,
        "the history and the line log still have it"
    );
    assert_eq!(diagnostics.health().refused, 1);
}

#[test]
fn the_first_failed_receipt_writes_sink_degraded_once_without_a_health_read() {
    let diagnostics = diagnostics(Arc::new(DroppingIngress));
    let log = Arc::new(EventLog::default());
    let sink = HostEventSink::new(Arc::clone(&log), Arc::new(ManualClock::starting_at(10_000)))
        .with_outlet(Arc::new(SharedPathOutlet::new(Arc::clone(&diagnostics))));

    sink.record(&received(1));
    sink.record(&received(2));

    let degraded: Vec<String> = log
        .newest(10)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("HOST sink_degraded"))
        .collect();
    assert_eq!(
        degraded,
        ["HOST sink_degraded outlet=shared_path cause=receipt_failed lost_always=0"]
    );
    let health = sink.health();
    assert!(health.degraded);
    assert_eq!(health.outlets[1].unconfirmed, 2);
    assert_eq!(
        log.newest(10)
            .iter()
            .filter(|line| line.text.starts_with("HOST sink_degraded"))
            .count(),
        1
    );
}

#[test]
fn once_composed_a_host_record_reaches_a_local_fixture_store() {
    let directory = TempDir::new().expect("a directory");
    let root = directory.path().join("telemetry");

    let (path, line) = compose_shared_path(Some(&root));

    assert_eq!(line, "HOST shared_path composed=true root_set=true");
    let path = path.expect("the path is composed");
    let sink = HostEventSink::new(
        Arc::new(EventLog::default()),
        Arc::new(ManualClock::starting_at(10_000)),
    )
    .with_outlet(path.outlet());
    sink.record(&spawned());
    sink.record(&received(1));
    path.shutdown(SETTLE).expect("the store confirms and stops");

    let shared = sink.health().shared.expect("the sink has the shared path");
    assert_eq!(
        shared,
        SharedPathHealth {
            spooled: 2,
            exported: 2,
            dropped: 0,
            lag_ms: 0,
        }
    );
    let mut reader = DiagnosticReader::open(&root).expect("a read-only reader");
    let page = reader
        .query(&QueryFilter::default(), 0, 100)
        .expect("a page");
    assert_eq!(page.records.len(), 2);
    assert!(contains(
        &page.records[0].otlp,
        &spawned().render_audit_line()
    ));
    assert!(contains(&page.records[1].otlp, "df-table-host"));
}

#[test]
fn without_a_root_nothing_is_composed_and_the_line_says_so() {
    let (path, line) = compose_shared_path(None);
    assert!(path.is_none());
    assert_eq!(line, "HOST shared_path composed=false root_set=false");
}

#[test]
fn a_root_the_store_cannot_own_is_not_composed_and_the_line_has_its_cause_but_not_the_path() {
    let directory = TempDir::new().expect("a directory");
    let file = directory.path().join("a-file");
    std::fs::write(&file, b"not a directory").expect("a file");
    let root = file.join("telemetry");

    let (path, line) = compose_shared_path(Some(&root));

    assert!(path.is_none());
    let cause = line
        .strip_prefix("HOST shared_path composed=false root_set=true cause=")
        .expect("a refusal line");
    assert!(SharedPathCause::from_key(cause).is_some(), "{line}");
    assert!(!line.contains("a-file"));
}

#[test]
fn the_shared_path_line_is_closed_keys_only() {
    assert_eq!(
        shared_path_line(true, Some(SharedPathCause::Ownership)),
        "HOST shared_path composed=false root_set=true cause=ownership"
    );
    for cause in SharedPathCause::ALL {
        let serde = serde_json::to_value(cause).expect("serializes");
        assert_eq!(serde, serde_json::Value::from(cause.key()));
        assert_eq!(SharedPathCause::from_key(cause.key()), Some(cause));
    }
}

#[test]
fn shared_health_reads_lag_and_loss_from_the_outlet() {
    let ingress = Arc::new(HoldingIngress::default());
    let diagnostics = diagnostics(ingress.clone());
    let (sink, clock) = sink_over(&diagnostics);

    sink.record(&received(1));
    clock.advance_ms(70);

    assert_eq!(
        sink.health().shared,
        Some(SharedPathHealth {
            spooled: 1,
            exported: 0,
            dropped: 0,
            lag_ms: 70,
        })
    );
    ingress.answer_all();
}
