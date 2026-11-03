//! The file spool: records written to disk as JSON lines by a thread of their own.
//!
//! The owner never touches the disk: `accept` serializes the record, puts it on a queue bounded by
//! bytes and returns; one writer thread appends the lines, flushes after each batch and counts what
//! it wrote (`confirmed`). A full queue, a spool at its size bound, a failed write or a closed
//! spool each lose the record and count it by class, so a loss is a number, never silence; nothing
//! is deleted to make room. The spool has no sampling and no retries: it is the dev engine's
//! durable copy where no telemetry store is configured, and the records are flushed to the
//! operating system, not forced to the platter (a crash of the process loses nothing that was
//! flushed; a power cut may).
//!
//! Dropping the spool closes it and lets the writer finish what is queued in the background; a
//! caller that needs the file complete first calls `settle`, which waits only as long as it is
//! told to.

use std::collections::VecDeque;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::limits::{MAXIMUM_SPOOL_BYTES, MAXIMUM_SPOOL_QUEUE_BYTES};
use super::outlet::{Losses, OutletRefusal, OutletStatus, RecordOutlet};
use super::record::{ObservedRecord, RecordClass};

/// Why the spool could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpoolError {
    /// The file could not be opened for appending.
    Open { path: PathBuf, reason: String },
    /// The writer thread could not be started.
    Thread { reason: String },
}

impl fmt::Display for SpoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { path, reason } => {
                write!(
                    formatter,
                    "the spool {} cannot be opened: {reason}",
                    path.display()
                )
            }
            Self::Thread { reason } => {
                write!(formatter, "the spool's writer did not start: {reason}")
            }
        }
    }
}

impl std::error::Error for SpoolError {}

struct Queued {
    line: Vec<u8>,
    class: RecordClass,
    observed_ms: u64,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Queued>,
    /// Bytes of lines queued or being written.
    queued_bytes: usize,
    /// Records accepted and not yet written or lost by the writer.
    pending: u64,
    in_flight_oldest_ms: Option<u64>,
    closed: bool,
}

struct Shared {
    state: Mutex<State>,
    wake_writer: Condvar,
    settled: Condvar,
    accepted: AtomicU64,
    confirmed: AtomicU64,
    losses: Losses,
    maximum_queue_bytes: usize,
}

impl Shared {
    fn locked(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Closes the spool when the writer ends for any reason, a panic included, so later records are
/// refused and counted instead of queueing for a writer that is gone.
struct CloseOnExit(Arc<Shared>);

impl Drop for CloseOnExit {
    fn drop(&mut self) {
        let mut state = self.0.locked();
        state.closed = true;
        state.pending = 0;
        state.queue.clear();
        state.queued_bytes = 0;
        drop(state);
        self.0.settled.notify_all();
    }
}

/// The durable JSON-lines outlet.
pub struct FileSpool {
    shared: Arc<Shared>,
}

impl FileSpool {
    /// Opens (creating it) the spool file at `path` with the standard bounds.
    pub fn open(path: &Path) -> Result<Self, SpoolError> {
        Self::open_with_bounds(path, MAXIMUM_SPOOL_BYTES, MAXIMUM_SPOOL_QUEUE_BYTES)
    }

    /// `open` with chosen bounds, so a test can fill the spool or its queue.
    pub fn open_with_bounds(
        path: &Path,
        maximum_spool_bytes: u64,
        maximum_queue_bytes: usize,
    ) -> Result<Self, SpoolError> {
        let open_error = |error: std::io::Error| SpoolError::Open {
            path: path.to_owned(),
            reason: error.to_string(),
        };
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(open_error)?;
        let existing = file.metadata().map_err(open_error)?.len();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake_writer: Condvar::new(),
            settled: Condvar::new(),
            accepted: AtomicU64::new(0),
            confirmed: AtomicU64::new(0),
            losses: Losses::default(),
            maximum_queue_bytes,
        });
        let writer_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("df-table-spool".to_owned())
            .spawn(move || {
                let _closes = CloseOnExit(Arc::clone(&writer_shared));
                write_loop(&writer_shared, file, existing, maximum_spool_bytes);
            })
            .map_err(|error| SpoolError::Thread {
                reason: error.to_string(),
            })?;
        Ok(Self { shared })
    }

