//! Immutable resolved execution selection retains the original admission allocation.

use super::{
    decision_binding, local_inspections,
    rows::{database_error, snapshot},
};
use df_imagery::execution::{CandidateReceipt, DispatchId, ExecutionStageId, StageKind};
use df_imagery::publication::{
    FinalDecisionRef, FinalOutputSelection, FrozenOutput, PublicationError, recipe_digest,
};
use df_imagery::runtime::{PublicationContext, QualifiedPipeline};
use df_imagery::{ImageJobId, ImageOwner, VersionedSnapshot, owner_scope_bytes};
use sha2::{Digest, Sha256};
use tokio_postgres::{Row, Transaction};

pub(super) struct StoredSelection {
    pub allocation_stage: ExecutionStageId,
    pub final_stage: ExecutionStageId,
    pub final_dispatch: DispatchId,
    pub recipe: VersionedSnapshot,
    pub source_binding: Vec<u8>,
    pub binding: Vec<u8>,
    pub candidate: Option<CandidateReceipt>,
    pub decisions: Vec<FinalDecisionRef>,
    pub source_candidates: Vec<CandidateReceipt>,
    pub local_inspections: Vec<df_imagery::execution::LocalInspectionCheckpoint>,
}

impl StoredSelection {
    pub(super) fn check_qualified(
        &self,
        job: &df_imagery::ImageJob,
        proof: Option<&QualifiedPipeline>,
    ) -> Result<(), PublicationError> {
        check_qualification(
            job,
            self.candidate.as_ref(),
            &self.decisions,
            &self.source_candidates,
            &self.local_inspections,
            proof,
        )
    }
}

fn check_qualification(
    job: &df_imagery::ImageJob,
    candidate: Option<&CandidateReceipt>,
    decisions: &[FinalDecisionRef],
    sources: &[CandidateReceipt],
    locals: &[df_imagery::execution::LocalInspectionCheckpoint],
    proof: Option<&QualifiedPipeline>,
) -> Result<(), PublicationError> {
    let valid = match (candidate, proof) {
        (Some(candidate), Some(proof)) => proof.matches(job, candidate, decisions, sources, locals),
        (None, None) => decisions.is_empty() && sources.is_empty() && locals.is_empty(),
        _ => false,
    };
    valid.then_some(()).ok_or(PublicationError::BindingConflict)
}

pub(super) async fn has_supplier_evidence_in(
    tx: &Transaction<'_>,
    command: &super::PublicationCommand<'_>,
    stage: &Row,
) -> Result<bool, PublicationError> {
    if read_in(tx, command.owner, &command.job).await?.is_some() {
        return tx.query_one("SELECT candidate_dispatch_id IS NOT NULL FROM imagery.image_final_selection WHERE job_id=$1 AND owner_scope=$2", &[&command.job.as_bytes().as_slice(),&owner_scope_bytes(command.owner)]).await.map_err(database_error).map(|row|row.get(0));
    }
    if !matches!(
        stage.get::<_, String>("stage_kind").as_str(),
        "generation" | "edit" | "correction" | "extraction"
    ) {
        return Ok(false);
    }
    let exists: bool = tx
        .query_one(
            "SELECT to_regclass('imagery.image_dispatch_usage') IS NOT NULL",
            &[],
        )
        .await
        .map_err(database_error)?
        .get(0);
    if !exists {
        return Ok(false);
    }
    let usage = stage.get::<_, i32>("usage_revision");
    tx.query_one("SELECT EXISTS(SELECT 1 FROM imagery.image_dispatch_usage WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND dispatch_id=$4 AND revision=$5 AND evidence<>'unknown' AND usable AND raw_output IS NOT NULL AND octet_length(raw_output)>0)", &[&owner_scope_bytes(command.owner),&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice(),&command.dispatch.as_bytes().as_slice(),&i64::from(usage)]).await.map_err(database_error).map(|row|row.get(0))
}

