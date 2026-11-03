//! Where records go after the sink has stamped them: the `RecordOutlet` port and what every
//! outlet reports about itself.
//!
//! An outlet takes one record without blocking the owner and without failing the caller: a record
//! it cannot take is refused with a typed `OutletRefusal` and counted by class, so a loss is
//! always a number a reader can see, and a class A loss is told apart from a class B one. There
//! are three: `MemoryOutlet` (the history `history` and `trace` read), `FileSpool` (a durable
//! JSON-lines file written by its own thread) and `SharedPathOutlet` (df-observe's shared
//! diagnostic path to the telemetry store). It owns the port, the counters outlets share and the
//! status type; it never decides which records an outlet gets.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::record::{ObservedRecord, RecordClass};

/// Why an outlet did not take a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutletRefusal {
    /// The outlet is at its bound (a full spool, a queue of unwritten records, a path at its limit).
    Full,
    /// The outlet is closed or its writer has gone.
    Closed,
    /// The outlet cannot represent this record (too large, malformed).
    Rejected,
}

impl OutletRefusal {
    pub fn key(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Closed => "closed",
            Self::Rejected => "rejected",
        }
    }
}

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
    fn accept(&self, record: &ObservedRecord) -> Result<(), OutletRefusal>;

    /// The outlet's counters as of `now_ms`, in milliseconds since the Unix epoch.
    fn status(&self, now_ms: u64) -> OutletStatus;
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
