//! Standalone analysis persists explicitly qualified results without publication or dispatch surrogates.
use super::{PostgresImageExecution, commit_execution, statements};
use crate::imagery_publication::PublicationSourceAuthority;
use df_imagery::execution::*;
use df_imagery::runtime::{QualifiedAnalysis, QualifiedLocalAnalysis};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use statements::common::{bump_job_revision, invalid, map_db, require_read_committed};
use tokio_postgres::{IsolationLevel, Transaction};

mod encoding;
mod rows;
mod verification;

pub(super) const MIGRATION_SQL: &str = include_str!("../../migrations/0046_imagery_analysis.sql");

fn shape(checkpoint: &AnalysisCheckpoint, now_ms: i64) -> Result<(), ExecutionError> {
    if now_ms <= 0
        || checkpoint.checkpoint_version == 0
        || checkpoint.schema_revision == 0
        || checkpoint.validator_revision == 0
        || !checkpoint.kind.stage_kind().is_standalone()
        || checkpoint.stages.is_empty()
    {
        return Err(ExecutionError::InvalidRecord);
    }
    Ok(())
}
async fn replay_final(
    tx: &Transaction<'_>,
    checkpoint: &AnalysisCheckpoint,
) -> Result<bool, ExecutionError> {
    let row=tx.query_opt("SELECT c.owner_scope,c.job_id,c.stage_id,c.decision_kind,c.checkpoint_version,c.schema_revision, \
        c.validator_revision,c.result_sha256,c.intent_digest,c.config_digest,c.source_digest,c.result_revision, \
        c.stages=$3::text::jsonb AS stages_match,c.dependencies=$4::text::jsonb AS dependencies_match \
        FROM imagery.image_analysis_checkpoint c WHERE c.owner_scope=$1 AND c.job_id=$2",
        &[&owner_scope_bytes(&checkpoint.scope.owner),&checkpoint.scope.job.as_bytes().as_slice(),
        &encoding::stages(&checkpoint.stages),&encoding::dependencies(&checkpoint.dependencies)]).await.map_err(map_db)?;
    let Some(row) = row else { return Ok(false) };
    if rows::reference(&row, &checkpoint.scope.owner)?
        != AnalysisCheckpointReference::from_checkpoint(checkpoint)
        || row.get::<_, i64>("result_revision") != i64::from(checkpoint.result.revision())
        || !row.get::<_, bool>("stages_match")
        || !row.get::<_, bool>("dependencies_match")
    {
        return Err(ExecutionError::OperationConflict);
    }
    Ok(true)
}
async fn insert_final(
    tx: &Transaction<'_>,
    checkpoint: &AnalysisCheckpoint,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let c = checkpoint;
    tx.execute("INSERT INTO imagery.image_analysis_checkpoint(job_id,stage_id,owner_scope,decision_kind,checkpoint_version, \
        schema_revision,validator_revision,result_revision,result,result_sha256,intent_digest,config_digest,source_digest, \
        stages,dependencies,selected_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14::text::jsonb,$15::text::jsonb,$16)",
        &[&c.scope.job.as_bytes().as_slice(),&c.scope.stage.as_bytes().as_slice(),&owner_scope_bytes(&c.scope.owner),
        &c.kind.label(),&i64::from(c.checkpoint_version),&i64::from(c.schema_revision),&i64::from(c.validator_revision),
        &i64::from(c.result.revision()),&c.result.bytes(),&c.result_sha256.as_slice(),&c.intent_digest.as_slice(),
        &c.config_digest.as_slice(),&c.source_digest.as_slice(),&encoding::stages(&c.stages),
        &encoding::dependencies(&c.dependencies),&now_ms]).await.map_err(map_db)?;
    Ok(())
}
/// Qualification and source catalog authorization precede execution locks; a matching selection replays read-only.
pub async fn select_analysis_checkpoint_in(
    tx: &Transaction<'_>,
    proof: &QualifiedAnalysis,
    authority: Option<&dyn PublicationSourceAuthority>,
    now_ms: i64,
) -> Result<AnalysisCheckpoint, ExecutionError> {
    require_read_committed(tx).await?;
    let c = proof.checkpoint();
    shape(c, now_ms)?;
    verification::sources(
        tx,
        authority,
        proof.job(),
        proof.audience(),
        proof.sources(),
        now_ms,
    )
    .await?;
    let (_, state) = verification::job(
        tx,
        proof.job(),
        &c.scope,
        &c.intent_digest,
        &c.config_digest,
    )
    .await?;
    if replay_final(tx, c).await? {
        return Ok(c.clone());
    }
    statements::check_output_open_in(tx, &c.scope.owner, &c.scope.job).await?;
    if state == "retired" {
        return Err(ExecutionError::Retired);
    }
    verification::bound(tx, &c.scope, c.result.bytes().len()).await?;
    verification::dependencies(tx, &c.scope, &c.dependencies).await?;
    verification::stages(tx, c).await?;
    insert_final(tx, c, now_ms).await?;
    bump_job_revision(tx, &c.scope.job, now_ms).await?;
    Ok(c.clone())
}
async fn local_replay(
    tx: &Transaction<'_>,
    proof: &QualifiedLocalAnalysis,
    claim: &StageClaim,
) -> Result<bool, ExecutionError> {
    let c = proof.checkpoint();
    let row=tx.query_opt("SELECT c.owner_scope,c.job_id,c.stage_id,c.decision_kind,c.checkpoint_version,c.schema_revision, \
        c.validator_revision,c.input_revision,c.input_digest,c.config_revision,c.config_digest,c.parent_dispatch,c.result_revision, \
        c.result_sha256,c.intent_digest,c.source_digest,c.claim_token,c.claim_generation, \
        c.dependencies=$4::text::jsonb AS dependencies_match FROM imagery.image_local_analysis c \
        WHERE c.owner_scope=$1 AND c.job_id=$2 AND c.stage_id=$3",
        &[&owner_scope_bytes(&c.scope.owner),&c.scope.job.as_bytes().as_slice(),&c.scope.stage.as_bytes().as_slice(),
        &encoding::dependencies(&c.dependencies)]).await.map_err(map_db)?;
    let Some(row) = row else { return Ok(false) };
    if rows::local_reference(&row, &c.scope.owner)? != LocalAnalysisReference::from_checkpoint(c)
        || row.get::<_, i64>("result_revision") != i64::from(c.result.revision())
        || !row.get::<_, bool>("dependencies_match")
        || row.get::<_, Vec<u8>>("claim_token") != claim.token.as_bytes()
        || row.get::<_, i64>("claim_generation") != invalid(i64::try_from(claim.generation))?
        || !replay_final(tx, proof.final_checkpoint()).await?
    {
        return Err(ExecutionError::OperationConflict);
    }
    Ok(true)
}
async fn insert_local(
    tx: &Transaction<'_>,
    proof: &QualifiedLocalAnalysis,
    claim: &StageClaim,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let c = proof.checkpoint();
    let b = &c.basis;
    tx.execute("INSERT INTO imagery.image_local_analysis(job_id,stage_id,owner_scope,decision_kind,checkpoint_version, \
        schema_revision,validator_revision,input_revision,input_digest,config_revision,config_digest,parent_dispatch,intent_digest, \
        source_digest,result_revision,result,result_sha256,dependencies,claim_token,claim_generation,recorded_at_ms) \
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18::text::jsonb,$19,$20,$21)",
        &[&c.scope.job.as_bytes().as_slice(),&c.scope.stage.as_bytes().as_slice(),&owner_scope_bytes(&c.scope.owner),
        &c.kind.label(),&i64::from(c.checkpoint_version),&i64::from(c.schema_revision),&i64::from(c.validator_revision),
        &i64::from(b.input_revision),&b.input_digest.as_slice(),&invalid(i64::try_from(b.config_revision))?,&c.config_digest.as_slice(),
        &b.parent_dispatch.map(|d|d.as_bytes().to_vec()),&c.intent_digest.as_slice(),&c.source_digest.as_slice(),
        &i64::from(c.result.revision()),&c.result.bytes(),&c.result_sha256.as_slice(),&encoding::dependencies(&c.dependencies),
        &claim.token.as_bytes().as_slice(),&invalid(i64::try_from(claim.generation))?,&now_ms]).await.map_err(map_db)?;
    Ok(())
}
fn local_final(proof: &QualifiedLocalAnalysis) -> Result<(), ExecutionError> {
    let c = proof.checkpoint();
    let f = proof.final_checkpoint();
    if f.scope != c.scope
        || f.kind != c.kind
        || f.result_sha256 != c.result_sha256
        || f.result != c.result
        || f.schema_revision != c.schema_revision
        || f.validator_revision != c.validator_revision
        || f.intent_digest != c.intent_digest
        || f.config_digest != c.config_digest
        || f.source_digest != c.source_digest
        || f.dependencies != c.dependencies
        || f.stages
            != vec![AnalysisStageReference::Local(
                LocalAnalysisReference::from_checkpoint(c),
            )]
    {
        return Err(ExecutionError::BasisConflict);
    }
    Ok(())
}
/// Records deterministic no-call evidence and its explicit final selection atomically; there is no recovery dispatch.
pub async fn record_local_analysis_in(
    tx: &Transaction<'_>,
    proof: &QualifiedLocalAnalysis,
    claim: &StageClaim,
    authority: Option<&dyn PublicationSourceAuthority>,
    now_ms: i64,
) -> Result<LocalAnalysisCheckpoint, ExecutionError> {
    require_read_committed(tx).await?;
    let c = proof.checkpoint();
    shape(proof.final_checkpoint(), now_ms)?;
    local_final(proof)?;
    if c.scope != claim.scope
        || c.checkpoint_version == 0
        || c.schema_revision == 0
        || c.validator_revision == 0
    {
        return Err(ExecutionError::BasisConflict);
    }
    verification::sources(
        tx,
        authority,
        proof.job(),
        proof.audience(),
        proof.sources(),
        now_ms,
    )
    .await?;
    let (deadline, state) = verification::job(
        tx,
        proof.job(),
        &c.scope,
        &c.intent_digest,
        &c.config_digest,
    )
    .await?;
    if local_replay(tx, proof, claim).await? {
        return Ok(c.clone());
    }
    statements::check_output_open_in(tx, &c.scope.owner, &c.scope.job).await?;
    if state == "retired" {
        return Err(ExecutionError::Retired);
    }
    if now_ms >= deadline {
        return Err(ExecutionError::Expired);
    }
    let row = verification::local_claim(tx, c, claim, now_ms).await?;
    verification::bound(tx, &c.scope, c.result.bytes().len()).await?;
    verification::dependencies(tx, &c.scope, &c.dependencies).await?;
    insert_local(tx, proof, claim, now_ms).await?;
    let capacity: String = row.get("capacity_scope");
    let changed=tx.execute("UPDATE imagery.image_execution_capacity SET in_flight=in_flight-1,updated_at_ms=$2 WHERE capacity_scope=$1 AND in_flight>0",
        &[&capacity,&now_ms]).await.map_err(map_db)?;
    if changed != 1 {
        return Err(ExecutionError::InvalidRecord);
    }
    tx.execute("UPDATE imagery.image_execution_stage SET stage_state='released',capacity_reserved=false,lease_until_ms=0, \
        claim_generation=claim_generation+1,updated_at_ms=$3 WHERE job_id=$1 AND stage_id=$2",
        &[&c.scope.job.as_bytes().as_slice(),&c.scope.stage.as_bytes().as_slice(),&now_ms]).await.map_err(map_db)?;
    verification::stages(tx, proof.final_checkpoint()).await?;
    insert_final(tx, proof.final_checkpoint(), now_ms).await?;
    bump_job_revision(tx, &c.scope.job, now_ms).await?;
    Ok(c.clone())
}
pub async fn read_analysis_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<AnalysisCheckpoint>, ExecutionError> {
    require_read_committed(tx).await?;
    rows::read_final(tx, owner, job).await
}
pub async fn read_local_analysis_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<Option<LocalAnalysisCheckpoint>, ExecutionError> {
    require_read_committed(tx).await?;
    rows::read_local(
        tx,
        &StageScope {
            owner: owner.clone(),
            job: *job,
            stage: *stage,
        },
    )
    .await
}
pub(super) async fn select(
    store: &PostgresImageExecution,
    proof: &QualifiedAnalysis,
    now_ms: i64,
) -> Result<AnalysisCheckpoint, ExecutionError> {
    let mut slot = store
        .pool
        .checkout()
        .await
        .map_err(|_| ExecutionError::Unavailable)?;
    let client = slot.client.as_mut().ok_or(ExecutionError::Unavailable)?;
    let tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .await
        .map_err(map_db)?;
    let result =
        select_analysis_checkpoint_in(&tx, proof, store.source_authority.as_deref(), now_ms)
            .await?;
    commit_execution(tx).await?;
    Ok(result)
}
pub(super) async fn record_local(
    store: &PostgresImageExecution,
    proof: &QualifiedLocalAnalysis,
    claim: &StageClaim,
    now_ms: i64,
) -> Result<LocalAnalysisCheckpoint, ExecutionError> {
    let mut slot = store
        .pool
        .checkout()
        .await
        .map_err(|_| ExecutionError::Unavailable)?;
    let client = slot.client.as_mut().ok_or(ExecutionError::Unavailable)?;
    let tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .await
        .map_err(map_db)?;
    let result =
        record_local_analysis_in(&tx, proof, claim, store.source_authority.as_deref(), now_ms)
            .await?;
    commit_execution(tx).await?;
    Ok(result)
}
