//! The echo of audit lines to the terminal.
//!
//! The canvas records its events through the host's one event sink (`observe::HostEventSink`),
//! which writes each line to the log and counts the verbose events its level leaves out. The echo
//! never blocks anything: it only queues lines for one dedicated writer thread, and that
//! thread alone writes to the terminal. A terminal that stops accepting output (a console in
//! selection mode, a paused pipe) blocks that thread and nothing else; lines pile up in the queue
//! up to a very large byte bound, then are dropped and counted, and the writer says how many when
//! the terminal moves again (also when nothing else follows). A line the log forgot before the echo
//! looked is counted as dropped too, and a writer that has died is reported once in the log.
//! `finish` waits, for a bounded time, for the writer to print what is queued, so the last lines
//! before a shutdown reach the terminal. Neither fails the table: an event that is not recorded is
//! dropped, and a closed terminal drops the echo.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use crate::event_log::EventLog;
use crate::limits::{
    ECHO_LINE_OVERHEAD_BYTES, ECHO_NOTICE_AFTER, MAXIMUM_LOGGED_LINES, MAXIMUM_PENDING_ECHO_BYTES,
};

/// What a queued line counts for against the bound: its text and what holding it costs.
fn cost(text: &str) -> usize {
    text.len().saturating_add(ECHO_LINE_OVERHEAD_BYTES)
}

/// Queues each logged line once, in order, for a writer thread that prints it with the writer the
/// caller supplies.
pub struct LineEcho {
    printed_through: Mutex<u64>,
    /// `None` once `finish` has taken it, or when the writer thread could not be started: every
    /// line is then counted as dropped.
    queue: Mutex<Option<mpsc::Sender<String>>>,
    /// Bytes queued (each with its overhead) and not yet written.
    pending_bytes: Arc<AtomicUsize>,
    maximum_pending_bytes: usize,
    /// Lines not echoed since the writer last said so.
    dropped: Arc<AtomicU64>,
    /// The writer says here that it has ended.
    ended: Mutex<Option<mpsc::Receiver<()>>>,
    /// Set when a line could not be queued because there is no writer to take it.
    writer_gone: AtomicBool,
    /// Set once the gone writer has been reported in the log.
    gone_reported: AtomicBool,
    /// Set by `finish`, so lines after it are not taken for a dead writer.
    finished: AtomicBool,
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
        let (ended_sender, ended) = mpsc::channel::<()>();
        let pending_bytes = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let (writer_pending, writer_dropped) = (Arc::clone(&pending_bytes), Arc::clone(&dropped));
        let started = std::thread::Builder::new()
            .name("df-table-echo".to_owned())
            .spawn(move || {
                let say_what_was_lost = |write_line: &mut dyn FnMut(&str)| {
                    let lost = writer_dropped.swap(0, Ordering::Relaxed);
                    if lost > 0 {
                        write_line(&format!(
                            "{lost} audit lines were not echoed (the terminal fell behind); see events"
                        ));
                    }
                };
                loop {
                    match receiver.recv_timeout(ECHO_NOTICE_AFTER) {
                        Ok(line) => {
                            writer_pending.fetch_sub(cost(&line), Ordering::Relaxed);
                            say_what_was_lost(&mut write_line);
                            write_line(&line);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => say_what_was_lost(&mut write_line),
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            say_what_was_lost(&mut write_line);
                            break;
                        }
                    }
                }
                // The reader of this may already be gone.
                let _ = ended_sender.send(());
            });
        let started = started.is_ok();
        Self {
            printed_through: Mutex::new(log.latest_sequence()),
            queue: Mutex::new(started.then_some(sender)),
            pending_bytes,
            maximum_pending_bytes,
            dropped,
            ended: Mutex::new(started.then_some(ended)),
            writer_gone: AtomicBool::new(!started),
            gone_reported: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }

    /// How many lines were not echoed and not yet reported by the writer.
    pub fn dropped_lines(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Whether the writer is gone: it could not be started, or it died (its writer panicked).
    pub fn writer_gone(&self) -> bool {
        self.writer_gone.load(Ordering::Relaxed)
    }

    /// Stops taking lines and waits up to `patience` for the writer to print what is queued.
    /// `true` means the writer has ended with the queue empty; `false` means it was still busy (a
    /// terminal that does not accept output) and some lines were left unprinted.
    pub fn finish(&self, patience: Duration) -> bool {
        self.finished.store(true, Ordering::Relaxed);
        let sender = self
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(sender);
        let ended = self
            .ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match ended {
            // Either the writer said it ended, or it died and dropped its end.
            Some(ended) => !matches!(
                ended.recv_timeout(patience),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            None => true,
        }
    }

    /// Queues every line logged since the last call. It never waits for the terminal. Lines the
    /// log forgot before this looked are counted as dropped, and a writer that is gone is said so
    /// in the log once.
    pub fn echo_new(&self, log: &EventLog) {
        let mut printed_through = self
            .printed_through
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let lines = log.after_sequence(*printed_through, MAXIMUM_LOGGED_LINES);
        if let Some(first) = lines.first() {
            let forgotten = first
                .sequence
                .saturating_sub(printed_through.saturating_add(1));
            if forgotten > 0 {
                self.dropped.fetch_add(forgotten, Ordering::Relaxed);
            }
        }
        for line in lines {
            *printed_through = line.sequence;
            self.enqueue(line.text);
        }
        drop(printed_through);
        if self.writer_gone() && !self.gone_reported.swap(true, Ordering::Relaxed) {
            log.push(format!(
                "HOST echo_writer_gone lines_not_echoed={}",
                self.dropped_lines()
            ));
        }
    }

    fn enqueue(&self, text: String) {
        let length = cost(&text);
        let waiting = self.pending_bytes.fetch_add(length, Ordering::Relaxed);
        let within_bound = waiting.saturating_add(length) <= self.maximum_pending_bytes;
        let sent = within_bound && {
            let queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            match queue.as_ref() {
                Some(queue) => {
                    let delivered = queue.send(text).is_ok();
                    if !delivered {
                        self.writer_gone.store(true, Ordering::Relaxed);
                    }
                    delivered
                }
                None => {
                    if !self.finished.load(Ordering::Relaxed) {
                        self.writer_gone.store(true, Ordering::Relaxed);
                    }
                    false
                }
            }
        };
        if !sent {
            self.pending_bytes.fetch_sub(length, Ordering::Relaxed);
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
