//! The host's event sink: every `TableEvent` becomes one audit line and one structured record, from
//! the same value, and the record goes to every outlet (H2, O.6, RES41).
//!
//! `HostEventSink` implements the table's `EventSink` port. For each event it renders the audit
//! line once (`render_audit_line`), appends that line to the host's `EventLog` (what `df-table dev
//! events` and the viewer show), builds the structured record from the same event, stamps it with
//! the next audit id and the sink's clock, and gives it to the in-memory history and to every
//! other outlet. Nothing is sampled: class A events are always recorded; class B (tick and step
//! detail) are a verbose level, and an event left out by a lower level is counted in
//! `filtered_verbose`, never silently dropped. An outlet that refuses a record counts the loss
//! itself, by class; an event that cannot be made into a record is counted as `unrenderable`
//! and its audit line is still written, so the line log never has a gap the records do not.
//!
//! `health` is the visible side: how many records were seen by class, how many were filtered, what
//! each outlet has confirmed and lost, how old the oldest unconfirmed record is (`lag_ms`), and
//! whether anything was lost (`degraded`). It never blocks the owner: every call here is bounded
//! work (rendering, one lock per outlet) and no call touches a disk or a network.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use df_table::event::TableEvent;
use df_table::ports::{Clock, EventSink};
use serde::{Deserialize, Serialize};

use crate::event_log::EventLog;

use super::limits::{MAXIMUM_RETAINED_BYTES, UNQUERIED_HISTORY_RECORDS};
use super::memory::{HistoryFilter, HistoryPage, MemoryOutlet};
use super::outlet::{OutletStatus, RecordOutlet};
use super::record::{ObservedRecord, RecordClass, RecordDraft};

/// Which events are recorded: class A only, or both classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    /// Class A: always on, never sampled.
    Always,
    /// Class A and class B detail.
    Verbose,
}

impl EventLevel {
    pub const ALL: [Self; 2] = [Self::Always, Self::Verbose];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.key() == key)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Verbose => "verbose",
        }
    }
}

/// Milliseconds since the Unix epoch, from the system clock: the sink's default clock, read only
/// here at the edge. A clock set before 1970 reads as zero.
#[derive(Debug, Default, Clone, Copy)]
pub struct WallClock;

impl Clock for WallClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            })
    }
}

/// What `health` shows about the event stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SinkHealth {
    pub level: String,
    /// Events offered to the sink, of either class, whether or not the level kept them.
    pub seen: u64,
    pub always: u64,
    pub verbose: u64,
    /// Class B events a lower level left out. Class A is never in this count.
    pub filtered_verbose: u64,
    /// Events whose line was written but that could not be made into a structured record.
    pub unrenderable: u64,
    /// Records the history holds now, and how many it has forgotten.
    pub retained: usize,
    pub evicted: u64,
    /// The age in milliseconds of the oldest record an outlet has accepted and not yet made safe.
    pub lag_ms: u64,
    /// Records an outlet refused or failed to write, by class, summed over the outlets.
    pub lost_always: u64,
    pub lost_verbose: u64,
    /// Something was lost or cannot be confirmed: a class A loss, an unrenderable event or a
    /// failed receipt. A degraded stream is never a complete corpus.
    pub degraded: bool,
    pub outlets: Vec<OutletStatus>,
}

/// The sink.
pub struct HostEventSink {
    log: Arc<EventLog>,
    clock: Arc<dyn Clock>,
    memory: Arc<MemoryOutlet>,
    outlets: Vec<Arc<dyn RecordOutlet>>,
    next_audit_id: AtomicU64,
    verbose: AtomicBool,
    always_seen: AtomicU64,
    verbose_seen: AtomicU64,
    filtered_verbose: AtomicU64,
    unrenderable: AtomicU64,
}

impl HostEventSink {
    /// A sink that writes lines to `log` and keeps the standard history, at the verbose level.
    /// `clock` reads milliseconds since the Unix epoch (`WallClock` outside tests).
    pub fn new(log: Arc<EventLog>, clock: Arc<dyn Clock>) -> Self {
        Self {
            log,
            clock,
            memory: Arc::new(MemoryOutlet::default()),
            outlets: Vec::new(),
            next_audit_id: AtomicU64::new(1),
            verbose: AtomicBool::new(true),
            always_seen: AtomicU64::new(0),
            verbose_seen: AtomicU64::new(0),
            filtered_verbose: AtomicU64::new(0),
            unrenderable: AtomicU64::new(0),
        }
    }

    /// A sink whose history nobody queries (the canvas workbench's): every line reaches `log` as
    /// with `new`, and the history keeps only the newest record, so it holds no memory.
    pub fn without_history(log: Arc<EventLog>, clock: Arc<dyn Clock>) -> Self {
        Self::new(log, clock).with_memory(Arc::new(MemoryOutlet::with_bounds(
            UNQUERIED_HISTORY_RECORDS,
            MAXIMUM_RETAINED_BYTES,
        )))
    }

