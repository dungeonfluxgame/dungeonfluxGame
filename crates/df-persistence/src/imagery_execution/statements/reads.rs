//! Stored stage and dispatch reads plus the overall queue scan's row shape.
//!
//! Reads enforce exact stored-owner equality: a wrong owner learns nothing.

use df_imagery::execution::{
    DispatchId, DispatchView, ExecutionError, ExecutionStageId, StageView,
};
use df_imagery::{ImageJobId, ImageOwner};
use tokio_postgres::Transaction;

use super::common::{
    check_owner, locked_job, optional_planned_pair, parse_stage, read_dispatch_view,
    require_read_committed,
};

/// Reads the stored stage with its current job revision. A wrong owner learns
/// nothing: the row reads as absent.
pub(crate) async fn read_stage_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<StageView, ExecutionError> {
    require_read_committed(tx).await?;
    let (_scope, revision, _deadline, _state, planned_asset, planned_attempt, _job_bytes) =
        locked_job(tx, owner, job).await?;
    let planned = optional_planned_pair(planned_asset, planned_attempt)?;
    let row = tx
        .query_opt(
            "SELECT job_id, stage_id, owner_scope, stage_kind, stage_state, \
             claim_generation, lease_until_ms, input_revision, input_digest, \
             config_revision, config_digest, capacity_scope, local_accounting \
             FROM imagery.image_execution_stage WHERE job_id = $1 AND stage_id = $2",
            &[&job.as_bytes().as_slice(), &stage.as_bytes().as_slice()],
        )
        .await
        .map_err(super::common::map_db)?
        .ok_or(ExecutionError::NotFound)?;
    let stored_scope: Vec<u8> = row.get("owner_scope");
    check_owner(&stored_scope, owner)?;
    parse_stage(&row, owner, revision, planned)
}

/// Reads one stored dispatch. A wrong owner learns nothing.
pub(crate) async fn read_dispatch_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    dispatch: &DispatchId,
) -> Result<DispatchView, ExecutionError> {
    require_read_committed(tx).await?;
    read_dispatch_view(tx, owner, dispatch).await
}

/// Reads the exact owned stage cursor and current dispatch in one job-locked transaction.
pub(crate) async fn read_current_dispatch_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<Option<DispatchView>, ExecutionError> {
    require_read_committed(tx).await?;
    read_stage_in(tx, owner, job, stage).await?;
    let row=tx.query_opt("SELECT current_dispatch FROM imagery.image_stage_cursor WHERE job_id=$1 AND stage_id=$2",
        &[&job.as_bytes().as_slice(),&stage.as_bytes().as_slice()]).await.map_err(super::common::map_db)?;
    let Some(row) = row else { return Ok(None) };
    let dispatch = super::common::invalid(DispatchId::from_bytes(
        &row.get::<_, Vec<u8>>("current_dispatch"),
    ))?;
    let view = read_dispatch_view(tx, owner, &dispatch).await?;
    if view.job != *job || view.stage != *stage {
        return Err(ExecutionError::InvalidRecord);
    }
    Ok(Some(view))
}

/// Reads installed scope policy without reserving capacity or inventing a default row.
pub(crate) async fn read_capacity_bounds_in(
    tx: &Transaction<'_>,
    scope: &str,
) -> Result<df_imagery::execution::CapacityBounds, ExecutionError> {
    require_read_committed(tx).await?;
    let row = tx
        .query_opt(
            "SELECT bound,pending_jobs_bound,pending_bytes_bound,decision_payload_bytes_bound, \
        urgency_promotion_ms FROM imagery.image_execution_capacity WHERE capacity_scope=$1",
            &[&scope],
        )
        .await
        .map_err(super::common::map_db)?
        .ok_or(ExecutionError::NotFound)?;
    Ok(df_imagery::execution::CapacityBounds {
        max_active: row.get("bound"),
        max_pending_jobs: row.get("pending_jobs_bound"),
        max_pending_bytes: row.get("pending_bytes_bound"),
        max_decision_payload_bytes: row.get("decision_payload_bytes_bound"),
        urgency_promotion_ms: row.get("urgency_promotion_ms"),
    })
}

/// Reads one immutable same-job child, preserving historical parent identity for recovery.
pub(crate) async fn read_child_stage_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    parent_dispatch: &DispatchId,
    kind: df_imagery::execution::StageKind,
) -> Result<Option<StageView>, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    if read_dispatch_view(tx, owner, parent_dispatch).await?.job != *job {
        return Err(ExecutionError::NotFound);
    }
    let row = tx
        .query_opt(
            "SELECT stage_id FROM imagery.image_execution_stage \
        WHERE owner_scope=$1 AND job_id=$2 AND parent_dispatch=$3 AND stage_kind=$4",
            &[
                &df_imagery::owner_scope_bytes(owner),
                &job.as_bytes().as_slice(),
                &parent_dispatch.as_bytes().as_slice(),
                &kind.key(),
            ],
        )
        .await
        .map_err(super::common::map_db)?;
    let Some(row) = row else { return Ok(None) };
    let stage = super::common::invalid(ExecutionStageId::from_bytes(
        &row.get::<_, Vec<u8>>("stage_id"),
    ))?;
    read_stage_in(tx, owner, job, &stage).await.map(Some)
}
