//! Size, count and time bounds of the native store, its wire decoder and its diagnostic reader.
use std::time::Duration;

/// Distinct records one store root holds in its lifetime: spool frames are never deleted, and
/// the identity index is rebuilt from every frame at open.
pub const MAXIMUM_ROOT_RECORDS: usize = 32_768;
/// Spool frames one root holds in its lifetime; equal to the record bound, so a root that
/// spools one record per frame reaches both together.
pub(crate) const MAXIMUM_SPOOL_FILES: usize = MAXIMUM_ROOT_RECORDS;
/// Free space kept below `sqlite_bytes` for WAL and index growth during one commit.
pub(crate) const MAXIMUM_SQLITE_GROWTH_RESERVE_BYTES: u64 = 4 * 1024 * 1024;
/// How long a statement waits for a competing SQLite lock before the sink reports unavailable.
pub(crate) const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_millis(50);

/// Emergency lines kept per store lifetime; later failures count as unconfirmed gaps.
pub(crate) const MAXIMUM_EMERGENCY_RECORDS: u64 = 32;
/// Size bound of the emergency file; the diagnostic reader refuses a larger one as corrupt.
pub(crate) const MAXIMUM_EMERGENCY_FILE_BYTES: u64 = 8192;

/// Ingestion lag above which health reports `Readiness::Warning`.
pub(crate) const WARNING_LAG_MS: u64 = 30_000;
/// Ingestion lag above which health reports `Readiness::Degraded`.
pub(crate) const DEGRADED_LAG_MS: u64 = 120_000;
/// How long the idle worker waits for a command before retrying its durable backlog.
pub(crate) const WORKER_IDLE_POLL: Duration = Duration::from_millis(50);
/// Back-off after a failed SQLite commit, so a blocked sink does not slow every admission.
pub(crate) const SINK_RETRY_BACKOFF: Duration = Duration::from_millis(250);
/// How long a crash-test barrier waits for its release before failing the stage.
pub(crate) const BARRIER_RELEASE_TIMEOUT: Duration = Duration::from_secs(30);

// The decoder's structural budgets are a wire contract: df-observe's producer bounds keep
// caller input inside them.
/// Deepest nesting of OTLP messages and values the ingress decoder walks.
pub(crate) const MAXIMUM_MESSAGE_DEPTH: usize = 12;
/// Most fields in one decoded OTLP message.
pub(crate) const MAXIMUM_MESSAGE_FIELDS: usize = 128;
/// Most fields across one decoded batch.
pub(crate) const MAXIMUM_BATCH_FIELDS: usize = 4096;

// D01 freezes these read bounds (planning/c-df-telemetry-d01-decision.md).
/// Most rows one diagnostic page returns.
pub const MAXIMUM_PAGE_ROWS: usize = 100;
/// Response budget; each row is charged twice its OTLP bytes (hex output) plus overhead.
pub(crate) const MAXIMUM_RESPONSE_BYTES: usize = 1024 * 1024;
/// Fixed page overhead charged before any row.
pub(crate) const PAGE_OVERHEAD_BYTES: usize = 32 * 1024;
/// Fixed per-row overhead charged beside its OTLP bytes.
pub(crate) const ROW_OVERHEAD_BYTES: usize = 256;
/// Longest session, operation or build filter text.
pub(crate) const MAXIMUM_FILTER_TEXT_BYTES: usize = 128;
