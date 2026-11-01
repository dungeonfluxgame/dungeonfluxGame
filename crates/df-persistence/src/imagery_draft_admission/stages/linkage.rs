//! Exact immutable finance/execution linkage and common original allocation.
use super::super::Config;
use super::{StagePayer, database_error};
use df_commerce::draft_image::{PaidStageKind, StageCall, StageRoute};
use df_finance::draft_image::Error;
use df_imagery::execution::FinanceTuple;
use df_imagery::{ImageOwner, owner_scope_bytes};
use serde_json::json;
use tokio_postgres::Transaction;

pub(super) struct Link<'a> {
    pub owner: &'a ImageOwner,
    pub payer: StagePayer<'a>,
    pub call: &'a StageCall,
    pub route: &'a StageRoute,
    pub route_key: [u8; 32],
    pub finance: FinanceTuple,
    pub reservation: [u8; 16],
    pub fingerprint: [u8; 32],
    pub replay: bool,
    pub at_ms: i64,
}
impl Link<'_> {
    fn account(&self) -> Option<Vec<u8>> {
        match self.payer {
            StagePayer::Account(account) => Some(account.as_bytes().to_vec()),
            _ => None,
        }
    }
    fn campaign(&self) -> Option<Vec<u8>> {
        match self.payer {
            StagePayer::Campaign(binding) => Some(binding.campaign.as_bytes().to_vec()),
            _ => None,
        }
    }
    fn tenant(&self) -> Option<Vec<u8>> {
        match self.payer {
            StagePayer::Campaign(binding) => Some(binding.tenant.as_bytes().to_vec()),
            _ => None,
        }
    }
    fn session(&self) -> Option<Vec<u8>> {
        match self.payer {
            StagePayer::Campaign(binding) => Some(binding.session.as_bytes().to_vec()),
            _ => None,
        }
    }
    fn namespace(&self) -> &'static str {
        match self.payer {
            StagePayer::Account(_) => "draft_admission",
            StagePayer::Campaign(_) => "admission",
        }
    }
    fn principal(&self) -> Vec<u8> {
        match self.payer {
            StagePayer::Account(account) => account.as_bytes().to_vec(),
            StagePayer::Campaign(binding) => binding.campaign.as_bytes().to_vec(),
        }
    }
}
pub(super) async fn check_finance(tx: &Transaction<'_>, link: &Link<'_>) -> Result<(), Error> {
    let asset = if link.call.kind == PaidStageKind::Classification || link.call.kind.is_standalone()
    {
        None
    } else {
        Some(
            link.call
                .planned_publication
                .ok_or(Error::LinkConflict)?
                .asset
                .0
                .to_vec(),
        )
    };
    let row=tx.query_opt("SELECT 1 FROM finance.provider_attempt a JOIN finance.reservation r USING(reservation_id) \
        JOIN finance.operation_receipt o ON o.namespace=$1 AND o.principal=$2 AND o.operation_id=r.operation_id \
        WHERE a.attempt_id=$3 AND a.reservation_id=$4 AND a.effect_id=$5 AND r.operation_id=$6 \
        AND a.account_id IS NOT DISTINCT FROM $7 AND a.campaign_id IS NOT DISTINCT FROM $8 \
        AND a.tenant_id IS NOT DISTINCT FROM $9 AND r.session_id IS NOT DISTINCT FROM $10 \
        AND r.price_version_id=$13 AND a.media_asset_id IS NOT DISTINCT FROM $11 AND o.fingerprint=$12 \
        AND (o.namespace='draft_admission' AND o.result_ref=r.reservation_id \
          OR o.namespace='admission' AND o.outcome=jsonb_build_object( \
            'reservation',encode(r.reservation_id,'hex'),'attempt',encode(a.attempt_id,'hex'), \
            'price_version',encode(r.price_version_id,'hex'), \
            'retail_maximum',r.retail_maximum_micro_dollars,'supplier_maximum',a.supplier_maximum_micro_dollars))",
        &[&link.namespace(),&link.principal(),&link.finance.attempt.as_slice(),&link.reservation.as_slice(),
          &link.finance.effect.as_slice(),&link.finance.operation.as_slice(),&link.account(),&link.campaign(),
          &link.tenant(),&link.session(),&asset,&link.fingerprint.as_slice(),&link.route.price_version.as_slice()]).await.map_err(database_error)?;
    if row.is_none() {
        return Err(Error::LinkConflict);
    }
    Ok(())
}

