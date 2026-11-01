//! Historical Ready reuse rechecks current authority without reviving execution demand.
use super::{
    PostgresImagePublication, PublicationSourceAuthority, commit_publication, rows, selection,
    sources,
};
use df_assets::{AssetId, AssetState};
use df_imagery::execution::{DispatchId, ExecutionScope, ExecutionStageId};
use df_imagery::publication::{PublicationError, PublishedImage};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::Transaction;

impl PostgresImagePublication {
    /// Reuses actual nonprivate Ready only while current source/output grants and catalog facts permit.
    pub async fn read_reusable_result_with_authority(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        now_ms: i64,
        authority: &dyn PublicationSourceAuthority,
    ) -> Result<Option<PublishedImage>, PublicationError> {
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(PublicationError::Catalog)?;
        let client = slot.client.as_mut().ok_or(PublicationError::Unavailable)?;
        let tx = client.transaction().await.map_err(rows::database_error)?;
        let result = read_in(&tx, owner, job, now_ms, authority).await;
        match result {
            Ok(result) => {
                commit_publication(tx).await?;
                Ok(result)
            }
            // A reuse probe never closes the job: any refusal by current source authority means
            // the historical result is not reusable, and the caller decides again.
            Err(
                PublicationError::BindingConflict
                | PublicationError::NotFound
                | PublicationError::SourceDenied(_),
            ) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

async fn read_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    now_ms: i64,
    authority: &dyn PublicationSourceAuthority,
) -> Result<Option<PublishedImage>, PublicationError> {
    if now_ms < 0 {
        return Err(PublicationError::InvalidRecord);
    }
    let Some(row)=tx.query_opt("SELECT stage_id,dispatch_id,asset_id FROM imagery.image_job_result WHERE owner_scope=$1 AND job_id=$2 AND NOT private_reconciliation", &[&owner_scope_bytes(owner),&job.as_bytes().as_slice()]).await.map_err(rows::database_error)? else {return Ok(None)};
    let scope = ExecutionScope {
        owner: owner.clone(),
        job: *job,
        stage: ExecutionStageId::from_bytes(&row.get::<_, Vec<u8>>("stage_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        dispatch: DispatchId::from_bytes(&row.get::<_, Vec<u8>>("dispatch_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
    };
    let asset = AssetId::from_bytes(&row.get::<_, Vec<u8>>("asset_id"))
        .map_err(|_| PublicationError::InvalidRecord)?;
    authority.lock_authority(tx, owner, now_ms).await?;
    let locked = sources::lock_in(tx, &scope, asset, Some(authority), now_ms).await?;
    tx.query_one(
        "SELECT job_id FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE",
        &[&owner_scope_bytes(owner), &job.as_bytes().as_slice()],
    )
    .await
    .map_err(rows::database_error)?;
    rows::require_delivery_open_in(tx, owner, job).await?;
    sources::authorize_in(tx, &locked, Some(authority), now_ms).await?;
    authority
        .authorize_sources(tx, &locked.job, &locked.sources, now_ms)
        .await?;
    if locked.output.state != AssetState::Ready
        || locked.output.audience
            != df_imagery::runtime::retained_audience(&locked.job)
                .map_err(|_| PublicationError::InvalidRecord)?
    {
        return Ok(None);
    }
    if let Some(selected) = selection::read_in(tx, owner, job).await? {
        if selected.final_stage != scope.stage
            || selected.final_dispatch != scope.dispatch
            || selected.source_binding != locked.binding
        {
            return Err(PublicationError::BindingConflict);
        }
    } else if df_imagery::runtime::requires_final_selection(&locked.job)
        .map_err(|_| PublicationError::InvalidRecord)?
    {
        return Err(PublicationError::BindingConflict);
    }
    let Some((result, _)) = rows::stored_result(tx, owner, job).await? else {
        return Ok(None);
    };
    if result.private_reconciliation
        || result.stage != scope.stage
        || result.dispatch != scope.dispatch
        || result.asset != locked.output.asset
        || result.attempt != locked.output.attempt
    {
        return Err(PublicationError::BindingConflict);
    }
    check_winners(tx, &result, &locked.output).await?;
    Ok(Some(result))
}

pub(super) async fn check_winners(
    tx: &Transaction<'_>,
    result: &PublishedImage,
    asset: &df_assets::AssetRecord,
) -> Result<(), PublicationError> {
    if result.variants.len() != asset.winning_writes.len() {
        return Err(PublicationError::BindingConflict);
    }
    for variant in &result.variants {
        if !asset.winning_writes.contains(&variant.write) {
            return Err(PublicationError::BindingConflict);
        }
        let row=tx.query_opt("SELECT variant,object_version,byte_length,sha256 FROM media.asset_variant WHERE asset_id=$1 AND attempt=$2 AND write_id=$3 AND state='written'", &[&result.asset.as_bytes().as_slice(),&result.attempt.as_bytes().as_slice(),&variant.write.as_bytes().as_slice()]).await.map_err(rows::database_error)?.ok_or(PublicationError::BindingConflict)?;
        if row.get::<_, String>("variant") != variant.variant.label()
            || row.get::<_, Option<String>>("object_version").as_deref()
                != Some(variant.object_version.as_str())
            || row.get::<_, Option<i64>>("byte_length")
                != Some(
                    i64::try_from(variant.byte_len).map_err(|_| PublicationError::InvalidRecord)?,
                )
            || row.get::<_, Option<Vec<u8>>>("sha256").as_deref() != Some(variant.sha256.as_slice())
        {
            return Err(PublicationError::BindingConflict);
        }
    }
    Ok(())
}
