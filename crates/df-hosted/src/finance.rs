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

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
enum CustomerPace {
    Story,
    Immersive,
    Cinematic,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomerPrice {
    name: CustomerPace,
    micro_dollars_per_hour: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomerPrices {
    price_version_id: String,
    effective_date: String,
    prices: [CustomerPrice; 3],
}

impl CustomerPrices {
    fn parse(bytes: &[u8]) -> PortResult<Self> {
        let prices: Self = serde_json::from_slice(bytes).map_err(|_| PortError::Unavailable)?;
        let date = prices.effective_date.as_bytes();
        if date.len() != 10
            || date.iter().enumerate().any(|(index, byte)| {
                if index == 4 || index == 7 {
                    *byte != b'-'
                } else {
                    !byte.is_ascii_digit()
                }
            })
        {
            return Err(PortError::Unavailable);
        }
        for name in [
            CustomerPace::Story,
            CustomerPace::Immersive,
            CustomerPace::Cinematic,
        ] {
            if prices
                .prices
                .iter()
                .filter(|price| price.name == name)
                .count()
                != 1
            {
                return Err(PortError::Unavailable);
            }
        }
        Ok(prices)
    }

    fn amount(&self, name: CustomerPace) -> PortResult<u64> {
        self.prices
            .iter()
            .find(|price| price.name == name)
            .map(|price| price.micro_dollars_per_hour)
            .ok_or(PortError::Unavailable)
    }
}

pub(crate) async fn publish_customer_prices(client: &mut tokio_postgres::Client) -> PortResult<()> {
    let bytes = tokio::task::spawn_blocking(|| std::fs::read("content/customer-prices.json"))
        .await
        .map_err(|_| PortError::Unavailable)?
        .map_err(|_| PortError::Unavailable)?;
    let config = CustomerPrices::parse(&bytes)?;
    let version_id = df_commerce::finance_ids::PriceVersionId::from_hex(&config.price_version_id)
        .ok_or(PortError::Unavailable)?;
    let effective: i64 = client.query_one(
        "SELECT (extract(epoch FROM ($1::text::date::timestamp AT TIME ZONE 'UTC')) * 1000)::bigint",
        &[&config.effective_date],
    ).await.map_err(|_| PortError::Unavailable)?.try_get(0).map_err(|_| PortError::Unavailable)?;
    let version = df_commerce::price_schedule::PriceVersion {
        id: version_id,
        label: format!("customer-{}", config.effective_date),
        currency: CurrencyTag::USD,
        platform_rate_micro_dollars_per_hour: 0,
        markup_numerator: 1,
        markup_denominator: 1,
        tariff: Vec::new(),
        proposed_pilot: true,
    };
    df_finance::host_funding::publish_host_price_version(
        client,
        &version,
        df_finance::host_funding::HostPrices {
            story_micro_dollars: config.amount(CustomerPace::Story)?,
            immersive_micro_dollars: config.amount(CustomerPace::Immersive)?,
            cinematic_micro_dollars: config.amount(CustomerPace::Cinematic)?,
        },
        effective,
    )
    .await
    .map_err(host_funding_error)?;
    df_observe::record_audit_line(
        "hosted.customer_prices_published",
        format!(
            "SPEND customer_prices_published version={} effective_date={}",
            config.price_version_id, config.effective_date,
        ),
    );
    Ok(())
}

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
        self.purchase_from(account, cents, "development", reference)
            .await
    }

    pub(crate) async fn stripe_purchase(
        &self,
        account: AccountId,
        cents: u32,
        reference: &str,
    ) -> PortResult<()> {
        self.purchase_from(account, cents, "stripe", reference)
            .await
    }

    pub(crate) async fn paypal_purchase(
        &self,
        account: AccountId,
        cents: u32,
        reference: &str,
    ) -> PortResult<()> {
        self.purchase_from(account, cents, "paypal", reference)
            .await
    }

