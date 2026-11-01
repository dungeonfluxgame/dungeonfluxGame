//! Atomic catalog Ready plus durable imagery result. Lock quota/asset before job/stage/dispatch.
//! Bytes are decoded before this transaction; no provider, object-store or finance work runs here.

use df_assets::{
    AssetKind, AssetState, Audience, FinalizeRequest, FinalizeResult, HeldStep, ImageProof,
    Provenance,
};
use df_imagery::execution::{ClaimToken, DispatchId, ExecutionStageId};
use df_imagery::publication::{FrozenOutput, PublicationError, PublishedImage, recipe_digest};
use df_imagery::{ImageJobId, ImageOwner, VersionedSnapshot, owner_scope_bytes};
use sha2::{Digest, Sha256};
use tokio_postgres::Transaction;

mod artifact;
mod decision_binding;
mod economic;
pub use economic::{StageEconomicUse, decision_economic_use_in, stage_economic_use_in};
mod local_inspections;
mod no_delivery;
pub use no_delivery::{
    NoDeliveryOutcome, finalize_no_delivery_in, no_delivery_if_sources_denied_in,
    read_no_delivery_in,
};
mod inspection;
mod reusable;
pub use inspection::{
    PublishedInspection, PublishedInspectionEvidence, read_published_inspection_in,
};
mod rows;
mod selection;
mod source_bound;
mod sources;
pub use source_bound::read_generated_source_bound_in;
pub use sources::{
    PublicationAuthorityFuture, PublicationSource, PublicationSourceAuthority,
    lock_image_sources_in, matches_source_receipt,
};
mod store;
pub use store::{PostgresImagePublication, PublicationOpenError};

pub const IMAGERY_PUBLICATION_MIGRATION_VERSION: i32 = 42;
pub const IMAGERY_PUBLICATION_MIGRATION_SQL: &str =
    include_str!("../migrations/0042_imagery_publication.sql");

/// Exact execution and catalog identities of one publication; unrelated inspection touches cannot route it.
pub struct PublicationCommand<'a> {
    pub owner: &'a ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub claim_generation: u64,
    pub held: HeldStep,
    pub finalize: &'a FinalizeRequest,
    pub proofs: &'a [ImageProof],
    pub now_ms: i64,
    pub recovery: Option<&'a df_imagery::execution::RecoveryLease>,
    pub source_authority: Option<&'a dyn PublicationSourceAuthority>,
    pub qualified: Option<&'a df_imagery::runtime::QualifiedPipeline>,
}

/// The catalog outcome remains authoritative when no result could be recorded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicationOutcome {
    Recorded(PublishedImage),
    Existing(PublishedImage),
    NotRecorded(FinalizeResult),
}

