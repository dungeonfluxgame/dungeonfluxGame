//! Native account API adapters; missing owner contracts refuse without changing state.

use df_account_app::{
    api::ProviderSwitch,
    wire::{
        AccountStats, ActionOutcome, BalanceView, ClosureBlocker, DisputeRow, FundingView,
        LedgerEntry, OverviewView, PacePreset, ProviderRow, ReconciliationView, RefundableLot,
        StatementFilter, StatementsView, TrialView,
    },
};
use df_commerce::{finance_ids::PayerId, ledger::CurrencyTag};
use df_customer::AccountStore;
use df_persistence::account_store::PostgresAccountStore;
use df_portal::app_api::ports::*;
use df_types::{AccountId, CampaignId, OperationId, SessionId};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::sync::Arc;

#[path = "finance_service.rs"]
mod backend;

#[derive(Clone)]
pub(crate) struct Finance {
    inner: Arc<Inner>,
}

struct Inner {
    service: tokio::sync::Mutex<backend::Service>,
    _driver: crate::database::Driver,
    accounts: PostgresAccountStore,
    trial_email_key: [u8; 32],
}

impl Finance {
    pub(crate) fn new(
        connection: crate::database::Connection,
        accounts: PostgresAccountStore,
        trial_email_key: [u8; 32],
    ) -> PortResult<Self> {
        Ok(Self {
            inner: Arc::new(Inner {
                service: tokio::sync::Mutex::new(backend::create(connection.client)?),
                _driver: connection.driver,
                accounts,
                trial_email_key,
            }),
        })
    }

    pub(crate) async fn purchase(
        &self,
        account: AccountId,
        cents: u32,
        reference: &str,
    ) -> PortResult<()> {
        let record = self
            .inner
            .accounts
            .account(account)
            .await
            .map_err(|_| PortError::Unavailable)?
            .ok_or(PortError::NotFound)?;
        if !record.is_verified {
            return Err(PortError::Forbidden);
        }
        self.inner
            .service
            .lock()
            .await
            .apply_purchase(df_finance::service::PurchaseCommand {
                payer: PayerId::from_array(*account.as_bytes()),
                processor: "development",
                payment_ref: reference,
                currency: CurrencyTag::USD,
                amount_micro_dollars: u64::from(cents) * 10_000,
            })
            .await
            .map_err(finance_error)?;
        df_observe::record_audit_line(
            "hosted.purchase_posted",
            format!(
                "SPEND purchase account={} amount_micro_dollars={}",
                df_commerce::finance_ids::hex(account.as_bytes()),
                u64::from(cents) * 10_000
            ),
        );
        Ok(())
    }
}

fn finance_error(error: df_finance::service::FinanceError) -> PortError {
    df_observe::record_audit_line(
        "hosted.finance_unavailable",
        "HOSTED finance_unavailable".to_owned(),
    );
    match error {
        df_finance::service::FinanceError::Funding(_) => PortError::Forbidden,
        _ => PortError::UnknownOutcome,
    }
}

