//! A bounded, ordered record of the audit lines the host has seen, for the viewer and the
//! `events` command.
//!
//! It owns the bound and the numbering. It never interprets a line and never grows without limit,
//! in lines or in bytes: the oldest lines are forgotten first. Every list it returns is oldest
//! first, so a reader that wants newest first reverses it.
//!
//! It is also where the audit lines of the libraries the host runs arrive: `install_as_audit_sink`
//! makes a log the df-observe audit sink, so `df_observe::record_audit_line` lands here instead of
//! on stdout.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};

use crate::limits::{MAXIMUM_LOGGED_BYTES, MAXIMUM_LOGGED_LINES};

/// One recorded line and its place in the order. Sequences start at 1 and restart when the host
/// does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedLine {
    pub sequence: u64,
    pub text: String,
}

#[derive(Debug, Default)]
struct Lines {
    last_sequence: u64,
    kept: VecDeque<LoggedLine>,
    /// Bytes of text in `kept`.
    bytes: usize,
}

/// The log of audit lines.
#[derive(Debug)]
pub struct EventLog {
    lines: Mutex<Lines>,
    maximum_lines: usize,
    maximum_bytes: usize,
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
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Lines> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn push(&self, text: String) {
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
        }
        lines.bytes = lines.bytes.saturating_add(length);
        lines.kept.push_back(LoggedLine { sequence, text });
    }

    /// The sequence of the newest line, or 0 when nothing has been logged.
    pub fn latest_sequence(&self) -> u64 {
        self.lock().last_sequence
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
/// and set once: `true` when `log` became it, `false` when an earlier log already is and keeps
/// the lines.
pub fn install_as_audit_sink(log: &Arc<EventLog>) -> bool {
    let installed = AUDIT_SINK_LOG.set(Arc::clone(log)).is_ok();
    df_observe::install_audit_sink(record_observe_line);
    installed
}
