//! Publication row locks, exact binding comparison and bounded result parsing.

use super::{PublicationCommand, VersionedSnapshot};
use df_assets::{
    AssetRecord, AssetVariant, Audience, ImageAlpha, ImageIntent, ObjectVersion, WriteId,
};
use df_imagery::execution::{CheckpointAlpha, DispatchId, ExecutionStageId, OutputCheckpoint};
use df_imagery::publication::{FrozenOutput, PublicationError, PublishedImage, PublishedVariant};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};

pub(super) fn record_failure(error: &tokio_postgres::Error) {
    let code = error.code().map_or("none", |code| code.code());
    df_observe::record_audit_line(
        "IMAGERY",
        format!("IMAGERY publication_failed sqlstate={code}"),
    );
}

pub(super) fn database_error(error: tokio_postgres::Error) -> PublicationError {
    record_failure(&error);
    PublicationError::Unavailable
}

pub(super) async fn require_read_committed(tx: &Transaction<'_>) -> Result<(), PublicationError> {
    let isolation: String = tx
        .query_one("SHOW transaction_isolation", &[])
        .await
        .map_err(database_error)?
        .get(0);
    if isolation != "read committed" {
        return Err(PublicationError::InvalidRecord);
    }
    Ok(())
}

pub(super) async fn require_delivery_open_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<(), PublicationError> {
    let sealed=tx.query_one("SELECT EXISTS(SELECT 1 FROM imagery.image_no_delivery WHERE owner_scope=$1 AND job_id=$2)", &[&owner_scope_bytes(owner),&job.as_bytes().as_slice()]).await.map_err(database_error)?.get::<_,bool>(0);
    if sealed {
        Err(PublicationError::BindingConflict)
    } else {
        Ok(())
    }
}

pub(super) fn check_catalog_owner(
    asset: &AssetRecord,
    owner: &ImageOwner,
) -> Result<(), PublicationError> {
    let matches = match owner {
        ImageOwner::Run { campaign, .. } => asset.campaign == Some(*campaign),
        ImageOwner::Draft { account } => {
            asset.campaign.is_none()
                && asset.origin_account == Some(*account)
                && asset.audience == Audience::OwnerPrivate(*account)
        }
    };
    if !matches {
        return Err(PublicationError::NotFound);
    }
    Ok(())
}

pub(super) async fn lock_job(
    tx: &Transaction<'_>,
    command: &PublicationCommand<'_>,
) -> Result<Row, PublicationError> {
    let row = tx
        .query_opt(
            "SELECT * FROM imagery.image_job WHERE job_id=$1 AND owner_scope=$2 FOR UPDATE",
            &[
                &command.job.as_bytes().as_slice(),
                &owner_scope_bytes(command.owner),
            ],
        )
        .await
        .map_err(database_error)?
        .ok_or(PublicationError::NotFound)?;
    if row
        .get::<_, Option<Vec<u8>>>("planned_publication_asset")
        .as_deref()
        != Some(command.held.asset.as_bytes().as_slice())
        || row
            .get::<_, Option<Vec<u8>>>("planned_publication_attempt")
            .as_deref()
            != Some(command.held.attempt.as_bytes().as_slice())
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(row)
}

pub(super) fn snapshot(
    row: &Row,
    revision: &str,
    bytes: &str,
) -> Result<VersionedSnapshot, PublicationError> {
    let revision = row
        .get::<_, Option<i64>>(revision)
        .ok_or(PublicationError::InvalidRecipe)?;
    let bytes = row
        .get::<_, Option<Vec<u8>>>(bytes)
        .ok_or(PublicationError::InvalidRecipe)?;
    VersionedSnapshot::new(
        revision
            .try_into()
            .map_err(|_| PublicationError::InvalidRecipe)?,
        bytes,
        df_imagery::publication::MAXIMUM_RECIPE_BYTES,
    )
    .map_err(|_| PublicationError::InvalidRecipe)
}

