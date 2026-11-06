//! Where records go after the sink has stamped them: the `RecordOutlet` port and what every
//! outlet reports about itself.
//!
//! An outlet takes one record without blocking the owner and without failing the caller: a record
//! it cannot take is refused with a typed `OutletError` and counted by class, so a loss is
//! always a number a reader can see, and a class A loss is told apart from a class B one. There
//! are three: `MemoryOutlet` (the history `history` and `trace` read), `FileSpool` (a durable
//! JSON-lines file written by its own thread) and `SharedPathOutlet` (df-observe's shared
//! diagnostic path to the telemetry store). It owns the port, the counters outlets share and the
//! status type; it never decides which records an outlet gets.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::event_log::EventLog;

use super::record::{ObservedRecord, RecordClass};

/// Why an outlet did not take a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutletError {
    /// The outlet is at its bound (a full spool, a queue of unwritten records, a path at its
    /// limit).
    Full,
    /// The outlet is closed or its writer has gone.
    Closed,
    /// The outlet cannot represent this record (too large, malformed).
    Unrepresentable,
}

impl OutletError {
    pub const ALL: [Self; 3] = [Self::Full, Self::Closed, Self::Unrepresentable];

    pub fn key(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Closed => "closed",
            Self::Unrepresentable => "unrepresentable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|error| error.key() == key)
    }
}

impl fmt::Display for OutletError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Full => "the outlet is at its bound",
            Self::Closed => "the outlet is closed",
            Self::Unrepresentable => "the outlet cannot represent the record",
        })
    }
}

impl std::error::Error for OutletError {}

/// What one outlet says about itself: how much it took, how much is safe, and how much is lost.
///
/// `accepted` counts records handed over; `confirmed` those the outlet has made safe (written to
/// the file, committed behind a receipt; for memory, all of them). `pending` is the rest, and
/// `lag_ms` is the age of the oldest of those. `lost_always` and `lost_verbose` count records
/// that were refused or could not be written, by class: they are never silent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutletStatus {
    pub name: String,
    pub accepted: u64,
    pub confirmed: u64,
    pub pending: u64,
    pub lag_ms: u64,
    pub lost_always: u64,
    pub lost_verbose: u64,
    /// Records the outlet forgot to stay within its memory bound (the history only).
    pub evicted: u64,
    /// Records handed over whose receipt failed, so they may be lost (the shared path only).
    pub unconfirmed: u64,
}

/// The port every outlet implements.
pub trait RecordOutlet: Send + Sync {
    /// The outlet's name in `health`.
    fn name(&self) -> &'static str;

    /// Takes one record. It must not block on a disk or a network and must not panic.
    fn accept(&self, record: &ObservedRecord) -> Result<(), OutletError>;

    /// The outlet's counters as of `now_ms`, in milliseconds since the Unix epoch.
    fn status(&self, now_ms: u64) -> OutletStatus;

    /// Keeps the sink's `notice`, so an outlet that can lose a record after taking it (a writer
    /// that cannot write, a receipt that fails) says so when it finds the loss. An outlet that
    /// only loses at `accept` (where the sink sees the refusal) keeps nothing.
    fn watch_losses(&self, _notice: Arc<DegradedNotice>) {}
}

/// Why an outlet first lost a record, in the `HOST sink_degraded` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LossCause {
    /// The outlet refused the record at `accept`.
    Refused(OutletError),
    /// The spool was at its size bound when its writer came to the record.
    SizeBound,
    /// The spool's writer could not write the record, or could not flush its batch.
    WriteFailed,
    /// The shared path took the record but its receipt failed, so it may be lost.
    ReceiptFailed,
}

impl LossCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::Refused(error) => error.key(),
            Self::SizeBound => "size_bound",
            Self::WriteFailed => "write_failed",
            Self::ReceiptFailed => "receipt_failed",
        }
    }
}

/// The one `HOST sink_degraded` line of a sink, written when the first loss is found, by the sink
/// (a refusal at `accept`) or by the outlet that found it (its writer or its receipts), so the
/// line's time is when the loss was found, not when someone next read the health.
pub struct DegradedNotice {
    log: Arc<EventLog>,
    written: AtomicBool,
}

impl DegradedNotice {
    pub fn new(log: Arc<EventLog>) -> Self {
        Self {
            log,
            written: AtomicBool::new(false),
        }
    }

    /// Writes `HOST sink_degraded outlet=<name> cause=<key> lost_always=<n>` the first time it is
    /// called; every later call writes nothing (later losses are counts in `health`).
    pub fn first_loss(&self, outlet: &str, cause: LossCause, lost_always: u64) {
        if !self.written.swap(true, Ordering::Relaxed) {
            self.log.push(format!(
                "HOST sink_degraded outlet={outlet} cause={} lost_always={lost_always}",
                cause.key()
            ));
        }
    }
}

/// Counts of lost records by class, shared by outlets.
#[derive(Debug, Default)]
pub struct Losses {
    always: AtomicU64,
    verbose: AtomicU64,
}

impl Losses {
    pub fn record(&self, class: RecordClass) {
        let counter = match class {
            RecordClass::Always => &self.always,
            RecordClass::Verbose => &self.verbose,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn always(&self) -> u64 {
        self.always.load(Ordering::Relaxed)
    }

    pub fn verbose(&self) -> u64 {
        self.verbose.load(Ordering::Relaxed)
    }
}
