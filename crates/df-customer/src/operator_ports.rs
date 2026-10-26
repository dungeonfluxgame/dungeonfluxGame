//! Consumer-owned durable operator store. Grant commands recheck current account and MFA
//! authority inside their transaction; no caller-provided verification boolean is accepted.

use std::future::Future;

use df_types::AccountId;

use crate::operator_contract::{
    OperatorAuditId, OperatorAuditRecord, OperatorCommand, OperatorError, OperatorGrant,
    OperatorGrantId, OperatorPageSize, OperatorReceipt,
};

pub trait OperatorStore: Clone + Send + Sync + 'static {
    /// Replay lookup precedes time/account checks; a conflicting payload always refuses.
    fn receipt(
        &self,
        command: &OperatorCommand,
    ) -> impl Future<Output = Result<Option<OperatorReceipt>, OperatorError>> + Send;

    /// Mutation, immutable receipt and authoritative audit commit together. `now_ms` is injected.
    fn execute(
        &self,
        command: OperatorCommand,
        now_ms: u64,
    ) -> impl Future<Output = Result<OperatorReceipt, OperatorError>> + Send;

    /// Only current verified accounts with active matching MFA and a live grant are returned.
    fn current_grant(
        &self,
        id: OperatorGrantId,
        now_ms: u64,
    ) -> impl Future<Output = Result<Option<OperatorGrant>, OperatorError>> + Send;

    /// Historical grants for one account, ordered by stable grant id, including revocations.
    fn grants(
        &self,
        account: AccountId,
        after: Option<OperatorGrantId>,
        limit: OperatorPageSize,
    ) -> impl Future<Output = Result<Vec<OperatorGrant>, OperatorError>> + Send;

    /// Account-scoped keyset audit. This internal projection requires caller admission.
    fn audit(
        &self,
        account: AccountId,
        after: Option<OperatorAuditId>,
        limit: OperatorPageSize,
    ) -> impl Future<Output = Result<Vec<OperatorAuditRecord>, OperatorError>> + Send;
}
