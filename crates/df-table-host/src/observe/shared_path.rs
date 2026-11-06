//! The outlet onto df-observe's shared diagnostic path: unsampled native logs through the one
//! producer to the durable ingress and the telemetry store.
//!
//! It adapts one `ObservedRecord` to df-observe's `DiagnosticRecord` (the audit line as the body,
//! the attributes as typed values, the event name, severity and time) and hands it to a
//! `NativeDiagnosticSink` that the composition owns: the producer identity, the ingress and the
//! store belong to whoever starts the process, never to this adapter. Admission is a bounded,
//! nonblocking encode on the owner's thread; durability is the ingress's, and a record counts as
//! `confirmed` only when its receipt arrives. A refusal (a full queue, a closed ingress, a record
//! the path will not take) is counted by class and leaves a source gap the store itself reports.
//! A receipt that fails is counted as `unconfirmed`; the first one the path finds (it harvests
//! receipts at each record it takes and at each status read) writes the sink's one
//! `HOST sink_degraded outlet=shared_path cause=receipt_failed` line.
//!
//! Ingestion lag is the age of the oldest record still waiting for its receipt. It keeps no
//! trace: spans (H.10) are not carried by this adapter, only logs.
//!
//! `ComposedSharedPath` is the composition `serve` makes when it is given a telemetry store root:
//! the df-telemetry store at that root, one df-observe producer for this host and the outlet over
//! it. `SharedPathHealth` is what a status read shows of it: how much reached the store's spool,
//! how much was confirmed, how much was lost, and the lag.

use std::collections::VecDeque;
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use df_observe::{
    AnyValue, DiagnosticRecord, DiagnosticSink, LogInput, NativeDiagnosticSink, NativeProducer,
    ProducerId, Severity, SourceSequence, TelemetryError, TelemetryLimits,
};
use df_telemetry::Store;
use df_types::BuildIdentity;
use serde::{Deserialize, Serialize};

use super::limits::MAXIMUM_SHARED_PATH_PENDING_RECEIPTS;
use super::outlet::{DegradedNotice, LossCause, Losses, OutletError, OutletStatus, RecordOutlet};
use super::record::{AttributeValue, ObservedRecord, RecordSeverity};

/// The shared path's name among a sink's outlets.
pub const SHARED_PATH_OUTLET: &str = "shared_path";
/// The service name the host's producer writes into every record.
const SERVICE_NAME: &str = "df-table-host";

/// What a status read shows of the shared path: lag and loss, without the telemetry binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedPathHealth {
    /// Records the path handed to the store's ingress.
    pub spooled: u64,
    /// Records the store confirmed with a receipt.
    pub exported: u64,
    /// Records refused by the path or whose receipt failed: lost, or not known to be kept.
    pub dropped: u64,
    /// The age of the oldest record still waiting for its receipt.
    pub lag_ms: u64,
}

impl SharedPathHealth {
    /// The health read from the shared path's outlet status.
    pub fn of(status: &OutletStatus) -> Self {
        Self {
            spooled: status.accepted,
            exported: status.confirmed,
            dropped: status
                .lost_always
                .saturating_add(status.lost_verbose)
                .saturating_add(status.unconfirmed),
            lag_ms: status.lag_ms,
        }
    }
}

/// Why the shared path could not be composed or closed, as a closed key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedPathCause {
    /// The root is not an absolute path the store may own (relative, a link, `..`).
    ForeignRoot,
    /// Another owner holds the root, or the store found a conflicting record there.
    Ownership,
    /// The store's files at the root are damaged.
    Corrupt,
    /// The disk refused a read or a write.
    Io,
    /// The producer's identity or limits were refused.
    InvalidConfiguration,
    /// The store is closed, at capacity, past a deadline or failed inside.
    Unavailable,
}

