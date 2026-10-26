//! Read-only host authority, published customer price and funding projections.
use super::{CONSENT_TERMS_VERSION, Error};
use crate::service::FinanceError;
use df_commerce::finance_ids::{CampaignKey, PayerId, PriceVersionId, SessionKey, TenantKey};
use tokio_postgres::{Client, GenericClient};

/// Customer-safe first-hour quote. No supplier cost, fee or markup is exposed.
#[derive(Clone, Copy, Debug)]
pub struct Quote {
    pub price_version: PriceVersionId,
    pub hourly_micro_dollars: u64,
    pub available_micro_dollars: u64,
    pub consented_at_ms: Option<i64>,
    pub cap_micro_dollars: u64,
    pub is_fundable: bool,
}

pub(super) struct Context {
    pub(super) campaign: CampaignKey,
    pub(super) tenant: TenantKey,
    pub(super) is_lobby: bool,
    pub(super) quote: Quote,
}

pub(super) fn invalid() -> Error {
    Error::Finance(FinanceError::Wire(crate::wire::WireError(
        "host funding record".into(),
    )))
}

pub(super) async fn context(
    client: &impl GenericClient,
    session: SessionKey,
    account: PayerId,
    now_ms: i64,
) -> Result<Context, Error> {
    let row = client
        .query_opt(
            "SELECT * FROM finance.host_funding_context($1,$2,$3)",
            &[
                &session.as_bytes().as_slice(),
                &account.as_bytes().as_slice(),
                &now_ms,
            ],
        )
        .await?
        .ok_or(Error::NotHost)?;
    if !row.try_get::<_, bool>("is_active")? || !row.try_get::<_, bool>("payer_active")? {
        return Err(Error::RestrictedAccount);
    }
    let id = |column| -> Result<[u8; 16], Error> {
        row.try_get::<_, Vec<u8>>(column)?
            .try_into()
            .map_err(|_| invalid())
    };
    let version: Option<Vec<u8>> = row.try_get("price_version_id")?;
    let price_version = PriceVersionId::from_array(
        version
            .ok_or(Error::PriceUnavailable)?
            .try_into()
            .map_err(|_| invalid())?,
    );
    let hourly: Option<i64> = row.try_get("hourly_micro_dollars")?;
    let hourly_micro_dollars = hourly
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(Error::PriceUnavailable)?;
    let available: i64 = row.try_get("available_micro_dollars")?;
    let available_micro_dollars = u64::try_from(available).map_err(|_| invalid())?;
    let balances = client.query_one(
        "SELECT coalesce(sum(b.natural_micro_dollars),0)::bigint,          EXISTS (SELECT 1 FROM finance.session_funding WHERE session_id=$1)          FROM finance.account_balance b JOIN finance.account_key k USING(key_id)          JOIN finance.allocation a ON a.allocation_id=k.allocation_id          JOIN finance.session_funding s ON s.session_id=a.session_id          WHERE a.session_id=$1 AND a.payer_account_id=$2 AND a.status='active'          AND s.status='open' AND s.price_version_id=$3 AND k.code IN (2120,2220)",
        &[&session.as_bytes().as_slice(), &account.as_bytes().as_slice(), &price_version.as_bytes().as_slice()])
        .await?;
    let allocated: i64 = balances.try_get(0)?;
    let is_opened: bool = balances.try_get(1)?;
    let drawable = if is_opened {
        u64::try_from(allocated).map_err(|_| invalid())?
    } else {
        available_micro_dollars
    };
    let consent = client
        .query_opt(
            "SELECT consented_at_ms, cap_micro_dollars FROM finance.host_session_consent \
         WHERE session_id=$1 AND account_id=$2 AND price_version_id=$3 \
         AND hourly_micro_dollars=$4 AND terms_version=$5 ORDER BY consented_at_ms DESC, operation_id DESC LIMIT 1",
            &[
                &session.as_bytes().as_slice(),
                &account.as_bytes().as_slice(),
                &price_version.as_bytes().as_slice(),
                &i64::try_from(hourly_micro_dollars).map_err(|_| invalid())?,
                &CONSENT_TERMS_VERSION,
            ],
        )
        .await?;
    let (consented_at_ms, cap_micro_dollars) = match consent {
        Some(row) => (
            Some(row.try_get(0)?),
            u64::try_from(row.try_get::<_, i64>(1)?).map_err(|_| invalid())?,
        ),
        None => (None, hourly_micro_dollars),
    };
    Ok(Context {
        campaign: CampaignKey::from_array(id("campaign_id")?),
        tenant: TenantKey::from_array(id("tenant_id")?),
        is_lobby: row.try_get::<_, &str>("phase")? == "lobby",
        quote: Quote {
            price_version,
            hourly_micro_dollars,
            available_micro_dollars,
            consented_at_ms,
            cap_micro_dollars,
            is_fundable: consented_at_ms.is_some() && drawable >= cap_micro_dollars,
        },
    })
}

/// Reads current host authority, consent, published price and spendable credit without allocating.
pub async fn quote(
    client: &Client,
    session: SessionKey,
    account: PayerId,
    now_ms: i64,
) -> Result<Quote, Error> {
    Ok(context(client, session, account, now_ms).await?.quote)
}

/// Resolves the current host from canonical membership; sponsor intentions are never read.
pub(super) async fn host(client: &Client, campaign: CampaignKey) -> Result<PayerId, Error> {
    let row = client
        .query_opt(
            "SELECT finance.host_funding_host($1)",
            &[&campaign.as_bytes().as_slice()],
        )
        .await?
        .ok_or(Error::NotHost)?;
    Ok(PayerId::from_array(
        row.try_get::<_, Option<Vec<u8>>>(0)?
            .ok_or(Error::NotHost)?
            .try_into()
            .map_err(|_| invalid())?,
    ))
}

/// Startup fundability checks consent and a full first hour before Start moves any credit.
pub async fn start_fundable(
    client: &Client,
    campaign: CampaignKey,
    session: SessionKey,
    now_ms: i64,
) -> Result<bool, Error> {
    let account = host(client, campaign).await?;
    let state = context(client, session, account, now_ms).await?;
    if state.campaign != campaign {
        return Err(Error::NotHost);
    }
    Ok(state.is_lobby && state.quote.is_fundable)
}
