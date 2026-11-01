//! Planned and published identity resolution for the journal.
//!
//! The published pair follows the retained 0039 allocation's stage id
//! exactly: resolution never guesses the latest-touched stage. The planned
//! pair prefers the output stage over classification, so touching
//! classification after the output stage exists cannot reroute recovery.
//! This journal never inserts admission bindings and never changes the
//! existing 0039 first-output admission behavior. Unknown pairs answer
//! not-found and create nothing. Local publications resolve to their staged
//! checkpoints, never to finance rows.

use df_imagery::execution::{DispatchId, ExecutionError, ExecutionIdentity, ResolvedExecution};
use df_imagery::{ImageJobId, ImageOwner};
use tokio_postgres::Transaction;

use super::common::{
    check_owner, invalid, map_db, parse_checkpoint, read_dispatch_view, require_read_committed,
};

/// Loads cursor, current dispatch, refetch selection and the local
/// checkpoint for one resolved job and stage.
async fn resolve_stage(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &df_imagery::execution::ExecutionStageId,
) -> Result<ResolvedExecution, ExecutionError> {
    let cursor = tx
        .query_opt(
            "SELECT current_dispatch, refetch_dispatch FROM imagery.image_stage_cursor \
             WHERE job_id = $1 AND stage_id = $2",
            &[&job.as_bytes().as_slice(), &stage.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?;
    let (current_id, refetch_id): (Option<DispatchId>, Option<DispatchId>) = match cursor {
        Some(row) => {
            let current: Vec<u8> = row.get("current_dispatch");
            let refetch: Option<Vec<u8>> = row.get("refetch_dispatch");
            (
                Some(invalid(DispatchId::from_bytes(&current))?),
                refetch
                    .map(|bytes| invalid(DispatchId::from_bytes(&bytes)))
                    .transpose()?,
            )
        }
        None => (None, None),
    };
    let current = match current_id {
        Some(id) => Some(read_dispatch_view(tx, owner, &id).await?),
        None => None,
    };
    let local_checkpoint = match &current {
        Some(view) if view.local_accounting => {
            let checkpoint = tx
                .query_opt(
                    "SELECT storage_key, object_version, byte_len, sha256, recipe_digest, \
                     mime, width_px, height_px, alpha, usage_revision, book_call_id \
                     FROM imagery.image_dispatch_checkpoint WHERE dispatch_id = $1",
                    &[&view.dispatch.as_bytes().as_slice()],
                )
                .await
                .map_err(map_db)?;
            checkpoint
                .map(|row| parse_checkpoint(&row, &view.dispatch))
                .transpose()?
        }
        _ => None,
    };
    Ok(ResolvedExecution {
        job: *job,
        stage: *stage,
        current,
        refetch: refetch_id,
        local_checkpoint,
    })
}

/// Resolves planned or published identity to its job, stage, current dispatch
/// and local checkpoint. The planned pair reaches the job's output stage; the
/// published pair reaches the retained allocation's stage id. This is
/// identity routing only; it authorizes nothing and reads no finance state.
pub(crate) async fn resolve_execution_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    identity: &ExecutionIdentity,
) -> Result<ResolvedExecution, ExecutionError> {
    require_read_committed(tx).await?;
    let row = tx
        .query_opt(
            "SELECT j.job_id, j.owner_scope, j.planned_output_stage FROM imagery.image_job j \
         WHERE j.job_id = COALESCE( \
             (SELECT a.job_id FROM imagery.image_publication_allocation a \
              WHERE a.publication_attempt = $2 AND a.asset_id = $1), \
             (SELECT p.job_id FROM imagery.image_job p \
              WHERE p.planned_publication_asset = $1 AND p.planned_publication_attempt = $2)) \
         FOR UPDATE",
            &[&identity.asset.as_slice(), &identity.attempt.as_slice()],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::NotFound)?;
    let scope: Vec<u8> = row.get("owner_scope");
    check_owner(&scope, owner)?;
    let job = invalid(ImageJobId::from_bytes(&row.get::<_, Vec<u8>>("job_id")))?;
    // Publication may have committed while this read waited for the job lock.
    let allocation = tx
        .query_opt(
            "SELECT stage_id FROM imagery.image_publication_allocation \
         WHERE job_id = $1 AND publication_attempt = $2 AND asset_id = $3",
            &[
                &job.as_bytes().as_slice(),
                &identity.attempt.as_slice(),
                &identity.asset.as_slice(),
            ],
        )
        .await
        .map_err(map_db)?;
    let stage = match allocation {
        Some(allocation) => allocation.get::<_, Vec<u8>>("stage_id"),
        None => row
            .get::<_, Option<Vec<u8>>>("planned_output_stage")
            .ok_or(ExecutionError::NotFound)?,
    };
    let stage = invalid(df_imagery::execution::ExecutionStageId::from_bytes(&stage))?;
    let exists = tx
        .query_opt(
            "SELECT 1 FROM imagery.image_execution_stage WHERE job_id = $1 AND stage_id = $2",
            &[&job.as_bytes().as_slice(), &stage.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .is_some();
    if !exists {
        return Err(ExecutionError::InvalidRecord);
    }
    resolve_stage(tx, owner, &job, &stage).await
}
