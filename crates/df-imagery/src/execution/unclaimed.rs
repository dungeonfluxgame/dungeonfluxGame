//! Four finite no-dispatch terminal states carry actual stored evidence, never invented claims.
use super::{DispatchId, ExecutionStageId, StageClaim, StageScope};
use crate::ImageJobId;
use std::fmt;
#[derive(Clone, Eq, PartialEq)]
pub struct UnclaimedTerminal {
    pub scope: StageScope,
    pub job_revision: u64,
}
#[derive(Clone, Eq, PartialEq)]
pub struct ClaimedTerminal {
    pub claim: StageClaim,
    pub job_revision: u64,
}
#[derive(Clone, Eq, PartialEq)]
pub struct DeferredTerminal {
    pub scope: StageScope,
    pub job_revision: u64,
    pub claim_generation: u64,
    pub not_before_ms: i64,
}
#[derive(Clone, Eq, PartialEq)]
pub struct ReleasedTerminal {
    pub scope: StageScope,
    pub job_revision: u64,
    pub claim_generation: u64,
    pub candidate_dispatch: DispatchId,
}
macro_rules! private_proof_debug {($($name:ident),*)=>{$(impl fmt::Debug for $name{fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result{f.debug_struct(stringify!($name)).field("job_revision",&self.job_revision).finish_non_exhaustive()}})*};}
private_proof_debug!(
    UnclaimedTerminal,
    ClaimedTerminal,
    DeferredTerminal,
    ReleasedTerminal
);
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnsentTerminal {
    Unclaimed(UnclaimedTerminal),
    Claimed(ClaimedTerminal),
    Deferred(DeferredTerminal),
    Released(ReleasedTerminal),
}
impl UnsentTerminal {
    pub fn scope(&self) -> &StageScope {
        match self {
            Self::Unclaimed(p) => &p.scope,
            Self::Claimed(p) => &p.claim.scope,
            Self::Deferred(p) => &p.scope,
            Self::Released(p) => &p.scope,
        }
    }
    pub fn job_revision(&self) -> u64 {
        match self {
            Self::Unclaimed(p) => p.job_revision,
            Self::Claimed(p) => p.job_revision,
            Self::Deferred(p) => p.job_revision,
            Self::Released(p) => p.job_revision,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalCursor {
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
}
