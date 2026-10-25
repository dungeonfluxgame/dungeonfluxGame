#![cfg(not(target_arch = "wasm32"))]
use df_observe::{
    AnyValue, CapturedBatch, Durability, LogInput, NativeProducer, ProducerId, RecordId, Severity,
    SourceSequence, TelemetryError, TelemetryLimits,
};
use df_telemetry::{DiagnosticReader, QueryFilter, Readiness, Store};
use df_types::BuildIdentity;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime},
};

const WAIT: Duration = Duration::from_secs(5);
const SMALL_SQLITE_BYTES: u64 = 262_144;

fn fixture_path(scenario: &str) -> PathBuf {
    assert!(
        [
            "quota-spool-exact",
            "quota-sqlite-recovery",
            "quota-combined"
        ]
        .contains(&scenario)
    );
    let namespace = std::env::var("DF_TELEMETRY_FIXTURE_NAMESPACE").unwrap_or_else(|_| {
        format!(
            "f02-{}-{}",
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
            .join(format!("ownedfixture-{namespace}-{scenario}"));
    assert!(path.file_name().unwrap().len() <= 100);
    assert!(
        !path.exists(),
        "quota fixture must be fresh: {}",
        path.display()
    );
    path
}

fn create_producer(store: &Store, limits: TelemetryLimits, sequence: i64) -> NativeProducer {
    let build = BuildIdentity::new(
        Some("quota-source"),
        Some("quota-native"),
        Some("native-only"),
        Some("quota-config"),
        Some("synthetic-safe"),
    )
    .unwrap();
    NativeProducer::new(
        ProducerId::new([74; 16]).unwrap(),
        SourceSequence::new(sequence).unwrap(),
        &build,
        store.ingress(),
        limits,
    )
    .unwrap()
}

fn capture_log(producer: &mut NativeProducer, sequence: u8) -> CapturedBatch {
    let key = producer
        .reserve(RecordId::new([sequence; 16]).unwrap())
        .unwrap();
    let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
    producer
        .emit_log(
            key,
            LogInput {
                timestamp,
                observed_timestamp: timestamp,
                severity: Severity::Info,
                body: AnyValue::String("synthetic-quota-fixture".repeat(12).into()),
                attributes: Vec::new(),
                trace: None,
            },
        )
        .unwrap()
}

fn frame_bytes(path: &std::path::Path) -> u64 {
    fs::read_dir(path.join("spool"))
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum()
}

#[test]
fn exact_spool_quota_refuses_new_record_without_losing_committed_identity() {
    let path = fixture_path("quota-spool-exact");
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut source = create_producer(&store, limits, 1);
    let first = capture_log(&mut source, 1);
    assert_eq!(
        first.pending.wait(WAIT).unwrap().durability,
        Durability::Committed
    );
    source.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
    drop(store);
    let retained_bytes = frame_bytes(&path);
    let limits = TelemetryLimits {
        spool_bytes: retained_bytes,
        ..limits
    };
    let mut store = Store::open(&path, limits, None).unwrap();
    assert_eq!(store.health().spool_bytes, limits.spool_bytes);
    let prior_health = store.health();
    let mut source = create_producer(&store, limits, 2);
    let refused = capture_log(&mut source, 2);
    assert_eq!(refused.pending.wait(WAIT), Err(TelemetryError::Capacity));
    let health = store.health();
    assert_eq!(
        (health.rejected, health.capacity, health.unconfirmed_gaps),
        (
            prior_health.rejected + 1,
            prior_health.capacity + 1,
            prior_health.unconfirmed_gaps + 1,
        )
    );
    assert_eq!(
        (
            health.backlog_files,
            health.pending_items,
            health.pending_bytes
        ),
        (0, 0, 0)
    );
    assert_eq!(health.readiness, Readiness::Healthy);
    assert_eq!(frame_bytes(&path), retained_bytes);
    let mut reader = DiagnosticReader::open(&path).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].record, RecordId::new([1; 16]).unwrap());
    assert_eq!((page.committed_watermark, page.spool_checkpoint), (1, 1));
    assert!(page.unconfirmed_capture_gaps);
    assert!(
        String::from_utf8(page.emergency_bytes)
            .unwrap()
            .contains("error=Capacity")
    );
    drop(reader);
    source.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
    drop(store);
    let mut recovered = Store::open(&path, TelemetryLimits::default(), None).unwrap();
    assert_eq!(
        recovered
            .ingress()
            .submit(refused.signal, &refused.bytes)
            .unwrap()
            .wait(WAIT)
            .unwrap()
            .durability,
        Durability::Committed
    );
    recovered.flush(WAIT).unwrap();
    let page = DiagnosticReader::open(&path)
        .unwrap()
        .query(&QueryFilter::default(), 0, 100)
        .unwrap();
    assert_eq!(
        page.records
            .iter()
            .map(|record| record.record)
            .collect::<Vec<_>>(),
        vec![
            RecordId::new([1; 16]).unwrap(),
            RecordId::new([2; 16]).unwrap()
        ]
    );
    assert_eq!((page.committed_watermark, page.spool_checkpoint), (2, 2));
    recovered.shutdown(WAIT).unwrap();
    println!(
        "quota_exact root={} retained_bytes={retained_bytes} rejected=1 unconfirmed_gaps={} refusal_gap_delta=1 preserved=1 recovered=2",
        path.display(),
        health.unconfirmed_gaps
    );
}

#[test]
fn sqlite_physical_quota_preserves_spooled_work_and_reports_controlled_lag() {
    let path = fixture_path("quota-sqlite-recovery");
    let limits = TelemetryLimits {
        sqlite_bytes: SMALL_SQLITE_BYTES,
        ..Default::default()
    };
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut source = create_producer(&store, limits, 1);
    let captured = capture_log(&mut source, 1);
    let receipt = captured.pending.wait(WAIT).unwrap();
    assert_eq!(receipt.durability, Durability::Spooled);
    assert_eq!(
        (receipt.spool_position, receipt.committed_watermark),
        (1, 0)
    );
    assert_eq!(store.flush(WAIT), Err(TelemetryError::Capacity));
    store.set_time(Duration::from_millis(120_001)).unwrap();
    let health = store.health();
    assert_eq!(health.readiness, Readiness::Degraded);
    assert_eq!(
        (
            health.backlog_files,
            health.checkpoint,
            health.committed_watermark
        ),
        (1, 0, 0)
    );
    assert!(health.capacity > 0);
    assert_eq!(health.rejected, 0);
    assert_eq!(health.spool_bytes, frame_bytes(&path));
    let page = DiagnosticReader::open(&path)
        .unwrap()
        .query(&QueryFilter::default(), 0, 100)
        .unwrap();
    assert!(page.records.is_empty());
    assert!(page.unconfirmed_capture_gaps);
    assert!(
        String::from_utf8(page.emergency_bytes)
            .unwrap()
            .contains("error=Capacity")
    );
    source.shutdown(WAIT).unwrap();
    assert_eq!(store.shutdown(WAIT), Err(TelemetryError::Capacity));
    drop(store);
    let mut recovered = Store::open(&path, TelemetryLimits::default(), None).unwrap();
    recovered.flush(WAIT).unwrap();
    assert_eq!(recovered.health().readiness, Readiness::Healthy);
    assert_eq!(recovered.health().backlog_files, 0);
    let page = DiagnosticReader::open(&path)
        .unwrap()
        .query(&QueryFilter::default(), 0, 100)
        .unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].record, RecordId::new([1; 16]).unwrap());
    assert_eq!((page.committed_watermark, page.spool_checkpoint), (1, 1));
    assert_eq!(
        recovered
            .ingress()
            .submit(captured.signal, &captured.bytes)
            .unwrap()
            .wait(WAIT)
            .unwrap()
            .durability,
        Durability::Committed
    );
    let page = DiagnosticReader::open(&path)
        .unwrap()
        .query(&QueryFilter::default(), 0, 100)
        .unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].position, 1);
    recovered.shutdown(WAIT).unwrap();
    println!(
        "quota_sqlite root={} physical_quota={} spooled=1 degraded_at_ms=120001 recovered=1 duplicate_rows=0",
        path.display(),
        limits.sqlite_bytes
    );
}