    async fn purchase_from(
        &self,
        account: AccountId,
        cents: u32,
        processor: &str,
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
        let posting = self
            .inner
            .service
            .lock()
            .await
            .apply_purchase(df_finance::service::PurchaseCommand {
                payer: PayerId::from_array(*account.as_bytes()),
                processor,
                payment_ref: reference,
                currency: CurrencyTag::USD,
                amount_micro_dollars: u64::from(cents) * 10_000,
            })
            .await
            .map_err(finance_error)?;
        let outcome = if posting.is_some() {
            "purchase"
        } else {
            "purchase_replayed"
        };
        df_observe::record_audit_line(
            "hosted.purchase_posted",
            format!(
                "SPEND {outcome} account={} processor={processor} operation={reference} amount_micro_dollars={}",
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

fn cancel_error(error: df_campaign::errors::StoreError) -> PortError {
    match error {
        df_campaign::errors::StoreError::InvalidRecord => PortError::Forbidden,
        df_campaign::errors::StoreError::Unknown => PortError::UnknownOutcome,
        _ => PortError::Unavailable,
    }
}

fn host_funding_error(error: df_finance::host_funding::Error) -> PortError {
    use df_finance::host_funding::Error;
    let (reason, refusal) = match error {
        Error::Finance(error) => return finance_error(error),
        Error::NotHost => ("not_host", PortError::NotFound),
        Error::RestrictedAccount => ("restricted_account", PortError::Forbidden),
        Error::PriceUnavailable => ("price_unavailable", PortError::Unavailable),
        Error::StalePriceVersion => (
            "stale_price",
            PortError::Because(
                df_account_app::wire::ErrorCode::StaleForm,
                df_account_app::wire_campaign::Reason::StaleRevision,
            ),
        ),
        Error::InvalidConsent | Error::ConsentRequired => (
            "consent_required",
            PortError::Refused(df_account_app::wire::Refusal::ConsentRequired),
        ),
        Error::InsufficientCredit => (
            "insufficient_credit",
            PortError::Refused(df_account_app::wire::Refusal::AmountOutOfRange),
        ),
        Error::NotInLobby | Error::Closed => ("session_closed", PortError::Forbidden),
    };
    df_observe::record_audit_line(
        "hosted.host_funding_refused",
        format!("SPEND host_funding_refused reason={reason}"),
    );
    refusal
}

impl FinancePort for Finance {
    async fn cancel_host_lobby(&self, account: AccountId, session: SessionId) -> PortResult<()> {
        let mut service = self.inner.service.lock().await;
        crate::campaign_sessions::Sessions::cancel_before_start(service.client(), account, session)
            .await
            .map_err(cancel_error)?;
        service
            .close_host_session(df_commerce::finance_ids::SessionKey::from_array(
                *session.as_bytes(),
            ))
            .await
            .map_err(host_funding_error)
    }

    async fn host_credit(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> PortResult<Option<df_account_app::wire_campaign::HostCreditView>> {
        let service = self.inner.service.lock().await;
        let now = i64::try_from(df_customer::Clock::now_ms(&df_customer::SystemClock))
            .map_err(|_| PortError::Unavailable)?;
        match df_finance::host_funding::quote(
            service.client(),
            df_commerce::finance_ids::SessionKey::from_array(*session.as_bytes()),
            PayerId::from_array(*account.as_bytes()),
            now,
        )
        .await
        {
            Ok(quote) => Ok(Some(df_account_app::wire_campaign::HostCreditView {
                price_version: *quote.price_version.as_bytes(),
                hourly_micro_dollars: quote.hourly_micro_dollars,
                available_micro_dollars: quote.available_micro_dollars,
                cap_micro_dollars: quote.cap_micro_dollars,
                terms_version: df_finance::host_funding::CONSENT_TERMS_VERSION,
                consented_at_ms: quote.consented_at_ms,
                is_fundable: quote.is_fundable,
            })),
            Err(df_finance::host_funding::Error::NotHost) => Ok(None),
            Err(df_finance::host_funding::Error::PriceUnavailable) => {
                df_observe::record_audit_line(
                    "hosted.host_price_unavailable",
                    format!(
                        "SPEND host_price_unavailable session={}",
                        df_commerce::finance_ids::hex(session.as_bytes())
                    ),
                );
                Ok(None)
            }
            Err(error) => Err(host_funding_error(error)),
        }
    }

    async fn consent_host_credit(
        &self,
        account: AccountId,
        session: SessionId,
        request: df_account_app::wire_campaign::HostConsentRequest,
        operation: OperationId,
    ) -> PortResult<()> {
        self.inner
            .service
            .lock()
            .await
            .record_host_consent(df_finance::host_funding::ConsentCommand {
                session: df_commerce::finance_ids::SessionKey::from_array(*session.as_bytes()),
                account: PayerId::from_array(*account.as_bytes()),
                operation: df_commerce::finance_ids::OperationKey::from_array(
                    *operation.as_bytes(),
                ),
                price_version: df_commerce::finance_ids::PriceVersionId::from_array(
                    request.price_version,
                ),
                cap_micro_dollars: request.cap_micro_dollars,
                terms_version: request.terms_version,
            })
            .await
            .map_err(host_funding_error)
    }

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
    fn published_prices(page: &str) -> Vec<(super::CustomerPace, u64)> {
        page.split("class=\"pace-item\"")
            .skip(1)
            .map(|block| {
                let block = block.split("</div>").next().unwrap();
                let field = |class: &str| {
                    block
                        .split(&format!("class=\"{class}\">"))
                        .nth(1)
                        .unwrap()
                        .split("</p>")
                        .next()
                        .unwrap()
                };
                let name = serde_json::from_value(serde_json::json!(field("pace-name"))).unwrap();
                let dollars: u64 = field("sum-big").strip_prefix('$').unwrap().parse().unwrap();
                assert_eq!(field("pace-per"), "per table-hour, at most");
                (name, dollars.checked_mul(1_000_000).unwrap())
            })
            .collect()
    }

    #[test]
    fn fundingwire_cancel_preserves_unknown_durable_outcomes() {
        use df_campaign::errors::StoreError;
        use df_portal::app_api::ports::PortError;
        assert!(matches!(
            super::cancel_error(StoreError::Unknown),
            PortError::UnknownOutcome
        ));
        assert!(matches!(
            super::cancel_error(StoreError::Unavailable),
            PortError::Unavailable
        ));
        assert!(matches!(
            super::cancel_error(StoreError::InvalidRecord),
            PortError::Forbidden
        ));
    }

    #[test]
    fn fundingwire_customer_config_matches_the_three_published_pace_blocks() {
        let config =
            super::CustomerPrices::parse(include_bytes!("../../../content/customer-prices.json"))
                .unwrap();
        let page = include_str!("../../../docs/pricing/index.html");
        let published = published_prices(page);
        assert_eq!(published.len(), 3);
        for price in &config.prices {
            assert_eq!(
                published
                    .iter()
                    .filter(|(name, _)| *name == price.name)
                    .count(),
                1
            );
            assert_eq!(
                published
                    .iter()
                    .find(|(name, _)| *name == price.name)
                    .unwrap()
                    .1,
                price.micro_dollars_per_hour
            );
        }
    }

    #[test]
    fn fundingwire_customer_config_rejects_duplicate_missing_and_unknown_prices() {
        let bytes = include_str!("../../../content/customer-prices.json");
        assert!(
            super::CustomerPrices::parse(bytes.replace("Cinematic", "Story").as_bytes()).is_err()
        );
        assert!(
            super::CustomerPrices::parse(bytes.replace("Cinematic", "Custom").as_bytes()).is_err()
        );
    }

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
