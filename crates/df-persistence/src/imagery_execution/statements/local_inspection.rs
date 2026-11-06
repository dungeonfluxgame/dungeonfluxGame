//! Exact no-dispatch local Unjudged evidence; protected policy decoding precedes this transaction.
use super::common::{
    bump_job_revision, bytes32, invalid, locked_claim, locked_job, map_db, require_read_committed,
};
use df_imagery::execution::{
    ExecutionError, ExecutionScope, ExecutionStageId, LocalInspectionCheckpoint,
    LocalInspectionCommand, LocalInspectionReason, StageKind, StageScope,
};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};

fn parse(
    row: &Row,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<LocalInspectionCheckpoint, ExecutionError> {
    Ok(LocalInspectionCheckpoint {
        scope: StageScope {
            owner: owner.clone(),
            job: *job,
            stage: *stage,
        },
        basis: super::decision::parse_basis(row)?,
        candidate_scope: ExecutionScope {
            owner: owner.clone(),
            job: *job,
            stage: invalid(ExecutionStageId::from_bytes(
                &row.get::<_, Vec<u8>>("candidate_stage_id"),
            ))?,
            dispatch: invalid(df_imagery::execution::DispatchId::from_bytes(
                &row.get::<_, Vec<u8>>("candidate_dispatch_id"),
            ))?,
        },
        candidate_observation_version: invalid(u32::try_from(
            row.get::<_, i64>("candidate_observation_version"),
        ))?,
        candidate_usage_revision: invalid(u32::try_from(
            row.get::<_, i32>("candidate_usage_revision"),
        ))?,
        source_digest: bytes32(&row.get::<_, Vec<u8>>("source_digest"))?,
        reason: LocalInspectionReason::from_key(&row.get::<_, String>("reason"))
            .ok_or(ExecutionError::InvalidRecord)?,
        observed_at_ms: row.get("observed_at_ms"),
        schema_revision: invalid(u32::try_from(row.get::<_, i64>("schema_revision")))?,
        validator_revision: invalid(u32::try_from(row.get::<_, i64>("validator_revision")))?,
    })
}
async fn read_row(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<Option<Row>, ExecutionError> {
    tx.query_opt("SELECT l.*,'inspection'::text AS stage_kind FROM imagery.image_local_inspection l WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3",
        &[&owner_scope_bytes(owner),&job.as_bytes().as_slice(),&stage.as_bytes().as_slice()]).await.map_err(map_db)
}
pub(crate) async fn read_local_inspection_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<Option<LocalInspectionCheckpoint>, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    read_row(tx, owner, job, stage)
        .await?
        .map(|row| parse(&row, owner, job, stage))
        .transpose()
}
async fn check_job_proof(
    tx: &Transaction<'_>,
    command: &LocalInspectionCommand,
) -> Result<(), ExecutionError> {
    let scope = &command.claim().scope;
    locked_job(tx, &scope.owner, &scope.job).await?;
    let row=tx.query_one("SELECT intent_revision,intent_digest,config_revision,config_digest FROM imagery.image_job WHERE job_id=$1",
        &[&scope.job.as_bytes().as_slice()]).await.map_err(map_db)?;
    if row.get::<_, i64>("intent_revision") != i64::from(command.intent_snapshot_revision())
        || row.get::<_, Vec<u8>>("intent_digest") != command.intent_digest().as_slice()
        || row.get::<_, i64>("config_revision") != i64::from(command.config_snapshot_revision())
        || row.get::<_, Vec<u8>>("config_digest") != command.config_digest().as_slice()
    {
        return Err(ExecutionError::BasisConflict);
    }
    Ok(())
}
fn replay_matches(row: &Row, command: &LocalInspectionCommand) -> Result<bool, ExecutionError> {
    Ok(
        row.get::<_, Vec<u8>>("claim_token") == command.claim().token.as_bytes()
            && row.get::<_, i64>("claim_generation")
                == invalid(i64::try_from(command.claim().generation))?
            && row.get::<_, Vec<u8>>("candidate_dispatch_id")
                == command.candidate_dispatch().as_bytes()
            && row.get::<_, Vec<u8>>("source_digest") == command.source_digest().as_slice()
            && row.get::<_, String>("reason") == command.reason().key()
            && row.get::<_, i64>("observed_at_ms") == command.observed_at_ms(),
    )
}
pub(crate) async fn record_local_inspection_in(
    tx: &Transaction<'_>,
    command: &LocalInspectionCommand,
) -> Result<LocalInspectionCheckpoint, ExecutionError> {
    require_read_committed(tx).await?;
    check_job_proof(tx, command).await?;
    let claim = command.claim();
    let scope = &claim.scope;
    if let Some(row) = read_row(tx, &scope.owner, &scope.job, &scope.stage).await? {
        return if replay_matches(&row, command)? {
            parse(&row, &scope.owner, &scope.job, &scope.stage)
        } else {
            Err(ExecutionError::OperationConflict)
        };
    }
    let row = locked_claim(
        tx,
        &scope.owner,
        &scope.job,
        &scope.stage,
        &claim.token,
        claim.generation,
        command.observed_at_ms(),
    )
    .await?;
    let basis =
        super::decision::read_stage_basis_in(tx, &scope.owner, &scope.job, &scope.stage).await?;
    if basis.kind != StageKind::Inspection
        || basis.parent_dispatch != Some(command.candidate_dispatch())
        || basis.config_digest != *command.config_digest()
    {
        return Err(ExecutionError::BasisConflict);
    }
    let parent =
        super::reads::read_dispatch_in(tx, &scope.owner, &command.candidate_dispatch()).await?;
    if parent.job != scope.job {
        return Err(ExecutionError::NotFound);
    }
    let candidate = super::candidate::read_candidate_reference_in(
        tx,
        &scope.owner,
        &scope.job,
        &parent.stage,
        &parent.dispatch,
    )
    .await?
    .ok_or(ExecutionError::ObservationMissing)?;
    if candidate.basis.config_revision != basis.config_revision
        || candidate.basis.config_digest != basis.config_digest
    {
        return Err(ExecutionError::BasisConflict);
    }
    let sources = crate::imagery_runtime::sources_digest_in(tx, &candidate.scope)
        .await
        .map_err(|_| ExecutionError::Unavailable)?
        .ok_or(ExecutionError::ObservationMissing)?;
    if sources != *command.source_digest() {
        return Err(ExecutionError::BasisConflict);
    }
    if tx
        .query_opt(
            "SELECT 1 FROM imagery.image_dispatch_attempt WHERE job_id=$1 AND stage_id=$2 LIMIT 1",
            &[
                &scope.job.as_bytes().as_slice(),
                &scope.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .is_some()
        || super::deferral::has_stage_finance(tx, &scope.owner, &scope.job, &scope.stage).await?
    {
        return Err(ExecutionError::Superseded);
    }
    if !row.get::<_, bool>("capacity_reserved") {
        return Err(ExecutionError::InvalidRecord);
    }
    let checkpoint = LocalInspectionCheckpoint {
        scope: scope.clone(),
        basis,
        candidate_scope: candidate.scope,
        candidate_observation_version: candidate.observation_version,
        candidate_usage_revision: candidate.receipt.usage_revision,
        source_digest: sources,
        reason: command.reason(),
        observed_at_ms: command.observed_at_ms(),
        schema_revision: 1,
        validator_revision: 1,
    };
    insert_checkpoint(tx, command, &checkpoint).await?;
    release_stage(tx, command, &row.get::<_, String>("capacity_scope")).await?;
    bump_job_revision(tx, &scope.job, command.observed_at_ms()).await?;
    Ok(checkpoint)
}
async fn insert_checkpoint(
    tx: &Transaction<'_>,
    command: &LocalInspectionCommand,
    checkpoint: &LocalInspectionCheckpoint,
) -> Result<(), ExecutionError> {
    let s = &checkpoint.scope;
    let b = &checkpoint.basis;
    let c = &checkpoint.candidate_scope;
    tx.execute("INSERT INTO imagery.image_local_inspection(job_id,stage_id,owner_scope,input_revision,input_digest,config_revision,config_digest, \
        parent_dispatch,candidate_stage_id,candidate_dispatch_id,candidate_observation_version,candidate_usage_revision,source_digest,reason, \
        observed_at_ms,schema_revision,validator_revision,claim_token,claim_generation,intent_snapshot_revision,intent_digest,config_snapshot_revision) \
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,1,1,$16,$17,$18,$19,$20)",
        &[&s.job.as_bytes().as_slice(),&s.stage.as_bytes().as_slice(),&owner_scope_bytes(&s.owner),&i64::from(b.input_revision),
        &b.input_digest.as_slice(),&invalid(i64::try_from(b.config_revision))?,&b.config_digest.as_slice(),&c.dispatch.as_bytes().as_slice(),
        &c.stage.as_bytes().as_slice(),&c.dispatch.as_bytes().as_slice(),&i64::from(checkpoint.candidate_observation_version),
        &invalid(i32::try_from(checkpoint.candidate_usage_revision))?,&checkpoint.source_digest.as_slice(),&checkpoint.reason.key(),
        &checkpoint.observed_at_ms,&command.claim().token.as_bytes().as_slice(),&invalid(i64::try_from(command.claim().generation))?,
        &i64::from(command.intent_snapshot_revision()),&command.intent_digest().as_slice(),&i64::from(command.config_snapshot_revision())])
        .await.map_err(map_db)?;
    Ok(())
}
async fn release_stage(
    tx: &Transaction<'_>,
    command: &LocalInspectionCommand,
    capacity_scope: &str,
) -> Result<(), ExecutionError> {
    let s = &command.claim().scope;
    if tx.execute("UPDATE imagery.image_execution_capacity SET in_flight=in_flight-1,updated_at_ms=$2 WHERE capacity_scope=$1 AND in_flight>0",
        &[&capacity_scope,&command.observed_at_ms()]).await.map_err(map_db)?!=1 {return Err(ExecutionError::InvalidRecord)}
    tx.execute("UPDATE imagery.image_execution_stage SET stage_state='released',capacity_reserved=false,lease_until_ms=0, \
        claim_generation=claim_generation+1,updated_at_ms=$3 WHERE job_id=$1 AND stage_id=$2",
        &[&s.job.as_bytes().as_slice(),&s.stage.as_bytes().as_slice(),&command.observed_at_ms()]).await.map_err(map_db)?;
    Ok(())
}
