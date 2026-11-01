//! Canonical paid-call descriptor and the existing catalog/execution admission hook.
use super::{
    DraftFunding, FundedImageAdmission, RuntimeFunding, RuntimeImageAuthority, draft_error,
    finance_error, stage_error,
};
use crate::imagery_draft_admission::{
    CanonicalStagePort, Config, StageAllocation, StageAuthority, StageConfig, StagePayer,
};
use df_assets::{
    Admission, AssetId, AssetKind, AttemptId, Audience, DedupeScope, Provenance, QuotaScope,
    RetentionClass,
};
use df_commerce::draft_image::{JobId, PaidStageKind, StageCall, StageId, StageRoute};
use df_finance::draft_image::{Error, StageQuoteCommand};
use df_finance::ports::{FencePort, MediaPort, MemberPort};
use df_imagery::execution::{
    DispatchId, DispatchMode, ExecutionScope, StageClaim, StageKind, StageScope,
};
use df_imagery::runtime::{
    DispatchAdmission, ImageDispatchAdmission, RuntimeError, SourceReceipt, retained_audience,
    retained_config,
};
use df_imagery::{AccountingMode, ImageOwner, owner_scope_bytes};
use tokio_postgres::{Client, Transaction};

pub(super) fn validate(input: &DispatchAdmission<'_>) -> Result<bool, RuntimeError> {
    if retained_config(input.job)? != *input.config
        || input.stage.owner != input.job.owner
        || input.stage.job != input.job.job
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let local = input.config.accounting.mode == AccountingMode::RecordOnly
        || input.stage.kind == StageKind::LocalTransform;
    if local != input.stage.local_accounting || (!local && input.quote.is_none()) {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(local)
}
pub(super) fn claim(input: &DispatchAdmission<'_>) -> StageClaim {
    StageClaim {
        scope: StageScope {
            owner: input.job.owner.clone(),
            job: input.job.job,
            stage: input.stage.stage,
        },
        token: input.claim,
        generation: input.stage.claim_generation,
    }
}
pub(super) fn scope(owner: &ImageOwner, call: &StageCall) -> Result<ExecutionScope, Error> {
    Ok(ExecutionScope {
        owner: owner.clone(),
        job: df_imagery::ImageJobId::from_bytes(&call.job.0).map_err(|_| Error::InvalidRecord)?,
        stage: df_imagery::execution::ExecutionStageId::from_bytes(&call.stage.0)
            .map_err(|_| Error::InvalidRecord)?,
        dispatch: DispatchId::from_bytes(&call.dispatch).map_err(|_| Error::InvalidRecord)?,
    })
}
fn kind(kind: StageKind) -> Result<PaidStageKind, RuntimeError> {
    Ok(match kind {
        StageKind::Classification => PaidStageKind::Classification,
        StageKind::Generation => PaidStageKind::Generation,
        StageKind::Edit => PaidStageKind::Edit,
        StageKind::Inspection => PaidStageKind::Inspection,
        StageKind::Extraction => PaidStageKind::Extraction,
        StageKind::Correction => PaidStageKind::Correction,
        StageKind::PhotoInspection => PaidStageKind::PhotoInspection,
        StageKind::HeadLocations => PaidStageKind::HeadLocations,
        StageKind::Segmentation => PaidStageKind::Segmentation,
        StageKind::StagingIdentity => PaidStageKind::StagingIdentity,
        StageKind::StagingFacing => PaidStageKind::StagingFacing,
        StageKind::LocalTransform => return Err(RuntimeError::InvalidRecord),
    })
}
fn fixed<const N: usize>(bytes: Vec<u8>) -> Result<[u8; N], RuntimeError> {
    bytes.try_into().map_err(|_| RuntimeError::InvalidRecord)
}

pub(super) async fn call(
    client: &Client,
    input: &DispatchAdmission<'_>,
) -> Result<StageCall, RuntimeError> {
    let row = client
        .query_opt(
            "SELECT stage_kind,input_revision,input_digest,config_revision,config_digest,parent_dispatch \
             FROM imagery.image_execution_stage WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3",
            &[
                &owner_scope_bytes(&input.job.owner),
                &input.job.job.as_bytes().as_slice(),
                &input.stage.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(|_| RuntimeError::StorageUnavailable)?
        .ok_or(RuntimeError::InvalidRecord)?;
    let input_digest = fixed(row.get("input_digest"))?;
    let config_digest = fixed(row.get("config_digest"))?;
    if row.get::<_, String>("stage_kind") != input.stage.kind.label()
        || config_digest != df_commerce::digest::sha256(input.job.config.bytes())
    {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(StageCall {
        job: JobId(*input.job.job.as_bytes()),
        stage: StageId(*input.stage.stage.as_bytes()),
        dispatch: *input.dispatch.as_bytes(),
        parent_dispatch: row
            .get::<_, Option<Vec<u8>>>("parent_dispatch")
            .map(fixed)
            .transpose()?,
        kind: kind(input.stage.kind)?,
        input_revision: u32::try_from(row.get::<_, i64>("input_revision"))
            .map_err(|_| RuntimeError::InvalidRecord)?,
        input_digest,
        config_revision: u64::try_from(row.get::<_, i64>("config_revision"))
            .map_err(|_| RuntimeError::InvalidRecord)?,
        config_digest,
        planned_publication: input.stage.planned_publication.map(|publication| {
            df_commerce::draft_image::StagePublication {
                asset: df_commerce::draft_image::AssetId(publication.asset),
                publication: df_commerce::draft_image::PublicationId(publication.attempt),
            }
        }),
        // This contract anchors immutable canonical stage input, not a private provider HTTP body.
        payload_digest: input_digest,
    })
}
pub(super) fn route_matches(route: &StageRoute, cost: &df_imagery::cost::CostRoute) -> bool {
    if route.provider != cost.provider
        || route.model != cost.model
        || route.cause != cost.cause
        || route.detail != cost.detail
        || route.maximum_units.len() != cost.maxima.len()
        || route.maximum_units.is_empty()
        || route
            .maximum_units
            .windows(2)
            .any(|pair| pair[0].0 >= pair[1].0)
    {
        return false;
    }
    let mut actual = cost.maxima.iter().collect::<Vec<_>>();
    actual.sort_unstable_by(|left, right| left.category.cmp(&right.category));
    route
        .maximum_units
        .iter()
        .zip(actual)
        .all(|((category, maximum), actual)| {
            category == &actual.category && maximum == &actual.quantity && *maximum > 0
        })
}

pub(super) struct Guard<'a, A> {
    pub authority: &'a A,
    pub owner: &'a ImageOwner,
    pub call: &'a StageCall,
    pub sources: &'a [SourceReceipt],
    pub sending: bool,
}
impl<A: RuntimeImageAuthority> StageAuthority for Guard<'_, A> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        payer: StagePayer<'_>,
        owner: &ImageOwner,
        call: &StageCall,
        replay: bool,
    ) -> Result<(), Error> {
        if owner != self.owner || call != self.call {
            return Err(Error::LinkConflict);
        }
        self.authority
            .verify(tx, payer, owner, call, self.sources, replay)
            .await?;
        if self.sending {
            self.record_prepared_sources(tx, owner, call, true).await?;
        }
        Ok(())
    }
    async fn record_prepared_sources(
        &self,
        tx: &Transaction<'_>,
        owner: &ImageOwner,
        call: &StageCall,
        replay: bool,
    ) -> Result<(), Error> {
        if owner != self.owner || call != self.call {
            return Err(Error::LinkConflict);
        }
        let scope = scope(owner, call)?;
        if replay {
            let stored = crate::imagery_runtime::read_sources_in(tx, &scope)
                .await
                .map_err(stage_error)?
                .ok_or(Error::LinkConflict)?;
            if stored != self.sources {
                return Err(Error::LinkConflict);
            }
            return Ok(());
        }
        crate::imagery_runtime::record_sources_in(tx, &scope, self.sources)
            .await
            .map_err(stage_error)
    }
}

fn allocation(
    input: &DispatchAdmission<'_>,
    catalog: df_assets::CatalogConfig,
    now_ms: i64,
) -> Result<StageAllocation, RuntimeError> {
    if input.stage.kind == StageKind::Classification || input.stage.kind.is_standalone() {
        return Ok(StageAllocation::Decision);
    }
    let audience = retained_audience(input.job)?;
    let (charge_scope, origin_account, campaign, dedupe_scope, retention) = match input.job.owner {
        ImageOwner::Draft { account } => (
            QuotaScope::Account(account),
            Some(account),
            None,
            DedupeScope::account(&account),
            RetentionClass::Personal,
        ),
        ImageOwner::Run { campaign, .. } => (
            QuotaScope::Campaign(campaign),
            match audience {
                Audience::OwnerPrivate(account) => Some(account),
                _ => None,
            },
            Some(campaign),
            DedupeScope::campaign(&campaign),
            RetentionClass::CampaignMedia,
        ),
    };
    let recipe = input
        .job
        .recipe
        .clone()
        .ok_or(RuntimeError::InvalidRecord)?;
    df_imagery::publication::FrozenOutput::decode(&recipe).map_err(RuntimeError::Publication)?;
    let recipe_key = input.job.resolved_key.ok_or(RuntimeError::InvalidRecord)?;
    let publication = input
        .stage
        .planned_publication
        .ok_or(RuntimeError::InvalidRecord)?;
    let admission = Admission {
        asset: AssetId::from_bytes(&publication.asset).map_err(|_| RuntimeError::InvalidRecord)?,
        attempt: AttemptId::from_bytes(&publication.attempt)
            .map_err(|_| RuntimeError::InvalidRecord)?,
        kind: AssetKind::Image,
        provenance: Provenance::Generated,
        charge_scope,
        origin_account,
        campaign,
        audience,
        dedupe_scope,
        request_key: Some(recipe_key),
        retention,
        reserve_bytes: input.config.transforms.max_output_bytes,
        byte_limit: catalog.quota.byte_limit(charge_scope),
        in_flight_upload_limit: None,
    };
    Ok(StageAllocation::Output(Box::new(Config {
        admission,
        recipe_key,
        recipe,
        config: input.job.config.clone(),
        expected_job_revision: input.job.revision,
        absolute_deadline_ms: input.job.absolute_deadline_ms,
        now_ms,
    })))
}

impl<F: FencePort, M: MediaPort, S: MemberPort, A: RuntimeImageAuthority, L: ImageDispatchAdmission>
    FundedImageAdmission<F, M, S, A, L>
{
    pub(super) async fn admit_stage(
        &self,
        input: &DispatchAdmission<'_>,
    ) -> Result<DispatchMode, RuntimeError> {
        if validate(input)? {
            return self.ports.local.admit(input).await;
        }
        let funding = self.ports.authority.funding(input).await?;
        let mut finance = self.ports.finance.lock().await;
        let call = call(finance.client(), input).await?;
        let cost = input.quote.ok_or(RuntimeError::InvalidRecord)?;
        let port = CanonicalStagePort::new(
            Guard {
                authority: &self.ports.authority,
                owner: &input.job.owner,
                call: &call,
                sources: input.sources,
                sending: false,
            },
            StageConfig {
                claim: claim(input),
                allocation: allocation(input, self.settings.catalog, finance.now_ms())?,
                now_ms: finance.now_ms(),
            },
        )
        .map_err(draft_error)?;
        match funding {
            RuntimeFunding::Draft(funding) => {
                let DraftFunding {
                    account,
                    consent,
                    consent_revision,
                    scope,
                    route,
                } = *funding;
                if input.job.owner != (ImageOwner::Draft { account })
                    || !route_matches(&route, cost)
                {
                    return Err(RuntimeError::SourceDenied);
                }
                call.validate(&scope, &route)
                    .map_err(|_| RuntimeError::SourceDenied)?;
                let quote = finance
                    .quote_draft_stage(
                        account,
                        &StageQuoteCommand {
                            consent,
                            consent_revision,
                            scope,
                            call: call.clone(),
                            route,
                            quote_lifetime_ms: self.settings.quote_lifetime_ms,
                        },
                    )
                    .await
                    .map_err(draft_error)?;
                let receipt = finance
                    .admit_draft_stage(account, &quote, &port)
                    .await
                    .map_err(draft_error)?;
                Ok(DispatchMode::Hosted(
                    df_imagery::execution::FinanceTuple::new(
                        receipt.quote.binding().attempt,
                        receipt.quote.binding().operation,
                        receipt.quote.binding().effect,
                    )
                    .map_err(|_| RuntimeError::InvalidRecord)?,
                ))
            }
            RuntimeFunding::Campaign(admission) => {
                let owns_campaign = match &input.job.owner {
                    ImageOwner::Run { campaign, .. } => {
                        campaign.as_bytes() == admission.binding.campaign.as_bytes()
                    }
                    ImageOwner::Draft { .. } => false,
                };
                if !owns_campaign
                    || admission.binding.call != call
                    || !route_matches(&admission.binding.route, cost)
                {
                    return Err(RuntimeError::SourceDenied);
                }
                finance
                    .admit_provider_stage(&admission, &port)
                    .await
                    .map_err(finance_error)?;
                Ok(DispatchMode::Hosted(
                    df_imagery::execution::FinanceTuple::new(
                        *admission.binding.attempt.as_bytes(),
                        *admission.binding.operation.as_bytes(),
                        *admission.binding.effect.as_bytes(),
                    )
                    .map_err(|_| RuntimeError::InvalidRecord)?,
                ))
            }
        }
    }
}