impl FinancePort for Finance {
    async fn overview(&self, account: AccountId, is_verified: bool) -> PortResult<OverviewView> {
        let record = self
            .inner
            .accounts
            .account(account)
            .await
            .map_err(|_| PortError::Unavailable)?
            .ok_or(PortError::NotFound)?;
        let mut service = self.inner.service.lock().await;
        if is_verified && record.is_verified {
            let email = record.email.as_ref().ok_or(PortError::Unavailable)?;
            let email_key: [u8; 32] =
                derive_email_key(&self.inner.trial_email_key, email.normalized())?;
            service
                .grant_trial(
                    PayerId::from_array(*account.as_bytes()),
                    email_key,
                    CurrencyTag::USD,
                )
                .await
                .map_err(finance_error)?;
        }
        let row = service.client().query_one(
            "SELECT coalesce(sum(b.natural_micro_dollars) FILTER (WHERE k.code = 2110),0)::bigint, \
             coalesce(sum(b.natural_micro_dollars) FILTER (WHERE k.code = 2210),0)::bigint, \
             coalesce(sum(b.natural_micro_dollars) FILTER (WHERE k.code IN (2120,2220)),0)::bigint, \
             coalesce(sum(b.natural_micro_dollars) FILTER (WHERE k.code IN (2130,2230)),0)::bigint \
             FROM finance.account_balance b JOIN finance.account_key k USING(key_id) \
             LEFT JOIN finance.allocation a ON a.allocation_id = k.allocation_id \
             WHERE k.payer_account_id = $1 OR a.payer_account_id = $1", &[&&account.as_bytes()[..]],
        ).await.map_err(|_| PortError::Unavailable)?;
        let amount = |index| -> PortResult<u64> {
            let number: i64 = row.try_get(index).map_err(|_| PortError::Unavailable)?;
            u64::try_from(number).map_err(|_| PortError::Unavailable)
        };
        let consumed = service.client().query_one(
            "SELECT coalesce(sum(l.amount_micro_dollars),0)::bigint \
             FROM finance.journal_entry e JOIN finance.journal_line l USING(entry_id) \
             JOIN finance.account_key k USING(key_id) \
             LEFT JOIN finance.allocation a ON a.allocation_id = k.allocation_id \
             WHERE (k.payer_account_id = $1 OR a.payer_account_id = $1) \
             AND e.kind IN ('settlement','meter_charge') AND k.code IN (2110,2120,2130,2140,2210,2220,2230) \
             AND e.effective_at_ms >= (extract(epoch FROM date_trunc('month', current_timestamp AT TIME ZONE 'UTC')) * 1000)::bigint",
            &[&&account.as_bytes()[..]],
        ).await.map_err(|_| PortError::Unavailable)?;
        let consumed: i64 = consumed.try_get(0).map_err(|_| PortError::Unavailable)?;
        let trial = service.client().query_opt(
            "SELECT l.original_micro_dollars, l.original_micro_dollars - l.consumed_micro_dollars - l.expired_micro_dollars - l.forfeited_micro_dollars \
             FROM finance.trial_grant t JOIN finance.funding_lot l USING(lot_id) WHERE t.account_id = $1",
            &[&&account.as_bytes()[..]],
        ).await.map_err(|_| PortError::Unavailable)?;
        let trial = match trial {
            Some(row) => {
                let granted: i64 = row.try_get(0).map_err(|_| PortError::Unavailable)?;
                let remaining: i64 = row.try_get(1).map_err(|_| PortError::Unavailable)?;
                let granted = u64::try_from(granted).map_err(|_| PortError::Unavailable)?;
                let remaining = u64::try_from(remaining).map_err(|_| PortError::Unavailable)?;
                if remaining == 0 {
                    TrialView::Spent {
                        granted_micro_dollars: granted,
                    }
                } else {
                    TrialView::Active {
                        granted_micro_dollars: granted,
                        remaining_micro_dollars: remaining,
                    }
                }
            }
            None if !record.is_verified => TrialView::AwaitingVerification {
                amount_micro_dollars: df_commerce::funding::TrialPolicy::default()
                    .amount_micro_dollars,
            },
            None => TrialView::NotEligible,
        };
        Ok(OverviewView {
            balance: BalanceView {
                purchased_available_micro_dollars: amount(0)?,
                promotional_available_micro_dollars: amount(1)?,
                allocated_micro_dollars: amount(2)?,
                reserved_micro_dollars: amount(3)?,
                consumed_this_month_micro_dollars: u64::try_from(consumed)
                    .map_err(|_| PortError::Unavailable)?,
            },
            trial,
        })
    }

    async fn funding(
        &self,
        _account: AccountId,
        _targets: &[FundingTarget],
    ) -> PortResult<FundingView> {
        crate::ports::unavailable("F")
    }

