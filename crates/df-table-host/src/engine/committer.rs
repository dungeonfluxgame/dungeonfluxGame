//! The owner's commit under a deadline it enforces itself.
//!
//! A store may hang (S1 measured `fsync` stalls of seconds), and the owner thread must not hang
//! with it. The write therefore runs on its own thread and the owner waits for the answer at most
//! `commit_deadline_ms`: past that the batch fails `deadline_exceeded` and the owner goes on. The
//! write is then of unknown outcome, so the committer takes no new write until the old one has
//! returned (a second attempt while one is stuck is `unavailable`, at once), and the owner gives
//! every attempt its own id so a write that lands late is never mistaken for a later one. The
//! thread is not joined on drop: it ends when its job channel closes and its write, if any,
//! returns.
//!
//! It never decides what is written or what a failure means: `owner` does.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

use super::commit::{Checkpoint, CommitError, CommitReceipt, CommitStore};
use super::setup::EngineStartError;

type Answer = Result<CommitReceipt, CommitError>;

struct Job {
    checkpoint: Checkpoint,
    deadline_ms: u64,
    reply: SyncSender<Answer>,
}

/// Runs a store's writes off the owner thread and bounds how long the owner waits for one.
pub struct Committer {
    jobs: SyncSender<Job>,
    /// Set while a write is on the thread, from the moment the owner hands it over until the store
    /// returns, even if the owner has already given up waiting.
    writing: Arc<AtomicBool>,
}

impl Committer {
    pub fn start(store: Arc<dyn CommitStore>) -> Result<Self, EngineStartError> {
        let (jobs, received) = sync_channel::<Job>(1);
        let writing = Arc::new(AtomicBool::new(false));
        let thread_writing = Arc::clone(&writing);
        std::thread::Builder::new()
            .name("df-table-committer".to_owned())
            .spawn(move || {
                for job in received {
                    let answer = store.commit(&job.checkpoint, job.deadline_ms);
                    // Release: the owner's next hand-over sees the write has ended.
                    thread_writing.store(false, Ordering::Release);
                    // The owner may have stopped waiting; the write's outcome is then unknown to
                    // it, which is what its failure said.
                    let _ = job.reply.send(answer);
                }
            })
            .map_err(|error| EngineStartError::Thread(error.kind()))?;
        Ok(Self { jobs, writing })
    }

    /// Writes `checkpoint`, waiting at most `deadline_ms`. A write already stuck makes this
    /// `unavailable` without trying; a write that outlasts the deadline is `deadline_exceeded`.
    pub fn commit(&self, checkpoint: Checkpoint, deadline_ms: u64) -> Answer {
        if self.writing.swap(true, Ordering::Acquire) {
            return Err(CommitError::Unavailable);
        }
        let (reply, answer) = sync_channel(1);
        let job = Job {
            checkpoint,
            deadline_ms,
            reply,
        };
        if let Err(error) = self.jobs.try_send(job) {
            self.writing.store(false, Ordering::Release);
            return Err(match error {
                TrySendError::Full(_) | TrySendError::Disconnected(_) => CommitError::Unavailable,
            });
        }
        match answer.recv_timeout(Duration::from_millis(deadline_ms)) {
            Ok(answer) => answer,
            Err(RecvTimeoutError::Timeout) => Err(CommitError::DeadlineExceeded { deadline_ms }),
            Err(RecvTimeoutError::Disconnected) => Err(CommitError::Unavailable),
        }
    }
}
