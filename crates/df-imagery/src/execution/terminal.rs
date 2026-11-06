//! Output-only terminal authorization retains supplier outcomes and money reconciliation.
use super::{
    CandidateReceipt, DispatchId, DispatchView, ExecutionScope, ExecutionStageId, RecoveryLease,
    StageBasis, StageClaim, StageScope, StageView,
};
use std::fmt;

#[derive(Clone, PartialEq)]
pub enum TerminalActor {
    Dispatch(ExecutionScope),
    UnsentStage(StageClaim),
    UnclaimedStage(StageScope),
    DeferredStage(StageScope),
    ReleasedStage(StageScope),
}
pub type TerminalTail = TerminalActor;
impl TerminalActor {
    pub fn scope(&self) -> StageScope {
        match self {
            Self::Dispatch(scope) => StageScope {
                owner: scope.owner.clone(),
                job: scope.job,
                stage: scope.stage,
            },
            Self::UnsentStage(claim) => claim.scope.clone(),
            Self::UnclaimedStage(scope)
            | Self::DeferredStage(scope)
            | Self::ReleasedStage(scope) => scope.clone(),
        }
    }
}
impl fmt::Debug for TerminalActor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Dispatch(_) => "Dispatch { .. }",
            Self::UnsentStage(_) => "UnsentStage { .. }",
            Self::UnclaimedStage(_) => "UnclaimedStage { .. }",
            Self::DeferredStage(_) => "DeferredStage { .. }",
            Self::ReleasedStage(_) => "ReleasedStage { .. }",
        })
    }
}
#[derive(Clone, PartialEq)]
// Public cross-crate enum built by df-persistence; boxing a variant would break that API.
#[allow(clippy::large_enum_variant)]
pub enum TerminalAuthority {
    Claim(StageClaim),
    Recovery(RecoveryLease),
    Unclaimed(super::UnclaimedTerminal),
    Deferred(super::DeferredTerminal),
    Released(super::ReleasedTerminal),
}
impl fmt::Debug for TerminalAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Claim(_) => "Claim { .. }",
            Self::Recovery(_) => "Recovery { .. }",
            Self::Unclaimed(_) => "Unclaimed { .. }",
            Self::Deferred(_) => "Deferred { .. }",
            Self::Released(_) => "Released { .. }",
        })
    }
}
/// First-write compare-and-set proof. Exact stored seal replay is recognized before validation.
#[derive(Clone, PartialEq)]
pub struct TerminalCommand {
    pub actor: TerminalActor,
    pub tail: TerminalTail,
    pub authority: TerminalAuthority,
    pub expected_job_revision: u64,
    pub candidate_dispatch: Option<DispatchId>,
    pub now_ms: i64,
}
impl fmt::Debug for TerminalCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalCommand")
            .field("actor", &self.actor)
            .field("tail", &self.tail)
            .field("expected_job_revision", &self.expected_job_revision)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, PartialEq)]
pub struct TerminalStageSnapshot {
    pub stage: StageView,
    pub basis: StageBasis,
    pub dispatch: Option<DispatchView>,
}
impl fmt::Debug for TerminalStageSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalStageSnapshot")
            .field("kind", &self.basis.kind)
            .finish_non_exhaustive()
    }
}
/// Absence means no actual allocation row, regardless of separately planned nonzero ids.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum TerminalAllocation {
    NoAllocation,
    Existing {
        stage: ExecutionStageId,
        asset: [u8; 16],
        attempt: [u8; 16],
    },
}
impl fmt::Debug for TerminalAllocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoAllocation => "NoAllocation",
            Self::Existing { .. } => "Existing { .. }",
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalValidation {
    pub actor: TerminalStageSnapshot,
    pub tail: TerminalStageSnapshot,
    pub candidate: Option<CandidateReceipt>,
    pub allocation: TerminalAllocation,
    pub job_revision: u64,
}
