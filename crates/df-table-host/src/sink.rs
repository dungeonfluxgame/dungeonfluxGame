//! The host's event sink and the echo of audit lines to the terminal.
//!
//! The sink only appends to the log, so it never blocks the canvas it is called under. The echo
//! is separate and runs after the canvas lock is released, because a blocked terminal must not
//! stall the viewer. Neither fails the table: an event that is not recorded is dropped, and a
//! closed terminal drops the echo.

use std::sync::{Arc, Mutex, PoisonError};

use df_table::event::{EventClass, TableEvent};
use df_table::ports::EventSink;

use crate::event_log::{EventLog, MAXIMUM_LOGGED_LINES};

/// Appends events to the log. Events of class `Verbose` (moves) are kept only when asked for, so
/// a log without them shows gaps in the revision numbers.
#[derive(Debug)]
pub struct HostSink {
    log: Arc<EventLog>,
    records_moves: bool,
}

impl HostSink {
    pub fn new(log: Arc<EventLog>, records_moves: bool) -> Self {
        Self { log, records_moves }
    }

    pub fn log(&self) -> &Arc<EventLog> {
        &self.log
    }
}

impl EventSink for HostSink {
    fn record(&self, event: &TableEvent) {
        if event.class() == EventClass::Verbose && !self.records_moves {
            return;
        }
        self.log.push(event.render_audit_line());
    }
}

/// Prints each logged line once, in order, to a writer the caller supplies.
pub struct LineEcho {
    printed_through: Mutex<u64>,
    write_line: Box<dyn Fn(&str) + Send + Sync>,
}

impl LineEcho {
    /// Echo only lines logged after this moment.
    pub fn starting_after(
        log: &EventLog,
        write_line: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            printed_through: Mutex::new(log.latest_sequence()),
            write_line: Box::new(write_line),
        }
    }

    /// Prints every line logged since the last call. Call it without holding any lock.
    pub fn echo_new(&self, log: &EventLog) {
        let mut printed_through = self
            .printed_through
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for line in log.after_sequence(*printed_through, MAXIMUM_LOGGED_LINES) {
            (self.write_line)(&line.text);
            *printed_through = line.sequence;
        }
    }
}
