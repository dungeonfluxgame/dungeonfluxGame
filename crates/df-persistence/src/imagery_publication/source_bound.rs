//! Reads the producer's retained output bound after current catalog source authorization.

use df_assets::{
    AssetKind, AssetRecord, AssetVariant, ObjectVersion, Provenance, StorageOwner, VariantRecord,
    WriteId,
};
use df_imagery::runtime::RuntimeError;
use df_imagery::{ImageJobId, ImageOwner, JournalError, owner_of_bytes};
use tokio_postgres::Transaction;

/// Requires the caller's current grant and winning catalog source locks; this is not a grant.
pub async fn read_generated_source_bound_in(
    tx: &Transaction<'_>,
    asset: &AssetRecord,
    master: &VariantRecord,
) -> Result<Option<u64>, RuntimeError> {
    if asset.kind != AssetKind::Image || asset.provenance != Provenance::Generated {
        return Ok(None);
    }
    if master.asset != asset.asset
        || master.attempt != asset.attempt
        || master.variant != AssetVariant::Master
        || !asset.winning_writes.contains(&master.write)
    {
        return Ok(None);
    }
    let manifest = master.manifest.ok_or(RuntimeError::InvalidRecord)?;
    let version = master.version.as_ref().ok_or(RuntimeError::InvalidRecord)?;
    let rows = tx
        .query(
            "SELECT r.job_id,r.owner_scope,v.write_id,v.object_version,v.byte_len,v.sha256 \
             FROM imagery.image_job_result r \
             JOIN imagery.image_publication_allocation a \
               ON a.publication_attempt=r.publication_attempt AND a.asset_id=r.asset_id \
              AND a.job_id=r.job_id AND a.owner_scope=r.owner_scope \
              AND a.stage_id=r.allocation_stage_id \
             JOIN imagery.image_result_variant v ON v.job_id=r.job_id AND v.variant='master' \
             WHERE r.asset_id=$1 AND r.publication_attempt=$2",
            &[
                &asset.asset.as_bytes().as_slice(),
                &asset.attempt.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(|_| RuntimeError::StorageUnavailable)?;
    let row = match rows.as_slice() {
        [] => return Ok(None),
        [row] => row,
        _ => return Err(RuntimeError::InvalidRecord),
    };
    let owner =
        owner_of_bytes(&row.get::<_, Vec<u8>>("owner_scope")).ok_or(RuntimeError::InvalidRecord)?;
    let job = ImageJobId::from_bytes(&row.get::<_, Vec<u8>>("job_id"))
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let expected_owner = match &owner {
        ImageOwner::Run { campaign, .. } => StorageOwner::Campaign(*campaign),
        ImageOwner::Draft { account } => StorageOwner::AccountDraft(*account),
    };
    let byte_len =
        u64::try_from(row.get::<_, i64>("byte_len")).map_err(|_| RuntimeError::InvalidRecord)?;
    let write = WriteId::from_bytes(&row.get::<_, Vec<u8>>("write_id"))
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let stored_version = ObjectVersion::new(row.get::<_, String>("object_version"))
        .ok_or(RuntimeError::InvalidRecord)?;
    let digest: [u8; 32] = row
        .get::<_, Vec<u8>>("sha256")
        .try_into()
        .map_err(|_| RuntimeError::InvalidRecord)?;
    if byte_len == 0 {
        return Err(RuntimeError::InvalidRecord);
    }
    if df_assets::record_owner(asset) != Some(expected_owner)
        || write != master.write
        || stored_version != *version
        || byte_len != manifest.byte_len
        || digest != manifest.sha256
    {
        return Ok(None);
    }
    let Some(job) = crate::imagery_sql::read_in(tx, &owner, &job)
        .await
        .map_err(|error| match error {
            JournalError::Unavailable | JournalError::Unknown => RuntimeError::StorageUnavailable,
            _ => RuntimeError::InvalidRecord,
        })?
    else {
        return Ok(None);
    };
    Ok(Some(
        df_imagery::runtime::retained_config(&job)?
            .transforms
            .max_output_bytes,
    ))
}
