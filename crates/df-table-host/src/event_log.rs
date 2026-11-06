//! A bounded, ordered record of the audit lines the host has seen, for the viewer and the
//! `events` command.
//!
//! It owns the bound, the numbering and the time each line was recorded at. It never interprets a
//! line beyond its first word (the area a reader may filter by) and never grows without limit, in
//! lines or in bytes: the oldest lines are forgotten first, and counted. Every list it returns is
//! oldest first, so a reader that wants newest first reverses it.
//!
//! It is also where the audit lines of the libraries the host runs arrive: `install_as_audit_sink`
//! makes a log the df-observe audit sink, so `df_observe::record_audit_line` lands here instead of
//! only being counted as undelivered.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use df_table::ports::Clock;
use serde::{Deserialize, Serialize};

use crate::limits::{MAXIMUM_LOGGED_BYTES, MAXIMUM_LOGGED_LINES};
use crate::observe::WallClock;

/// One recorded line and its place in the order. Sequences start at 1 and restart when the host
/// does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedLine {
    pub sequence: u64,
    pub text: String,
    /// When the line was recorded, in milliseconds since the Unix epoch (the log's clock).
    #[serde(default)]
    pub at_ms: u64,
}

impl LoggedLine {
    /// The line's first word: the area that wrote it (`CANVAS`, `PLAYGROUND`, `HOST`, ...).
    pub fn area(&self) -> &str {
        self.text.split(' ').next().unwrap_or("")
    }
}

/// How full the log is, for `host status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogCounts {
    /// The sequence of the newest line, or 0 when nothing has been logged.
    pub latest: u64,
    /// Lines kept now.
    pub kept: usize,
    /// Bytes of text kept now.
    pub bytes: usize,
    /// Lines forgotten to keep the line or byte bound since the host started.
    pub forgotten: u64,
}

#[derive(Debug, Default)]
struct Lines {
    last_sequence: u64,
    kept: VecDeque<LoggedLine>,
    /// Bytes of text in `kept`.
    bytes: usize,
    /// Lines pushed out by the line or byte bound.
    forgotten: u64,
}

/// The log of audit lines.
pub struct EventLog {
    lines: Mutex<Lines>,
    maximum_lines: usize,
    maximum_bytes: usize,
    /// Reads the time a line is recorded at: the wall clock outside tests.
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for EventLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventLog")
            .field("lines", &self.lines)
            .field("maximum_lines", &self.maximum_lines)
            .field("maximum_bytes", &self.maximum_bytes)
            .finish_non_exhaustive()
    }
}

impl Default for EventLog {
    fn default() -> Self {
        Self::with_bounds(MAXIMUM_LOGGED_LINES, MAXIMUM_LOGGED_BYTES)
    }
}

impl EventLog {
    /// A log that keeps at most `maximum_lines` lines and `maximum_bytes` bytes of text.
    pub fn with_bounds(maximum_lines: usize, maximum_bytes: usize) -> Self {
        Self {
            lines: Mutex::new(Lines::default()),
            maximum_lines,
            maximum_bytes,
            clock: Arc::new(WallClock),
        }
    }

    /// The same log with `clock` stamping its lines, so a test controls `at_ms`.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// The log's clock now, in milliseconds since the Unix epoch.
    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Lines> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn push(&self, text: String) {
        // The clock is read before the lock, so a slow clock never holds up another writer.
        let at_ms = self.clock.now_ms();
        let mut lines = self.lock();
        lines.last_sequence += 1;
        let sequence = lines.last_sequence;
        let length = text.len();
        while lines.kept.len() >= self.maximum_lines
            || (lines.bytes.saturating_add(length) > self.maximum_bytes && !lines.kept.is_empty())
        {
            let Some(forgotten) = lines.kept.pop_front() else {
                break;
            };
            lines.bytes = lines.bytes.saturating_sub(forgotten.text.len());
            lines.forgotten = lines.forgotten.saturating_add(1);
        }
        lines.bytes = lines.bytes.saturating_add(length);
        lines.kept.push_back(LoggedLine {
            sequence,
            text,
            at_ms,
        });
    }

    /// The sequence of the newest line, or 0 when nothing has been logged.
    pub fn latest_sequence(&self) -> u64 {
        self.lock().last_sequence
    }

    /// How many lines were forgotten to keep the bounds since the host started.
    pub fn forgotten(&self) -> u64 {
        self.lock().forgotten
    }

    /// The newest sequence, what is kept and what was forgotten, read together.
    pub fn counts(&self) -> LogCounts {
        let lines = self.lock();
        LogCounts {
            latest: lines.last_sequence,
            kept: lines.kept.len(),
            bytes: lines.bytes,
            forgotten: lines.forgotten,
        }
    }

    /// Lines newer than `sequence`, oldest first, at most `limit` of them.
    pub fn after_sequence(&self, sequence: u64, limit: usize) -> Vec<LoggedLine> {
        let lines = self.lock();
        // The kept lines have consecutive sequences, so the first newer one is found by
        // arithmetic instead of by walking a log that can hold a hundred thousand lines.
        let first_kept = lines.kept.front().map_or(0, |line| line.sequence);
        let skip = usize::try_from(sequence.saturating_add(1).saturating_sub(first_kept))
            .unwrap_or(usize::MAX)
            .min(lines.kept.len());
        lines.kept.range(skip..).take(limit).cloned().collect()
    }

    /// The newest `limit` lines, oldest first.
    pub fn newest(&self, limit: usize) -> Vec<LoggedLine> {
        let lines = self.lock();
        let skip = lines.kept.len().saturating_sub(limit);
        lines.kept.iter().skip(skip).cloned().collect()
    }

    /// Lines of `area` (their first word) newer than `sequence`, oldest first, at most `limit`.
    pub fn after_sequence_in(&self, sequence: u64, limit: usize, area: &str) -> Vec<LoggedLine> {
        let lines = self.lock();
        lines
            .kept
            .iter()
            .filter(|line| line.sequence > sequence && line.area() == area)
            .take(limit)
            .cloned()
            .collect()
    }

    /// The newest `limit` lines of `area` (their first word), oldest first.
    pub fn newest_in(&self, limit: usize, area: &str) -> Vec<LoggedLine> {
        let lines = self.lock();
        let mut newest: Vec<LoggedLine> = lines
            .kept
            .iter()
            .rev()
            .filter(|line| line.area() == area)
            .take(limit)
            .cloned()
            .collect();
        newest.reverse();
        newest
    }
}

/// The log that df-observe's audit lines go to, set once per process.
static AUDIT_SINK_LOG: OnceLock<Arc<EventLog>> = OnceLock::new();

/// Receives one df-observe audit line. `install_as_audit_sink` sets the log before it installs
/// this, so a line always finds it.
fn record_observe_line(_event: &'static str, line: String) {
    if let Some(log) = AUDIT_SINK_LOG.get() {
        log.push(line);
    }
}

/// Makes `log` the df-observe audit sink of this process: every `df_observe::record_audit_line`
/// from then on is pushed to it (and echoed with the host's next echo). The sink is process-wide
/// and set once: `true` when `log` became it, `false` when an earlier log or another df-observe
/// sink already is and keeps the lines.
pub fn install_as_audit_sink(log: &Arc<EventLog>) -> bool {
    let installed = AUDIT_SINK_LOG.set(Arc::clone(log)).is_ok();
    let observe_installed = df_observe::install_audit_sink(record_observe_line);
    installed && observe_installed
}
