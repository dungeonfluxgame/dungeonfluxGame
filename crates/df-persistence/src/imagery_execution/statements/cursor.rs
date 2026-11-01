//! Refetch selection for the journal.
//!
//! The cursor names the current output dispatch plus the persisted refetch
//! selection, so a late result for a superseded dispatch is discarded
//! instead of finalizing the wrong attempt.

use df_imagery::execution::{DispatchView, ExecutionError, RefetchCommand};
use tokio_postgres::Transaction;

use super::common::{
    bump_job_revision, locked_dispatch, locked_job, map_db, read_dispatch_view,
    require_read_committed,
};

/// Persists which dispatch a refetch was requested for, so a late result for
/// a superseded dispatch is discarded instead of finalizing the wrong attempt.
pub(crate) async fn request_refetch_in(
    tx: &Transaction<'_>,
    command: &RefetchCommand,
) -> Result<DispatchView, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, &command.owner, &command.job).await?;
    locked_dispatch(tx, &command.job, &command.stage, &command.dispatch).await?;
    let updated = tx
        .execute(
            "UPDATE imagery.image_stage_cursor SET refetch_dispatch = $3, updated_at_ms = $4 \
             WHERE job_id = $1 AND stage_id = $2",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
                &command.dispatch.as_bytes().as_slice(),
                &command.now_ms,
            ],
        )
        .await
        .map_err(map_db)?;
    if updated != 1 {
        return Err(ExecutionError::InvalidRecord);
    }
    bump_job_revision(tx, &command.job, command.now_ms).await?;
    read_dispatch_view(tx, &command.owner, &command.dispatch).await
}