pub(super) async fn allocation(
    tx: &Transaction<'_>,
    config: &Config,
    scope: &[u8],
    call: &StageCall,
) -> Result<Option<Vec<u8>>, Error> {
    let publication = call.planned_publication.ok_or(Error::LinkConflict)?;
    let row=tx.query_opt("SELECT stage_id FROM imagery.image_publication_allocation WHERE publication_attempt=$1 \
        AND asset_id=$2 AND job_id=$3 AND owner_scope=$4 AND original_recipe_key=$5 AND original_recipe_revision=$6 \
        AND original_recipe_bytes=$7 AND original_config_revision=$8 AND original_config_bytes=$9",
        &[&publication.publication.0.as_slice(),&publication.asset.0.as_slice(),&call.job.0.as_slice(),&scope,
          &config.recipe_key.as_bytes().as_slice(),&i64::from(config.recipe.revision()),&config.recipe.bytes(),
          &i64::from(config.config.revision()),&config.config.bytes()]).await.map_err(database_error)?;
    if let Some(row) = row {
        return Ok(Some(row.get(0)));
    }
    if tx.query_opt("SELECT 1 FROM imagery.image_publication_allocation WHERE publication_attempt=$1 OR asset_id=$2 OR job_id=$3",
        &[&publication.publication.0.as_slice(),&publication.asset.0.as_slice(),&call.job.0.as_slice()]).await.map_err(database_error)?.is_some() {
        return Err(Error::LinkConflict);
    }
    Ok(None)
}
pub(super) async fn first_allocation(
    tx: &Transaction<'_>,
    config: &Config,
    scope: &[u8],
    call: &StageCall,
    at_ms: i64,
) -> Result<(), Error> {
    let publication = call.planned_publication.ok_or(Error::LinkConflict)?;
    let job = df_imagery::ImageJobId::from_bytes(&call.job.0).map_err(|_| Error::InvalidRecord)?;
    let row=tx.query_opt("SELECT 1 FROM imagery.image_job WHERE job_id=$1 AND owner_scope=$2 \
        AND planned_output_stage=$3 AND planned_publication_asset=$4 AND planned_publication_attempt=$5 FOR UPDATE",
        &[&call.job.0.as_slice(),&scope,&call.stage.0.as_slice(),&publication.asset.0.as_slice(),&publication.publication.0.as_slice()])
        .await.map_err(database_error)?;
    if row.is_none() {
        return Err(Error::LinkConflict);
    }
    if config.admission.campaign.is_some() {
        crate::imagery_admission::rows::check_catalog_identity(tx, &config.admission)
            .await
            .map_err(Error::Finance)?;
    } else {
        super::super::rows::check_catalog(tx, config).await?;
    }
    crate::imagery_admission::rows::check_frozen_job(
        tx,
        crate::imagery_admission::rows::FrozenJobCheck {
            owner_scope: scope,
            job: &job,
            recipe_key: &config.recipe_key,
            recipe: &config.recipe,
            config: &config.config,
            expected_revision: config.expected_job_revision,
            absolute_deadline_ms: config.absolute_deadline_ms,
            now_ms: at_ms,
        },
    )
    .await
    .map_err(Error::Finance)?;
    let publication = df_finance::PublicationAttemptId::new(publication.publication.0)
        .map_err(|_| Error::InvalidRecord)?;
    crate::imagery_admission::rows::insert_allocation(
        tx,
        crate::imagery_admission::rows::PublicationAllocationInsert {
            publication_attempt: publication,
            admission: &config.admission,
            owner_scope: scope,
            job: &job,
            stage: &call.stage.0,
            recipe_key: &config.recipe_key,
            recipe: &config.recipe,
            config: &config.config,
            now_ms: at_ms,
        },
    )
    .await
    .map_err(Error::Finance)
}

