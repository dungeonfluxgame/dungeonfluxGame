//! Cheap lineage checks under execution locks; semantic decoding belongs to opaque runtime proofs.
use super::{encoding, rows};
use crate::imagery_execution::statements;
use crate::imagery_publication::{
    PublicationSourceAuthority, lock_image_sources_in, matches_source_receipt,
};
use df_imagery::execution::*;
use df_imagery::publication::PublicationError;
use df_imagery::runtime::SourceReceipt;
use df_imagery::{Audience, ImageJob, owner_scope_bytes};
use statements::common::{
    attached_interest_count, invalid, locked_job, map_db, require_gate_revision,
};
use std::collections::BTreeSet;
use tokio_postgres::{Row, Transaction};

fn authority_error(error: PublicationError) -> ExecutionError {
    match error {
        PublicationError::Unavailable => ExecutionError::Unavailable,
        PublicationError::Unknown => ExecutionError::Unknown,
        PublicationError::Execution(error) => error,
        PublicationError::Catalog(df_assets::CatalogError::Unavailable) => {
            ExecutionError::Unavailable
        }
        PublicationError::Catalog(df_assets::CatalogError::Unknown) => ExecutionError::Unknown,
        _ => ExecutionError::SourceDenied,
    }
}
pub(super) async fn sources(
    tx: &Transaction<'_>,
    authority: Option<&dyn PublicationSourceAuthority>,
    job: &ImageJob,
    audience: &Audience,
    receipts: &[SourceReceipt],
    now_ms: i64,
) -> Result<(), ExecutionError> {
    if receipts.is_empty() {
        return Ok(());
    }
    let authority = authority.ok_or(ExecutionError::SourceDenied)?;
    authority
        .lock_authority(tx, &job.owner, now_ms)
        .await
        .map_err(authority_error)?;
    let pins = receipts.iter().map(|r| r.pin.clone()).collect::<Vec<_>>();
    let facts = lock_image_sources_in(tx, &pins, audience)
        .await
        .map_err(authority_error)?;
    if facts.len() != receipts.len()
        || facts
            .iter()
            .zip(receipts)
            .any(|(f, r)| !matches_source_receipt(f, r))
    {
        return Err(ExecutionError::SourceDenied);
    }
    authority
        .authorize_sources(tx, job, &facts, now_ms)
        .await
        .map_err(authority_error)
}
pub(super) async fn job(
    tx: &Transaction<'_>,
    proof: &ImageJob,
    scope: &StageScope,
    intent: &[u8; 32],
    config: &[u8; 32],
) -> Result<(i64, String), ExecutionError> {
    if proof.owner != scope.owner || proof.job != scope.job {
        return Err(ExecutionError::NotFound);
    }
    let (_, _, deadline, state, asset, attempt, _) =
        locked_job(tx, &scope.owner, &scope.job).await?;
    if asset.is_some() || attempt.is_some() {
        return Err(ExecutionError::PlannedIdentityConflict);
    }
    let row=tx.query_one("SELECT intent_digest,config_digest,intent_revision,config_revision FROM imagery.image_job WHERE job_id=$1",
        &[&scope.job.as_bytes().as_slice()]).await.map_err(map_db)?;
    if row.get::<_, Vec<u8>>("intent_digest") != intent.as_slice()
        || row.get::<_, Vec<u8>>("config_digest") != config.as_slice()
        || row.get::<_, i64>("intent_revision") != i64::from(proof.intent.revision())
        || row.get::<_, i64>("config_revision") != i64::from(proof.config.revision())
    {
        return Err(ExecutionError::BasisConflict);
    }
    Ok((deadline, state))
}
pub(super) async fn dependencies(
    tx: &Transaction<'_>,
    scope: &StageScope,
    values: &[AnalysisDependencyRef],
) -> Result<(), ExecutionError> {
    let mut operations = BTreeSet::new();
    for value in values {
        let r = &value.checkpoint;
        if value.handle.owner != scope.owner
            || r.scope.owner != scope.owner
            || r.scope.job == scope.job
            || !operations.insert(*value.handle.operation.as_bytes())
        {
            return Err(ExecutionError::SourceDenied);
        }
        let row=tx.query_opt("SELECT c.owner_scope,c.job_id,c.stage_id,c.decision_kind,c.checkpoint_version,c.schema_revision, \
            c.validator_revision,c.result_sha256,c.intent_digest,c.config_digest,c.source_digest \
            FROM imagery.image_analysis_checkpoint c JOIN imagery.image_operation o ON o.job_id=c.job_id AND o.owner_scope=c.owner_scope \
            WHERE o.owner_scope=$1 AND o.operation_id=$2 AND c.job_id=$3",
            &[&owner_scope_bytes(&value.handle.owner),&value.handle.operation.as_bytes().as_slice(),&r.scope.job.as_bytes().as_slice()])
            .await.map_err(map_db)?.ok_or(ExecutionError::ObservationMissing)?;
        if rows::reference(&row, &scope.owner)? != *r {
            return Err(ExecutionError::BasisConflict);
        }
    }
    Ok(())
}
pub(super) async fn local_claim(
    tx: &Transaction<'_>,
    checkpoint: &LocalAnalysisCheckpoint,
    claim: &StageClaim,
    now_ms: i64,
) -> Result<Row, ExecutionError> {
    if checkpoint.scope != claim.scope
        || checkpoint.basis.kind != checkpoint.kind.stage_kind()
        || checkpoint.basis.config_digest != checkpoint.config_digest
        || !checkpoint.kind.stage_kind().is_standalone()
    {
        return Err(ExecutionError::BasisConflict);
    }
    let scope = &claim.scope;
    let row = statements::common::locked_claim(
        tx,
        &scope.owner,
        &scope.job,
        &scope.stage,
        &claim.token,
        claim.generation,
        now_ms,
    )
    .await?;
    if attached_interest_count(tx, &scope.job).await? == 0 {
        return Err(ExecutionError::DemandLapsed);
    }
    let capacity: String = row.get("capacity_scope");
    require_gate_revision(
        tx,
        &capacity,
        invalid(u64::try_from(row.get::<_, i64>("claim_config_revision")))?,
    )
    .await?;
    let basis = statements::read_stage_basis_in(tx, &scope.owner, &scope.job, &scope.stage).await?;
    if basis != checkpoint.basis {
        return Err(ExecutionError::BasisConflict);
    }
    if !row.get::<_, bool>("capacity_reserved") {
        return Err(ExecutionError::InvalidRecord);
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
        || statements::has_stage_finance(tx, &scope.owner, &scope.job, &scope.stage).await?
    {
        return Err(ExecutionError::Superseded);
    }
    Ok(row)
}
pub(super) async fn bound(
    tx: &Transaction<'_>,
    scope: &StageScope,
    length: usize,
) -> Result<(), ExecutionError> {
    let row=tx.query_opt("SELECT c.decision_payload_bytes_bound FROM imagery.image_execution_capacity c \
        JOIN imagery.image_execution_stage s USING(capacity_scope) WHERE s.job_id=$1 AND s.stage_id=$2 AND s.owner_scope=$3",
        &[&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),&owner_scope_bytes(&scope.owner)])
        .await.map_err(map_db)?.ok_or(ExecutionError::NotFound)?;
    let maximum = invalid(u64::try_from(row.get::<_, i64>(0)))?;
    if invalid(u64::try_from(length))? > maximum {
        return Err(ExecutionError::DecisionPayloadTooLarge {
            maximum_bytes: maximum,
        });
    }
    Ok(())
}
pub(super) async fn stages(
    tx: &Transaction<'_>,
    checkpoint: &AnalysisCheckpoint,
) -> Result<(), ExecutionError> {
    let scope = &checkpoint.scope;
    let selected = checkpoint
        .stages
        .iter()
        .map(|reference| match reference {
            AnalysisStageReference::Supplier(reference) => {
                reference.scope.stage.as_bytes().to_vec()
            }
            AnalysisStageReference::Local(reference) => reference.scope.stage.as_bytes().to_vec(),
        })
        .collect::<Vec<_>>();
    let outside: bool = tx
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM imagery.image_execution_stage s \
        LEFT JOIN imagery.image_stage_cursor c USING(job_id,stage_id) \
        LEFT JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=c.current_dispatch \
        WHERE s.owner_scope=$1 AND s.job_id=$2 AND NOT(s.stage_id=ANY($3::bytea[])) \
        AND (d.dispatch_state IN ('prepared','sent','unknown') OR s.stage_state='planned' \
            OR (s.stage_state='admitted' AND s.capacity_reserved)))",
            &[
                &owner_scope_bytes(&scope.owner),
                &scope.job.as_bytes().as_slice(),
                &selected,
            ],
        )
        .await
        .map_err(map_db)?
        .get(0);
    if outside {
        return Err(ExecutionError::ObservationMissing);
    }
    let mut seen = BTreeSet::new();
    let mut previous = None;
    for (index, value) in checkpoint.stages.iter().enumerate() {
        let current = match value {
            AnalysisStageReference::Supplier(reference) => {
                let s = &reference.scope;
                if s.owner != scope.owner
                    || s.job != scope.job
                    || reference.kind != checkpoint.kind
                    || reference.basis.stage.parent_dispatch != previous
                    || !seen.insert(s.stage)
                {
                    return Err(ExecutionError::BasisConflict);
                }
                let stored = statements::read_decision_reference_unlocked_in(
                    tx,
                    &s.owner,
                    &s.job,
                    &s.stage,
                    &s.dispatch,
                )
                .await?
                .ok_or(ExecutionError::ObservationMissing)?;
                if stored != *reference {
                    return Err(ExecutionError::BasisConflict);
                }
                let sources = crate::imagery_runtime::sources_digest_in(tx, s)
                    .await
                    .map_err(|_| ExecutionError::Unavailable)?
                    .ok_or(ExecutionError::ObservationMissing)?;
                if sources != checkpoint.source_digest
                    || reference.basis.stage.config_digest != checkpoint.config_digest
                {
                    return Err(ExecutionError::BasisConflict);
                }
                previous = Some(s.dispatch);
                s.stage
            }
            AnalysisStageReference::Local(reference) => {
                if checkpoint.stages.len() != 1
                    || index != 0
                    || reference.scope != *scope
                    || reference.kind != checkpoint.kind
                {
                    return Err(ExecutionError::BasisConflict);
                }
                let row=tx.query_opt("SELECT c.owner_scope,c.job_id,c.stage_id,c.decision_kind,c.checkpoint_version,c.schema_revision, \
                    c.validator_revision,c.input_revision,c.input_digest,c.config_revision,c.config_digest,c.parent_dispatch, \
                    c.result_sha256,c.intent_digest,c.source_digest,c.dependencies::text AS dependencies_text \
                    FROM imagery.image_local_analysis c JOIN imagery.image_execution_stage s USING(job_id,stage_id) \
                    WHERE c.owner_scope=$1 AND c.job_id=$2 AND c.stage_id=$3 AND s.stage_state='released' AND NOT s.capacity_reserved \
                    AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt d WHERE d.job_id=c.job_id AND d.stage_id=c.stage_id)",
                    &[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice()])
                    .await.map_err(map_db)?.ok_or(ExecutionError::ObservationMissing)?;
                if rows::local_reference(&row, &scope.owner)? != *reference
                    || reference.result_sha256 != checkpoint.result_sha256
                    || reference.intent_digest != checkpoint.intent_digest
                    || reference.config_digest != checkpoint.config_digest
                    || reference.source_digest != checkpoint.source_digest
                    || encoding::parse_dependencies(&row.get::<_, String>("dependencies_text"))?
                        != checkpoint.dependencies
                {
                    return Err(ExecutionError::BasisConflict);
                }
                reference.scope.stage
            }
        };
        if index + 1 == checkpoint.stages.len() && current != scope.stage {
            return Err(ExecutionError::BasisConflict);
        }
    }
    if checkpoint.stages.is_empty() {
        return Err(ExecutionError::ObservationMissing);
    }
    Ok(())
}
