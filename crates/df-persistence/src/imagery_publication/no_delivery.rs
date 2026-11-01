//! Atomic output-only terminal seal; supplier evidence and finance stay independent.

use super::rows::database_error;
use df_assets::{AssetId, AssetState, AttemptId, RejectGuard, RejectReason, RejectResult};
use df_imagery::execution::{TerminalAllocation, TerminalCommand, TerminalValidation};
use df_imagery::publication::{
    NoDeliveryReceipt, NoDeliveryRequest, NoDeliveryStage, PublicationError,
};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::Transaction;

pub(super) mod candidates;
mod evidence;
mod rows;
mod source_denial;
pub use source_denial::no_delivery_if_sources_denied_in;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NoDeliveryOutcome {
    Recorded(NoDeliveryReceipt),
    Existing(NoDeliveryReceipt),
}

impl NoDeliveryOutcome {
    pub fn receipt(self) -> NoDeliveryReceipt {
        match self {
            Self::Recorded(receipt) | Self::Existing(receipt) => receipt,
        }
    }
}

/// Reads the exact owner/job terminal receipt; absence does not authorize fresh supplier work.
pub async fn read_no_delivery_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<NoDeliveryReceipt>, PublicationError> {
    rows::read_receipt_in(tx, owner, job).await
}

