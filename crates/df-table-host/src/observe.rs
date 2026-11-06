//! The event sink of the host (H2): each typed event as one audit line and one structured record,
//! through bounded outlets, with its health visible.
//!
//! `df-table` produces `TableEvent`s and never decides where they go. This module is where they
//! go: `sink` (`HostEventSink`, the table's `EventSink` port) renders each event twice from the
//! same value, `record` (the structured rendering, with the correlation lifted into the `df.*`
//! attributes the telemetry store indexes), and hands the record to its outlets (`outlet`), of
//! which there are three: `memory` (the history that `history` and `trace` read), `spool` (a
//! durable JSON-lines file with its own writer thread) and `shared_path` (df-observe's shared
//! diagnostic path to the telemetry store). Every bound is a very large named constant in
//! `limits`, and reaching one is counted by class and shown by `health`, never silent.
//!
//! What is wired, and what is not, is stated in `planning/gameplay-engine-redesign.md` (O.12 and
//! H.16): the dev engine keeps the history and, when `DF_TABLE_EVENT_SPOOL` names a directory, the
//! spool; the shared path is built and tested against the real df-telemetry store, but a process
//! composes it (the producer identity, the ingress and the store are the composition's), which
//! the dev host does not yet do.

mod limits;
mod memory;
mod outlet;
mod record;
mod shared_path;
mod sink;
mod spool;

pub use self::limits::{
    DEFAULT_HISTORY_RECORDS, EVENT_SPOOL_ENVIRONMENT_VARIABLE, MAXIMUM_ATTRIBUTE_DEPTH,
    MAXIMUM_RECORDS_PER_HISTORY_ANSWER, MAXIMUM_RETAINED_BYTES, MAXIMUM_RETAINED_RECORDS,
    MAXIMUM_SPOOL_BYTES, MAXIMUM_SPOOL_QUEUE_BYTES, RECORD_OVERHEAD_BYTES, SPOOL_FILE_NAME,
};
pub use self::memory::{HistoryFilter, HistoryPage, MemoryOutlet, Retention};
pub use self::outlet::{Losses, OutletError, OutletStatus, RecordOutlet};
pub use self::record::{
    AttributeValue, ObservedRecord, RecordClass, RecordCorrelation, RecordDraft, RecordError,
    RecordScope, RecordSeverity,
};
pub use self::shared_path::{SharedPathOutlet, diagnostic_record};
pub use self::sink::{EventLevel, HostEventSink, SinkHealth, WallClock};
pub use self::spool::{FileSpool, SpoolError};

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod record_tests;
#[cfg(test)]
mod shared_path_tests;
#[cfg(test)]
mod sink_tests;
#[cfg(test)]
mod spool_tests;
