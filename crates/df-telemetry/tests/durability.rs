#![cfg(not(target_arch = "wasm32"))]
use df_observe::{Durability, Signal, TelemetryError, TelemetryLimits};
use df_telemetry::{BarrierStage, DiagnosticReader, FailureBarrier, QueryFilter, QueryPage, Store};
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::{AnyValue, KeyValue, any_value::Value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
};
use prost::Message;
use rusqlite::Connection;
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

const FIXTURE_TIMEOUT: Duration = Duration::from_secs(5);
const MAXIMUM_OUTPUT_BYTES: usize = 8192;
const READY_MARKER: &[u8] = b"durability-ready\n";
const DURABILITY_CASES: [&str; 6] = [
    "durable-preaccept",
    "durable-spooled",
    "durable-committed",
    "durable-spooled-receipt",
    "durable-committed-receipt",
    "durable-spool-write-refusal",
];

fn fixture_root(namespace: &str, case: &str) -> PathBuf {
    assert!(DURABILITY_CASES.contains(&case));
    assert!(!namespace.is_empty());
    assert!(
        namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    );
    let path =
        PathBuf::from("/Users/earlcameron/Desktop/dungeonflux/development/runtime/telemetry-g06")
            .join(format!("ownedfixture-{namespace}-{case}"));
    assert!(path.file_name().unwrap().len() <= 100);
    path
}

fn attribute(name: &str, value: Value) -> KeyValue {
    KeyValue {
        key: name.into(),
        value: Some(AnyValue { value: Some(value) }),
    }
}

fn record(record_id_byte: u8, sequence: i64) -> LogRecord {
    LogRecord {
        time_unix_nano: 123,
        body: Some(AnyValue {
            value: Some(Value::StringValue(format!("durable-record-{sequence}"))),
        }),
        attributes: vec![
            attribute("df.producer_id", Value::StringValue("07".repeat(16))),
            attribute(
                "df.record_id",
                Value::StringValue(format!("{record_id_byte:02x}").repeat(16)),
            ),
            attribute("df.source_sequence", Value::IntValue(sequence)),
        ],
        ..Default::default()
    }
}

fn request(records: Vec<LogRecord>) -> Vec<u8> {
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: records,
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

fn durability_batch() -> Vec<u8> {
    request(vec![record(8, 1), record(9, 2)])
}

fn query(path: &Path) -> QueryPage {
    DiagnosticReader::open(path)
        .unwrap()
        .query(&QueryFilter::default(), 0, 100)
        .unwrap()
}

fn assert_corpus(page: &QueryPage) {
    assert_eq!(page.records.len(), 2);
    assert_eq!(page.committed_watermark, 2);
    for (view, (record_id_byte, sequence)) in page.records.iter().zip([(8, 1), (9, 2)]) {
        assert_eq!(view.position, sequence as u64);
        assert_eq!(view.producer.bytes(), [7; 16]);
        assert_eq!(view.record.bytes(), [record_id_byte; 16]);
        assert_eq!(view.source_sequence, sequence);
        assert_eq!(view.otlp, request(vec![record(record_id_byte, sequence)]));
    }
    assert!(page.unconfirmed_capture_gaps);
}

// Drain both pipes throughout the child's lifetime; retain only bounded synthetic output.
fn drain(mut pipe: impl Read, ready: Option<mpsc::SyncSender<()>>) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0; 1024];
    let mut ready = ready;
    loop {
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            return Ok(output);
        }
        let retained = count.min(MAXIMUM_OUTPUT_BYTES.saturating_sub(output.len()));
        output.extend_from_slice(&buffer[..retained]);
        if output
            .windows(READY_MARKER.len())
            .any(|bytes| bytes == READY_MARKER)
            && let Some(sender) = ready.take()
        {
            sender.try_send(()).map_err(io::Error::other)?;
        }
    }
}

struct FixtureProcess {
    child: Child,
    ready: mpsc::Receiver<()>,
    stdout: Option<JoinHandle<io::Result<Vec<u8>>>>,
    stderr: Option<JoinHandle<io::Result<Vec<u8>>>>,
}

impl FixtureProcess {
    fn spawn(namespace: &str, case: &str, phase: &str) -> Self {
        // Re-execute this freshly compiled test, never an earlier telemetry CLI build.
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "durability_process_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("DF_TELEMETRY_FIXTURE_NAMESPACE", namespace)
            .env("DF_TELEMETRY_DURABILITY_CASE", case)
            .env("DF_TELEMETRY_DURABILITY_PHASE", phase)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (sender, ready) = mpsc::sync_channel(1);
        let mut fixture = Self {
            child,
            ready,
            stdout: None,
            stderr: None,
        };
        let stdout = fixture.child.stdout.take().unwrap();
        fixture.stdout = Some(thread::spawn(move || drain(stdout, Some(sender))));
        let stderr = fixture.child.stderr.take().unwrap();
        fixture.stderr = Some(thread::spawn(move || drain(stderr, None)));
        fixture
    }

