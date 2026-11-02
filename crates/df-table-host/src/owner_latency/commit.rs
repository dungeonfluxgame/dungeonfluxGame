//! The stand-in whole-checkpoint commit that the owner pays inside its reduce time.
//!
//! One commit serialises a checkpoint document (the table state plus a journal of past turns that
//! pads it to a target size), hashes it, and then stores it in one of three ways: nowhere (the cost
//! of the first two steps alone), a file that is written and `fsync`ed, or a fixed sleep that
//! models a Postgres round trip. The document is built into a fresh buffer each time, as a real
//! commit that must own its bytes would.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::error::HarnessError;
use super::sha256::sha256;
use super::state::TableState;
use super::statistics::nanoseconds;

/// The sleep that models one Postgres commit round trip (the task brief's assumption).
pub const MODELLED_POSTGRES_MS: u64 = 20;

const CHECKPOINT_FILE: &str = "checkpoint.json";
const BUFFER_SLACK_BYTES: usize = 4_096;

/// Where a committed checkpoint goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitMode {
    /// Serialise and hash only; no storage.
    Memory,
    /// Write a file and `fsync` it.
    Fsync,
    /// Sleep `MODELLED_POSTGRES_MS` instead of storing.
    ModelledPostgres,
}

impl CommitMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Fsync => "fsync",
            Self::ModelledPostgres => "postgres20ms",
        }
    }
}

#[derive(Serialize)]
struct JournalLine {
    turn: u64,
    kind: &'static str,
    note: String,
}

#[derive(Serialize)]
struct Checkpoint<'a> {
    state: &'a TableState,
    journal: &'a VecDeque<JournalLine>,
}

/// What one commit cost, by step, and how big the document was.
#[derive(Debug, Clone, Copy)]
pub struct CommitTimes {
    pub serialise_ns: u64,
    pub hash_ns: u64,
    pub store_ns: u64,
    pub document_bytes: usize,
}

fn journal_line(turn: u64) -> JournalLine {
    JournalLine {
        turn,
        kind: "applied",
        note: format!("turn {turn}: the party moved, a fact was learned and a line was narrated"),
    }
}

/// Commits checkpoints of one table.
pub struct Committer {
    mode: CommitMode,
    path: PathBuf,
    journal: VecDeque<JournalLine>,
    last_document_bytes: usize,
}

impl Committer {
    /// A committer whose checkpoint is at least `target_bytes` for `state`: the journal is padded
    /// until it is, and the document is larger than the target when the state alone is. The file
    /// of `CommitMode::Fsync` goes in `directory`.
    pub fn new(
        mode: CommitMode,
        directory: &Path,
        target_bytes: usize,
        state: &TableState,
    ) -> Result<Self, HarnessError> {
        let state_bytes = serde_json::to_vec(state)?.len();
        let line_bytes = serde_json::to_vec(&journal_line(1))?.len() + 1;
        let lines = target_bytes.saturating_sub(state_bytes) / line_bytes.max(1);
        let journal = (0..lines).map(|turn| journal_line(turn as u64)).collect();
        Ok(Self {
            mode,
            path: directory.join(CHECKPOINT_FILE),
            journal,
            last_document_bytes: state_bytes + lines * line_bytes,
        })
    }

    /// The size of the last document, for the report; the first document's before any commit.
    pub fn document_bytes(&self) -> usize {
        self.last_document_bytes
    }

    /// Serialises, hashes and stores the checkpoint of `state` after `turn`, keeping the journal at
    /// a constant length.
    pub fn commit(&mut self, state: &TableState, turn: u64) -> Result<CommitTimes, HarnessError> {
        if !self.journal.is_empty() {
            self.journal.pop_front();
            self.journal.push_back(journal_line(turn));
        }
        let started = Instant::now();
        let mut document = Vec::with_capacity(self.last_document_bytes + BUFFER_SLACK_BYTES);
        serde_json::to_writer(
            &mut document,
            &Checkpoint {
                state,
                journal: &self.journal,
            },
        )?;
        let serialise_ns = nanoseconds(started.elapsed());
        self.last_document_bytes = document.len();

        let started = Instant::now();
        let digest = sha256(&document);
        std::hint::black_box(digest);
        let hash_ns = nanoseconds(started.elapsed());

        let started = Instant::now();
        self.store(&document)?;
        Ok(CommitTimes {
            serialise_ns,
            hash_ns,
            store_ns: nanoseconds(started.elapsed()),
            document_bytes: document.len(),
        })
    }

    fn store(&self, document: &[u8]) -> Result<(), HarnessError> {
        match self.mode {
            CommitMode::Memory => Ok(()),
            CommitMode::ModelledPostgres => {
                std::thread::sleep(Duration::from_millis(MODELLED_POSTGRES_MS));
                Ok(())
            }
            CommitMode::Fsync => {
                let io = |step| move |source| HarnessError::Io { step, source };
                let mut file = File::create(&self.path).map_err(io("creating the checkpoint"))?;
                file.write_all(document)
                    .map_err(io("writing the checkpoint"))?;
                file.sync_all().map_err(io("syncing the checkpoint"))
            }
        }
    }
}
