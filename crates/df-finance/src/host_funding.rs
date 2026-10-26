//! Host-only pilot consent (campaigns-and-table-research 8.1/8.4). Consent moves no money;
//! Start opens and allocates atomically through the existing finance posting boundary.

use df_commerce::{
    finance_ids::{
        CampaignKey, EntryId, EventId, OperationKey, PayerId, PriceVersionId, SessionKey,
    },
    funding_flows::{
        AllocateRequest, Consent, OpenSession, SessionPolicy, WithdrawRequest, plan_allocation,
        plan_open_session, plan_withdrawal,
    },
    ledger::CurrencyTag,
    plan::FinancePlan,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_postgres::{Client, Transaction};

use crate::{
    service::FinanceError,
    wire::{event_json, parse_allocate_snapshot, parse_withdrawal_snapshot},
};

mod reads;
pub use reads::{Quote, quote, start_fundable};
use reads::{context, host, invalid};

/// Exact text version: "Use my credit for this session, up to $X" (USD).
pub const CONSENT_TERMS_VERSION: i32 = 1;

#[derive(Debug)]
pub enum Error {
    NotHost,
    RestrictedAccount,
    NotInLobby,
    PriceUnavailable,
    StalePriceVersion,
    ConsentRequired,
    InvalidConsent,
    InsufficientCredit,
    Closed,
    Finance(FinanceError),
}

impl From<tokio_postgres::Error> for Error {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Finance(error.into())
    }
}

/// Customer preset ceilings loaded from the owner's pricing configuration, in USD micro-dollars.
#[derive(Clone, Copy, Debug)]
pub struct HostPrices {
    pub story_micro_dollars: u64,
    pub immersive_micro_dollars: u64,
    pub cinematic_micro_dollars: u64,
}

/// The authenticated host approves only their own credit, at the displayed version and cap.
#[derive(Clone, Copy, Debug)]
pub struct ConsentCommand {
    pub session: SessionKey,
    pub account: PayerId,
    pub operation: OperationKey,
    pub price_version: PriceVersionId,
    pub cap_micro_dollars: u64,
    pub terms_version: i32,
}

async fn lock(tx: &Transaction<'_>, session: SessionKey, account: PayerId) -> Result<(), Error> {
    tx.execute(
        "SELECT finance.lock_host_funding($1,$2)",
        &[
            &session.as_bytes().as_slice(),
            &account.as_bytes().as_slice(),
        ],
    )
    .await?;
    Ok(())
}

/// Persists dated, versioned consent. Replay preserves its original date and moves no credit.
pub async fn record_consent(
    client: &mut Client,
    command: ConsentCommand,
    now_ms: i64,
) -> Result<(), Error> {
    let tx = client.transaction().await?;
    lock(&tx, command.session, command.account).await?;
    let state = context(&tx, command.session, command.account, now_ms).await?;
    if !state.is_lobby {
        return Err(Error::NotInLobby);
    }
    if command.price_version != state.quote.price_version {
        return Err(Error::StalePriceVersion);
    }
    if command.terms_version != CONSENT_TERMS_VERSION
        || command.cap_micro_dollars < state.quote.hourly_micro_dollars
    {
        return Err(Error::InvalidConsent);
    }
    let cap = i64::try_from(command.cap_micro_dollars).map_err(|_| Error::InvalidConsent)?;
    let recorded = tx
        .execute(
            "SELECT finance.record_host_consent($1,$2,$3,$4,$5,$6,$7)",
            &[
                &command.session.as_bytes().as_slice(),
                &command.account.as_bytes().as_slice(),
                &command.operation.as_bytes().as_slice(),
                &command.price_version.as_bytes().as_slice(),
                &cap,
                &command.terms_version,
                &now_ms,
            ],
        )
        .await;
    if let Err(error) = recorded {
        if error.code().is_some_and(|code| code.code() == "FH002") {
            return Err(Error::InvalidConsent);
        }
        return Err(error.into());
    }
    tx.commit().await?;
    Ok(())
}

