//! How the owner saves what it did: the commit policy and the store behind it (S1 decision).
//!
//! S1 measured that the commit is over 90 percent of a turn and that eighteen inputs at once miss
//! the 400 ms budget at one commit per input, so the owner commits once per drained batch: it runs
//! each input in serve order against in-memory state, writes one checkpoint for the batch, and
//! releases every input's record and projection after that write. `CommitPolicy` is the seam:
//! `PerInput` is a batch of one, `PerBatch` (the default) is cut by a time slice. A commit has a
//! deadline; past it, or on any failure, the whole batch fails with a typed cause, nothing of it is
//! kept, and the owner backs off. This module owns the policy, the checkpoint and the store port;
//! it never decides what a batch holds (`owner` does). Who enforces the deadline is the owner
//! (`committer`), not the store: the owner waits for the write at most `commit_deadline_ms`, so a
//! store that hangs costs one refused batch and never the owner thread.
//!
//! `CommitStore` is the port, with two implementations in the host: `MemoryCommitStore` (which
//! keeps the newest checkpoint and the operations committed) and `ScriptedCommitStore` (scripted
//! latency and failure, for tests and the `chaos` verb). A store may enforce the deadline itself,
//! as a real one will with a timeout on the write (the scripted one does, with simulated latency),
//! but the owner does not rely on it. Every attempt has its own id, burned whether it succeeded or
//! not, so a write that lands late can never be mistaken for the next batch's.

use std::collections::VecDeque;
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use super::limits::{BATCH_TIME_SLICE_MS, MAXIMUM_BATCH_INPUTS, MAXIMUM_COMMIT_RECORDS};

/// How many inputs one commit covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitPolicy {
    /// One commit per input: a batch of one.
    PerInput,
    /// One commit per drained batch, cut after `time_slice_ms` or `maximum_inputs`.
    PerBatch {
        time_slice_ms: u64,
        maximum_inputs: NonZeroUsize,
    },
}

impl CommitPolicy {
    /// The default: per batch, with the large named bounds of `limits`.
    pub fn per_batch() -> Self {
        Self::PerBatch {
            time_slice_ms: BATCH_TIME_SLICE_MS,
            maximum_inputs: NonZeroUsize::new(MAXIMUM_BATCH_INPUTS).unwrap_or(NonZeroUsize::MIN),
        }
    }

    /// The most inputs one batch serves before it commits.
    pub fn maximum_inputs(self) -> usize {
        match self {
            Self::PerInput => 1,
            Self::PerBatch { maximum_inputs, .. } => maximum_inputs.get(),
        }
    }

    /// How long a batch may keep serving inputs, or `None` when the count alone cuts it.
    pub fn time_slice_ms(self) -> Option<u64> {
        match self {
            Self::PerInput => None,
            Self::PerBatch { time_slice_ms, .. } => Some(time_slice_ms),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::PerInput => "per_input",
            Self::PerBatch { .. } => "per_batch",
        }
    }
}

/// The identity of one commit, one more than the last that succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CommitId(u64);

impl CommitId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// What a commit writes: the whole table state after a batch, as the bytes of its dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// The id of this attempt, one more than the last attempt's, whether or not that one wrote.
    pub commit: CommitId,
    /// How many inputs the batch held.
    pub inputs: usize,
    /// The checkpoint document as JSON: the table's dump and everything the pipeline holds,
    /// including the results of the operations the commit covers, which is what answers a
    /// duplicate after a restart.
    pub bytes: Vec<u8>,
}

/// What a store hands back when the owner starts: the newest attempt id it has seen, so ids go on
/// from there. The table itself is restored by C7, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub last_attempt: CommitId,
}

/// What a successful write reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitReceipt {
    pub took_ms: u64,
}

/// Why a write did not happen. Each is typed so the owner records one cause per batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum CommitError {
    /// The write took longer than the deadline, so it was given up.
    DeadlineExceeded { deadline_ms: u64 },
    /// The store could not be reached.
    Unavailable,
    /// The store refused the write.
    Refused,
}

impl CommitError {
    pub fn key(self) -> &'static str {
        match self {
            Self::DeadlineExceeded { .. } => "deadline_exceeded",
            Self::Unavailable => "unavailable",
            Self::Refused => "refused",
        }
    }
}

impl fmt::Display for CommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeadlineExceeded { deadline_ms } => {
                write!(formatter, "the commit took longer than {deadline_ms} ms")
            }
            Self::Unavailable => formatter.write_str("the store is unavailable"),
            Self::Refused => formatter.write_str("the store refused the write"),
        }
    }
}

impl std::error::Error for CommitError {}

/// The place a checkpoint is written. The owner calls it on its own thread, so an implementation
/// must enforce `deadline_ms` itself and return `DeadlineExceeded` rather than run past it.
pub trait CommitStore: Send + Sync {
    fn commit(
        &self,
        checkpoint: &Checkpoint,
        deadline_ms: u64,
    ) -> Result<CommitReceipt, CommitError>;

    /// What the store holds from earlier runs, or `None` when it holds nothing.
    fn restore(&self) -> Option<Restored>;
}

/// One write the store kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRecord {
    pub commit: CommitId,
    pub inputs: usize,
    pub bytes: usize,
}

