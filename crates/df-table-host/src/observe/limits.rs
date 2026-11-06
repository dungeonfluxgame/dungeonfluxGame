//! Every bound of the event sink, in one place.
//!
//! The owner's rule is that there are no small invented caps: a bound is a very large named
//! constant, memory stays bounded by design, and a value is scoped down later when a measurement
//! says so (owner, 2026-11-02). Every bound here is a count of records or bytes; reaching one is
//! never silent: it is counted (`evicted`, `lost_always`, `lost_verbose`) and `health` shows it.

const MEBIBYTE: usize = 1024 * 1024;
const GIBIBYTE: u64 = 1024 * 1024 * 1024;

/// Most records the in-memory history keeps. The oldest are forgotten first and counted.
pub const MAXIMUM_RETAINED_RECORDS: usize = 1_000_000;
/// Most bytes the in-memory history keeps, whatever the number of records: the memory bound.
pub const MAXIMUM_RETAINED_BYTES: usize = 256 * MEBIBYTE;
/// Most records kept by a history nobody queries (the canvas workbench's sink, which is read
/// through its log lines only). A history always keeps its newest record, so this is the least.
pub const UNQUERIED_HISTORY_RECORDS: usize = 1;
/// What a retained record costs besides its text: the struct, the strings' headers and the queue
/// node, so the byte bound is not undercounted.
pub const RECORD_OVERHEAD_BYTES: usize = 256;

/// Most bytes the file spool may hold on disk, including what was there when it was opened. A
/// record that does not fit is refused and counted as lost; nothing is ever deleted to make room.
pub const MAXIMUM_SPOOL_BYTES: u64 = 4 * GIBIBYTE;
/// Most bytes of records that may wait for the spool's writer thread. A writer that stops fills
/// this and then records are refused and counted, so a stalled disk costs memory up to this and
/// nothing else, and never blocks the owner.
pub const MAXIMUM_SPOOL_QUEUE_BYTES: usize = 256 * MEBIBYTE;

/// Most records one `history` answer carries.
pub const MAXIMUM_RECORDS_PER_HISTORY_ANSWER: usize = 10_000;
/// How many records `history` shows when no limit is given.
pub const DEFAULT_HISTORY_RECORDS: usize = 20;

/// Deepest nesting of a typed event that is flattened into attributes. Typed events nest scope,
/// correlation and revision, about three levels, so this is far above any real event; a deeper
/// one is refused as unrenderable and counted, never truncated.
pub const MAXIMUM_ATTRIBUTE_DEPTH: usize = 32;

/// Most records the composed shared path may hand to the telemetry store without a receipt yet.
/// One more is refused and counted as lost, so a stalled store costs memory up to this.
pub const MAXIMUM_SHARED_PATH_PENDING_RECEIPTS: usize = 1_000_000;

/// The environment variable that names a directory for the file spool of the dev engine. Left
/// unset, the dev engine keeps its history in memory only.
pub const EVENT_SPOOL_VARIABLE: &str = "DF_TABLE_EVENT_SPOOL";
/// The spool's file inside that directory: one JSON record per line.
pub const SPOOL_FILE_NAME: &str = "table-events.jsonl";