impl SharedPathCause {
    pub const ALL: [Self; 6] = [
        Self::ForeignRoot,
        Self::Ownership,
        Self::Corrupt,
        Self::Io,
        Self::InvalidConfiguration,
        Self::Unavailable,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::ForeignRoot => "foreign_root",
            Self::Ownership => "ownership",
            Self::Corrupt => "corrupt",
            Self::Io => "io",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }

    /// Exhaustive, so a new telemetry error does not compile until it is placed.
    fn of(error: &TelemetryError) -> Self {
        match error {
            TelemetryError::ForeignRoot => Self::ForeignRoot,
            TelemetryError::Ownership | TelemetryError::Conflict => Self::Ownership,
            TelemetryError::Corrupt => Self::Corrupt,
            TelemetryError::Io => Self::Io,
            TelemetryError::InvalidIdentity
            | TelemetryError::InvalidLimits
            | TelemetryError::SequenceExhausted
            | TelemetryError::Malformed
            | TelemetryError::Oversized => Self::InvalidConfiguration,
            TelemetryError::Capacity
            | TelemetryError::Closed
            | TelemetryError::Deadline
            | TelemetryError::SinkUnavailable
            | TelemetryError::Internal => Self::Unavailable,
        }
    }
}

/// The shared path could not be composed or closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedPathError {
    pub cause: SharedPathCause,
}

impl SharedPathError {
    fn of(error: &TelemetryError) -> Self {
        Self {
            cause: SharedPathCause::of(error),
        }
    }
}

impl fmt::Display for SharedPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the telemetry store's shared path is unavailable ({})",
            self.cause.key()
        )
    }
}

impl std::error::Error for SharedPathError {}

/// Opaque entropy for this process, made once from its id and start time; never from content.
fn process_entropy() -> u64 {
    static ENTROPY: OnceLock<u64> = OnceLock::new();
    *ENTROPY.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        df_observe::operation_hash(format!("{}:{nanos}", std::process::id()).as_bytes())
    })
}

/// The host producer's identity: this process's entropy and id, so two hosts never share one.
fn producer_identity() -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&process_entropy().to_be_bytes());
    bytes[8..].copy_from_slice(&u64::from(std::process::id()).to_be_bytes());
    bytes
}

/// Fresh opaque entropy for each record, as `NativeDiagnosticSink` asks: the process's entropy
/// and a counter, independent of what the record says.
fn next_record_entropy() -> [u8; 16] {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&process_entropy().to_be_bytes());
    bytes[8..].copy_from_slice(&(NEXT.fetch_add(1, Ordering::Relaxed) + 1).to_be_bytes());
    bytes
}

/// The `HOST shared_path` line: `composed=true root_set=true` when the path is composed,
/// `composed=false root_set=false` when no root was given, and `composed=false root_set=true
/// cause=<key>` when the root was given and the path could not be composed. Never the root.
pub fn shared_path_line(root_set: bool, failure: Option<SharedPathCause>) -> String {
    match (root_set, failure) {
        (false, _) => "HOST shared_path composed=false root_set=false".to_owned(),
        (true, None) => "HOST shared_path composed=true root_set=true".to_owned(),
        (true, Some(cause)) => format!(
            "HOST shared_path composed=false root_set=true cause={}",
            cause.key()
        ),
    }
}

/// The shared path for `root` when one is given, and its `HOST shared_path` line. It does disk
/// work, so an async caller runs it in `spawn_blocking`.
pub fn compose_shared_path(root: Option<&Path>) -> (Option<ComposedSharedPath>, String) {
    let Some(root) = root else {
        return (None, shared_path_line(false, None));
    };
    match ComposedSharedPath::open(root) {
        Ok(path) => (Some(path), shared_path_line(true, None)),
        Err(error) => (None, shared_path_line(true, Some(error.cause))),
    }
}

/// The store, the producer and the outlet `serve` composes for a configured telemetry root.
pub struct ComposedSharedPath {
    store: Store,
    diagnostics: Arc<NativeDiagnosticSink>,
    outlet: Arc<SharedPathOutlet>,
}

