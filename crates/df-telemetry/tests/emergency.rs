#![cfg(not(target_arch = "wasm32"))]

use df_observe::{
    AnyValue, Durability, LogInput, NativeProducer, ProducerId, RecordId, Severity, SourceSequence,
    TelemetryError, TelemetryLimits,
};
use df_telemetry::{DiagnosticReader, QueryFilter, Readiness, Store};
use df_types::BuildIdentity;
use rusqlite::Connection;
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime},
};

const WAIT: Duration = Duration::from_secs(5);
const PRIVATE_PAYLOAD: &str = "synthetic-private-payload-must-not-enter-emergency";

fn root(case: &str) -> PathBuf {
    let namespace = std::env::var("DF_TELEMETRY_FIXTURE_NAMESPACE")
        .expect("coordinator must preregister the exact fixture namespace");
    assert!(
        !namespace.is_empty()
            && namespace
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    );
    PathBuf::from("/Users/earlcameron/Desktop/dungeonflux/development/runtime/telemetry-g06")
        .join(format!("ownedfixture-{namespace}-{case}"))
}

fn producer(store: &Store, limits: TelemetryLimits) -> NativeProducer {
    let build = BuildIdentity::new(
        Some("emergency-caller-fixture-v1"),
        Some("native-fixture"),
        Some("native-only-unqualified"),
        Some("synthetic-v1"),
        Some("synthetic-safe"),
    )
    .unwrap();
    NativeProducer::new(
        ProducerId::new([7; 16]).unwrap(),
        SourceSequence::new(1).unwrap(),
        &build,
        store.ingress(),
        limits,
    )
    .unwrap()
}

fn input() -> LogInput {
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    LogInput {
        timestamp: time,
        observed_timestamp: time,
        severity: Severity::Info,
        body: AnyValue::String(PRIVATE_PAYLOAD.into()),
        attributes: Vec::new(),
        trace: None,
    }
}

fn safe_lines(bytes: &[u8], error: TelemetryError) -> usize {
    let text = std::str::from_utf8(bytes).unwrap();
    assert!(!text.contains(PRIVATE_PAYLOAD));
    assert!(text.ends_with('\n'));
    let mut line_count = 0;
    for line in text.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 7);
        assert_eq!(fields[0], "df-telemetry");
        assert_eq!(fields[1], "emergency");
        assert_eq!(fields[2], "source=local-spool");
        assert_eq!(fields[3], format!("error={error}"));
        fields[4]
            .strip_prefix("checkpoint=")
            .unwrap()
            .parse::<u64>()
            .unwrap();
        fields[5]
            .strip_prefix("backlog=")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(fields[6], "unconfirmed=true");
        line_count += 1;
    }
    line_count
}

#[test]
fn normal_capture_commits_without_emergency_and_does_not_invent_caller_success() {
    let path = root("emergency-normal");
    assert!(!path.exists(), "fixture root must be fresh");
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = producer(&store, limits);
    let caller_result = fs::read(path.join("absent-caller-input"));
    let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
    let receipt = producer
        .emit_log(key, input())
        .unwrap()
        .pending
        .wait(WAIT)
        .unwrap();
    assert_eq!(receipt.durability, Durability::Committed);
    assert_eq!(
        caller_result.unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    let mut reader = DiagnosticReader::open(&path).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].record, key.record);
    assert_eq!(page.committed_watermark, receipt.committed_watermark);
    assert_eq!(page.spool_checkpoint, receipt.spool_position);
    assert!(page.emergency_source.is_none());
    assert!(page.emergency_bytes.is_empty());
    assert!(!path.join("emergency").exists());
    assert_eq!(store.health().emergency_records, 0);
    assert!(page.unconfirmed_capture_gaps);
    store.shutdown(WAIT).unwrap();
    let key = producer.reserve(RecordId::new([9; 16]).unwrap()).unwrap();
    assert!(matches!(
        producer.emit_log(key, input()),
        Err(TelemetryError::Closed)
    ));
    assert_eq!(store.health().admitted, 1);
    assert_eq!(store.health().emergency_records, 0);
    producer.shutdown(WAIT).unwrap();
}