fn stable_id(session: SessionKey, namespace: &[u8]) -> [u8; 16] {
    let mut digest = Sha256::new();
    digest.update(b"df.host-funding.v1");
    digest.update(namespace);
    digest.update(session.as_bytes());
    let mut id = [0; 16];
    id.copy_from_slice(&digest.finalize()[..16]);
    id
}

async fn post(tx: &Transaction<'_>, plan: &FinancePlan) -> Result<(), Error> {
    tx.query_one(
        "SELECT finance.commit_event($1::text::jsonb)",
        &[&event_json(plan).to_string()],
    )
    .await?;
    Ok(())
}

async fn snapshot(tx: &Transaction<'_>, flow: &str, args: Value) -> Result<Value, Error> {
    let row = tx
        .query_one(
            "SELECT finance.lock_flow($1,$2::text::jsonb)::text",
            &[&flow, &args.to_string()],
        )
        .await?;
    serde_json::from_str(&row.try_get::<_, String>(0)?).map_err(|_| invalid())
}

/// Atomically pins the price and allocates the consent cap once. Any refusal rolls both back.
pub async fn open(
    client: &mut Client,
    campaign: CampaignKey,
    session: SessionKey,
    now_ms: i64,
) -> Result<(), Error> {
    let account = host(client, campaign).await?;
    let tx = client.transaction().await?;
    lock(&tx, session, account).await?;
    let closed: bool = tx
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM finance.host_session_close WHERE session_id=$1)",
            &[&session.as_bytes().as_slice()],
        )
        .await?
        .try_get(0)?;
    if closed {
        return Err(Error::Closed);
    }
    let state = context(&tx, session, account, now_ms).await?;
    if state.campaign != campaign {
        return Err(Error::NotHost);
    }
    let existing = tx
        .query_opt(
            "SELECT status FROM finance.session_funding WHERE session_id=$1",
            &[&session.as_bytes().as_slice()],
        )
        .await?;
    if let Some(row) = existing {
        if row.try_get::<_, &str>(0)? != "open" {
            return Err(Error::Closed);
        }
        let allocated: bool = tx
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM finance.allocation \
            WHERE session_id=$1 AND payer_account_id=$2 AND status='active')",
                &[
                    &session.as_bytes().as_slice(),
                    &account.as_bytes().as_slice(),
                ],
            )
            .await?
            .try_get(0)?;
        if allocated {
            tx.commit().await?;
            return Ok(());
        }
        return Err(Error::InvalidConsent);
    }
    if !state.is_lobby {
        return Err(Error::NotInLobby);
    }
    let quote = state.quote;
    if quote.consented_at_ms.is_none() {
        let previous: bool = tx.query_one("SELECT EXISTS (SELECT 1 FROM finance.host_session_consent WHERE session_id=$1 AND account_id=$2)",
            &[&session.as_bytes().as_slice(), &account.as_bytes().as_slice()]).await?.try_get(0)?;
        return Err(if previous {
            Error::StalePriceVersion
        } else {
            Error::ConsentRequired
        });
    }
    if !quote.is_fundable {
        return Err(Error::InsufficientCredit);
    }
    post(
        &tx,
        &plan_open_session(OpenSession {
            event_id: EventId::from_array(stable_id(session, b"open")),
            session,
            tenant: state.tenant,
            campaign,
            price_version: quote.price_version,
            currency: CurrencyTag::USD,
            policy: SessionPolicy {
                hourly_ceiling_micro_dollars: quote.hourly_micro_dollars,
                session_ceiling_micro_dollars: Some(quote.cap_micro_dollars),
                per_request_threshold_micro_dollars: quote.cap_micro_dollars,
            },
            now_ms,
        }),
    )
    .await?;
    let mut raw = snapshot(
        &tx,
        "allocate",
        json!({"payer":account.hex(),"session":session.hex()}),
    )
    .await?;
    let lots = raw
        .get_mut("lots")
        .and_then(Value::as_array_mut)
        .ok_or_else(invalid)?;
    for entry in lots {
        let lot = entry.get("lot").ok_or_else(invalid)?;
        let expired = lot
            .get("expires_at_ms")
            .and_then(Value::as_i64)
            .is_some_and(|end| end <= now_ms);
        let wrong_currency = lot.get("currency").and_then(Value::as_str) != Some("USD");
        if expired || wrong_currency {
            entry["available"] = json!(0);
        }
    }
    let allocation = parse_allocate_snapshot(&raw)
        .map_err(FinanceError::from)
        .map_err(Error::Finance)?;
    let plan = plan_allocation(
        &allocation,
        &AllocateRequest {
            event_id: EventId::from_array(stable_id(session, b"allocate")),
            entry_id: EntryId::from_array(stable_id(session, b"allocation-entry")),
            new_allocation: df_commerce::finance_ids::AllocationId::from_array(stable_id(
                session,
                b"allocation",
            )),
            operation: OperationKey::from_array(stable_id(session, b"allocation-operation")),
            payer: account,
            amount_micro_dollars: quote.cap_micro_dollars,
            consent: Consent {
                approved_hourly_ceiling_micro_dollars: quote.hourly_micro_dollars,
                approved_session_ceiling_micro_dollars: Some(quote.cap_micro_dollars),
                approval_threshold_micro_dollars: quote.cap_micro_dollars,
                terms_version: i64::from(CONSENT_TERMS_VERSION),
            },
            now_ms,
        },
    )
    .map_err(|error| Error::Finance(FinanceError::Funding(error)))?;
    post(&tx, &plan).await?;
    tx.commit().await?;
    Ok(())
}