impl ComposedSharedPath {
    /// Opens (creating it) the store at `root` and the host's producer onto it. It does disk
    /// work, so an async caller runs it in `spawn_blocking`.
    pub fn open(root: &Path) -> Result<Self, SharedPathError> {
        let root = std::path::absolute(root).map_err(|_| SharedPathError {
            cause: SharedPathCause::ForeignRoot,
        })?;
        let store = Store::open_configured(&root, TelemetryLimits::default())
            .map_err(|error| SharedPathError::of(&error))?;
        let version = concat!("df-table-host-", env!("CARGO_PKG_VERSION"));
        let build = BuildIdentity::new(
            Some(version),
            Some(version),
            Some("not-applicable"),
            Some("df-table-host-telemetry-v1"),
            Some("not-applicable"),
        )
        .map_err(|_| SharedPathError {
            cause: SharedPathCause::InvalidConfiguration,
        })?;
        let producer = NativeProducer::for_service(
            ProducerId::new(producer_identity()).map_err(|error| SharedPathError::of(&error))?,
            SourceSequence::new(1).map_err(|error| SharedPathError::of(&error))?,
            &build,
            store.ingress(),
            TelemetryLimits::default(),
            SERVICE_NAME,
        )
        .map_err(|error| SharedPathError::of(&error))?;
        let diagnostics = Arc::new(
            NativeDiagnosticSink::new(
                producer,
                MAXIMUM_SHARED_PATH_PENDING_RECEIPTS,
                next_record_entropy,
            )
            .map_err(|error| SharedPathError::of(&error))?,
        );
        let outlet = Arc::new(SharedPathOutlet::new(Arc::clone(&diagnostics)));
        Ok(Self {
            store,
            diagnostics,
            outlet,
        })
    }

    /// The outlet a sink adds to send its records down the path.
    pub fn outlet(&self) -> Arc<SharedPathOutlet> {
        Arc::clone(&self.outlet)
    }

    /// Waits up to `timeout` for the receipts already admitted, then commits and stops the store.
    /// It blocks, so an async caller runs it in `spawn_blocking`.
    pub fn shutdown(mut self, timeout: Duration) -> Result<(), SharedPathError> {
        let drained = self.diagnostics.shutdown(timeout);
        let stopped = self.store.shutdown(timeout);
        drained
            .and(stopped)
            .map_err(|error| SharedPathError::of(&error))
    }
}

/// The shared diagnostic path as an outlet.
pub struct SharedPathOutlet {
    sink: Arc<NativeDiagnosticSink>,
    /// The observed time of each record still waiting for its receipt, oldest first.
    waiting: Mutex<VecDeque<u64>>,
    losses: Losses,
    /// The sink's `HOST sink_degraded` line, written when a receipt is first found failed.
    notice: OnceLock<Arc<DegradedNotice>>,
}

impl SharedPathOutlet {
    pub fn new(sink: Arc<NativeDiagnosticSink>) -> Self {
        Self {
            sink,
            waiting: Mutex::new(VecDeque::new()),
            losses: Losses::default(),
            notice: OnceLock::new(),
        }
    }

    /// Writes the sink's one `sink_degraded` line once a receipt has failed: the record may be
    /// lost, and its class is not known, so `lost_always` stays the refusals' count.
    fn announce_receipt_failures(&self, receipt_failures: u64) {
        if receipt_failures > 0
            && let Some(notice) = self.notice.get()
        {
            notice.first_loss(
                SHARED_PATH_OUTLET,
                LossCause::ReceiptFailed,
                self.losses.always(),
            );
        }
    }

    fn locked(&self) -> MutexGuard<'_, VecDeque<u64>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps only the records the path still reports as waiting: it settles them in order, so the
    /// newest `pending` are the ones whose receipt has not come, and a receipt that failed is no
    /// longer waiting (the path counts it as a receipt failure).
    fn forget_settled(&self, waiting: &mut VecDeque<u64>) {
        let pending = usize::try_from(self.sink.health().pending).unwrap_or(usize::MAX);
        while waiting.len() > pending {
            waiting.pop_front();
        }
    }
}