#[test]
fn successful_caller_read_survives_actual_sdk_export_outage_and_identity_recovery() {
    let path = root("emergency-sink");
    assert!(!path.exists(), "fixture root must be fresh");
    let limits = TelemetryLimits::default();
    let mut store = Store::open(&path, limits, None).unwrap();
    let mut producer = producer(&store, limits);
    let lock = Connection::open(path.join("telemetry.sqlite3")).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();

    // This is an actual native caller result, not an invented gameplay reducer.
    let caller_result = fs::read(path.join("OWNER"));
    let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
    let captured = producer.emit_log(key, input()).unwrap();
    let receipt = captured.pending.wait(WAIT).unwrap();
    assert_eq!(receipt.durability, Durability::Spooled);
    assert_eq!(receipt.committed_watermark, 0);
    assert_eq!(store.flush(WAIT), Err(TelemetryError::SinkUnavailable));
    let caller_value = caller_result.unwrap();
    assert_eq!(
        caller_value,
        b"df-telemetry native synthetic owned root v1\n"
    );
    let health = store.health();
    assert_eq!(health.committed, 0);
    assert_eq!(health.checkpoint, 0);
    assert_eq!(health.committed_watermark, 0);
    assert_eq!(health.backlog_files, 1);
    assert!(health.emergency_records >= 2);
    store.set_time(Duration::from_secs(121)).unwrap();
    assert_eq!(store.health().readiness, Readiness::Degraded);
    let mut reader = DiagnosticReader::open(&path).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert!(page.records.is_empty());
    assert_eq!(page.spool_checkpoint, 0);
    assert_eq!(page.committed_watermark, 0);
    assert_eq!(page.emergency_source, Some(path.join("emergency")));
    assert!(safe_lines(&page.emergency_bytes, TelemetryError::SinkUnavailable) >= 2);
    assert!(page.unconfirmed_capture_gaps);

    lock.execute_batch("COMMIT").unwrap();
    store.flush(WAIT).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 1);
    let position = page.records[0].position;
    assert_eq!(page.records[0].producer, key.producer);
    assert_eq!(page.records[0].record, key.record);
    assert_eq!(page.records[0].source_sequence, key.sequence.get());
    assert_eq!(page.spool_checkpoint, receipt.spool_position);
    let retry = store
        .ingress()
        .submit(captured.signal, &captured.bytes)
        .unwrap()
        .wait(WAIT)
        .unwrap();
    assert_eq!(retry.records, 1);
    assert_eq!(retry.committed_watermark, page.committed_watermark);
    // The retry may remain Spooled during the existing export backoff; flush confirms it.
    store.flush(WAIT).unwrap();
    let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].position, position);
    assert_eq!(page.records[0].record, key.record);
    assert_eq!(page.spool_checkpoint, retry.spool_position);
    assert_eq!(page.committed_watermark, 1);
    assert_eq!(store.health().duplicates, 1);
    assert_eq!(store.health().readiness, Readiness::Healthy);
    assert_eq!(caller_value, fs::read(path.join("OWNER")).unwrap());
    safe_lines(&page.emergency_bytes, TelemetryError::SinkUnavailable);
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
}

#[test]
fn startup_refusal_is_typed_and_preserves_foreign_input_without_worker_output() {
    let path = root("emergency-startup");
    assert!(!path.exists(), "fixture root must be fresh");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("OWNER"), b"synthetic-foreign-owner").unwrap();
    assert!(matches!(
        Store::open(&path, TelemetryLimits::default(), None),
        Err(TelemetryError::ForeignRoot)
    ));
    assert_eq!(
        fs::read(path.join("OWNER")).unwrap(),
        b"synthetic-foreign-owner"
    );
    assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
    assert!(matches!(
        DiagnosticReader::open(&path),
        Err(TelemetryError::ForeignRoot)
    ));
}

fn child_output(test: &str) -> (Vec<u8>, Vec<u8>) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--ignored", "--nocapture"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read = |pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.take(16385).read_to_end(&mut bytes).unwrap();
            bytes
        })
    };
    let stdout = read(Box::new(stdout));
    let stderr = read(Box::new(stderr));
    let deadline = Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        thread::sleep(Duration::from_millis(5));
    };
    let stdout = stdout.join().unwrap();
    let stderr = stderr.join().unwrap();
    assert!(
        status.is_some_and(|status| status.success()),
        "child failed: {} {}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    assert!(stdout.len() <= 16384 && stderr.len() <= 16384);
    (stdout, stderr)
}

