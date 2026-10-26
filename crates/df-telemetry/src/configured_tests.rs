use super::*;
use df_observe::{
    AnyValue, LogInput, NativeProducer, ProducerId, RecordId, Severity, SourceSequence,
};
use rusqlite::Connection;
use std::{fs, path::PathBuf, time::SystemTime};

fn root() -> PathBuf {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("artifacts/tmp/agents/OBSEXPORT/data");
    fs::create_dir_all(&parent).unwrap();
    parent.join(format!(
        "spool-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn configured_root_recovers_durable_spool_after_sqlite_outage_and_restart() {
    let path = root();
    fs::create_dir(&path).unwrap();
    let limits = TelemetryLimits::default();
    let mut store = Store::open_configured(&path, limits).unwrap();
    let build = df_types::BuildIdentity::new(
        Some("obsexport"),
        Some("native"),
        Some("none"),
        Some("test"),
        Some("none"),
    )
    .unwrap();
    let mut producer = NativeProducer::for_service(
        ProducerId::new([7; 16]).unwrap(),
        SourceSequence::new(1).unwrap(),
        &build,
        store.ingress(),
        limits,
        "df-portal-hosted",
    )
    .unwrap();
    let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
    let lock = Connection::open(path.join("telemetry.sqlite3")).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let caller_result = fs::read(path.join("OWNER"));
    let captured = producer
        .emit_log(
            key,
            LogInput {
                timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                observed_timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                severity: Severity::Info,
                body: AnyValue::String("private-test-payload".into()),
                attributes: Vec::new(),
                trace: None,
            },
        )
        .unwrap();
    let receipt = captured.pending.wait(Duration::from_secs(5)).unwrap();
    assert_eq!(receipt.durability, Durability::Spooled);
    assert!(caller_result.is_ok());
    assert_eq!(
        store.flush(Duration::from_secs(5)),
        Err(TelemetryError::SinkUnavailable)
    );
    let health = store.health();
    assert_eq!(health.committed, 0);
    assert_eq!(health.checkpoint, 0);
    assert_eq!(health.backlog_files, 1);
    assert!(health.emergency_records >= 2);
    store.set_time(Duration::from_secs(121)).unwrap();
    assert_eq!(store.health().readiness, Readiness::Degraded);
    let emergency = fs::read_to_string(path.join("emergency")).unwrap();
    assert!(emergency.contains("SinkUnavailable"));
    assert!(!emergency.contains("private-test-payload"));
    producer.shutdown(Duration::from_secs(5)).unwrap();
    assert_eq!(
        store.shutdown(Duration::from_secs(5)),
        Err(TelemetryError::SinkUnavailable)
    );
    drop(store);
    lock.execute_batch("COMMIT").unwrap();
    drop(lock);
    let mut recovered = Store::open_configured(&path, limits).unwrap();
    recovered.flush(Duration::from_secs(5)).unwrap();
    let mut reader = DiagnosticReader::open(&path).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].producer, key.producer);
    assert_eq!(page.records[0].record, key.record);
    assert_eq!(page.records[0].source_sequence, 1);
    recovered
        .ingress()
        .submit(captured.signal, &captured.bytes)
        .unwrap()
        .wait(Duration::from_secs(5))
        .unwrap();
    recovered.flush(Duration::from_secs(5)).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(recovered.health().duplicates, 1);
    assert_eq!(recovered.health().readiness, Readiness::Healthy);
    assert!(reader.verify_write_denied().unwrap());
    recovered.shutdown(Duration::from_secs(5)).unwrap();
    drop(reader);
    drop(recovered);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn configured_root_refuses_nonempty_foreign_directory_without_mutating_it() {
    let path = root();
    fs::create_dir(&path).unwrap();
    fs::write(path.join("foreign"), b"preserve").unwrap();
    assert!(matches!(
        Store::open_configured(&path, TelemetryLimits::default()),
        Err(TelemetryError::ForeignRoot)
    ));
    assert_eq!(fs::read(path.join("foreign")).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
    fs::remove_dir_all(path).unwrap();
}