pub(super) async fn insert(
    tx: &Transaction<'_>,
    link: &Link<'_>,
    anchor: Option<&Vec<u8>>,
) -> Result<(), Error> {
    let planned = link.call.planned_publication;
    let publication = anchor.and_then(|_| planned.map(|p| p.publication.0.to_vec()));
    let asset = anchor.and_then(|_| planned.map(|p| p.asset.0.to_vec()));
    if anchor.is_some() && planned.is_none() {
        return Err(Error::LinkConflict);
    }
    tx.execute("INSERT INTO imagery.image_finance_admission_binding(finance_attempt,publication_attempt,job_id,stage_id,asset_id, \
        owner_scope,account_id,tenant_id,campaign_id,session_id,operation_id,effect_id,receipt_namespace,receipt_fingerprint, \
        reservation_id,created_at_ms,execution_stage_id,dispatch_id,parent_dispatch,stage_kind,stage_call,route_key,planned_asset,planned_publication,cost_route) \
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21::text::jsonb,$22,$23,$24,$25::text::jsonb)",
        &[&link.finance.attempt.as_slice(),&publication,&link.call.job.0.as_slice(),&anchor,&asset,
          &owner_scope_bytes(link.owner),&link.account(),&link.tenant(),&link.campaign(),&link.session(),
          &link.finance.operation.as_slice(),&link.finance.effect.as_slice(),&link.namespace(),&link.fingerprint.as_slice(),
          &link.reservation.as_slice(),&link.at_ms,&link.call.stage.0.as_slice(),&link.call.dispatch.as_slice(),
          &link.call.parent_dispatch.map(|id|id.to_vec()),&link.call.kind.label(),&json!(link.call).to_string(),
          &link.route_key.as_slice(),&planned.map(|p|p.asset.0.to_vec()),&planned.map(|p|p.publication.0.to_vec()),&json!(link.route).to_string()])
        .await.map_err(database_error)?;
    Ok(())
}
pub(super) async fn verify(tx: &Transaction<'_>, link: &Link<'_>) -> Result<(), Error> {
    let row=tx.query_opt("SELECT 1 FROM imagery.image_finance_admission_binding b \
        JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=b.dispatch_id AND d.job_id=b.job_id AND d.stage_id=b.execution_stage_id \
        LEFT JOIN imagery.image_publication_allocation a ON a.publication_attempt=b.publication_attempt AND a.job_id=b.job_id AND a.stage_id=b.stage_id AND a.asset_id=b.asset_id AND a.owner_scope=b.owner_scope \
        WHERE b.finance_attempt=$1 AND b.owner_scope=$2 AND b.job_id=$3 AND b.execution_stage_id=$4 AND b.dispatch_id=$5 \
        AND b.stage_call=$6::text::jsonb AND b.route_key=$7 AND b.cost_route=$14::text::jsonb AND b.receipt_fingerprint=$8 AND b.reservation_id=$9 \
        AND b.operation_id=$10 AND b.effect_id=$11 AND b.receipt_namespace=$12 AND b.receipt_principal=$13 \
        AND d.finance_attempt=b.finance_attempt AND d.finance_operation=b.operation_id AND d.finance_effect=b.effect_id \
        AND (b.stage_kind IN ('classification','photo_inspection','head_locations','segmentation','staging_identity','staging_facing') AND b.asset_id IS NULL OR a.asset_id IS NOT NULL)",
        &[&link.finance.attempt.as_slice(),&owner_scope_bytes(link.owner),&link.call.job.0.as_slice(),
          &link.call.stage.0.as_slice(),&link.call.dispatch.as_slice(),&json!(link.call).to_string(),
          &link.route_key.as_slice(),&link.fingerprint.as_slice(),&link.reservation.as_slice(),
          &link.finance.operation.as_slice(),&link.finance.effect.as_slice(),&link.namespace(),&link.principal(),&json!(link.route).to_string()])
        .await.map_err(database_error)?;
    if row.is_none() {
        return Err(Error::LinkConflict);
    }
    Ok(())
}