pub(super) async fn check_allocation(
    tx: &Transaction<'_>,
    command: &PublicationCommand<'_>,
    allocation_stage: ExecutionStageId,
    job: &Row,
    recipe: &VersionedSnapshot,
) -> Result<(), PublicationError> {
    let allocation = tx.query_opt("SELECT * FROM imagery.image_publication_allocation WHERE publication_attempt=$1 AND asset_id=$2 AND job_id=$3 AND stage_id=$4 AND owner_scope=$5", &[&command.held.attempt.as_bytes().as_slice(), &command.held.asset.as_bytes().as_slice(), &command.job.as_bytes().as_slice(), &allocation_stage.as_bytes().as_slice(), &owner_scope_bytes(command.owner)]).await.map_err(database_error)?.ok_or(PublicationError::BindingConflict)?;
    if allocation.get::<_, Vec<u8>>("original_recipe_key")
        != job
            .get::<_, Option<Vec<u8>>>("resolved_recipe_key")
            .ok_or(PublicationError::InvalidRecipe)?
        || allocation.get::<_, i64>("original_recipe_revision") != i64::from(recipe.revision())
        || allocation.get::<_, Vec<u8>>("original_recipe_bytes") != recipe.bytes()
        || allocation.get::<_, i64>("original_config_revision")
            != job.get::<_, i64>("config_revision")
        || allocation.get::<_, Vec<u8>>("original_config_bytes")
            != job.get::<_, Vec<u8>>("config_bytes")
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(())
}

pub(super) async fn lock_stage_dispatch(
    tx: &Transaction<'_>,
    command: &PublicationCommand<'_>,
) -> Result<Row, PublicationError> {
    let stage = tx.query_opt("SELECT s.claim_token AS current_claim, s.claim_generation AS current_generation, s.lease_until_ms, s.owner_scope, s.stage_kind, d.*, c.current_dispatch FROM imagery.image_execution_stage s JOIN imagery.image_dispatch_attempt d USING(job_id,stage_id) JOIN imagery.image_stage_cursor c USING(job_id,stage_id) WHERE s.job_id=$1 AND s.stage_id=$2 AND d.dispatch_id=$3 FOR UPDATE OF s,d,c", &[&command.job.as_bytes().as_slice(), &command.stage.as_bytes().as_slice(), &command.dispatch.as_bytes().as_slice()]).await.map_err(database_error)?.ok_or(PublicationError::NotFound)?;
    if stage.get::<_, Vec<u8>>("owner_scope") != owner_scope_bytes(command.owner)
        || stage.get::<_, Vec<u8>>("current_dispatch") != command.dispatch.as_bytes()
        || stage.get::<_, String>("dispatch_state") != "succeeded"
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(stage)
}

pub(super) fn check_claim(
    command: &PublicationCommand<'_>,
    stage: &Row,
    private_reconciliation: bool,
) -> Result<(), PublicationError> {
    let generation =
        i64::try_from(command.claim_generation).map_err(|_| PublicationError::InvalidRecord)?;
    if stage.get::<_, Vec<u8>>("current_claim") != command.claim.as_bytes()
        || stage.get::<_, i64>("current_generation") != generation
        || stage.get::<_, Vec<u8>>("claim_token") != command.claim.as_bytes()
        || stage.get::<_, i64>("claim_generation") != generation
        || (!private_reconciliation && stage.get::<_, i64>("lease_until_ms") < command.now_ms)
    {
        return Err(PublicationError::StaleClaim);
    }
    Ok(())
}

pub(super) fn check_proofs(
    command: &PublicationCommand<'_>,
    frozen: &FrozenOutput,
    digest: &[u8; 32],
) -> Result<(), PublicationError> {
    let expected_count = if frozen.intent(AssetVariant::Display).is_some() {
        2
    } else {
        1
    };
    if command.proofs.len() != expected_count || command.finalize.winners.len() != expected_count {
        return Err(PublicationError::BindingConflict);
    }
    for variant in [AssetVariant::Master, AssetVariant::Display] {
        let Some(intent) = frozen.intent(variant) else {
            continue;
        };
        let proof = command
            .proofs
            .iter()
            .find(|proof| proof.variant() == variant)
            .ok_or(PublicationError::BindingConflict)?;
        let record = proof.record();
        if proof.recipe_digest() != digest
            || !intent.accepts(proof.intent(), proof.alpha())
            || record.asset != command.held.asset
            || record.attempt != command.held.attempt
            || record.owner != command.held.owner
            || command
                .finalize
                .winners
                .iter()
                .filter(|winner| winner.variant == variant && winner.write == record.write)
                .count()
                != 1
        {
            return Err(PublicationError::BindingConflict);
        }
    }
    let master = frozen
        .intent(AssetVariant::Master)
        .ok_or(PublicationError::InvalidRecipe)?;
    if command.finalize.mime != master.mime()
        || command.finalize.width_px != Some(master.width_px())
        || command.finalize.height_px != Some(master.height_px())
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(())
}