/// Finalizes and records in the caller's READ COMMITTED transaction; an error requires rollback.
pub async fn finalize_image_in(
    tx: &Transaction<'_>,
    command: &PublicationCommand<'_>,
) -> Result<PublicationOutcome, PublicationError> {
    rows::require_read_committed(tx).await?;
    if command.now_ms < 0 {
        return Err(PublicationError::InvalidRecord);
    }
    let locked_sources = sources::lock_in(
        tx,
        &df_imagery::execution::ExecutionScope {
            owner: command.owner.clone(),
            job: command.job,
            stage: command.stage,
            dispatch: command.dispatch,
        },
        command.held.asset,
        command.source_authority,
        command.now_ms,
    )
    .await?;
    let asset = &locked_sources.output;
    if asset.attempt != command.held.attempt
        || asset.kind != AssetKind::Image
        || asset.provenance != Provenance::Generated
    {
        return Err(PublicationError::BindingConflict);
    }
    rows::check_catalog_owner(asset, command.owner)?;
    let job = rows::lock_job(tx, command).await?;
    rows::require_delivery_open_in(tx, command.owner, &command.job).await?;
    sources::authorize_in(
        tx,
        &locked_sources,
        command.source_authority,
        command.now_ms,
    )
    .await?;
    rows::check_recipe_key(asset, &job)?;
    let plan = rows::snapshot(&job, "recipe_revision", "recipe_bytes")?;
    let selected = selection::read_in(tx, command.owner, &command.job).await?;
    if selected.is_none()
        && df_imagery::runtime::requires_final_selection(&locked_sources.job)
            .map_err(|_| PublicationError::InvalidRecord)?
    {
        return Err(PublicationError::BindingConflict);
    }
    if let Some(selected) = &selected {
        selected.check_qualified(&locked_sources.job, command.qualified)?;
    }
    if selected
        .as_ref()
        .is_some_and(|selected| selected.source_binding != locked_sources.binding)
    {
        return Err(PublicationError::BindingConflict);
    }
    let allocation_stage = selected
        .as_ref()
        .map_or(command.stage, |selected| selected.allocation_stage);
    if selected.as_ref().is_some_and(|selected| {
        selected.final_stage != command.stage || selected.final_dispatch != command.dispatch
    }) || (selected.is_none()
        && job
            .get::<_, Option<Vec<u8>>>("planned_output_stage")
            .as_deref()
            != Some(command.stage.as_bytes().as_slice()))
    {
        return Err(PublicationError::BindingConflict);
    }
    let recipe = selected.as_ref().map_or(&plan, |selected| &selected.recipe);
    let frozen = FrozenOutput::decode(recipe)?;
    let digest = recipe_digest(recipe);
    rows::check_allocation(tx, command, allocation_stage, &job, &plan).await?;
    let stage = rows::lock_stage_dispatch(tx, command).await?;
    let checkpoint = crate::imagery_execution::read_output_checkpoint_in(
        tx,
        command.owner,
        &command.job,
        &command.stage,
        &command.dispatch,
    )
    .await
    .map_err(PublicationError::Execution)?
    .ok_or(PublicationError::CheckpointMismatch)?;
    let existing = rows::stored_result(tx, command.owner, &command.job).await?;
    rows::check_proofs(command, &frozen, &digest)?;
    rows::check_written_proofs(tx, command).await?;
    rows::check_checkpoint(command, &stage, &checkpoint, &digest)?;
    let mut binding = rows::binding_bytes(command, &digest, &checkpoint)?;
    binding.extend_from_slice(&Sha256::digest(&locked_sources.binding));
    if let Some(selected) = &selected {
        binding.extend_from_slice(&Sha256::digest(&selected.binding));
    }
    if let Some((stored, stored_binding)) = existing {
        if stored_binding != binding || asset.state != AssetState::Ready {
            return Err(PublicationError::BindingConflict);
        }
        return Ok(PublicationOutcome::Existing(stored));
    }
    let private = job.get::<_, String>("state") == "retired"
        || job.get::<_, i64>("absolute_deadline_ms") <= command.now_ms;
    if let Some(recovery) = command.recovery {
        rows::check_recovery_lineage(command, recovery, &stage)?;
        crate::imagery_execution::validate_recovery_in(tx, recovery, command.now_ms)
            .await
            .map_err(PublicationError::Execution)?;
    } else {
        rows::check_claim(command, &stage, private)?;
    }
    if asset.state != AssetState::Validated
        || asset.lease.is_none_or(|lease| {
            lease.owner != command.held.owner
                || lease.until_ms < command.now_ms
                || lease.step != df_assets::Step::Publish
        })
    {
        return Ok(PublicationOutcome::NotRecorded(FinalizeResult::LeaseLost));
    }
    rows::privatize_retired(tx, command, asset, &stage, private).await?;
    let outcome =
        crate::media_catalog_sql::finalize_in(tx, command.held, command.finalize, command.now_ms)
            .await
            .map_err(PublicationError::Catalog)?;
    if !matches!(outcome, FinalizeResult::Ready { .. }) {
        return Ok(PublicationOutcome::NotRecorded(outcome));
    }
    rows::insert_result(tx, command, allocation_stage, &digest, &binding, private).await?;
    let result = rows::stored_result(tx, command.owner, &command.job)
        .await?
        .ok_or(PublicationError::InvalidRecord)?
        .0;
    Ok(PublicationOutcome::Recorded(result))
}

/// Rereads the same owner/job after Unknown; absence never authorizes another provider dispatch.
pub async fn read_result_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<PublishedImage>, PublicationError> {
    Ok(rows::stored_result(tx, owner, job)
        .await?
        .map(|(result, _)| result))
}

/// Cache reuse is Ready-only, exact-owner and exact-audience; private reconciliations never enter it.
pub async fn read_recipe_result_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    digest: &[u8; 32],
    audience: &Audience,
) -> Result<Option<PublishedImage>, PublicationError> {
    let rows = tx.query("SELECT r.job_id FROM imagery.image_job_result r JOIN imagery.image_job j USING(job_id) JOIN media.asset a ON a.asset_id=r.asset_id WHERE r.owner_scope=$1 AND r.recipe_digest=$2 AND NOT r.private_reconciliation AND a.state='ready' AND j.state='recipe_frozen' AND EXISTS(SELECT 1 FROM imagery.image_interest i WHERE i.job_id=j.job_id AND i.state='attached') ORDER BY r.published_at_ms DESC LIMIT 32", &[&owner_scope_bytes(owner), &digest.as_slice()]).await.map_err(rows::database_error)?;
    for row in rows {
        let job = ImageJobId::from_bytes(&row.get::<_, Vec<u8>>(0))
            .map_err(|_| PublicationError::InvalidRecord)?;
        if let Some(result) = read_result_in(tx, owner, &job).await? {
            let asset = crate::media_catalog_sql::asset_in(tx, result.asset)
                .await
                .map_err(PublicationError::Catalog)?
                .ok_or(PublicationError::InvalidRecord)?;
            if asset.audience == *audience {
                return Ok(Some(result));
            }
        }
    }
    Ok(None)
}