/// Rejects only the canonical existing allocation and seals the exact current execution tail.
/// Errors require rollback; catalog quota, outbox, job and capacity changes share this transaction.
pub async fn finalize_no_delivery_in(
    tx: &Transaction<'_>,
    request: &NoDeliveryRequest<'_>,
) -> Result<NoDeliveryOutcome, PublicationError> {
    super::rows::require_read_committed(tx).await?;
    let command = request.terminal;
    let scope = command.actor.scope();
    if command.now_ms < 0 {
        return Err(PublicationError::InvalidRecord);
    }
    let allocation = lock_allocation_in(tx, &scope.owner, &scope.job).await?;
    tx.query_one(
        "SELECT job_id FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE",
        &[
            &owner_scope_bytes(&scope.owner),
            &scope.job.as_bytes().as_slice(),
        ],
    )
    .await
    .map_err(database_error)?;
    let job = crate::read_in(tx, &scope.owner, scope.job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    let input_binding = evidence::request_binding(request, &job)?;
    if let Some((receipt, binding)) = rows::read_in(tx, &scope.owner, &scope.job).await? {
        if binding != input_binding || command.now_ms < receipt.sealed_at_ms {
            return Err(PublicationError::BindingConflict);
        }
        return Ok(NoDeliveryOutcome::Existing(receipt));
    }
    let validation = crate::imagery_execution::validate_terminal_tail_in(tx, command)
        .await
        .map_err(PublicationError::Execution)?;
    if validation.allocation != allocation || validation.job_revision != job.revision {
        return Err(PublicationError::BindingConflict);
    }
    check_allocation_in(tx, &scope.owner, &scope.job, allocation).await?;
    let retained = evidence::verify_in(tx, request, &job, &validation).await?;
    commit_in(tx, command, &validation, &retained, &input_binding)
        .await
        .map(NoDeliveryOutcome::Recorded)
}

async fn commit_in(
    tx: &Transaction<'_>,
    command: &TerminalCommand,
    validation: &TerminalValidation,
    retained: &evidence::RetainedEvidence,
    binding: &[u8],
) -> Result<NoDeliveryReceipt, PublicationError> {
    let scope = command.actor.scope();
    if tx.query_one("SELECT EXISTS(SELECT 1 FROM imagery.image_job_result WHERE owner_scope=$1 AND job_id=$2)", &[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice()]).await.map_err(database_error)?.get::<_,bool>(0) {return Err(PublicationError::BindingConflict)}
    if let TerminalAllocation::Existing { asset, attempt, .. } = validation.allocation {
        let reason = match retained.reason {
            df_imagery::runtime::NoDeliveryReason::SourceDenied => RejectReason::SourceDenied,
            df_imagery::runtime::NoDeliveryReason::SupplierDeclined => {
                RejectReason::SupplierDeclined
            }
            df_imagery::runtime::NoDeliveryReason::MissingCredential
            | df_imagery::runtime::NoDeliveryReason::UnsupportedProfile => {
                RejectReason::PreflightRejected
            }
            df_imagery::runtime::NoDeliveryReason::SupplierNoOutput => RejectReason::SupplierFailed,
            df_imagery::runtime::NoDeliveryReason::Retired
            | df_imagery::runtime::NoDeliveryReason::Expired => RejectReason::DemandEnded,
            df_imagery::runtime::NoDeliveryReason::RequiredInspectionUnjudged
            | df_imagery::runtime::NoDeliveryReason::RequiredInspectionPreparation
            | df_imagery::runtime::NoDeliveryReason::RequiredInspectionDisabled => {
                RejectReason::ScreenUnavailable
            }
            _ => RejectReason::ScreenRejected,
        };
        let result = crate::media_catalog_sql::reject_in(
            tx,
            AssetId::from_bytes(&asset).map_err(|_| PublicationError::InvalidRecord)?,
            AttemptId::from_bytes(&attempt).map_err(|_| PublicationError::InvalidRecord)?,
            reason,
            RejectGuard::Current,
            command.now_ms,
        )
        .await
        .map_err(PublicationError::Catalog)?;
        if !matches!(result, RejectResult::Rejected(_)) {
            return Err(PublicationError::BindingConflict);
        }
    }
    let receipt = rows::insert_in(tx, command, validation, retained, binding).await?;
    crate::imagery_execution::close_terminal_in(tx, command)
        .await
        .map_err(PublicationError::Execution)?;
    let revision = tx
        .query_one(
            "SELECT revision FROM imagery.image_job WHERE job_id=$1",
            &[&scope.job.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?
        .get::<_, i64>(0);
    if u64::try_from(revision).map_err(|_| PublicationError::InvalidRecord)? != receipt.job_revision
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(receipt)
}

pub(super) async fn candidate_use_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &df_imagery::execution::ExecutionStageId,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<Option<u32>, PublicationError> {
    let Some(receipt) = read_no_delivery_in(tx, owner, job).await? else {
        return Ok(None);
    };
    if tx.query_one("SELECT EXISTS(SELECT 1 FROM imagery.image_job_result WHERE owner_scope=$1 AND job_id=$2)", &[&owner_scope_bytes(owner),&job.as_bytes().as_slice()]).await.map_err(database_error)?.get::<_,bool>(0) {return Err(PublicationError::BindingConflict)}
    let Some(candidate) =
        crate::imagery_execution::read_candidate_reference_in(tx, owner, job, stage, dispatch)
            .await
            .map_err(PublicationError::Execution)?
    else {
        return Ok(None);
    };
    let witnesses = candidates::stored_in(tx, owner, job)
        .await?
        .ok_or(PublicationError::BindingConflict)?;
    if witnesses.contains(&candidate) {
        return Ok(Some(candidate.receipt.usage_revision));
    }
    let sealed_scope = [receipt.actor, receipt.tail]
        .into_iter()
        .any(|actor| match actor {
            NoDeliveryStage::Dispatch {
                scope,
                usage_revision,
            } => scope == candidate.scope && usage_revision <= candidate.receipt.usage_revision,
            NoDeliveryStage::UnsentStage(_)
            | NoDeliveryStage::UnclaimedStage(_)
            | NoDeliveryStage::DeferredStage(_)
            | NoDeliveryStage::ReleasedStage(_) => false,
        });
    Ok(sealed_scope.then_some(candidate.receipt.usage_revision))
}

pub(super) async fn no_output_use_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &df_imagery::execution::ExecutionStageId,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<Option<u32>, PublicationError> {
    let row=tx.query_opt("SELECT d.usage_revision FROM imagery.image_no_delivery s \
        JOIN imagery.image_dispatch_attempt d ON d.job_id=s.job_id AND d.stage_id=s.actor_stage_id AND d.dispatch_id=s.actor_dispatch_id \
        JOIN imagery.image_stage_cursor c ON c.job_id=d.job_id AND c.stage_id=d.stage_id AND c.current_dispatch=d.dispatch_id \
        JOIN imagery.image_dispatch_usage u ON u.owner_scope=s.owner_scope AND u.job_id=s.job_id AND u.stage_id=d.stage_id AND u.dispatch_id=d.dispatch_id AND u.revision=s.actor_usage_revision \
        WHERE s.owner_scope=$1 AND s.job_id=$2 AND s.actor_stage_id=$3 AND s.actor_dispatch_id=$4 \
        AND s.reason='supplier_no_output' AND s.actor_kind='dispatch' AND s.tail_kind='dispatch' \
        AND s.tail_stage_id=s.actor_stage_id AND s.tail_dispatch_id=s.actor_dispatch_id \
        AND d.dispatch_state='failed' AND d.usage_revision=s.actor_usage_revision \
        AND u.evidence<>'unknown' AND NOT u.usable AND u.raw_output_sha256 IS NOT DISTINCT FROM s.no_output_raw_sha256",
        &[&owner_scope_bytes(owner),&job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&dispatch.as_bytes().as_slice()]).await.map_err(database_error)?;
    row.map(|row| u32::try_from(row.get::<_, i32>(0)).map_err(|_| PublicationError::InvalidRecord))
        .transpose()
}

pub(super) async fn legacy_output_use_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &df_imagery::execution::ExecutionStageId,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<Option<u32>, PublicationError> {
    let row=tx.query_opt("SELECT d.usage_revision FROM imagery.image_no_delivery n JOIN imagery.image_dispatch_attempt d ON d.job_id=n.job_id AND d.stage_id=n.actor_stage_id AND d.dispatch_id=n.actor_dispatch_id JOIN imagery.image_execution_stage s ON s.job_id=d.job_id AND s.stage_id=d.stage_id JOIN imagery.image_stage_cursor c ON c.job_id=d.job_id AND c.stage_id=d.stage_id AND c.current_dispatch=d.dispatch_id JOIN imagery.image_dispatch_checkpoint p ON p.dispatch_id=d.dispatch_id AND p.usage_revision=d.usage_revision AND p.book_call_id=d.book_call_id WHERE n.owner_scope=$1 AND n.job_id=$2 AND d.stage_id=$3 AND d.dispatch_id=$4 AND n.reason='source_denied' AND n.actor_kind='dispatch' AND n.tail_kind='dispatch' AND n.tail_stage_id=n.actor_stage_id AND n.tail_dispatch_id=n.actor_dispatch_id AND n.candidate_dispatch_id IS NULL AND n.legacy_output_write_id IS NOT NULL AND d.dispatch_state='succeeded' AND d.usage_revision=n.actor_usage_revision AND s.owner_scope=n.owner_scope AND s.stage_kind IN ('generation','edit','correction','extraction') AND p.object_version=n.legacy_output_version AND p.sha256=n.legacy_output_sha256 AND p.recipe_digest=n.legacy_output_recipe_digest AND NOT EXISTS(SELECT 1 FROM imagery.image_candidate_checkpoint x WHERE x.dispatch_id=d.dispatch_id) AND NOT EXISTS(SELECT 1 FROM imagery.image_final_selection x WHERE x.job_id=d.job_id) AND NOT EXISTS(SELECT 1 FROM imagery.image_job_result x WHERE x.job_id=d.job_id)", &[&owner_scope_bytes(owner),&job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&dispatch.as_bytes().as_slice()]).await.map_err(database_error)?;
    row.map(|row| u32::try_from(row.get::<_, i32>(0)).map_err(|_| PublicationError::InvalidRecord))
        .transpose()
}

async fn lock_allocation_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<TerminalAllocation, PublicationError> {
    let row=tx.query_opt("SELECT stage_id,asset_id,publication_attempt FROM imagery.image_publication_allocation WHERE owner_scope=$1 AND job_id=$2", &[&owner_scope_bytes(owner),&job.as_bytes().as_slice()]).await.map_err(database_error)?;
    let Some(row) = row else {
        return Ok(TerminalAllocation::NoAllocation);
    };
    let asset = AssetId::from_bytes(&row.get::<_, Vec<u8>>("asset_id"))
        .map_err(|_| PublicationError::InvalidRecord)?;
    let (_, record) = crate::media_catalog_sql::lock_quota_then_asset(tx, asset)
        .await
        .map_err(PublicationError::Catalog)?
        .ok_or(PublicationError::BindingConflict)?;
    let attempt = AttemptId::from_bytes(&row.get::<_, Vec<u8>>("publication_attempt"))
        .map_err(|_| PublicationError::InvalidRecord)?;
    if record.attempt != attempt
        || record.kind != df_assets::AssetKind::Image
        || record.provenance != df_assets::Provenance::Generated
        || matches!(
            record.state,
            AssetState::Ready
                | AssetState::Archived
                | AssetState::Deleted
                | AssetState::Purging
                | AssetState::Purged
        )
    {
        return Err(PublicationError::BindingConflict);
    }
    super::rows::check_catalog_owner(&record, owner)?;
    Ok(TerminalAllocation::Existing {
        stage: df_imagery::execution::ExecutionStageId::from_bytes(
            &row.get::<_, Vec<u8>>("stage_id"),
        )
        .map_err(|_| PublicationError::InvalidRecord)?,
        asset: *asset.as_bytes(),
        attempt: *attempt.as_bytes(),
    })
}

async fn check_allocation_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    allocation: TerminalAllocation,
) -> Result<(), PublicationError> {
    let query = match allocation {
        TerminalAllocation::NoAllocation => {
            "SELECT NOT EXISTS(SELECT 1 FROM media.asset a WHERE a.asset_id=j.planned_publication_asset OR a.attempt=j.planned_publication_attempt) FROM imagery.image_job j WHERE j.owner_scope=$1 AND j.job_id=$2"
        }
        TerminalAllocation::Existing { .. } => {
            "SELECT EXISTS(SELECT 1 FROM imagery.image_publication_allocation p JOIN media.asset a ON a.asset_id=p.asset_id AND a.attempt=p.publication_attempt WHERE p.job_id=j.job_id AND p.owner_scope=j.owner_scope AND p.stage_id=j.planned_output_stage AND p.asset_id=j.planned_publication_asset AND p.publication_attempt=j.planned_publication_attempt AND p.original_recipe_key=j.resolved_recipe_key AND a.request_key=p.original_recipe_key AND p.original_recipe_revision=j.recipe_revision AND p.original_recipe_bytes=j.recipe_bytes AND p.original_config_revision=j.config_revision AND p.original_config_bytes=j.config_bytes) FROM imagery.image_job j WHERE j.owner_scope=$1 AND j.job_id=$2"
        }
    };
    if !tx
        .query_one(
            query,
            &[&owner_scope_bytes(owner), &job.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?
        .get::<_, bool>(0)
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(())
}