    /// A spool whose writer never runs, so a test can read what waiting looks like.
    #[cfg(test)]
    pub(super) fn without_writer(maximum_queue_bytes: usize) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                wake_writer: Condvar::new(),
                settled: Condvar::new(),
                accepted: AtomicU64::new(0),
                confirmed: AtomicU64::new(0),
                losses: Losses::default(),
                maximum_queue_bytes,
            }),
        }
    }

    /// Waits until everything accepted so far has been written or lost, for at most `timeout`.
    /// It returns whether the spool settled. It is for a shutdown or a test; the owner never
    /// calls it.
    pub fn settle(&self, timeout: Duration) -> bool {
        let state = self.shared.locked();
        let (state, _) = self
            .shared
            .settled
            .wait_timeout_while(state, timeout, |state| state.pending > 0)
            .unwrap_or_else(PoisonError::into_inner);
        state.pending == 0
    }
}

impl Drop for FileSpool {
    fn drop(&mut self) {
        self.shared.locked().closed = true;
        self.shared.wake_writer.notify_all();
    }
}

fn write_loop(shared: &Shared, file: File, existing_bytes: u64, maximum_spool_bytes: u64) {
    let mut out = BufWriter::new(file);
    let mut on_disk = existing_bytes;
    loop {
        let batch = {
            let mut state = shared.locked();
            while state.queue.is_empty() {
                if state.closed {
                    return;
                }
                state = shared
                    .wake_writer
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            state.in_flight_oldest_ms = state.queue.front().map(|item| item.observed_ms);
            std::mem::take(&mut state.queue)
        };
        let count = batch.len();
        let bytes: usize = batch.iter().map(|item| item.line.len()).sum();
        let mut written_classes = Vec::with_capacity(count);
        for item in batch {
            let length = u64::try_from(item.line.len()).unwrap_or(u64::MAX);
            if on_disk.saturating_add(length) > maximum_spool_bytes
                || out.write_all(&item.line).is_err()
            {
                shared.losses.record(item.class);
                continue;
            }
            on_disk = on_disk.saturating_add(length);
            written_classes.push(item.class);
        }
        if out.flush().is_ok() {
            let written = u64::try_from(written_classes.len()).unwrap_or(u64::MAX);
            shared.confirmed.fetch_add(written, Ordering::Relaxed);
        } else {
            // What was buffered cannot be said to be on disk, so none of this batch is confirmed.
            written_classes
                .into_iter()
                .for_each(|class| shared.losses.record(class));
        }
        let mut state = shared.locked();
        state.queued_bytes = state.queued_bytes.saturating_sub(bytes);
        state.pending = state
            .pending
            .saturating_sub(u64::try_from(count).unwrap_or(u64::MAX));
        state.in_flight_oldest_ms = None;
        drop(state);
        shared.settled.notify_all();
    }
}

impl RecordOutlet for FileSpool {
    fn name(&self) -> &'static str {
        "spool"
    }

    fn accept(&self, record: &ObservedRecord) -> Result<(), OutletRefusal> {
        let losses = &self.shared.losses;
        let Ok(mut line) = serde_json::to_vec(record) else {
            losses.record(record.class);
            return Err(OutletRefusal::Rejected);
        };
        line.push(b'\n');
        let cost = line.len();
        let mut state = self.shared.locked();
        if state.closed {
            losses.record(record.class);
            return Err(OutletRefusal::Closed);
        }
        if state.queued_bytes.saturating_add(cost) > self.shared.maximum_queue_bytes {
            losses.record(record.class);
            return Err(OutletRefusal::Full);
        }
        state.queued_bytes += cost;
        state.pending += 1;
        state.queue.push_back(Queued {
            line,
            class: record.class,
            observed_ms: record.observed_ms,
        });
        drop(state);
        self.shared.accepted.fetch_add(1, Ordering::Relaxed);
        self.shared.wake_writer.notify_one();
        Ok(())
    }

    fn status(&self, now_ms: u64) -> OutletStatus {
        let state = self.shared.locked();
        let oldest = state
            .queue
            .front()
            .map(|item| item.observed_ms)
            .into_iter()
            .chain(state.in_flight_oldest_ms)
            .min();
        OutletStatus {
            name: self.name().to_owned(),
            accepted: self.shared.accepted.load(Ordering::Relaxed),
            confirmed: self.shared.confirmed.load(Ordering::Relaxed),
            pending: state.pending,
            lag_ms: oldest.map_or(0, |observed| now_ms.saturating_sub(observed)),
            lost_always: self.shared.losses.always(),
            lost_verbose: self.shared.losses.verbose(),
            evicted: 0,
            unconfirmed: 0,
        }
    }
}