#[test]
fn simultaneous_sink_and_spool_quota_exposes_loss_and_recovers_prior_acceptance() {
    let path = fixture_path("quota-combined");
    let limits = TelemetryLimits {
        sqlite_bytes: SMALL_SQLITE_BYTES,
        spool_bytes: 4096,
        ..Default::default()
    };
    let mut store = Store::open(&path, limits, None).unwrap();
    let prior_gaps = store.health().unconfirmed_gaps;
    let mut source = create_producer(&store, limits, 1);
    let mut accepted = Vec::new();
    let mut refused = None;
    for sequence in 1..=16 {
        let captured = capture_log(&mut source, sequence);
        match captured.pending.wait(WAIT) {
            Ok(receipt) => {
                assert_eq!(receipt.durability, Durability::Spooled);
                accepted.push(RecordId::new([sequence; 16]).unwrap());
            }
            Err(error) => {
                assert_eq!(error, TelemetryError::Capacity);
                refused = Some((
                    captured.signal,
                    captured.bytes,
                    RecordId::new([sequence; 16]).unwrap(),
                ));
                break;
            }
        }
    }
    let (signal, bytes, refused_id) =
        refused.expect("4096-byte spool must fill within16 fixed-size records");
    assert!(!accepted.is_empty());
    assert_eq!(store.flush(WAIT), Err(TelemetryError::Capacity));
    store.set_time(Duration::from_millis(120_001)).unwrap();
    let health = store.health();
    assert_eq!(health.readiness, Readiness::Degraded);
    assert_eq!(health.rejected, 1);
    assert!(health.unconfirmed_gaps > prior_gaps);
    assert!(health.capacity > 0);
    assert_eq!(health.backlog_files, accepted.len());
    assert_eq!((health.checkpoint, health.committed_watermark), (0, 0));
    assert_eq!((health.pending_items, health.pending_bytes), (0, 0));
    assert_eq!(health.spool_bytes, frame_bytes(&path));
    assert!(health.spool_bytes <= limits.spool_bytes);
    let page = DiagnosticReader::open(&path)
        .unwrap()
        .query(&QueryFilter::default(), 0, 100)
        .unwrap();
    assert!(page.records.is_empty());
    assert!(page.unconfirmed_capture_gaps);
    source.shutdown(WAIT).unwrap();
    assert_eq!(store.shutdown(WAIT), Err(TelemetryError::Capacity));
    drop(store);
    let mut recovered = Store::open(&path, TelemetryLimits::default(), None).unwrap();
    recovered.flush(WAIT).unwrap();
    let page = DiagnosticReader::open(&path)
        .unwrap()
        .query(&QueryFilter::default(), 0, 100)
        .unwrap();
    assert_eq!(
        page.records
            .iter()
            .map(|record| record.record)
            .collect::<Vec<_>>(),
        accepted
    );
    assert_eq!(
        (page.committed_watermark, page.spool_checkpoint),
        (accepted.len() as u64, accepted.len() as u64)
    );
    let receipt = recovered
        .ingress()
        .submit(signal, &bytes)
        .unwrap()
        .wait(WAIT)
        .unwrap();
    assert_eq!(receipt.durability, Durability::Committed);
    let page = DiagnosticReader::open(&path)
        .unwrap()
        .query(&QueryFilter::default(), 0, 100)
        .unwrap();
    assert_eq!(page.records.len(), accepted.len() + 1);
    assert_eq!(page.records.last().unwrap().record, refused_id);
    assert_eq!(recovered.health().backlog_files, 0);
    recovered.shutdown(WAIT).unwrap();
    println!(
        "quota_combined root={} spool_limit=4096 preserved={} rejected=1 gaps={} readiness=Degraded recovered={} retained=true",
        path.display(),
        accepted.len(),
        health.unconfirmed_gaps,
        page.records.len()
    );
}
