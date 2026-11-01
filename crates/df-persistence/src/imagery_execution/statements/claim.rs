//! Stage claims, the overall queue scan and stage release.
//!
//! A claim binds the worker to the actively applied gate revision and moves
//! the pending reserve to active concurrency on first claim. Lock order:
//! job first; gate verification stays locked through commit.

use df_imagery::execution::{
    ClaimCommand, ExecutionError, QueuedStage, ReleaseCommand, StageView, claim_owns, lease_is_live,
};
use tokio_postgres::Transaction;

use super::common::{
    attached_interest_count, bump_job_revision, bytes16, check_owner, invalid, locked_job, map_db,
    move_pending_to_active_in, optional_planned_pair, parse_stage, release_capacity_slot,
    require_gate_revision, require_read_committed, reserve_active_in,
};

/// Claims a stage for one owner: live interest, live deadline, an applied
/// gate matching the worker's revision, and a durable slot are all checked
/// atomically. Tokens are minted fresh per claim, so the stored token under
/// its original applied revision is an exact replay and returns the claim without
/// advancing it. An expired lease supersedes with a fresh token and the next
/// generation; another token under a live lease is a lost claim that stops
/// without deleting. The lease window must fit inside the retained job
/// deadline: no fixed ceiling, just the deadline the job carries.
pub(crate) async fn claim_stage_in(
    tx: &Transaction<'_>,
    command: &ClaimCommand,
) -> Result<StageView, ExecutionError> {
    require_read_committed(tx).await?;
    let (_scope, revision, deadline, state, planned_asset, planned_attempt, job_bytes) =
        locked_job(tx, &command.owner, &command.job).await?;
    let row = tx
        .query_opt(
            "SELECT job_id, stage_id, owner_scope, stage_kind, stage_state, \
             claim_token, claim_generation, claim_config_revision, claim_lease_ms, lease_until_ms, queued_not_before_ms, retry_dispatch_id, input_revision, input_digest, \
             config_revision, config_digest, capacity_scope, capacity_reserved, \
             local_accounting \
             FROM imagery.image_execution_stage \
             WHERE job_id = $1 AND stage_id = $2 FOR UPDATE",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::NotFound)?;
    let stored_scope: Vec<u8> = row.get("owner_scope");
    check_owner(&stored_scope, &command.owner)?;
    let stage_state: String = row.get("stage_state");
    if stage_state == "released" {
        return Err(ExecutionError::Stale {
            current_revision: revision,
        });
    }
    let scope_text: String = row.get("capacity_scope");
    let stored_token = bytes16(&row.get::<_, Vec<u8>>("claim_token"))?;
    let stored_generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
    let stored_lease: i64 = row.get("lease_until_ms");
    // Tokens are minted fresh per claim and never reused, so the stored
    // token identifies retained history even after its lease expires.
    if stored_generation > 0 && stored_token == *command.claim.as_bytes() {
        let applied: i64 = row.get("claim_config_revision");
        if invalid(u64::try_from(applied))? != command.expected_config_revision
            || row.get::<_, i64>("claim_lease_ms") != command.lease_ms
        {
            return Err(ExecutionError::BasisConflict);
        }
        let planned = optional_planned_pair(planned_asset, planned_attempt)?;
        return parse_stage(&row, &command.owner, revision, planned);
    }
    super::terminal::check_output_open_in(tx, &command.owner, &command.job).await?;
    if state == "retired" {
        return Err(ExecutionError::Retired);
    }
    if command.now_ms >= deadline {
        return Err(ExecutionError::Expired);
    }
    let not_before_ms: i64 = row.get("queued_not_before_ms");
    if not_before_ms > command.now_ms {
        return Err(ExecutionError::Deferred { not_before_ms });
    }
    if attached_interest_count(tx, &command.job).await? == 0 {
        return Err(ExecutionError::DemandLapsed);
    }
    require_gate_revision(tx, &scope_text, command.expected_config_revision).await?;
    let lease_until_ms = command.now_ms.saturating_add(command.lease_ms);
    if lease_until_ms > deadline {
        return Err(ExecutionError::Expired);
    }
    if lease_is_live(stored_lease, command.now_ms) {
        return Err(ExecutionError::LostClaim);
    }
    let recovering = tx.query_opt(
        "SELECT 1 FROM imagery.image_dispatch_recovery r \
         JOIN imagery.image_stage_cursor c ON c.current_dispatch=r.dispatch_id \
         WHERE c.job_id=$1 AND c.stage_id=$2 AND r.recovery_state='claimed' AND r.lease_until_ms>$3",
        &[&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice(),&command.now_ms],
    ).await.map_err(map_db)?.is_some();
    if recovering {
        return Err(ExecutionError::LostClaim);
    }
    let retry: Option<Vec<u8>> = row.get("retry_dispatch_id");
    if retry.is_none() && tx.query_opt("SELECT 1 FROM imagery.image_stage_cursor c JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=c.current_dispatch WHERE c.job_id=$1 AND c.stage_id=$2 AND d.dispatch_state='declined'",&[&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice()]).await.map_err(map_db)?.is_some(){return Err(ExecutionError::Superseded)}
    if let Some(retry) = &retry {
        super::decline::require_retry_in(
            tx,
            &df_imagery::execution::ExecutionScope {
                owner: command.owner.clone(),
                job: command.job,
                stage: command.stage,
                dispatch: invalid(df_imagery::execution::DispatchId::from_bytes(retry))?,
            },
            command.now_ms,
            true,
        )
        .await?;
    }
    let reserved: bool = row.get("capacity_reserved");
    if !reserved {
        let prior_dispatches: i64 = tx
            .query_one(
                "SELECT COUNT(*) FROM imagery.image_dispatch_attempt \
                 WHERE job_id = $1 AND stage_id = $2",
                &[
                    &command.job.as_bytes().as_slice(),
                    &command.stage.as_bytes().as_slice(),
                ],
            )
            .await
            .map_err(map_db)?
            .get("count");
        if prior_dispatches == 0 || (retry.is_some() && stage_state == "planned") {
            move_pending_to_active_in(tx, &scope_text, job_bytes, command.now_ms).await?;
        } else {
            reserve_active_in(tx, &scope_text, command.now_ms).await?;
        }
    }
    let next_generation = invalid(i64::try_from(stored_generation.saturating_add(1)))?;
    tx.execute(
        "UPDATE imagery.image_execution_stage SET claim_token = $3, claim_generation = $4, \
         lease_until_ms = $5, stage_state = 'admitted', capacity_reserved = true, queued_notice=NULL, \
         updated_at_ms = $6, claim_config_revision = $7, claim_lease_ms = $8 WHERE job_id = $1 AND stage_id = $2",
        &[
            &command.job.as_bytes().as_slice(),
            &command.stage.as_bytes().as_slice(),
            &command.claim.as_bytes().as_slice(),
            &next_generation,
            &lease_until_ms,
            &command.now_ms,
            &invalid(i64::try_from(command.expected_config_revision))?,
            &command.lease_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    let planned = optional_planned_pair(planned_asset, planned_attempt)?;
    let revision = bump_job_revision(tx, &command.job, command.now_ms).await?;
    let row = tx
        .query_one(
            "SELECT job_id, stage_id, owner_scope, stage_kind, stage_state, \
             claim_generation, lease_until_ms, input_revision, input_digest, \
             config_revision, config_digest, capacity_scope, local_accounting \
             FROM imagery.image_execution_stage WHERE job_id = $1 AND stage_id = $2",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?;
    parse_stage(&row, &command.owner, revision, planned)
}

/// The runtime's overall queue scan: planned or admitted stages in one
/// scope, oldest first. Reads only; claiming still rechecks the gate, lease
/// and capacity under lock.
pub(crate) async fn scan_queue_in(
    tx: &Transaction<'_>,
    scope: &str,
    limit: u32,
    now_ms: i64,
) -> Result<Vec<QueuedStage>, ExecutionError> {
    Ok(
        super::queue::scan_queue_after_in(tx, scope, None, limit, now_ms)
            .await?
            .stages,
    )
}

/// Releases a stage and retires its durable reserve. Only a stage with no
/// outstanding dispatch may release: prepared, sent and unknown dispatches
/// block it, so a sent attempt never loses its liability row. Terminal
/// outcomes already retired the reserve; release retires it otherwise.
pub(crate) async fn release_stage_in(
    tx: &Transaction<'_>,
    command: &ReleaseCommand,
) -> Result<StageView, ExecutionError> {
    require_read_committed(tx).await?;
    let (_scope, revision, _deadline, _state, planned_asset, planned_attempt, job_bytes) =
        locked_job(tx, &command.owner, &command.job).await?;
    let planned = optional_planned_pair(planned_asset, planned_attempt)?;
    let row = tx
        .query_opt(
            "SELECT job_id, stage_id, owner_scope, stage_kind, stage_state, \
             claim_token, claim_generation, lease_until_ms, input_revision, input_digest, \
             config_revision, config_digest, capacity_scope, capacity_reserved,retry_dispatch_id, \
             local_accounting FROM imagery.image_execution_stage \
             WHERE job_id = $1 AND stage_id = $2 FOR UPDATE",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::NotFound)?;
    let stored_scope: Vec<u8> = row.get("owner_scope");
    check_owner(&stored_scope, &command.owner)?;
    let stage_state: String = row.get("stage_state");
    if stage_state == "released" {
        return parse_stage(&row, &command.owner, revision, planned);
    }
    let stored_token = bytes16(&row.get::<_, Vec<u8>>("claim_token"))?;
    let stored_generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
    if !claim_owns(
        command.claim.as_bytes(),
        command.claim_generation,
        &stored_token,
        stored_generation,
    ) {
        return Err(ExecutionError::LostClaim);
    }
    let retry: Option<Vec<u8>> = row.get("retry_dispatch_id");
    let retry_claim = stage_state == "admitted"
        && row.get::<_, bool>("capacity_reserved")
        && row.get::<_, i64>("lease_until_ms") > command.now_ms
        && retry.is_some();
    let outstanding: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM imagery.image_dispatch_attempt \
             WHERE job_id = $1 AND stage_id = $2 \
             AND dispatch_state NOT IN ('succeeded', 'failed', 'released') AND NOT ($3::boolean AND dispatch_state='declined' AND EXISTS(SELECT 1 FROM imagery.image_decline_checkpoint p WHERE p.dispatch_id=image_dispatch_attempt.dispatch_id AND p.usage_revision=image_dispatch_attempt.usage_revision))",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
                &retry_claim,
            ],
        )
        .await
        .map_err(map_db)?
        .get("count");
    if outstanding != 0 {
        return Err(ExecutionError::Stale {
            current_revision: revision,
        });
    }
    let reserved: bool = row.get("capacity_reserved");
    let scope: String = row.get("capacity_scope");
    if reserved {
        release_capacity_slot(tx, &scope, command.now_ms).await?;
    } else if stage_state == "planned" {
        super::common::release_pending_in(tx, &scope, job_bytes, command.now_ms).await?;
    }
    tx.execute(
        "UPDATE imagery.image_execution_stage SET stage_state = 'released', \
         capacity_reserved = false, claim_generation = claim_generation + 1, lease_until_ms=0,retry_dispatch_id=NULL, \
         updated_at_ms = $3 WHERE job_id = $1 AND stage_id = $2",
        &[
            &command.job.as_bytes().as_slice(),
            &command.stage.as_bytes().as_slice(),
            &command.now_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    let revision = bump_job_revision(tx, &command.job, command.now_ms).await?;
    let row = tx
        .query_one(
            "SELECT job_id, stage_id, owner_scope, stage_kind, stage_state, \
             claim_generation, lease_until_ms, input_revision, input_digest, \
             config_revision, config_digest, capacity_scope, local_accounting \
             FROM imagery.image_execution_stage WHERE job_id = $1 AND stage_id = $2",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?;
    parse_stage(&row, &command.owner, revision, planned)
}
