//! Read-only recovery of the existing immutable admission linkage and opaque finance quote.
use super::draft_error;
use df_commerce::draft_image::{StageCall, StageRoute};
use df_commerce::finance_ids::{
    AttemptId, CampaignKey, EffectId, OperationKey, SessionKey, TenantKey,
};
use df_finance::draft_image::StoredStageQuote;
use df_finance::ports::{FencePort, MediaPort, MemberPort};
use df_finance::service::FinanceService;
use df_finance::{ImageAssetId, OwnerScope, ProviderStageBinding};
use df_imagery::execution::{DispatchId, FinanceTuple};
use df_imagery::runtime::RuntimeError;
use df_imagery::{ImageJob, ImageOwner, owner_scope_bytes};
use tokio_postgres::Row;

/// Recovered stored money identity; reading it never creates a quote, consent or attempt.
pub enum AdmittedFunding {
    Draft {
        account: df_types::AccountId,
        quote: Box<StoredStageQuote>,
    },
    Campaign(Box<ProviderStageBinding>),
}
impl AdmittedFunding {
    pub fn call(&self) -> &StageCall {
        match self {
            Self::Draft { quote, .. } => quote.call(),
            Self::Campaign(binding) => &binding.call,
        }
    }
    pub fn route(&self) -> &StageRoute {
        match self {
            Self::Draft { quote, .. } => quote.route(),
            Self::Campaign(binding) => &binding.route,
        }
    }
    pub fn finance(&self) -> Result<FinanceTuple, RuntimeError> {
        let tuple = match self {
            Self::Draft { quote, .. } => {
                let binding = quote.binding();
                (binding.attempt, binding.operation, binding.effect)
            }
            Self::Campaign(binding) => (
                *binding.attempt.as_bytes(),
                *binding.operation.as_bytes(),
                *binding.effect.as_bytes(),
            ),
        };
        FinanceTuple::new(tuple.0, tuple.1, tuple.2).map_err(|_| RuntimeError::InvalidRecord)
    }
}
fn bytes(row: &Row, column: &str) -> Result<[u8; 16], RuntimeError> {
    let bytes: Vec<u8> = row
        .try_get(column)
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let value: [u8; 16] = bytes.try_into().map_err(|_| RuntimeError::InvalidRecord)?;
    if value == [0; 16] {
        Err(RuntimeError::InvalidRecord)
    } else {
        Ok(value)
    }
}
fn text(row: &Row, column: &str) -> Result<String, RuntimeError> {
    row.try_get(column).map_err(|_| RuntimeError::InvalidRecord)
}

/// Loads the exact already-admitted tuple. No absent row is repaired and no ids are minted.
pub async fn read_admitted_funding<F: FencePort, M: MediaPort, S: MemberPort>(
    finance: &FinanceService<F, M, S>,
    job: &ImageJob,
    dispatch: DispatchId,
) -> Result<AdmittedFunding, RuntimeError> {
    let row = finance
        .client()
        .query_opt(
            "SELECT b.*,b.stage_call::text AS call_text,b.cost_route::text AS route_text \
         FROM imagery.image_finance_admission_binding b \
         JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=b.dispatch_id \
         AND d.job_id=b.job_id AND d.stage_id=b.execution_stage_id \
         WHERE b.owner_scope=$1 AND b.job_id=$2 AND b.dispatch_id=$3 \
         AND d.finance_attempt=b.finance_attempt AND d.finance_operation=b.operation_id \
         AND d.finance_effect=b.effect_id AND NOT d.local_accounting",
            &[
                &owner_scope_bytes(&job.owner),
                &job.job.as_bytes().as_slice(),
                &dispatch.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(|_| RuntimeError::StorageUnavailable)?
        .ok_or(RuntimeError::InvalidRecord)?;
    let call: StageCall =
        serde_json::from_str(&text(&row, "call_text")?).map_err(|_| RuntimeError::InvalidRecord)?;
    let route: StageRoute = serde_json::from_str(&text(&row, "route_text")?)
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let expected = FinanceTuple::new(
        bytes(&row, "finance_attempt")?,
        bytes(&row, "operation_id")?,
        bytes(&row, "effect_id")?,
    )
    .map_err(|_| RuntimeError::InvalidRecord)?;
    let planned_publication = match (
        row.get::<_, Option<Vec<u8>>>("planned_asset"),
        row.get::<_, Option<Vec<u8>>>("planned_publication"),
    ) {
        (None, None) => None,
        (Some(asset), Some(publication)) => Some(df_commerce::draft_image::StagePublication {
            asset: df_commerce::draft_image::AssetId(
                asset.try_into().map_err(|_| RuntimeError::InvalidRecord)?,
            ),
            publication: df_commerce::draft_image::PublicationId(
                publication
                    .try_into()
                    .map_err(|_| RuntimeError::InvalidRecord)?,
            ),
        }),
        _ => return Err(RuntimeError::InvalidRecord),
    };
    if call.job.0 != *job.job.as_bytes()
        || call.dispatch != *dispatch.as_bytes()
        || call.stage.0 != bytes(&row, "execution_stage_id")?
        || route.key.as_slice() != row.get::<_, Vec<u8>>("route_key")
        || call.planned_publication != planned_publication
        || call.kind.is_standalone() != planned_publication.is_none()
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let account: Option<Vec<u8>> = row.get("account_id");
    let result = match (&job.owner, account) {
        (ImageOwner::Draft { account }, Some(stored))
            if account.as_bytes().as_slice() == stored =>
        {
            let quote = finance
                .read_draft_stage_quote(*account, &call)
                .await
                .map_err(draft_error)?
                .ok_or(RuntimeError::InvalidRecord)?;
            if quote.route() != &route {
                return Err(RuntimeError::InvalidRecord);
            }
            AdmittedFunding::Draft {
                account: *account,
                quote: Box::new(quote),
            }
        }
        (ImageOwner::Run { campaign, .. }, None)
            if campaign.as_bytes() == &bytes(&row, "campaign_id")? =>
        {
            let asset: Option<Vec<u8>> = row.get("asset_id");
            let media_asset = asset
                .map(|value| {
                    value
                        .try_into()
                        .map_err(|_| RuntimeError::InvalidRecord)
                        .and_then(|value| {
                            ImageAssetId::new(value).map_err(|_| RuntimeError::InvalidRecord)
                        })
                })
                .transpose()?;
            AdmittedFunding::Campaign(Box::new(ProviderStageBinding {
                owner_scope: OwnerScope::new(owner_scope_bytes(&job.owner))
                    .map_err(|_| RuntimeError::InvalidRecord)?,
                call,
                route_key: route.key,
                route,
                tenant: TenantKey::from_array(bytes(&row, "tenant_id")?),
                campaign: CampaignKey::from_array(bytes(&row, "campaign_id")?),
                session: SessionKey::from_array(bytes(&row, "session_id")?),
                operation: OperationKey::from_array(expected.operation),
                attempt: AttemptId::from_array(expected.attempt),
                effect: EffectId::from_array(expected.effect),
                media_asset,
            }))
        }
        _ => return Err(RuntimeError::InvalidRecord),
    };
    if result.finance()? != expected {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(result)
}