/// Returns only unreserved credit on cancellation/end. Unknown paid work remains reserved.
pub async fn close(client: &mut Client, session: SessionKey, now_ms: i64) -> Result<(), Error> {
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT finance.lock_host_session($1)",
        &[&session.as_bytes().as_slice()],
    )
    .await?;
    let allocation = stable_id(session, b"allocation");
    let payers = tx.query_opt(
        "SELECT payer_account_id FROM finance.allocation WHERE session_id=$1 AND allocation_id=$2",
        &[&session.as_bytes().as_slice(), &allocation.as_slice()]).await?;
    if let Some(row) = payers {
        let payer = PayerId::from_array(
            row.try_get::<_, Vec<u8>>(0)?
                .try_into()
                .map_err(|_| invalid())?,
        );
        lock(&tx, session, payer).await?;
        let operation = OperationKey::from_array(stable_id(session, b"close-operation"));
        let raw = snapshot(
            &tx,
            "withdrawal",
            json!({"payer":payer.hex(),"session":session.hex(),
            "operation":operation.hex()}),
        )
        .await?;
        let state = parse_withdrawal_snapshot(&raw)
            .map_err(FinanceError::from)
            .map_err(Error::Finance)?;
        if state.lots.iter().any(|lot| lot.allocated_micro_dollars > 0)
            && let Some(plan) = plan_withdrawal(
                &state,
                &WithdrawRequest {
                    event_id: EventId::from_array(stable_id(session, b"close")),
                    entry_id: EntryId::from_array(stable_id(session, b"close-entry")),
                    operation,
                    now_ms,
                },
            )
            .map_err(|error| Error::Finance(FinanceError::Withdrawal(error)))?
        {
            post(&tx, &plan).await?;
        }
    }
    tx.execute(
        "SELECT finance.close_host_funding($1,$2)",
        &[&session.as_bytes().as_slice(), &now_ms],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Publishes a customer price version with a fixed effective date and replay identity.
/// Startup retries reproduce the same posting payload; changing an existing version is refused.
pub async fn publish_host_price_version(
    client: &mut Client,
    version: &df_commerce::price_schedule::PriceVersion,
    prices: HostPrices,
    effective_from_ms: i64,
) -> Result<(), Error> {
    if !version.tariff.is_empty() {
        return Err(Error::PriceUnavailable);
    }
    let plan = host_price_plan(version, prices, effective_from_ms)?;
    let expected = event_json(&plan)
        .pointer("/changes/0/row")
        .ok_or_else(invalid)?
        .to_string();
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended($1,31))",
        &[&version.id.hex()],
    )
    .await?;
    let existing = tx.query_opt(
        "SELECT to_jsonb(v) @> $2::text::jsonb AND NOT EXISTS (SELECT 1 FROM finance.price_version_tariff t WHERE t.price_version_id=v.price_version_id) FROM finance.price_version v WHERE v.price_version_id=$1",
        &[&version.id.as_bytes().as_slice(), &expected],
    ).await?;
    if let Some(row) = existing {
        if !row.try_get::<_, bool>(0)? {
            return Err(Error::StalePriceVersion);
        }
    } else {
        post(&tx, &plan).await?;
    }
    tx.commit().await?;
    Ok(())
}