fn append(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), PublicationError> {
    let length = u64::try_from(value.len()).map_err(|_| PublicationError::InvalidRecord)?;
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

pub(super) fn candidate_binding(candidate: &CandidateReceipt) -> Result<Vec<u8>, PublicationError> {
    let mut bytes = b"df-imagery/candidate-selection-v1".to_vec();
    append(&mut bytes, &owner_scope_bytes(&candidate.scope.owner))?;
    append(&mut bytes, candidate.scope.job.as_bytes())?;
    append(&mut bytes, candidate.scope.stage.as_bytes())?;
    append(&mut bytes, candidate.scope.dispatch.as_bytes())?;
    append(&mut bytes, candidate.basis.kind.key().as_bytes())?;
    bytes.extend_from_slice(&candidate.basis.input_revision.to_be_bytes());
    bytes.extend_from_slice(&candidate.basis.input_digest);
    bytes.extend_from_slice(&candidate.basis.config_revision.to_be_bytes());
    bytes.extend_from_slice(&candidate.basis.config_digest);
    if let Some(parent) = &candidate.basis.parent_dispatch {
        append(&mut bytes, parent.as_bytes())?;
    } else {
        append(&mut bytes, &[])?;
    }
    bytes.extend_from_slice(&candidate.observation_version.to_be_bytes());
    bytes.extend_from_slice(&candidate.receipt.usage_revision.to_be_bytes());
    bytes.extend_from_slice(&candidate.receipt.observed_at_ms.to_be_bytes());
    append(&mut bytes, candidate.descriptor.mime.as_bytes())?;
    bytes.extend_from_slice(&candidate.descriptor.width_px.to_be_bytes());
    bytes.extend_from_slice(&candidate.descriptor.height_px.to_be_bytes());
    bytes.push(match candidate.descriptor.alpha {
        df_imagery::execution::CheckpointAlpha::Opaque => 0,
        df_imagery::execution::CheckpointAlpha::Transparent => 1,
    });
    bytes.extend_from_slice(&candidate.descriptor.byte_len.to_be_bytes());
    bytes.extend_from_slice(&candidate.descriptor.sha256);
    bytes.extend_from_slice(&candidate.descriptor.validator_revision.to_be_bytes());
    Ok(bytes)
}

async fn check_candidate(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
    expected: &[u8],
) -> Result<CandidateReceipt, PublicationError> {
    let candidate =
        crate::imagery_execution::read_candidate_checkpoint_in(tx, owner, job, stage, dispatch)
            .await
            .map_err(PublicationError::Execution)?
            .ok_or(PublicationError::BindingConflict)?;
    let stored=tx.query_one("SELECT j.config_bytes,c.current_dispatch FROM imagery.image_job j JOIN imagery.image_stage_cursor c USING(job_id) WHERE j.job_id=$1 AND j.owner_scope=$2 AND c.stage_id=$3 FOR UPDATE OF c", &[&job.as_bytes().as_slice(),&owner_scope_bytes(owner),&stage.as_bytes().as_slice()]).await.map_err(database_error)?;
    if candidate_binding(&candidate)? != expected
        || candidate.basis.config_digest
            != <[u8; 32]>::from(Sha256::digest(stored.get::<_, Vec<u8>>("config_bytes")))
        || stored.get::<_, Vec<u8>>("current_dispatch") != dispatch.as_bytes()
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(candidate)
}

pub(super) async fn read_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<StoredSelection>, PublicationError> {
    let Some(row) = tx
        .query_opt(
            "SELECT * FROM imagery.image_final_selection WHERE owner_scope=$1 AND job_id=$2",
            &[&owner_scope_bytes(owner), &job.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?
    else {
        return Ok(None);
    };
    let recipe = snapshot(&row, "resolved_recipe_revision", "resolved_recipe_bytes")?;
    check_stored(tx, owner, job, &row, &recipe).await?;
    FrozenOutput::decode(&recipe)?;
    if row.get::<_, Vec<u8>>("resolved_recipe_digest") != recipe_digest(&recipe) {
        return Err(PublicationError::BindingConflict);
    }
    let candidate_stage = row.get::<_, Option<Vec<u8>>>("candidate_stage_id");
    let candidate_dispatch = row.get::<_, Option<Vec<u8>>>("candidate_dispatch_id");
    let candidate_bytes = row.get::<_, Option<Vec<u8>>>("candidate_binding");
    let candidate = if let (Some(stage), Some(dispatch), Some(binding)) =
        (&candidate_stage, &candidate_dispatch, &candidate_bytes)
    {
        Some(
            check_candidate(
                tx,
                owner,
                job,
                &ExecutionStageId::from_bytes(stage)
                    .map_err(|_| PublicationError::InvalidRecord)?,
                &DispatchId::from_bytes(dispatch).map_err(|_| PublicationError::InvalidRecord)?,
                binding,
            )
            .await?,
        )
    } else if candidate_stage.is_some() || candidate_dispatch.is_some() || candidate_bytes.is_some()
    {
        return Err(PublicationError::InvalidRecord);
    } else {
        None
    };
    let canonical_job = crate::read_in(tx, owner, *job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    let decisions = decision_binding::read_refs_in(
        tx,
        &canonical_job,
        &row.get::<_, String>("decision_binding"),
    )
    .await?;
    let source_candidates = if let Some(candidate) = &candidate {
        decision_binding::verify_in(tx, &canonical_job, candidate, &decisions).await?
    } else if decisions.is_empty() {
        vec![]
    } else {
        return Err(PublicationError::BindingConflict);
    };
    let local_inspections = local_inspections::read_in(
        tx,
        &canonical_job,
        &row.get::<_, String>("local_inspection_binding"),
    )
    .await?;
    local_inspections::verify_in(tx, &canonical_job, &local_inspections, &source_candidates)
        .await?;
    let mut binding = b"df-imagery/final-selection-v1".to_vec();
    for field in [
        "allocation_stage_id",
        "final_stage_id",
        "final_dispatch_id",
        "asset_id",
        "publication_attempt",
        "plan_recipe_digest",
        "resolved_recipe_digest",
        "source_binding",
    ] {
        append(&mut binding, &row.get::<_, Vec<u8>>(field))?;
    }
    if let Some(candidate) = candidate_bytes {
        append(&mut binding, &candidate)?;
    }
    append(
        &mut binding,
        row.get::<_, String>("decision_binding").as_bytes(),
    )?;
    append(
        &mut binding,
        row.get::<_, String>("local_inspection_binding").as_bytes(),
    )?;
    Ok(Some(StoredSelection {
        allocation_stage: ExecutionStageId::from_bytes(
            &row.get::<_, Vec<u8>>("allocation_stage_id"),
        )
        .map_err(|_| PublicationError::InvalidRecord)?,
        final_stage: ExecutionStageId::from_bytes(&row.get::<_, Vec<u8>>("final_stage_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        final_dispatch: DispatchId::from_bytes(&row.get::<_, Vec<u8>>("final_dispatch_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        recipe,
        source_binding: row.get("source_binding"),
        binding,
        candidate,
        decisions,
        source_candidates,
        local_inspections,
    }))
}

async fn check_stored(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job_id: &ImageJobId,
    row: &Row,
    recipe: &VersionedSnapshot,
) -> Result<(), PublicationError> {
    let job = tx
        .query_opt(
            "SELECT * FROM imagery.image_job WHERE job_id=$1 AND owner_scope=$2 FOR UPDATE",
            &[&job_id.as_bytes().as_slice(), &owner_scope_bytes(owner)],
        )
        .await
        .map_err(database_error)?
        .ok_or(PublicationError::NotFound)?;
    let plan = snapshot(&job, "recipe_revision", "recipe_bytes")?;
    let planned = FrozenOutput::decode(&plan)?;
    let resolved = FrozenOutput::decode(recipe)?;
    if row.get::<_, Vec<u8>>("plan_recipe_digest") != recipe_digest(&plan)
        || job
            .get::<_, Option<Vec<u8>>>("planned_output_stage")
            .as_deref()
            != Some(row.get::<_, Vec<u8>>("allocation_stage_id").as_slice())
        || job
            .get::<_, Option<Vec<u8>>>("planned_publication_asset")
            .as_deref()
            != Some(row.get::<_, Vec<u8>>("asset_id").as_slice())
        || job
            .get::<_, Option<Vec<u8>>>("planned_publication_attempt")
            .as_deref()
            != Some(row.get::<_, Vec<u8>>("publication_attempt").as_slice())
        || [
            df_assets::AssetVariant::Master,
            df_assets::AssetVariant::Display,
        ]
        .into_iter()
        .any(|variant| planned.intent(variant) != resolved.intent(variant))
    {
        return Err(PublicationError::BindingConflict);
    }
    let stage = ExecutionStageId::from_bytes(&row.get::<_, Vec<u8>>("final_stage_id"))
        .map_err(|_| PublicationError::InvalidRecord)?;
    let basis = crate::imagery_execution::read_stage_basis_in(tx, owner, job_id, &stage)
        .await
        .map_err(PublicationError::Execution)?;
    let anchor = ExecutionStageId::from_bytes(&row.get::<_, Vec<u8>>("allocation_stage_id"))
        .map_err(|_| PublicationError::InvalidRecord)?;
    let anchor_basis = crate::imagery_execution::read_stage_basis_in(tx, owner, job_id, &anchor)
        .await
        .map_err(PublicationError::Execution)?;
    let candidate = row
        .get::<_, Option<Vec<u8>>>("candidate_dispatch_id")
        .map(|bytes| DispatchId::from_bytes(&bytes).map_err(|_| PublicationError::InvalidRecord))
        .transpose()?;
    if basis.kind != StageKind::LocalTransform
        || basis.input_revision != recipe.revision()
        || basis.input_digest != recipe_digest(recipe)
        || basis.config_revision != anchor_basis.config_revision
        || basis.config_digest != anchor_basis.config_digest
        || basis.config_digest
            != <[u8; 32]>::from(Sha256::digest(job.get::<_, Vec<u8>>("config_bytes")))
        || basis.parent_dispatch != candidate
        || (candidate.is_none()
            && row.get::<_, Vec<u8>>("allocation_stage_id")
                != row.get::<_, Vec<u8>>("final_stage_id"))
    {
        return Err(PublicationError::BindingConflict);
    }
    let current=tx.query_opt("SELECT s.local_accounting AS stage_local,d.local_accounting,d.finance_attempt,c.current_dispatch FROM imagery.image_execution_stage s JOIN imagery.image_stage_cursor c USING(job_id,stage_id) JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=c.current_dispatch WHERE s.job_id=$1 AND s.stage_id=$2 FOR UPDATE OF s,c,d", &[&job_id.as_bytes().as_slice(),&stage.as_bytes().as_slice()]).await.map_err(database_error)?.ok_or(PublicationError::BindingConflict)?;
    if !current.get::<_, bool>("stage_local")
        || !current.get::<_, bool>("local_accounting")
        || current
            .get::<_, Option<Vec<u8>>>("finance_attempt")
            .is_some()
        || current.get::<_, Vec<u8>>("current_dispatch")
            != row.get::<_, Vec<u8>>("final_dispatch_id")
    {
        return Err(PublicationError::BindingConflict);
    }
    let allocation=tx.query_opt("SELECT * FROM imagery.image_publication_allocation WHERE job_id=$1 AND owner_scope=$2 AND stage_id=$3 AND asset_id=$4 AND publication_attempt=$5", &[&job_id.as_bytes().as_slice(),&owner_scope_bytes(owner),&row.get::<_,Vec<u8>>("allocation_stage_id"),&row.get::<_,Vec<u8>>("asset_id"),&row.get::<_,Vec<u8>>("publication_attempt")]).await.map_err(database_error)?.ok_or(PublicationError::BindingConflict)?;
    if allocation.get::<_, Vec<u8>>("original_recipe_bytes") != plan.bytes()
        || allocation.get::<_, i64>("original_recipe_revision") != i64::from(plan.revision())
        || allocation.get::<_, Vec<u8>>("original_recipe_key")
            != job
                .get::<_, Option<Vec<u8>>>("resolved_recipe_key")
                .ok_or(PublicationError::InvalidRecipe)?
        || allocation.get::<_, Vec<u8>>("original_config_bytes")
            != job.get::<_, Vec<u8>>("config_bytes")
        || allocation.get::<_, i64>("original_config_revision")
            != job.get::<_, i64>("config_revision")
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(())
}

pub(super) async fn select_in(
    tx: &Transaction<'_>,
    context: &PublicationContext,
    selected: &FinalOutputSelection,
    source_binding: &[u8],
    qualified: Option<&QualifiedPipeline>,
    now_ms: i64,
) -> Result<(), PublicationError> {
    super::rows::require_read_committed(tx).await?;
    FrozenOutput::decode(&selected.resolved_recipe)?;
    let job = tx
        .query_opt(
            "SELECT * FROM imagery.image_job WHERE job_id=$1 AND owner_scope=$2 FOR UPDATE",
            &[
                &context.job.job.as_bytes().as_slice(),
                &owner_scope_bytes(&context.job.owner),
            ],
        )
        .await
        .map_err(database_error)?
        .ok_or(PublicationError::NotFound)?;
    check_context(tx, context, selected, &job, now_ms).await?;
    super::rows::require_delivery_open_in(tx, &context.job.owner, &context.job.job).await?;
    let plan = snapshot(&job, "recipe_revision", "recipe_bytes")?;
    let candidate = selected
        .candidate
        .as_ref()
        .map(candidate_binding)
        .transpose()?;
    if let Some(receipt) = &selected.candidate {
        if receipt.scope.owner != context.job.owner || receipt.scope.job != context.job.job {
            return Err(PublicationError::BindingConflict);
        }
        check_candidate(
            tx,
            &context.job.owner,
            &context.job.job,
            &receipt.scope.stage,
            &receipt.scope.dispatch,
            candidate
                .as_deref()
                .ok_or(PublicationError::InvalidRecord)?,
        )
        .await?;
    }
    let canonical_job = crate::read_in(tx, &context.job.owner, context.job.job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    let source_candidates = if let Some(candidate) = &selected.candidate {
        decision_binding::verify_in(tx, &canonical_job, candidate, &selected.decisions).await?
    } else {
        vec![]
    };
    local_inspections::verify_in(
        tx,
        &canonical_job,
        &selected.local_inspections,
        &source_candidates,
    )
    .await?;
    check_qualification(
        &canonical_job,
        selected.candidate.as_ref(),
        &selected.decisions,
        &source_candidates,
        &selected.local_inspections,
        qualified,
    )?;
    let decisions = decision_binding::encode(&selected.decisions)?;
    let planned = context
        .stage
        .planned_publication
        .ok_or(PublicationError::BindingConflict)?;
    let local_inspection_binding = local_inspections::encode(&selected.local_inspections)?;
    let candidate_stage = selected
        .candidate
        .as_ref()
        .map(|receipt| receipt.scope.stage.as_bytes().to_vec());
    let candidate_dispatch = selected
        .candidate
        .as_ref()
        .map(|receipt| receipt.scope.dispatch.as_bytes().to_vec());
    let inserted=tx.execute("INSERT INTO imagery.image_final_selection(job_id,owner_scope,allocation_stage_id,final_stage_id,final_dispatch_id,asset_id,publication_attempt,plan_recipe_digest,resolved_recipe_revision,resolved_recipe_bytes,resolved_recipe_digest,candidate_stage_id,candidate_dispatch_id,candidate_binding,source_binding,selected_at_ms,decision_binding,local_inspection_binding) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18) ON CONFLICT(job_id) DO NOTHING",
        &[&context.job.job.as_bytes().as_slice(),&owner_scope_bytes(&context.job.owner),&selected.allocation_stage.as_bytes().as_slice(),&context.stage.stage.as_bytes().as_slice(),&context.dispatch.as_bytes().as_slice(),&planned.asset.as_slice(),&planned.attempt.as_slice(),&recipe_digest(&plan).as_slice(),&i64::from(selected.resolved_recipe.revision()),&selected.resolved_recipe.bytes(),&recipe_digest(&selected.resolved_recipe).as_slice(),&candidate_stage,&candidate_dispatch,&candidate,&source_binding,&now_ms,&decisions,&local_inspection_binding]).await.map_err(database_error)?;
    let row = tx
        .query_one(
            "SELECT * FROM imagery.image_final_selection WHERE job_id=$1",
            &[&context.job.job.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?;
    if row.get::<_, Vec<u8>>("allocation_stage_id") != selected.allocation_stage.as_bytes()
        || row.get::<_, Vec<u8>>("final_stage_id") != context.stage.stage.as_bytes()
        || row.get::<_, Vec<u8>>("final_dispatch_id") != context.dispatch.as_bytes()
        || row.get::<_, i64>("resolved_recipe_revision")
            != i64::from(selected.resolved_recipe.revision())
        || row.get::<_, Vec<u8>>("resolved_recipe_bytes") != selected.resolved_recipe.bytes()
        || row.get::<_, Option<Vec<u8>>>("candidate_binding") != candidate
        || row.get::<_, Vec<u8>>("source_binding") != source_binding
        || row.get::<_, String>("decision_binding") != decisions
        || row.get::<_, String>("local_inspection_binding") != local_inspection_binding
    {
        return Err(PublicationError::BindingConflict);
    }
    if inserted != 0 {
        tx.execute("UPDATE imagery.image_job SET revision=revision+1,updated_at_ms=GREATEST(updated_at_ms,$2) WHERE job_id=$1", &[&context.job.job.as_bytes().as_slice(),&now_ms]).await.map_err(database_error)?;
    }
    Ok(())
}

async fn check_context(
    tx: &Transaction<'_>,
    context: &PublicationContext,
    selected: &FinalOutputSelection,
    job: &Row,
    now_ms: i64,
) -> Result<(), PublicationError> {
    let plan = snapshot(job, "recipe_revision", "recipe_bytes")?;
    if job
        .get::<_, Option<Vec<u8>>>("planned_output_stage")
        .as_deref()
        != Some(selected.allocation_stage.as_bytes().as_slice())
        || context.job.recipe.as_ref() != Some(&plan)
        || job.get::<_, i64>("config_revision") != i64::from(context.job.config.revision())
        || job.get::<_, Vec<u8>>("config_bytes") != context.job.config.bytes()
    {
        return Err(PublicationError::BindingConflict);
    }
    let basis = crate::imagery_execution::read_stage_basis_in(
        tx,
        &context.job.owner,
        &context.job.job,
        &context.stage.stage,
    )
    .await
    .map_err(PublicationError::Execution)?;
    let anchor_basis = crate::imagery_execution::read_stage_basis_in(
        tx,
        &context.job.owner,
        &context.job.job,
        &selected.allocation_stage,
    )
    .await
    .map_err(PublicationError::Execution)?;
    if basis.kind != StageKind::LocalTransform
        || basis.input_revision != selected.resolved_recipe.revision()
        || basis.input_digest != recipe_digest(&selected.resolved_recipe)
        || basis.config_revision != anchor_basis.config_revision
        || basis.config_digest != anchor_basis.config_digest
        || basis.config_digest != <[u8; 32]>::from(Sha256::digest(context.job.config.bytes()))
        || basis.parent_dispatch
            != selected
                .candidate
                .as_ref()
                .map(|candidate| candidate.scope.dispatch)
        || (selected.candidate.is_none() && selected.allocation_stage != context.stage.stage)
    {
        return Err(PublicationError::BindingConflict);
    }
    let row=tx.query_opt("SELECT s.claim_token AS current_claim,s.claim_generation AS current_generation,s.lease_until_ms,s.local_accounting AS stage_local,d.*,c.current_dispatch FROM imagery.image_execution_stage s JOIN imagery.image_dispatch_attempt d USING(job_id,stage_id) JOIN imagery.image_stage_cursor c USING(job_id,stage_id) WHERE s.job_id=$1 AND s.stage_id=$2 AND d.dispatch_id=$3 FOR UPDATE OF s,d,c", &[&context.job.job.as_bytes().as_slice(),&context.stage.stage.as_bytes().as_slice(),&context.dispatch.as_bytes().as_slice()]).await.map_err(database_error)?.ok_or(PublicationError::NotFound)?;
    let generation = i64::try_from(context.stage.claim_generation)
        .map_err(|_| PublicationError::InvalidRecord)?;
    if !row.get::<_, bool>("stage_local")
        || !row.get::<_, bool>("local_accounting")
        || row.get::<_, Option<Vec<u8>>>("finance_attempt").is_some()
        || row.get::<_, Vec<u8>>("current_dispatch") != context.dispatch.as_bytes()
        || row.get::<_, Vec<u8>>("claim_token") != context.claim.as_bytes()
        || row.get::<_, i64>("claim_generation") != generation
    {
        return Err(PublicationError::BindingConflict);
    }
    if let Some(recovery) = &context.recovery {
        if recovery.stage.owner != context.job.owner
            || recovery.stage.job != context.job.job
            || recovery.stage.stage != context.stage.stage
            || recovery.stage.current.dispatch != context.dispatch
        {
            return Err(PublicationError::BindingConflict);
        }
        crate::imagery_execution::validate_recovery_in(tx, recovery, now_ms)
            .await
            .map_err(PublicationError::Execution)?;
    } else if row.get::<_, Vec<u8>>("current_claim") != context.claim.as_bytes()
        || row.get::<_, i64>("current_generation") != generation
        || row.get::<_, i64>("lease_until_ms") < now_ms
    {
        return Err(PublicationError::StaleClaim);
    }
    let planned = context
        .stage
        .planned_publication
        .ok_or(PublicationError::BindingConflict)?;
    let allocation=tx.query_opt("SELECT * FROM imagery.image_publication_allocation WHERE job_id=$1 AND owner_scope=$2 AND stage_id=$3 AND asset_id=$4 AND publication_attempt=$5", &[&context.job.job.as_bytes().as_slice(),&owner_scope_bytes(&context.job.owner),&selected.allocation_stage.as_bytes().as_slice(),&planned.asset.as_slice(),&planned.attempt.as_slice()]).await.map_err(database_error)?.ok_or(PublicationError::BindingConflict)?;
    if allocation.get::<_, Vec<u8>>("original_recipe_bytes") != plan.bytes()
        || allocation.get::<_, i64>("original_recipe_revision") != i64::from(plan.revision())
        || allocation.get::<_, Vec<u8>>("original_config_bytes") != context.job.config.bytes()
        || allocation.get::<_, i64>("original_config_revision")
            != i64::from(context.job.config.revision())
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(())
}
