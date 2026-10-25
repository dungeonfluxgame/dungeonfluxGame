//! Fake operator directory (TODO(B): replaced by B's operator grants, second factor and audit log)
//! and fake review queue (TODO(O2): replaced by `CatalogService::review_queue`).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use df_account_app::wire::{
    ActionOutcome, AuditRow, OperatorCapability, ReviewDecision, ReviewFlag, ReviewItem, ReviewKind,
};
use df_customer::Clock;
use df_types::{AccountId, OperationId};

use super::{Receipts, lock};
use crate::app_api::ports::{
    AuditEntry, OperatorAct, OperatorDirectory, OperatorGrant, PortError, PortResult, ReviewQueue,
};

#[derive(Default)]
struct DirectoryState {
    grants: HashMap<AccountId, OperatorGrant>,
    /// Newest first.
    audit: Vec<AuditRow>,
    audited: HashMap<(AccountId, OperationId), String>,
}

pub struct FakeOperators {
    clock: Arc<dyn Clock>,
    state: Mutex<DirectoryState>,
}

impl FakeOperators {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Mutex::new(DirectoryState::default()),
        }
    }

    /// Makes `account` an operator with the given capabilities.
    pub fn set_grant(&self, account: AccountId, grant: OperatorGrant) {
        lock(&self.state).grants.insert(account, grant);
    }

    pub fn grant_all(&self, account: AccountId, has_second_factor: bool) {
        self.set_grant(
            account,
            OperatorGrant {
                capabilities: vec![
                    OperatorCapability::SupportInspect,
                    OperatorCapability::Refund,
                    OperatorCapability::GoodwillCredit,
                    OperatorCapability::DisputeResponse,
                    OperatorCapability::ProviderControl,
                    OperatorCapability::ContentReview,
                ],
                has_second_factor,
            },
        );
    }
}

impl OperatorDirectory for FakeOperators {
    async fn grant(&self, account: AccountId) -> PortResult<Option<OperatorGrant>> {
        Ok(lock(&self.state).grants.get(&account).cloned())
    }

    async fn append_audit(&self, entry: AuditEntry) -> PortResult<String> {
        let mut state = lock(&self.state);
        let key = (entry.operator, entry.operation);
        if let Some(first) = state.audited.get(&key) {
            return Ok(first.clone());
        }
        let audit_id = format!("audit-{}", state.audit.len() + 1);
        state.audit.insert(
            0,
            AuditRow {
                audit_id: audit_id.clone(),
                at_ms: self.clock.now_ms(),
                operator: entry.operator_label,
                action: entry.action,
                target: entry.target,
                reason: entry.reason,
            },
        );
        state.audited.insert(key, audit_id.clone());
        Ok(audit_id)
    }

    async fn audit(&self) -> PortResult<Vec<AuditRow>> {
        Ok(lock(&self.state).audit.clone())
    }

    async fn revoke_account_session(
        &self,
        _act: crate::app_api::ports::OperatorAct,
        _account: AccountId,
        _session_id: &str,
    ) -> PortResult<ActionOutcome> {
        // The fake has no cross-account session store; the handler's audit entry is the record.
        Ok(ActionOutcome::Done)
    }
}

const REVIEW_WINDOW_MS: u64 = 3 * 24 * 60 * 60 * 1000;

#[derive(Default)]
struct ReviewState {
    items: Vec<ReviewItem>,
    receipts: Receipts<ActionOutcome>,
    decided: Vec<(String, ReviewDecision)>,
}

pub struct FakeReview {
    clock: Arc<dyn Clock>,
    state: Mutex<ReviewState>,
}

impl FakeReview {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Mutex::new(ReviewState::default()),
        }
    }

    /// Queues an uploaded portrait for review; its deadline is three days after submission.
    pub fn seed_item(&self, item_id: &str, campaign_name: &str, flag: ReviewFlag) {
        let submitted_at_ms = self.clock.now_ms();
        lock(&self.state).items.push(ReviewItem {
            item_id: item_id.to_owned(),
            kind: ReviewKind::UploadedPortrait,
            campaign_name: campaign_name.to_owned(),
            submitted_at_ms,
            review_deadline_ms: submitted_at_ms + REVIEW_WINDOW_MS,
            flag,
        });
    }

    /// The decisions taken so far, oldest first.
    pub fn decisions(&self) -> Vec<(String, ReviewDecision)> {
        lock(&self.state).decided.clone()
    }
}

impl ReviewQueue for FakeReview {
    async fn pending(&self) -> PortResult<Vec<ReviewItem>> {
        Ok(lock(&self.state).items.clone())
    }

    async fn decide(
        &self,
        act: OperatorAct,
        item_id: &str,
        decision: ReviewDecision,
    ) -> PortResult<ActionOutcome> {
        let mut state = lock(&self.state);
        if let Some(first) = state.receipts.replay(act.operator, act.operation) {
            return Ok(first);
        }
        let position = state
            .items
            .iter()
            .position(|item| item.item_id == item_id)
            .ok_or(PortError::NotFound)?;
        state.items.remove(position);
        state.decided.push((item_id.to_owned(), decision));
        state
            .receipts
            .record(act.operator, act.operation, ActionOutcome::Done);
        Ok(ActionOutcome::Done)
    }
}
