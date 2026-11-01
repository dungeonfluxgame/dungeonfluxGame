//! Discovery and exclusive recovery ownership for retained dispatches.
//! Recovery never replaces a normal stage claim or authorizes a fresh send.

use super::{ClaimToken, DispatchId, DispatchView, ExecutionStageId, ImageJobId, StageKind};
use crate::request::ImageOwner;

/// A retained dispatch and its original claim lineage, for scoped reconciliation only.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryStage {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub kind: StageKind,
    pub claim: ClaimToken,
    pub claim_generation: u64,
    pub current: DispatchView,
}

/// An exclusive recovery lease, distinct from the original dispatch's normal claim.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryLease {
    pub stage: RecoveryStage,
    pub token: ClaimToken,
    pub generation: u64,
    pub lease_until_ms: i64,
}

/// Requests bounded recovery ownership of one exact current dispatch using a fresh token.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimRecoveryCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub lease_ms: i64,
    pub now_ms: i64,
}

/// Completed requires the exact terminal receipt and settlement; retry persists the next poll time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryDisposition {
    Completed,
    RetryAt { next_at_ms: i64 },
}

/// Releases one exact recovery lease, retaining its completion or next-poll receipt for replay.
#[derive(Clone, Debug, PartialEq)]
pub struct FinishRecoveryCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub claim_generation: u64,
    pub disposition: RecoveryDisposition,
    pub now_ms: i64,
}

/// Explicit local-only finalization derived from one already-paid candidate recovery lease.
#[derive(Clone, PartialEq)]
pub struct ReconciliationCommand {
    pub candidate_dispatch: DispatchId,
    pub child: ExecutionStageId,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub capacity: super::CapacityScope,
    pub recipe_revision: u32,
    pub recipe_digest: [u8; 32],
    pub now_ms: i64,
}
impl std::fmt::Debug for ReconciliationCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReconciliationCommand")
            .field("recipe_revision", &self.recipe_revision)
            .finish_non_exhaustive()
    }
}
/// Prepared LOCAL work; the current dispatch and stage claim are never supplier send authority.
#[derive(Clone, Debug, PartialEq)]
pub struct ReconciliationDispatch {
    pub stage: super::StageView,
    pub claim: super::StageClaim,
    pub current: DispatchView,
}
