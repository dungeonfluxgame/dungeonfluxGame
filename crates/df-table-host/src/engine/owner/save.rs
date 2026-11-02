//! Writing one checkpoint: the single place the owner commits.
//!
//! Both a served batch and a forced change commit through `commit_world`, so the rules of S1 live
//! once: the attempt gets an id that is never used again (a write that times out may still land, so
//! the next attempt must not share its id); the table's revision is stamped with that id (the
//! pipeline never stamps it from an input); the checkpoint holds the table's dump and everything
//! the pipeline holds, including the results of the operations committed so far, which answer a
//! duplicate after a restart (H.2); the write runs under the owner's deadline (`committer`); and
//! only a successful write makes the new table the committed one and moves the revision. A failure
//! counts, starts the backoff and leaves the committed table as it was. The caller decides what
//! else a success or a failure means (release the turns, fail the batch).

use df_table::identity::{Correlation, RecoveryEpoch, SessionRevision, TurnSeq};
use df_table::pipeline::PipelineTable;

use crate::engine::audit::EngineAudit;
use crate::engine::commit::{Checkpoint, CommitError, CommitId};
use crate::engine::report::CheckpointDocument;

use super::Owner;

/// What a successful commit reports.
#[derive(Debug, Clone, Copy)]
pub(super) struct Saved {
    pub(super) commit: CommitId,
    /// The correlation the table's revision was stamped with, for the work that follows a commit.
    pub(super) correlation: Correlation,
}

impl Owner {
    /// Writes `working` as the next attempt, after the input served as turn `last_turn`. On
    /// success it becomes the committed table; on failure nothing changes but the counters and
    /// the backoff. `discarded_events` is how many staged events a failure throws away.
    pub(super) fn commit_world(
        &mut self,
        mut working: PipelineTable,
        inputs: usize,
        discarded_events: usize,
        last_turn: TurnSeq,
    ) -> Result<Saved, CommitError> {
        let attempt = self.last_attempt.next();
        self.last_attempt = attempt;
        let operation = self.mint_engine_operation();
        let correlation = Correlation {
            operation_id: operation,
            turn_seq: last_turn,
            revision: SessionRevision::new(RecoveryEpoch::FIRST, attempt.get()),
        };
        match self.write_checkpoint(&mut working, attempt, inputs, correlation) {
            Ok((took_ms, bytes)) => {
                self.committed = working;
                self.commit_id = attempt;
                self.stats.commits += 1;
                self.stats.last_bytes = Some(bytes);
                self.audit(EngineAudit::Committed {
                    commit: attempt.get(),
                    revision: attempt.get(),
                    inputs,
                    bytes,
                    ms: took_ms,
                    policy: self.config.policy.key(),
                });
                Ok(Saved {
                    commit: attempt,
                    correlation,
                })
            }
            Err(error) => {
                self.stats.failures += 1;
                let now_ms = self.now_ms();
                self.backoff_until_ms = Some(now_ms.saturating_add(self.config.commit_backoff_ms));
                self.audit(EngineAudit::CommitFailed {
                    inputs,
                    error,
                    discarded_events,
                });
                Err(error)
            }
        }
    }

    /// A checkpoint that cannot be built or written is the store's refusal as far as the batch is
    /// concerned.
    fn write_checkpoint(
        &self,
        working: &mut PipelineTable,
        attempt: CommitId,
        inputs: usize,
        correlation: Correlation,
    ) -> Result<(u64, usize), CommitError> {
        working
            .stamp_commit_revision(correlation)
            .map_err(|_| CommitError::Refused)?;
        let document = CheckpointDocument {
            dump: self
                .dump_of(working, attempt.get(), attempt)
                .map_err(|_| CommitError::Refused)?,
            record: working.to_record(self.now_ms()),
        };
        let bytes = serde_json::to_vec(&document).map_err(|_| CommitError::Refused)?;
        let length = bytes.len();
        let receipt = self.committer.commit(
            Checkpoint {
                commit: attempt,
                inputs,
                bytes,
            },
            self.config.commit_deadline_ms,
        )?;
        Ok((receipt.took_ms, length))
    }
}
