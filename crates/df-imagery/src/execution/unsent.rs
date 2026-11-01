//! Historical Prepared discovery is metadata, never send or financial release authority.
use super::{DispatchView, ExecutionScope, StageClaim};
use std::fmt;
#[derive(Clone, PartialEq)]
pub struct UnsentPrepared {
    pub scope: ExecutionScope,
    pub claim: StageClaim,
    pub current: DispatchView,
    pub job_revision: u64,
    pub original_lease_until_ms: i64,
    /// Actual retirement, deadline or demand absence at this snapshot; a writer must recheck.
    pub terminal_condition: bool,
    /// Exact immutable output seal names this dispatch as its tail.
    pub sealed: bool,
}
impl fmt::Debug for UnsentPrepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnsentPrepared")
            .field("state", &self.current.state)
            .field("terminal_condition", &self.terminal_condition)
            .field("sealed", &self.sealed)
            .finish_non_exhaustive()
    }
}