    fn wait_ready(&self) {
        self.ready.recv_timeout(FIXTURE_TIMEOUT).unwrap();
    }

    fn finish(mut self, crash: bool) {
        if crash {
            self.child.kill().unwrap();
        }
        let deadline = Instant::now() + FIXTURE_TIMEOUT;
        let status: ExitStatus = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "fixture child exceeded deadline");
            // This polls only owned OS-process completion, not ingestion ordering.
            thread::sleep(Duration::from_millis(5));
        };
        self.child.wait().unwrap();
        let stdout = self.stdout.take().unwrap().join().unwrap().unwrap();
        let stderr = self.stderr.take().unwrap().join().unwrap().unwrap();
        assert_eq!(
            status.success(),
            !crash,
            "child status={status} stdout={} stderr={}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        );
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_)))
            && let Err(error) = self.child.kill()
        {
            eprintln!("owned fixture kill failed: {error}");
        }
        if let Err(error) = self.child.wait() {
            eprintln!("owned fixture reap failed: {error}");
        }
        for reader in [self.stdout.take(), self.stderr.take()]
            .into_iter()
            .flatten()
        {
            match reader.join() {
                Ok(Ok(_)) => (),
                result => eprintln!("owned fixture pipe drain failed: {result:?}"),
            }
        }
    }
}

fn wait_for_process_death() {
    println!("durability-ready");
    io::stdout().flush().unwrap();
    // The parent owns termination after the marker. Keep the receipt/barrier alive.
    let mut byte = [0];
    io::stdin().read_exact(&mut byte).unwrap();
    panic!("crash fixture must be killed by its owner");
}

fn crash_at_barrier(path: &Path, stage: BarrierStage) {
    let (reached, receiver) = mpsc::sync_channel(1);
    let (_release, gate) = mpsc::channel();
    let store = Store::open(
        path,
        TelemetryLimits::default(),
        Some(FailureBarrier {
            stage,
            reached,
            release: gate,
        }),
    )
    .unwrap();
    let pending = store
        .ingress()
        .submit(Signal::Logs, &durability_batch())
        .unwrap();
    assert_eq!(receiver.recv_timeout(FIXTURE_TIMEOUT).unwrap(), stage);
    assert_eq!(pending.try_receive().unwrap(), None);
    let health = store.health();
    assert_eq!(health.checkpoint, 0);
    assert_eq!(
        health.spooled,
        if stage == BarrierStage::BeforeSpool {
            0
        } else {
            2
        }
    );
    assert_eq!(
        health.committed,
        if stage == BarrierStage::CommittedBeforeCheckpoint {
            2
        } else {
            0
        }
    );
    wait_for_process_death();
}

fn crash_after_receipt(path: &Path, durability: Durability) {
    let store = Store::open(path, TelemetryLimits::default(), None).unwrap();
    let lock = Connection::open(path.join("telemetry.sqlite3")).unwrap();
    if durability == Durability::Spooled {
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    }
    let receipt = store
        .ingress()
        .submit(Signal::Logs, &durability_batch())
        .unwrap()
        .wait(FIXTURE_TIMEOUT)
        .unwrap();
    assert_eq!(receipt.durability, durability);
    assert_eq!(receipt.records, 2);
    assert_eq!(receipt.spool_position, 1);
    assert_eq!(store.health().spooled, 2);
    let page = query(path);
    if durability == Durability::Spooled {
        assert_eq!(receipt.committed_watermark, 0);
        assert_eq!(page.spool_checkpoint, 0);
        assert!(page.records.is_empty());
    } else {
        assert_eq!(receipt.committed_watermark, 2);
        assert_eq!(page.spool_checkpoint, receipt.spool_position);
        assert_corpus(&page);
    }
    wait_for_process_death();
}

fn recover_and_retry(path: &Path, case: &str) {
    let preaccept = case == "durable-preaccept";
    let committed = matches!(case, "durable-committed" | "durable-committed-receipt");
    let before = query(path);
    assert_eq!(before.records.len(), if committed { 2 } else { 0 });
    assert_eq!(
        before.spool_checkpoint,
        u64::from(case == "durable-committed-receipt")
    );
    let mut store = Store::open(path, TelemetryLimits::default(), None).unwrap();
    store.flush(FIXTURE_TIMEOUT).unwrap();
    let recovered = query(path);
    if preaccept {
        assert!(recovered.records.is_empty());
        assert_eq!(recovered.committed_watermark, 0);
        assert_eq!(recovered.spool_checkpoint, 0);
    } else {
        assert_corpus(&recovered);
        assert_eq!(recovered.spool_checkpoint, 1);
    }
    let receipt = store
        .ingress()
        .submit(Signal::Logs, &durability_batch())
        .unwrap()
        .wait(FIXTURE_TIMEOUT)
        .unwrap();
    assert_eq!(receipt.durability, Durability::Committed);
    assert_eq!(receipt.records, 2);
    assert_eq!(receipt.committed_watermark, 2);
    let retried = query(path);
    assert_corpus(&retried);
    assert_eq!(retried.spool_checkpoint, receipt.spool_position);
    if !preaccept {
        assert_eq!(
            store.health().duplicates,
            if committed && case != "durable-committed-receipt" {
                4
            } else {
                2
            }
        );
        for (original, retry) in recovered.records.iter().zip(&retried.records) {
            assert_eq!(original.position, retry.position);
            assert_eq!(original.otlp, retry.otlp);
        }
    }
    store.shutdown(FIXTURE_TIMEOUT).unwrap();
}

