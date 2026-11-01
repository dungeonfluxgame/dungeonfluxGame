//! A durable same-call cost tombstone closes a never-sent supplier projection obligation.
use super::common::{
    bump_job_revision, locked_dispatch, locked_job, map_db, require_read_committed,
};
use df_imagery::execution::{AcknowledgeUnsentProjection, ExecutionError, book_call_id_for};
use df_imagery::publication::NoDeliveryStage;
use tokio_postgres::Transaction;
pub(crate) async fn acknowledge_unsent_projection_in(
    tx: &Transaction<'_>,
    command: &AcknowledgeUnsentProjection,
) -> Result<(), ExecutionError> {
    require_read_committed(tx).await?;
    let s = &command.scope;
    if command.now_ms <= 0 || command.projection.call_id() != book_call_id_for(&s.dispatch) {
        return Err(ExecutionError::InvalidRecord);
    }
    locked_job(tx, &s.owner, &s.job).await?;
    let stored = crate::imagery_publication::read_no_delivery_in(tx, &s.owner, &s.job)
        .await
        .map_err(|_| ExecutionError::Unavailable)?
        .ok_or(ExecutionError::ObservationMissing)?;
    if stored != command.delivery
        || !matches!(&stored.tail,NoDeliveryStage::Dispatch{scope,usage_revision:0} if *scope==*s)
    {
        return Err(ExecutionError::BasisConflict);
    }
    let row = locked_dispatch(tx, &s.job, &s.stage, &s.dispatch).await?;
    if row.get::<_, String>("dispatch_state") != "released"
        || row.get::<_, String>("settlement") != "released"
        || row.get::<_, i32>("usage_revision") != 0
        || row.get::<_, String>("book_call_id") != command.projection.call_id()
    {
        return Err(ExecutionError::OperationConflict);
    }
    if tx
        .query_opt(
            "SELECT 1 FROM imagery.image_dispatch_usage WHERE dispatch_id=$1 LIMIT 1",
            &[&s.dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .is_some()
    {
        return Err(ExecutionError::OperationConflict);
    }
    if row.get::<_, bool>("cost_unsent_ack") {
        return Ok(());
    }
    tx.execute("UPDATE imagery.image_dispatch_attempt SET cost_unsent_ack=true,updated_at_ms=GREATEST(updated_at_ms,$2) WHERE dispatch_id=$1",&[&s.dispatch.as_bytes().as_slice(),&command.now_ms]).await.map_err(map_db)?;
    bump_job_revision(tx, &s.job, command.now_ms).await?;
    Ok(())
}
