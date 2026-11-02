//! The host's event sink and the echo of audit lines to the terminal.
//!
//! The sink only appends to the log, so it never blocks the canvas it is called under. The echo
//! never blocks anything either: it only queues lines for one dedicated writer thread, and that
//! thread alone writes to the terminal. A terminal that stops accepting output (a console in
//! selection mode, a paused pipe) blocks that thread and nothing else; lines pile up in the queue
//! up to a very large byte bound, then are dropped and counted, and the writer says how many when
//! the terminal moves again. Neither fails the table: an event that is not recorded is dropped,
//! and a closed terminal drops the echo.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};

use df_table::event::{EventClass, TableEvent};
use df_table::ports::EventSink;

use crate::event_log::EventLog;
use crate::limits::{MAXIMUM_LOGGED_LINES, MAXIMUM_PENDING_ECHO_BYTES};

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

/// Queues each logged line once, in order, for a writer thread that prints it with the writer the
/// caller supplies.
pub struct LineEcho {
    printed_through: Mutex<u64>,
    /// `None` when the writer thread could not be started: every line is then counted as dropped.
    queue: Option<mpsc::Sender<String>>,
    /// Bytes queued and not yet written.
    pending_bytes: Arc<AtomicUsize>,
    maximum_pending_bytes: usize,
    /// Lines dropped since the writer last said so.
    dropped: Arc<AtomicU64>,
}

impl LineEcho {
    /// Echo only lines logged after this moment.
    pub fn starting_after(log: &EventLog, write_line: impl FnMut(&str) + Send + 'static) -> Self {
        Self::bounded(log, MAXIMUM_PENDING_ECHO_BYTES, write_line)
    }

    /// `starting_after` with a chosen bound on the bytes waiting for the writer, so a test can
    /// reach it.
    pub fn bounded(
        log: &EventLog,
        maximum_pending_bytes: usize,
        mut write_line: impl FnMut(&str) + Send + 'static,
    ) -> Self {
        let (sender, receiver) = mpsc::channel::<String>();
        let pending_bytes = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let (writer_pending, writer_dropped) = (Arc::clone(&pending_bytes), Arc::clone(&dropped));
        let started = std::thread::Builder::new()
            .name("df-table-echo".to_owned())
            .spawn(move || {
                for line in receiver {
                    writer_pending.fetch_sub(line.len(), Ordering::Relaxed);
                    let lost = writer_dropped.swap(0, Ordering::Relaxed);
                    if lost > 0 {
                        write_line(&format!(
                            "{lost} audit lines were not echoed (the terminal fell behind); see events"
                        ));
                    }
                    write_line(&line);
                }
            });
        Self {
            printed_through: Mutex::new(log.latest_sequence()),
            queue: started.is_ok().then_some(sender),
            pending_bytes,
            maximum_pending_bytes,
            dropped,
        }
    }

    /// How many lines were dropped and not yet reported by the writer.
    pub fn dropped_lines(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Queues every line logged since the last call. It never waits for the terminal.
    pub fn echo_new(&self, log: &EventLog) {
        let mut printed_through = self
            .printed_through
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for line in log.after_sequence(*printed_through, MAXIMUM_LOGGED_LINES) {
            *printed_through = line.sequence;
            self.enqueue(line.text);
        }
    }

    fn enqueue(&self, text: String) {
        let length = text.len();
        let waiting = self.pending_bytes.fetch_add(length, Ordering::Relaxed);
        let sent = waiting.saturating_add(length) <= self.maximum_pending_bytes
            && self
                .queue
                .as_ref()
                .is_some_and(|queue| queue.send(text).is_ok());
        if !sent {
            self.pending_bytes.fetch_sub(length, Ordering::Relaxed);
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