fn physical_spool_refusal(path: &Path) {
    let (reached, receiver) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::channel();
    let mut store = Store::open(
        path,
        TelemetryLimits::default(),
        Some(FailureBarrier {
            stage: BarrierStage::BeforeSpool,
            reached,
            release: gate,
        }),
    )
    .unwrap();
    let pending = store
        .ingress()
        .submit(Signal::Logs, &durability_batch())
        .unwrap();
    assert_eq!(
        receiver.recv_timeout(FIXTURE_TIMEOUT).unwrap(),
        BarrierStage::BeforeSpool
    );
    assert_eq!(pending.try_receive().unwrap(), None);
    let collision = path.join("spool").join("00000000000000000001.spool");
    fs::create_dir(&collision).unwrap();
    release.send(()).unwrap();
    assert_eq!(pending.wait(FIXTURE_TIMEOUT), Err(TelemetryError::Io));
    let health = store.health();
    assert_eq!(health.spooled, 0);
    assert_eq!(health.committed, 0);
    assert_eq!(health.checkpoint, 0);
    assert_eq!(health.pending_items, 0);
    assert_eq!(health.pending_bytes, 0);
    assert!(query(path).records.is_empty());
    fs::remove_dir(collision).unwrap();
    let receipt = store
        .ingress()
        .submit(Signal::Logs, &durability_batch())
        .unwrap()
        .wait(FIXTURE_TIMEOUT)
        .unwrap();
    assert_eq!(receipt.durability, Durability::Committed);
    assert_eq!(receipt.spool_position, 1);
    assert_eq!(receipt.records, 2);
    assert_corpus(&query(path));
    store.shutdown(FIXTURE_TIMEOUT).unwrap();
}

#[test]
#[ignore = "owned child entrypoint; selected only by this freshly compiled test binary"]
fn durability_process_fixture() {
    let namespace = std::env::var("DF_TELEMETRY_FIXTURE_NAMESPACE").unwrap();
    let case = std::env::var("DF_TELEMETRY_DURABILITY_CASE").unwrap();
    let phase = std::env::var("DF_TELEMETRY_DURABILITY_PHASE").unwrap();
    let path = fixture_root(&namespace, &case);
    if phase == "recover" {
        recover_and_retry(&path, &case);
        return;
    }
    assert!(!path.exists(), "fixture must be fresh: {}", path.display());
    assert_eq!(phase, "initial");
    match case.as_str() {
        "durable-preaccept" => crash_at_barrier(&path, BarrierStage::BeforeSpool),
        "durable-spooled" => crash_at_barrier(&path, BarrierStage::Spooled),
        "durable-committed" => crash_at_barrier(&path, BarrierStage::CommittedBeforeCheckpoint),
        "durable-spooled-receipt" => crash_after_receipt(&path, Durability::Spooled),
        "durable-committed-receipt" => crash_after_receipt(&path, Durability::Committed),
        "durable-spool-write-refusal" => physical_spool_refusal(&path),
        _ => unreachable!(),
    }
}

#[test]
fn recoverable_receipts_follow_disk_success_across_process_death_and_retry() {
    let namespace = std::env::var("DF_TELEMETRY_FIXTURE_NAMESPACE").unwrap_or_else(|_| {
        format!(
            "i02-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    });
    // One parent test owns all six cases, so even default parallel libtest has one child.
    for case in DURABILITY_CASES {
        let path = fixture_root(&namespace, case);
        assert!(!path.exists(), "fixture must be fresh: {}", path.display());
        let child = FixtureProcess::spawn(&namespace, case, "initial");
        if case == "durable-spool-write-refusal" {
            child.finish(false);
        } else {
            child.wait_ready();
            child.finish(true);
            FixtureProcess::spawn(&namespace, case, "recover").finish(false);
        }
        println!(
            "durability_case={case} root={} current_exe={} passed=true",
            path.display(),
            std::env::current_exe().unwrap().display()
        );
    }
}
