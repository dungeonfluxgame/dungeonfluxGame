//! A bounded, ordered record of the audit lines the host has seen, for the viewer and the
//! `events` command.
//!
//! It owns the bound and the numbering. It never interprets a line and never grows without limit:
//! the oldest lines are forgotten first. Every list it returns is oldest first, so a reader that
//! wants newest first reverses it.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::limits::MAXIMUM_LOGGED_LINES;

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
}

/// The log of audit lines.
#[derive(Debug, Default)]
pub struct EventLog {
    lines: Mutex<Lines>,
}

impl EventLog {
    fn lock(&self) -> std::sync::MutexGuard<'_, Lines> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn push(&self, text: String) {
        let mut lines = self.lock();
        lines.last_sequence += 1;
        let sequence = lines.last_sequence;
        if lines.kept.len() >= MAXIMUM_LOGGED_LINES {
            lines.kept.pop_front();
        }
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