fn append(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), PublicationError> {
    let length = u64::try_from(value.len()).map_err(|_| PublicationError::InvalidRecord)?;
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

pub(super) fn binding_bytes(
    command: &PublicationCommand<'_>,
    digest: &[u8; 32],
    checkpoint: &OutputCheckpoint,
) -> Result<Vec<u8>, PublicationError> {
    let mut bytes = b"df-imagery/publication-v1".to_vec();
    append(&mut bytes, &owner_scope_bytes(command.owner))?;
    for value in [
        command.job.as_bytes(),
        command.stage.as_bytes(),
        command.dispatch.as_bytes(),
        command.claim.as_bytes(),
        command.held.asset.as_bytes(),
        command.held.attempt.as_bytes(),
        command.held.owner.as_bytes(),
    ] {
        append(&mut bytes, value)?;
    }
    bytes.extend_from_slice(&command.claim_generation.to_be_bytes());
    bytes.extend_from_slice(digest);
    bytes.extend_from_slice(&checkpoint.usage_revision.to_be_bytes());
    append(&mut bytes, checkpoint.book_call_id.as_bytes())?;
    for variant in [AssetVariant::Master, AssetVariant::Display] {
        let Some(proof) = command
            .proofs
            .iter()
            .find(|proof| proof.variant() == variant)
        else {
            continue;
        };
        let record = proof.record();
        append(&mut bytes, variant.label().as_bytes())?;
        append(&mut bytes, record.write.as_bytes())?;
        append(&mut bytes, record.key.path().as_bytes())?;
        if let Some(version) = &record.version {
            append(&mut bytes, version.as_str().as_bytes())?;
        }
        if let Some(manifest) = record.manifest {
            bytes.extend_from_slice(&manifest.byte_len.to_be_bytes());
            bytes.extend_from_slice(&manifest.sha256);
        }
        append(&mut bytes, proof.intent().mime().as_bytes())?;
        bytes.extend_from_slice(&proof.intent().width_px().to_be_bytes());
        bytes.extend_from_slice(&proof.intent().height_px().to_be_bytes());
        bytes.push(u8::from(proof.alpha() == ImageAlpha::Transparent));
    }
    // Winner order is part of the catalog's exact replay contract.
    for winner in &command.finalize.winners {
        append(&mut bytes, winner.write.as_bytes())?;
        append(&mut bytes, winner.variant.label().as_bytes())?;
    }
    Ok(bytes)
}

pub(super) async fn insert_result(
    tx: &Transaction<'_>,
    command: &PublicationCommand<'_>,
    allocation_stage: ExecutionStageId,
    digest: &[u8; 32],
    binding: &[u8],
    private: bool,
) -> Result<(), PublicationError> {
    let revision: i64 = tx.query_one("UPDATE imagery.image_job SET revision=revision+1, updated_at_ms=GREATEST(updated_at_ms,$2) WHERE job_id=$1 RETURNING revision", &[&command.job.as_bytes().as_slice(), &command.now_ms]).await.map_err(database_error)?.get(0);
    tx.execute("INSERT INTO imagery.image_job_result(job_id,owner_scope,stage_id,dispatch_id,asset_id,publication_attempt,recipe_digest,binding_bytes,job_revision,published_at_ms,private_reconciliation,allocation_stage_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)", &[&command.job.as_bytes().as_slice(), &owner_scope_bytes(command.owner), &command.stage.as_bytes().as_slice(), &command.dispatch.as_bytes().as_slice(), &command.held.asset.as_bytes().as_slice(), &command.held.attempt.as_bytes().as_slice(), &digest.as_slice(), &binding, &revision, &command.now_ms, &private,&allocation_stage.as_bytes().as_slice()]).await.map_err(database_error)?;
    for proof in command.proofs {
        let record = proof.record();
        let version = record
            .version
            .as_ref()
            .ok_or(PublicationError::InvalidRecord)?;
        let manifest = record.manifest.ok_or(PublicationError::InvalidRecord)?;
        let length =
            i64::try_from(manifest.byte_len).map_err(|_| PublicationError::InvalidRecord)?;
        let width = i32::try_from(proof.intent().width_px())
            .map_err(|_| PublicationError::InvalidRecord)?;
        let height = i32::try_from(proof.intent().height_px())
            .map_err(|_| PublicationError::InvalidRecord)?;
        let alpha = match proof.alpha() {
            ImageAlpha::Opaque => "opaque",
            ImageAlpha::Transparent => "transparent",
        };
        tx.execute("INSERT INTO imagery.image_result_variant(job_id,variant,write_id,object_version,byte_len,sha256,mime,width_px,height_px,alpha) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)", &[&command.job.as_bytes().as_slice(), &proof.variant().label(), &record.write.as_bytes().as_slice(), &version.as_str(), &length, &manifest.sha256.as_slice(), &proof.intent().mime(), &width, &height, &alpha]).await.map_err(database_error)?;
    }
    Ok(())
}

pub(super) async fn stored_result(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<(PublishedImage, Vec<u8>)>, PublicationError> {
    let Some(row) = tx.query_opt("SELECT r.* FROM imagery.image_job_result r JOIN media.asset a ON a.asset_id=r.asset_id AND a.attempt=r.publication_attempt WHERE r.job_id=$1 AND r.owner_scope=$2 AND a.state='ready' FOR SHARE OF a", &[&job.as_bytes().as_slice(), &owner_scope_bytes(owner)]).await.map_err(database_error)? else { return Ok(None); };
    let rows = tx
        .query(
            "SELECT * FROM imagery.image_result_variant WHERE job_id=$1 ORDER BY variant",
            &[&job.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?;
    if rows.is_empty() || rows.len() > 2 {
        return Err(PublicationError::InvalidRecord);
    }
    let variants = rows
        .iter()
        .map(parse_variant)
        .collect::<Result<Vec<_>, _>>()?;
    let result = PublishedImage {
        job: *job,
        stage: ExecutionStageId::from_bytes(&row.get::<_, Vec<u8>>("stage_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        dispatch: DispatchId::from_bytes(&row.get::<_, Vec<u8>>("dispatch_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        asset: df_assets::AssetId::from_bytes(&row.get::<_, Vec<u8>>("asset_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        attempt: df_assets::AttemptId::from_bytes(&row.get::<_, Vec<u8>>("publication_attempt"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        recipe_digest: row
            .get::<_, Vec<u8>>("recipe_digest")
            .try_into()
            .map_err(|_| PublicationError::InvalidRecord)?,
        job_revision: row
            .get::<_, i64>("job_revision")
            .try_into()
            .map_err(|_| PublicationError::InvalidRecord)?,
        published_at_ms: row.get("published_at_ms"),
        private_reconciliation: row.get("private_reconciliation"),
        variants,
    };
    Ok(Some((result, row.get("binding_bytes"))))
}

fn parse_variant(row: &Row) -> Result<PublishedVariant, PublicationError> {
    let variant = match row.get::<_, String>("variant").as_str() {
        "master" => AssetVariant::Master,
        "display" => AssetVariant::Display,
        _ => return Err(PublicationError::InvalidRecord),
    };
    let alpha = match row.get::<_, String>("alpha").as_str() {
        "opaque" => ImageAlpha::Opaque,
        "transparent" => ImageAlpha::Transparent,
        _ => return Err(PublicationError::InvalidRecord),
    };
    let width = row
        .get::<_, i32>("width_px")
        .try_into()
        .map_err(|_| PublicationError::InvalidRecord)?;
    let height = row
        .get::<_, i32>("height_px")
        .try_into()
        .map_err(|_| PublicationError::InvalidRecord)?;
    Ok(PublishedVariant {
        variant,
        write: WriteId::from_bytes(&row.get::<_, Vec<u8>>("write_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        object_version: ObjectVersion::new(row.get("object_version"))
            .ok_or(PublicationError::InvalidRecord)?,
        byte_len: row
            .get::<_, i64>("byte_len")
            .try_into()
            .map_err(|_| PublicationError::InvalidRecord)?,
        sha256: row
            .get::<_, Vec<u8>>("sha256")
            .try_into()
            .map_err(|_| PublicationError::InvalidRecord)?,
        intent: ImageIntent::new(&row.get::<_, String>("mime"), width, height, alpha)
            .map_err(PublicationError::Proof)?,
        alpha,
    })
}

pub(super) async fn check_written_proofs(
    tx: &Transaction<'_>,
    command: &PublicationCommand<'_>,
) -> Result<(), PublicationError> {
    for proof in command.proofs {
        let record = proof.record();
        let version = record
            .version
            .as_ref()
            .ok_or(PublicationError::InvalidRecord)?;
        let manifest = record.manifest.ok_or(PublicationError::InvalidRecord)?;
        let length =
            i64::try_from(manifest.byte_len).map_err(|_| PublicationError::InvalidRecord)?;
        let row = tx.query_opt("SELECT 1 FROM media.asset_variant WHERE asset_id=$1 AND attempt=$2 AND write_id=$3 AND variant=$4 AND owner=$5 AND state='written' AND object_domain='private_media' AND object_key=$6 AND object_version=$7 AND byte_length=$8 AND sha256=$9 AND created_at_ms=$10", &[&record.asset.as_bytes().as_slice(), &record.attempt.as_bytes().as_slice(), &record.write.as_bytes().as_slice(), &record.variant.label(), &record.owner.as_bytes().as_slice(), &record.key.path(), &version.as_str(), &length, &manifest.sha256.as_slice(), &record.created_at_ms]).await.map_err(database_error)?;
        if row.is_none() {
            return Err(PublicationError::BindingConflict);
        }
    }
    Ok(())
}

pub(super) fn check_checkpoint(
    command: &PublicationCommand<'_>,
    stage: &Row,
    checkpoint: &OutputCheckpoint,
    digest: &[u8; 32],
) -> Result<(), PublicationError> {
    let master = command
        .proofs
        .iter()
        .find(|proof| proof.variant() == AssetVariant::Master)
        .ok_or(PublicationError::BindingConflict)?;
    let master_record = master.record();
    let manifest = master_record
        .manifest
        .ok_or(PublicationError::InvalidRecord)?;
    if checkpoint.storage_key != master_record.key.path()
        || master_record
            .version
            .as_ref()
            .is_none_or(|version| version.as_str() != checkpoint.object_version)
        || checkpoint.byte_len != manifest.byte_len
        || checkpoint.sha256 != manifest.sha256
        || checkpoint.recipe_digest != *digest
        || checkpoint.mime != master.intent().mime()
        || checkpoint.width_px != Some(master.intent().width_px())
        || checkpoint.height_px != Some(master.intent().height_px())
        || checkpoint.alpha
            != match master.alpha() {
                df_assets::ImageAlpha::Opaque => CheckpointAlpha::Opaque,
                df_assets::ImageAlpha::Transparent => CheckpointAlpha::Transparent,
            }
        || checkpoint.usage_revision
            != u32::try_from(stage.get::<_, i32>("usage_revision"))
                .map_err(|_| PublicationError::InvalidRecord)?
        || checkpoint.book_call_id != stage.get::<_, String>("book_call_id")
    {
        return Err(PublicationError::CheckpointMismatch);
    }
    Ok(())
}

pub(super) fn check_recipe_key(asset: &AssetRecord, job: &Row) -> Result<(), PublicationError> {
    if asset
        .request_key
        .as_ref()
        .map(|key| key.as_bytes().as_slice())
        != job
            .get::<_, Option<Vec<u8>>>("resolved_recipe_key")
            .as_deref()
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(())
}

pub(super) async fn privatize_retired(
    tx: &Transaction<'_>,
    command: &PublicationCommand<'_>,
    asset: &AssetRecord,
    stage: &Row,
    private: bool,
) -> Result<(), PublicationError> {
    if private {
        if stage.get::<_, bool>("local_accounting")
            && !super::selection::has_supplier_evidence_in(tx, command, stage).await?
        {
            return Err(PublicationError::BindingConflict);
        }
        let account = asset
            .origin_account
            .ok_or(PublicationError::BindingConflict)?;
        tx.execute("UPDATE media.asset SET audience_kind = 'owner_private', audience_account = $2, audience_campaign = NULL, audience_members = NULL WHERE asset_id = $1", &[&asset.asset.as_bytes().as_slice(), &account.as_bytes().as_slice()]).await.map_err(database_error)?;
    }
    Ok(())
}

pub(super) fn check_recovery_lineage(
    command: &PublicationCommand<'_>,
    recovery: &df_imagery::execution::RecoveryLease,
    stage: &Row,
) -> Result<(), PublicationError> {
    if recovery.stage.owner != *command.owner
        || recovery.stage.job != command.job
        || recovery.stage.stage != command.stage
        || recovery.stage.current.dispatch != command.dispatch
        || recovery.stage.claim != command.claim
        || recovery.stage.claim_generation != command.claim_generation
        || stage.get::<_, Vec<u8>>("claim_token") != command.claim.as_bytes()
        || stage.get::<_, i64>("claim_generation")
            != i64::try_from(command.claim_generation)
                .map_err(|_| PublicationError::InvalidRecord)?
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(())
}