    async fn set_pace(
        &self,
        _account: AccountId,
        _campaign: CampaignId,
        _preset: PacePreset,
        _custom_ceiling_cents_per_hour: Option<u32>,
        _operation: OperationId,
    ) -> PortResult<()> {
        crate::ports::unavailable("F")
    }

    async fn allocate(
        &self,
        _account: AccountId,
        _session: SessionId,
        _amount_cents: u32,
        _operation: OperationId,
    ) -> PortResult<()> {
        crate::ports::unavailable("F")
    }

    async fn statements(
        &self,
        _account: AccountId,
        _cursor: Option<&str>,
    ) -> PortResult<StatementsView> {
        crate::ports::unavailable("F")
    }

    async fn statements_filtered(
        &self,
        _account: AccountId,
        _filter: StatementFilter,
        _from_ms: Option<u64>,
        _to_ms: Option<u64>,
        _cursor: Option<&str>,
    ) -> PortResult<StatementsView> {
        crate::ports::unavailable("F")
    }

    async fn account_stats(&self, _account: AccountId, _now_ms: u64) -> PortResult<AccountStats> {
        crate::ports::unavailable("F")
    }

    async fn operator_account_totals(
        &self,
        _account: AccountId,
    ) -> PortResult<(u64, u64, Option<u64>)> {
        crate::ports::unavailable("F")
    }

    async fn closure_blockers(&self, _account: AccountId) -> PortResult<Vec<ClosureBlocker>> {
        crate::ports::unavailable("F")
    }

    async fn operator_summary(&self) -> PortResult<FinanceSummary> {
        crate::ports::unavailable("F")
    }

    async fn operator_balance(&self, _account: AccountId) -> PortResult<u64> {
        crate::ports::unavailable("F")
    }

    async fn operator_ledger(
        &self,
        _account: AccountId,
    ) -> PortResult<(Vec<LedgerEntry>, Vec<RefundableLot>)> {
        crate::ports::unavailable("F")
    }

    async fn refund(
        &self,
        _act: OperatorAct,
        _account: AccountId,
        _lot_id: &str,
        _amount_cents: u32,
    ) -> PortResult<ActionOutcome> {
        crate::ports::unavailable("F")
    }

    async fn goodwill_credit(
        &self,
        _act: OperatorAct,
        _account: AccountId,
        _amount_cents: u32,
        _support_case: &str,
    ) -> PortResult<ActionOutcome> {
        crate::ports::unavailable("F")
    }

    async fn disputes(&self) -> PortResult<Vec<DisputeRow>> {
        crate::ports::unavailable("F")
    }

    async fn dispute_evidence(
        &self,
        _act: OperatorAct,
        _dispute_id: &str,
    ) -> PortResult<ActionOutcome> {
        crate::ports::unavailable("F")
    }

    async fn reconciliation(&self) -> PortResult<ReconciliationView> {
        crate::ports::unavailable("F")
    }

    async fn providers(&self) -> PortResult<Vec<ProviderRow>> {
        crate::ports::unavailable("F")
    }

    async fn switch_provider(
        &self,
        _act: OperatorAct,
        _provider_id: &str,
        _switch: ProviderSwitch,
    ) -> PortResult<ActionOutcome> {
        crate::ports::unavailable("F")
    }
}

fn derive_email_key(key: &[u8], normalized_email: &str) -> PortResult<[u8; 32]> {
    let mut hmac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(key).map_err(|_| PortError::Unavailable)?;
    hmac.update(normalized_email.as_bytes());
    Ok(hmac.finalize().into_bytes().into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn email_grant_key_matches_hmac_sha256_vector_and_is_stable() {
        let key = [0x0b; 20];
        let result = super::derive_email_key(&key, "Hi There").unwrap();
        assert_eq!(
            df_commerce::finance_ids::hex(&result),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(result, super::derive_email_key(&key, "Hi There").unwrap());
        assert_ne!(
            result,
            super::derive_email_key(&[0x0c; 20], "Hi There").unwrap()
        );
    }
}
