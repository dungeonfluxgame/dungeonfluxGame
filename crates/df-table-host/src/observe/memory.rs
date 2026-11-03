//! The in-memory outlet: the bounded history that `history` and `trace` read.
//!
//! It keeps the newest records up to a count and a byte bound and forgets the oldest first,
//! counting every record it forgets so a reader can tell "nothing happened" from "it was
//! forgotten" (`Retention::evicted`). It never refuses a record: staying within its bound is its
//! design, not a loss. A query reads a copy under the lock and the lock is never held while the
//! caller works with the answer.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use super::limits::{MAXIMUM_RETAINED_BYTES, MAXIMUM_RETAINED_RECORDS};
use super::outlet::{OutletRefusal, OutletStatus, RecordOutlet};
use super::record::{ObservedRecord, RecordClass};

/// Which retained records a query wants. A field left out matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryFilter {
    /// The audit area, such as `INPUT` (compared without regard to case).
    pub area: Option<String>,
    pub class: Option<RecordClass>,
    /// An operation id, 32 lowercase hexadecimal digits.
    pub operation: Option<String>,
}

impl HistoryFilter {
    fn matches(&self, record: &ObservedRecord) -> bool {
        self.area
            .as_deref()
            .is_none_or(|area| record.area.eq_ignore_ascii_case(area))
            && self.class.is_none_or(|class| record.class == class)
            && self
                .operation
                .as_deref()
                .is_none_or(|operation| record.is_about(operation))
    }
}

/// What the history still holds, so an answer can say how much it cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retention {
    pub retained: usize,
    /// Records forgotten since the run began.
    pub evicted: u64,
    pub oldest_audit_id: Option<u64>,
    pub newest_audit_id: Option<u64>,
}

/// The newest records that match a query, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    pub records: Vec<ObservedRecord>,
    /// How many retained records matched, of which `records` is the newest part.
    pub matched: usize,
    pub retention: Retention,
}

#[derive(Debug, Default)]
struct Retained {
    records: VecDeque<ObservedRecord>,
    bytes: usize,
}

/// The bounded history of records.
#[derive(Debug)]
pub struct MemoryOutlet {
    retained: Mutex<Retained>,
    maximum_records: usize,
    maximum_bytes: usize,
    accepted: AtomicU64,
    evicted: AtomicU64,
}

impl Default for MemoryOutlet {
    fn default() -> Self {
        Self::with_bounds(MAXIMUM_RETAINED_RECORDS, MAXIMUM_RETAINED_BYTES)
    }
}

impl MemoryOutlet {
    /// A history that keeps at most `maximum_records` records and `maximum_bytes` bytes.
    pub fn with_bounds(maximum_records: usize, maximum_bytes: usize) -> Self {
        Self {
            retained: Mutex::new(Retained::default()),
            maximum_records,
            maximum_bytes,
            accepted: AtomicU64::new(0),
            evicted: AtomicU64::new(0),
        }
    }

    fn locked(&self) -> MutexGuard<'_, Retained> {
        self.retained.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn retention(&self, retained: &Retained) -> Retention {
        Retention {
            retained: retained.records.len(),
            evicted: self.evicted.load(Ordering::Relaxed),
            oldest_audit_id: retained.records.front().map(|record| record.audit_id),
            newest_audit_id: retained.records.back().map(|record| record.audit_id),
        }
    }

    /// The newest `limit` retained records that match `filter`, oldest first.
    pub fn query(&self, filter: &HistoryFilter, limit: usize) -> HistoryPage {
        let retained = self.locked();
        let matching: Vec<&ObservedRecord> = retained
            .records
            .iter()
            .filter(|record| filter.matches(record))
            .collect();
        let matched = matching.len();
        let skip = matched.saturating_sub(limit);
        HistoryPage {
            records: matching.into_iter().skip(skip).cloned().collect(),
            matched,
            retention: self.retention(&retained),
        }
    }

    pub fn retention_now(&self) -> Retention {
        self.retention(&self.locked())
    }
}

impl RecordOutlet for MemoryOutlet {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn accept(&self, record: &ObservedRecord) -> Result<(), OutletRefusal> {
        let cost = record.approximate_bytes();
        let mut retained = self.locked();
        retained.records.push_back(record.clone());
        retained.bytes = retained.bytes.saturating_add(cost);
        // The newest record is always kept, even when it alone is over the byte bound.
        while retained.records.len() > 1
            && (retained.records.len() > self.maximum_records
                || retained.bytes > self.maximum_bytes)
        {
            let Some(forgotten) = retained.records.pop_front() else {
                break;
            };
            retained.bytes = retained.bytes.saturating_sub(forgotten.approximate_bytes());
            self.evicted.fetch_add(1, Ordering::Relaxed);
        }
        self.accepted.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn status(&self, _now_ms: u64) -> OutletStatus {
        let accepted = self.accepted.load(Ordering::Relaxed);
        OutletStatus {
            name: self.name().to_owned(),
            accepted,
            confirmed: accepted,
            pending: 0,
            lag_ms: 0,
            lost_always: 0,
            lost_verbose: 0,
            evicted: self.evicted.load(Ordering::Relaxed),
            unconfirmed: 0,
        }
    }
}