#[derive(Debug, Default)]
struct Kept {
    records: VecDeque<CommitRecord>,
    latest: Option<Vec<u8>>,
    last_attempt: Option<CommitId>,
}

/// A store that keeps the newest checkpoint and a record of every write, in memory.
#[derive(Debug, Default)]
pub struct MemoryCommitStore {
    kept: Mutex<Kept>,
}

impl MemoryCommitStore {
    fn keep(&self, checkpoint: &Checkpoint) {
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        kept.records.push_back(CommitRecord {
            commit: checkpoint.commit,
            inputs: checkpoint.inputs,
            bytes: checkpoint.bytes.len(),
        });
        while kept.records.len() > MAXIMUM_COMMIT_RECORDS {
            kept.records.pop_front();
        }
        kept.latest = Some(checkpoint.bytes.clone());
        kept.last_attempt = Some(checkpoint.commit);
    }

    /// The writes that succeeded, oldest first.
    pub fn records(&self) -> Vec<CommitRecord> {
        let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        kept.records.iter().cloned().collect()
    }

    /// The bytes of the newest checkpoint.
    pub fn latest(&self) -> Option<Vec<u8>> {
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .latest
            .clone()
    }
}

impl CommitStore for MemoryCommitStore {
    fn commit(
        &self,
        checkpoint: &Checkpoint,
        _deadline_ms: u64,
    ) -> Result<CommitReceipt, CommitError> {
        self.keep(checkpoint);
        Ok(CommitReceipt { took_ms: 0 })
    }

    fn restore(&self) -> Option<Restored> {
        let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        kept.last_attempt
            .map(|last_attempt| Restored { last_attempt })
    }
}

/// What a scripted write does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptedCommit {
    /// Succeeds after `latency_ms`; past the deadline that is `DeadlineExceeded` and nothing is
    /// kept.
    Succeeds {
        latency_ms: u64,
    },
    Fails(CommitError),
    /// Reports a failure although the write landed: the unknown outcome of a timed-out write.
    FailsButLands(CommitError),
}

/// A run of identical scripted writes: `commit`, for each of the next `remaining` attempts. A run
/// is one entry however long, so `chaos commit <fault> <count>` costs no memory per write.
#[derive(Debug, Clone, Copy)]
struct ScriptedRun {
    commit: ScriptedCommit,
    remaining: u64,
}

/// A store whose writes follow a script, one entry per attempt, then succeed at once.
#[derive(Debug, Default)]
pub struct ScriptedCommitStore {
    script: Mutex<VecDeque<ScriptedRun>>,
    memory: MemoryCommitStore,
    attempts: AtomicU64,
}

impl ScriptedCommitStore {
    pub fn scripted(script: impl IntoIterator<Item = ScriptedCommit>) -> Self {
        Self {
            script: Mutex::new(
                script
                    .into_iter()
                    .map(|commit| ScriptedRun {
                        commit,
                        remaining: 1,
                    })
                    .collect(),
            ),
            ..Self::default()
        }
    }

    /// Adds `count` writes that each do `scripted`, after those already waiting. A count of 0
    /// adds nothing.
    pub fn push(&self, scripted: ScriptedCommit, count: u64) {
        if count == 0 {
            return;
        }
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(ScriptedRun {
                commit: scripted,
                remaining: count,
            });
    }

    /// How many writes were attempted, whether or not they succeeded.
    pub fn attempts(&self) -> u64 {
        // Relaxed: a counter that orders no other data.
        self.attempts.load(Ordering::Relaxed)
    }

    pub fn records(&self) -> Vec<CommitRecord> {
        self.memory.records()
    }

    pub fn latest(&self) -> Option<Vec<u8>> {
        self.memory.latest()
    }
}

impl CommitStore for ScriptedCommitStore {
    fn commit(
        &self,
        checkpoint: &Checkpoint,
        deadline_ms: u64,
    ) -> Result<CommitReceipt, CommitError> {
        self.attempts.fetch_add(1, Ordering::Relaxed);
        let next = {
            let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
            match script.front_mut() {
                Some(run) if run.remaining > 1 => {
                    run.remaining -= 1;
                    Some(run.commit)
                }
                Some(_) => script.pop_front().map(|run| run.commit),
                None => None,
            }
        };
        match next {
            Some(ScriptedCommit::Fails(error)) => Err(error),
            Some(ScriptedCommit::FailsButLands(error)) => {
                self.memory.keep(checkpoint);
                Err(error)
            }
            Some(ScriptedCommit::Succeeds { latency_ms }) if latency_ms > deadline_ms => {
                Err(CommitError::DeadlineExceeded { deadline_ms })
            }
            Some(ScriptedCommit::Succeeds { latency_ms }) => {
                self.memory.keep(checkpoint);
                Ok(CommitReceipt {
                    took_ms: latency_ms,
                })
            }
            None => {
                self.memory.keep(checkpoint);
                Ok(CommitReceipt { took_ms: 0 })
            }
        }
    }

    fn restore(&self) -> Option<Restored> {
        self.memory.restore()
    }
}

impl ScriptedCommitStore {
    /// How many scripted writes still wait.
    pub fn scripted_remaining(&self) -> u64 {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|run| run.remaining)
            .fold(0, u64::saturating_add)
    }
}
