//! Durable source-denied queue deferral returns unsent active capacity without inventing demand.
use super::common::{bump_job_revision, check_owner, locked_job, map_db, require_read_committed};
use df_imagery::execution::{ExecutionError, ExecutionStageId};
use df_imagery::{ImageJobId, ImageOwner};
use tokio_postgres::Transaction;

pub(crate) async fn defer_queued_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::DeferQueuedCommand,
) -> Result<(), ExecutionError> {
    let owner = &command.scope.owner;
    let job = &command.scope.job;
    let stage = &command.scope.stage;
    let not_before_ms = command.not_before_ms;
    let now_ms = command.now_ms;
    let notice = command
        .notice
        .map(df_imagery::execution::QuotaDeferralReason::label);
    require_read_committed(tx).await?;
    if now_ms <= 0 || not_before_ms <= now_ms {
        return Err(ExecutionError::InvalidRecord);
    }
    let (_, revision, _, _, _, _, job_bytes) = locked_job(tx, owner, job).await?;
    let row=tx.query_opt("SELECT owner_scope,stage_state,lease_until_ms,capacity_reserved,capacity_scope,queued_not_before_ms,queued_notice \
        FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_id=$2 FOR UPDATE",
        &[&job.as_bytes().as_slice(),&stage.as_bytes().as_slice()]).await.map_err(map_db)?.ok_or(ExecutionError::NotFound)?;
    check_owner(&row.get::<_, Vec<u8>>("owner_scope"), owner)?;
    let state: String = row.get("stage_state");
    if state == "released" {
        return Err(ExecutionError::Stale {
            current_revision: revision,
        });
    }
    if row.get::<_, i64>("lease_until_ms") > now_ms {
        return Err(ExecutionError::LostClaim);
    }
    if tx
        .query_opt(
            "SELECT 1 FROM imagery.image_dispatch_attempt WHERE job_id=$1 AND stage_id=$2 LIMIT 1",
            &[&job.as_bytes().as_slice(), &stage.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .is_some()
        || has_stage_finance(tx, owner, job, stage).await?
    {
        return Err(ExecutionError::Superseded);
    }
    let reserved: bool = row.get("capacity_reserved");
    if reserved && state != "admitted" || !reserved && state != "planned" {
        return Err(ExecutionError::InvalidRecord);
    }
    if !reserved
        && not_before_ms <= row.get::<_, i64>("queued_not_before_ms")
        && row.get::<_, Option<String>>("queued_notice").as_deref() == notice
    {
        return Ok(());
    }
    if reserved {
        let scope: String = row.get("capacity_scope");
        let changed = tx
            .execute(
                "UPDATE imagery.image_execution_capacity SET in_flight=in_flight-1, \
            pending_jobs=pending_jobs+1,pending_bytes=pending_bytes+$2,updated_at_ms=$3 \
            WHERE capacity_scope=$1 AND in_flight>0",
                &[&scope, &job_bytes, &now_ms],
            )
            .await
            .map_err(map_db)?;
        if changed != 1 {
            return Err(ExecutionError::InvalidRecord);
        }
    }
    tx.execute("UPDATE imagery.image_execution_stage SET stage_state='planned',capacity_reserved=false, \
        claim_token=stage_id,claim_generation=claim_generation+1,claim_config_revision=0,claim_lease_ms=0,lease_until_ms=0, \
        queued_not_before_ms=GREATEST(queued_not_before_ms,$3),queued_notice=$5,updated_at_ms=$4 WHERE job_id=$1 AND stage_id=$2",
        &[&job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&not_before_ms,&now_ms,&notice]).await.map_err(map_db)?;
    bump_job_revision(tx, job, now_ms).await?;
    Ok(())
}
pub(crate) async fn has_stage_finance(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<bool, ExecutionError> {
    tx.query_one(
        "SELECT imagery.stage_has_finance($1,$2,$3)",
        &[
            &df_imagery::owner_scope_bytes(owner),
            &job.as_bytes().as_slice(),
            &stage.as_bytes().as_slice(),
        ],
    )
    .await
    .map_err(map_db)?
    .get::<_, Option<bool>>(0)
    .ok_or(ExecutionError::NotFound)
}