/// Only failed commit acknowledgements map to Unknown; caller must reconnect and reread the same ids.
pub async fn commit_publication(tx: Transaction<'_>) -> Result<(), PublicationError> {
    tx.commit().await.map_err(|error| {
        rows::record_failure(&error);
        PublicationError::Unknown
    })
}

/// Recovery always follows the immutable allocation and its current dispatch, including hosted checkpoints.
#[derive(Clone, Debug, PartialEq)]
pub struct PublicationRecovery {
    pub execution: df_imagery::execution::ResolvedExecution,
    pub output: Option<df_imagery::execution::OutputCheckpoint>,
}

/// Routes a catalog attempt or settlement-outbox entry without treating it as a finance attempt.
pub async fn resolve_publication_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    asset: df_assets::AssetId,
    attempt: df_assets::AttemptId,
) -> Result<PublicationRecovery, PublicationError> {
    let allocation = tx.query_opt("SELECT job_id,stage_id FROM imagery.image_publication_allocation WHERE asset_id=$1 AND publication_attempt=$2 AND owner_scope=$3", &[&asset.as_bytes().as_slice(), &attempt.as_bytes().as_slice(), &owner_scope_bytes(owner)]).await.map_err(rows::database_error)?.ok_or(PublicationError::NotFound)?;
    let job = ImageJobId::from_bytes(&allocation.get::<_, Vec<u8>>("job_id"))
        .map_err(|_| PublicationError::InvalidRecord)?;
    if let Some(selected) = selection::read_in(tx, owner, &job).await? {
        if allocation.get::<_, Vec<u8>>("stage_id") != selected.allocation_stage.as_bytes() {
            return Err(PublicationError::BindingConflict);
        }
        let current =
            crate::imagery_execution::read_dispatch_in(tx, owner, &selected.final_dispatch)
                .await
                .map_err(PublicationError::Execution)?;
        if current.job != job || current.stage != selected.final_stage {
            return Err(PublicationError::BindingConflict);
        }
        let output = crate::imagery_execution::read_output_checkpoint_in(
            tx,
            owner,
            &job,
            &selected.final_stage,
            &selected.final_dispatch,
        )
        .await
        .map_err(PublicationError::Execution)?;
        let refetch=tx.query_one("SELECT refetch_dispatch FROM imagery.image_stage_cursor WHERE job_id=$1 AND stage_id=$2", &[&job.as_bytes().as_slice(),&selected.final_stage.as_bytes().as_slice()]).await.map_err(rows::database_error)?.get::<_,Option<Vec<u8>>>(0).map(|bytes|DispatchId::from_bytes(&bytes).map_err(|_|PublicationError::InvalidRecord)).transpose()?;
        return Ok(PublicationRecovery {
            execution: df_imagery::execution::ResolvedExecution {
                job,
                stage: selected.final_stage,
                current: Some(current),
                refetch,
                local_checkpoint: output.clone(),
            },
            output,
        });
    }
    let execution = crate::imagery_execution::resolve_execution_in(
        tx,
        owner,
        &df_imagery::execution::ExecutionIdentity::new(*asset.as_bytes(), *attempt.as_bytes())
            .map_err(|_| PublicationError::InvalidRecord)?,
    )
    .await
    .map_err(PublicationError::Execution)?;
    if allocation.get::<_, Vec<u8>>("job_id") != execution.job.as_bytes()
        || allocation.get::<_, Vec<u8>>("stage_id") != execution.stage.as_bytes()
    {
        return Err(PublicationError::BindingConflict);
    }
    let output = match &execution.current {
        Some(dispatch) => crate::imagery_execution::read_output_checkpoint_in(
            tx,
            owner,
            &execution.job,
            &execution.stage,
            &dispatch.dispatch,
        )
        .await
        .map_err(PublicationError::Execution)?,
        None => None,
    };
    Ok(PublicationRecovery { execution, output })
}

mod runtime;
pub use runtime::{NativeImagePublisher, RuntimePublicationPorts, RuntimePublicationSettings};