fn severity(severity: RecordSeverity) -> Severity {
    match severity {
        RecordSeverity::Debug => Severity::Debug,
        RecordSeverity::Info => Severity::Info,
        RecordSeverity::Warn => Severity::Warn,
        RecordSeverity::Error => Severity::Error,
    }
}

fn value(value: &AttributeValue) -> AnyValue {
    match value {
        AttributeValue::Text(text) => AnyValue::String(text.clone().into()),
        AttributeValue::Whole(whole) => AnyValue::Int(*whole),
        AttributeValue::Flag(flag) => AnyValue::Boolean(*flag),
    }
}

/// One record as df-observe takes it: its audit line is the body, so the two renderings carry the
/// same text, and its time is the observed time in both of the record's clocks.
pub fn diagnostic_record(record: &ObservedRecord) -> DiagnosticRecord {
    let at = UNIX_EPOCH + Duration::from_millis(record.observed_ms);
    DiagnosticRecord {
        event_name: record.event_name,
        input: LogInput {
            timestamp: at,
            observed_timestamp: at,
            severity: severity(record.severity),
            body: AnyValue::String(record.audit_line.clone().into()),
            attributes: record
                .attributes
                .iter()
                .map(|(name, attribute)| (name.clone(), value(attribute)))
                .collect(),
            trace: None,
        },
    }
}

/// How a refusal of the shared path reads to the sink. Exhaustive, so a new refusal of the path
/// does not compile until it is placed.
fn outlet_error_of(error: &TelemetryError) -> OutletError {
    match error {
        TelemetryError::Capacity | TelemetryError::SequenceExhausted => OutletError::Full,
        TelemetryError::Oversized
        | TelemetryError::Malformed
        | TelemetryError::InvalidIdentity
        | TelemetryError::InvalidLimits => OutletError::Unrepresentable,
        TelemetryError::Closed
        | TelemetryError::Deadline
        | TelemetryError::Conflict
        | TelemetryError::Corrupt
        | TelemetryError::ForeignRoot
        | TelemetryError::Ownership
        | TelemetryError::Io
        | TelemetryError::SinkUnavailable
        | TelemetryError::Internal => OutletError::Closed,
    }
}

impl RecordOutlet for SharedPathOutlet {
    fn name(&self) -> &'static str {
        SHARED_PATH_OUTLET
    }

    fn accept(&self, record: &ObservedRecord) -> Result<(), OutletError> {
        let mut waiting = self.locked();
        match self.sink.emit(diagnostic_record(record)) {
            Ok(()) => {
                waiting.push_back(record.observed_ms);
                self.forget_settled(&mut waiting);
                drop(waiting);
                self.announce_receipt_failures(self.sink.health().receipt_failures);
                Ok(())
            }
            Err(error) => {
                self.losses.record(record.class);
                Err(outlet_error_of(&error))
            }
        }
    }

    fn status(&self, now_ms: u64) -> OutletStatus {
        self.sink.poll_receipts();
        let mut waiting = self.locked();
        self.forget_settled(&mut waiting);
        let health = self.sink.health();
        let lag_ms = waiting
            .front()
            .map_or(0, |observed| now_ms.saturating_sub(*observed));
        drop(waiting);
        self.announce_receipt_failures(health.receipt_failures);
        OutletStatus {
            name: self.name().to_owned(),
            accepted: health.admitted,
            confirmed: health.confirmed,
            pending: health.pending,
            lag_ms,
            lost_always: self.losses.always(),
            lost_verbose: self.losses.verbose(),
            evicted: 0,
            unconfirmed: health.receipt_failures,
        }
    }

    fn watch_losses(&self, notice: Arc<DegradedNotice>) {
        // The path's outlet belongs to one sink; a second sink's notice is not kept.
        let _ = self.notice.set(notice);
    }
}