fn host_price_plan(
    version: &df_commerce::price_schedule::PriceVersion,
    prices: HostPrices,
    effective_from_ms: i64,
) -> Result<FinancePlan, Error> {
    for amount in [
        prices.story_micro_dollars,
        prices.immersive_micro_dollars,
        prices.cinematic_micro_dollars,
    ] {
        if amount == 0 || i64::try_from(amount).is_err() {
            return Err(Error::PriceUnavailable);
        }
    }
    let mut digest = Sha256::new();
    digest.update(b"df.customer-price-publication.v1");
    digest.update(version.id.as_bytes());
    let mut event = [0; 16];
    event.copy_from_slice(&digest.finalize()[..16]);
    let mut plan = df_commerce::funding_flows::plan_publish_price_version(
        EventId::from_array(event),
        version,
        effective_from_ms,
    );
    let change = plan
        .changes
        .iter_mut()
        .find(|change| change.kind == df_commerce::plan::ChangeKind::PriceVersionInsert)
        .ok_or_else(invalid)?;
    change.row.0.insert(
        "presets",
        df_commerce::plan::Value::Json(
            json!({
                "story":{"hourly_ceiling_micro_dollars":prices.story_micro_dollars},
                "immersive":{"hourly_ceiling_micro_dollars":prices.immersive_micro_dollars},
                "cinematic":{"hourly_ceiling_micro_dollars":prices.cinematic_micro_dollars},
            })
            .to_string(),
        ),
    );
    Ok(plan)
}

impl<F: crate::ports::FencePort, M: crate::ports::MediaPort, S: crate::ports::MemberPort>
    crate::service::FinanceService<F, M, S>
{
    /// Publishes customer preset prices with the immutable version, through finance's posting API.
    pub async fn publish_host_price_version(
        &mut self,
        version: &df_commerce::price_schedule::PriceVersion,
        prices: HostPrices,
    ) -> Result<crate::service::Committed, Error> {
        let mut plan = host_price_plan(version, prices, self.clock.now_ms())?;
        plan.event_id = EventId::from_array(self.id());
        self.record_event(&plan).await.map_err(Error::Finance)
    }

    /// Releases a durably ended/cancelled session; unresolved provider holds remain protected.
    pub async fn close_host_session(&mut self, session: SessionKey) -> Result<(), Error> {
        close(&mut self.client, session, self.clock.now_ms()).await
    }

    /// Records the authenticated host's consent through this service's owned connection and clock.
    pub async fn record_host_consent(&mut self, command: ConsentCommand) -> Result<(), Error> {
        record_consent(&mut self.client, command, self.clock.now_ms()).await
    }
}