#[test]
fn file_obstruction_uses_safe_bounded_stderr_and_preserves_typed_capture_error() {
    let (_, stderr) = child_output("emergency_stderr_child");
    assert_eq!(safe_lines(&stderr, TelemetryError::Capacity), 1);
}

#[test]
fn emergency_record_and_byte_bounds_keep_overflow_visible() {
    let (_, stderr) = child_output("emergency_bounds_child");
    let path = root("emergency-bound");
    let bytes = fs::read(path.join("emergency")).unwrap();
    let retained = safe_lines(&bytes, TelemetryError::Capacity);
    let fallback = safe_lines(&stderr, TelemetryError::Capacity);
    assert_eq!(retained + fallback, 96);
    assert!(bytes.len() <= 8192);
    let line = std::str::from_utf8(&stderr)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    assert!(bytes.len() + line.len() + 1 > 8192);
}

#[test]
#[ignore = "owned child entered only by stderr boundary test"]
fn emergency_stderr_child() {
    let path = root("emergency-stderr");
    assert!(!path.exists(), "fixture root must be fresh");
    let limits = TelemetryLimits {
        spool_bytes: 128,
        ..Default::default()
    };
    let mut store = Store::open(&path, limits, None).unwrap();
    fs::create_dir(path.join("emergency")).unwrap();
    let mut producer = producer(&store, limits);
    let key = producer.reserve(RecordId::new([8; 16]).unwrap()).unwrap();
    assert_eq!(
        producer.emit_log(key, input()).unwrap().pending.wait(WAIT),
        Err(TelemetryError::Capacity)
    );
    assert_eq!(store.health().emergency_records, 1);
    assert_eq!(store.health().committed, 0);
    assert_eq!(store.health().rejected, 1);
    let mut reader = DiagnosticReader::open(&path).unwrap();
    assert!(matches!(
        reader.query(&QueryFilter::default(), 0, 100),
        Err(TelemetryError::Io)
    ));
    assert!(path.join("emergency").is_dir());
    producer.shutdown(WAIT).unwrap();
    store.shutdown(WAIT).unwrap();
}

#[test]
#[ignore = "owned child entered only by emergency bounds test"]
fn emergency_bounds_child() {
    let path = root("emergency-bound");
    assert!(!path.exists(), "fixture root must be fresh");
    let limits = TelemetryLimits {
        spool_bytes: 128,
        ..Default::default()
    };
    for lifetime in 0..3 {
        let mut store = Store::open(&path, limits, None).unwrap();
        let mut producer = producer(&store, limits);
        for record in 1..=34 {
            let key = producer
                .reserve(RecordId::new([record; 16]).unwrap())
                .unwrap();
            assert_eq!(
                producer.emit_log(key, input()).unwrap().pending.wait(WAIT),
                Err(TelemetryError::Capacity)
            );
        }
        let health = store.health();
        assert_eq!(health.emergency_records, 32);
        assert_eq!(health.rejected, 34);
        assert_eq!(health.capacity, 34);
        assert_eq!(health.unconfirmed_gaps, 37);
        assert_eq!(health.admitted, 34);
        assert_eq!(health.committed, 0);
        assert_eq!(health.spooled, 0);
        let mut reader = DiagnosticReader::open(&path).unwrap();
        let page = reader.query(&QueryFilter::default(), 0, 100).unwrap();
        assert!(page.records.is_empty());
        assert!(page.unconfirmed_capture_gaps);
        assert_eq!(
            page.emergency_bytes,
            fs::read(path.join("emergency")).unwrap()
        );
        assert!(page.emergency_bytes.len() <= 8192);
        if lifetime == 0 {
            assert_eq!(
                safe_lines(&page.emergency_bytes, TelemetryError::Capacity),
                32
            );
        }
        producer.shutdown(WAIT).unwrap();
        store.shutdown(WAIT).unwrap();
    }
}