    /// Replaces the history, so a test can give it small bounds.
    pub fn with_memory(mut self, memory: Arc<MemoryOutlet>) -> Self {
        self.memory = memory;
        self
    }

    /// Adds an outlet beside the history: a file spool, the shared diagnostic path.
    pub fn with_outlet(mut self, outlet: Arc<dyn RecordOutlet>) -> Self {
        self.outlets.push(outlet);
        self
    }

    pub fn with_level(self, level: EventLevel) -> Self {
        self.set_level(level);
        self
    }

    pub fn set_level(&self, level: EventLevel) {
        self.verbose
            .store(level == EventLevel::Verbose, Ordering::Relaxed);
    }

    pub fn level(&self) -> EventLevel {
        if self.verbose.load(Ordering::Relaxed) {
            EventLevel::Verbose
        } else {
            EventLevel::Always
        }
    }

    /// The log the sink writes its audit lines to.
    pub fn log(&self) -> &Arc<EventLog> {
        &self.log
    }

    /// The newest `limit` retained records that match `filter`, oldest first.
    pub fn query(&self, filter: &HistoryFilter, limit: usize) -> HistoryPage {
        self.memory.query(filter, limit)
    }

    /// Whether the level keeps a record of this class, counting it either way.
    fn admits(&self, class: RecordClass) -> bool {
        match class {
            RecordClass::Always => {
                self.always_seen.fetch_add(1, Ordering::Relaxed);
                true
            }
            RecordClass::Verbose => {
                self.verbose_seen.fetch_add(1, Ordering::Relaxed);
                let kept = self.verbose.load(Ordering::Relaxed);
                if !kept {
                    self.filtered_verbose.fetch_add(1, Ordering::Relaxed);
                }
                kept
            }
        }
    }

    /// Records one draft: writes its line, stamps it and gives it to every outlet. An outlet's
    /// refusal is that outlet's to count; the sink never fails its caller.
    pub fn record_draft(&self, draft: RecordDraft) {
        if self.admits(draft.class) {
            self.write(draft);
        }
    }

    fn write(&self, draft: RecordDraft) {
        self.log.push(draft.audit_line.clone());
        let audit_id = self.next_audit_id.fetch_add(1, Ordering::Relaxed);
        let record = ObservedRecord::stamp(draft, audit_id, self.clock.now_ms());
        self.deliver(&record);
    }

    fn deliver(&self, record: &ObservedRecord) {
        // A refusal is counted by the outlet that made it; the caller has nothing to do with it.
        let _ = self.memory.accept(record);
        for outlet in &self.outlets {
            let _ = outlet.accept(record);
        }
    }

    pub fn health(&self) -> SinkHealth {
        let now_ms = self.clock.now_ms();
        let outlets: Vec<OutletStatus> = std::iter::once(&*self.memory as &dyn RecordOutlet)
            .chain(self.outlets.iter().map(|outlet| &**outlet))
            .map(|outlet| outlet.status(now_ms))
            .collect();
        let always = self.always_seen.load(Ordering::Relaxed);
        let verbose = self.verbose_seen.load(Ordering::Relaxed);
        let unrenderable = self.unrenderable.load(Ordering::Relaxed);
        let retention = self.memory.retention_now();
        let lost_always = outlets.iter().map(|status| status.lost_always).sum();
        let lost_verbose = outlets.iter().map(|status| status.lost_verbose).sum();
        let unconfirmed: u64 = outlets.iter().map(|status| status.unconfirmed).sum();
        SinkHealth {
            level: self.level().key().to_owned(),
            seen: always.saturating_add(verbose),
            always,
            verbose,
            filtered_verbose: self.filtered_verbose.load(Ordering::Relaxed),
            unrenderable,
            retained: retention.retained,
            evicted: retention.evicted,
            lag_ms: outlets
                .iter()
                .map(|status| status.lag_ms)
                .max()
                .unwrap_or(0),
            lost_always,
            lost_verbose,
            degraded: lost_always > 0 || unrenderable > 0 || unconfirmed > 0,
            outlets,
        }
    }
}

impl EventSink for HostEventSink {
    fn record(&self, event: &TableEvent) {
        if !self.admits(RecordClass::from(event.class())) {
            return;
        }
        match RecordDraft::from_event(event) {
            Ok(draft) => self.write(draft),
            Err(_) => {
                // The line is never lost because the structured form could not be made, and the
                // failure is counted so a gap in the records is visible.
                self.log.push(event.render_audit_line());
                self.unrenderable.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
