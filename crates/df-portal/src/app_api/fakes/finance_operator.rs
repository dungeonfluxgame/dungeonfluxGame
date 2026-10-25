//! The operator side of the fake finance: ledger, refunds, goodwill credits, disputes,
//! reconciliation and providers (TODO(F): replaced by the finance service's operator reads).

use df_account_app::{
    api::ProviderSwitch,
    wire::{
        ActionOutcome, Direction, DisputeRow, DisputeStatus, LedgerEntry, LedgerKind, LineStatus,
        ProviderHealth, ProviderRow, ReconciliationDifference, ReconciliationKind, RefundableLot,
        Refusal, StatementCategory,
    },
};
use df_types::AccountId;

use super::{
    finance::{FakeFinance, MICRO_DOLLARS_PER_CENT},
    lock,
};
use crate::app_api::{
    ids::account_text,
    ports::{FinanceSummary, OperatorAct, PortError, PortResult},
};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

impl FakeFinance {
    pub(super) fn summary_now(&self) -> FinanceSummary {
        let state = lock(&self.state);
        FinanceSummary {
            customer_liability_micro_dollars: state
                .wallets
                .values()
                .map(|wallet| {
                    wallet.purchased_micro + wallet.promotional_micro + wallet.allocated_micro
                })
                .sum(),
            open_disputes: state
                .disputes
                .iter()
                .filter(|dispute| dispute.status == DisputeStatus::NeedsResponse)
                .count() as u64,
            reconciliation_differences: state.reconciliation.differences.len() as u64,
            providers_degraded: state
                .providers
                .iter()
                .filter(|provider| provider.health != ProviderHealth::Healthy)
                .count() as u64,
        }
    }

    pub(super) fn balance_now(&self, account: AccountId) -> u64 {
        lock(&self.state).wallets.get(&account).map_or(0, |wallet| {
            wallet.purchased_micro + wallet.promotional_micro
        })
    }

    pub(super) fn ledger_now(&self, account: AccountId) -> (Vec<LedgerEntry>, Vec<RefundableLot>) {
        lock(&self.state).wallets.get(&account).map_or_else(
            || (Vec::new(), Vec::new()),
            |wallet| {
                let lots = wallet
                    .lots
                    .iter()
                    .filter(|lot| lot.refundable_cents > 0)
                    .cloned()
                    .collect();
                (wallet.ledger.clone(), lots)
            },
        )
    }

    pub(super) fn refund_now(
        &self,
        act: &OperatorAct,
        account: AccountId,
        lot_id: &str,
        amount_cents: u32,
    ) -> PortResult<ActionOutcome> {
        let mut state = lock(&self.state);
        if let Some(first) = state.receipts.replay(act.operator, act.operation) {
            return Ok(first);
        }
        let wallet = state.wallets.get_mut(&account).ok_or(PortError::NotFound)?;
        let lot = wallet
            .lots
            .iter_mut()
            .find(|lot| lot.lot_id == lot_id)
            .ok_or(PortError::NotFound)?;
        let micro = u64::from(amount_cents) * MICRO_DOLLARS_PER_CENT;
        if amount_cents > lot.refundable_cents || micro > wallet.purchased_micro {
            return Err(PortError::Refused(Refusal::AmountOutOfRange));
        }
        lot.refundable_cents -= amount_cents;
        wallet.purchased_micro -= micro;
        let mut line = self.line_for(StatementCategory::Refund, Direction::Charge, micro);
        line.status = LineStatus::Pending;
        let mut entry = self.entry_for(LedgerKind::Refund, micro);
        entry.status = LineStatus::Pending;
        self.post_line(&mut state, account, line, Some(entry));
        state
            .receipts
            .record(act.operator, act.operation, ActionOutcome::Pending);
        Ok(ActionOutcome::Pending)
    }

    pub(super) fn goodwill_now(
        &self,
        act: &OperatorAct,
        account: AccountId,
        amount_cents: u32,
    ) -> PortResult<ActionOutcome> {
        let mut state = lock(&self.state);
        if let Some(first) = state.receipts.replay(act.operator, act.operation) {
            return Ok(first);
        }
        let micro = u64::from(amount_cents) * MICRO_DOLLARS_PER_CENT;
        state.wallet(account).promotional_micro += micro;
        let line = self.line_for(StatementCategory::GoodwillCredit, Direction::Credit, micro);
        let entry = self.entry_for(LedgerKind::GoodwillCredit, micro);
        self.post_line(&mut state, account, line, Some(entry));
        state
            .receipts
            .record(act.operator, act.operation, ActionOutcome::Done);
        Ok(ActionOutcome::Done)
    }

    pub(super) fn evidence_now(
        &self,
        act: &OperatorAct,
        dispute_id: &str,
    ) -> PortResult<ActionOutcome> {
        let mut state = lock(&self.state);
        if let Some(first) = state.receipts.replay(act.operator, act.operation) {
            return Ok(first);
        }
        let dispute = state
            .disputes
            .iter_mut()
            .find(|dispute| dispute.dispute_id == dispute_id)
            .ok_or(PortError::NotFound)?;
        dispute.status = DisputeStatus::UnderReview;
        state
            .receipts
            .record(act.operator, act.operation, ActionOutcome::Done);
        Ok(ActionOutcome::Done)
    }

    pub(super) fn switch_now(
        &self,
        act: &OperatorAct,
        provider_id: &str,
        switch: ProviderSwitch,
    ) -> PortResult<ActionOutcome> {
        let mut state = lock(&self.state);
        if let Some(first) = state.receipts.replay(act.operator, act.operation) {
            return Ok(first);
        }
        let provider = state
            .providers
            .iter_mut()
            .find(|provider| provider.provider_id == provider_id)
            .ok_or(PortError::NotFound)?;
        provider.health = match switch {
            ProviderSwitch::Pause => ProviderHealth::Paused,
            ProviderSwitch::Resume => ProviderHealth::Healthy,
        };
        state
            .receipts
            .record(act.operator, act.operation, ActionOutcome::Done);
        Ok(ActionOutcome::Done)
    }

    /// Opens a dispute that needs a response within five days.
    pub fn seed_dispute(&self, account: AccountId, amount_cents: u32) -> String {
        let mut state = lock(&self.state);
        let dispute_id = state.next_id("dispute");
        let now_ms = self.now_ms();
        state.disputes.push(DisputeRow {
            dispute_id: dispute_id.clone(),
            account_id: account_text(account),
            amount_cents,
            status: DisputeStatus::NeedsResponse,
            opened_at_ms: now_ms,
            evidence_due_at_ms: now_ms + 5 * DAY_MS,
        });
        dispute_id
    }

    pub fn seed_reconciliation_difference(
        &self,
        kind: ReconciliationKind,
        reference: &str,
        difference_micro_dollars: u64,
    ) {
        let mut state = lock(&self.state);
        let now_ms = self.now_ms();
        state.reconciliation.last_run_at_ms = Some(now_ms);
        state
            .reconciliation
            .differences
            .push(ReconciliationDifference {
                kind,
                reference: reference.to_owned(),
                difference_micro_dollars,
                found_at_ms: now_ms,
            });
    }

    pub fn seed_provider(
        &self,
        provider_id: &str,
        name: &str,
        health: ProviderHealth,
        error_rate_permille: u32,
        open_exposure_micro_dollars: u64,
    ) {
        lock(&self.state).providers.push(ProviderRow {
            provider_id: provider_id.to_owned(),
            name: name.to_owned(),
            health,
            error_rate_permille,
            open_exposure_micro_dollars,
        });
    }
}
