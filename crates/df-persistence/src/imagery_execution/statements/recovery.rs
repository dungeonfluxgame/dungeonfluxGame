//! Bounded recovery discovery, exclusive leases and durable completion receipts.
//! The owned job lock serializes recovery against normal claims and dispatch changes.

use super::common::{
    bump_job_revision, invalid, locked_job, map_db, parse_dispatch, parse_owner,
    require_read_committed, stage_kind_of,
};
use df_imagery::execution::{
    ClaimRecoveryCommand, ClaimToken, DispatchId, DispatchState, ExecutionError, ExecutionStageId,
    FinishRecoveryCommand, RecoveryDisposition, RecoveryLease, RecoveryStage,
};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};

const MAXIMUM_RECOVERY_SCAN_LIMIT: u32 = 128;

fn parse_stage(row: &Row, owner: &ImageOwner) -> Result<RecoveryStage, ExecutionError> {
    let current = parse_dispatch(row, owner)?;
    Ok(RecoveryStage {
        owner: owner.clone(),
        job: current.job,
        stage: current.stage,
        kind: stage_kind_of(&row.get::<_, String>("stage_kind"))?,
        claim: invalid(ClaimToken::from_bytes(
            &row.get::<_, Vec<u8>>("claim_token"),
        ))?,
        claim_generation: current.claim_generation,
        current,
    })
}

async fn lock_current(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<(RecoveryStage, i64), ExecutionError> {
    locked_job(tx, owner, job).await?;
    let row = tx
        .query_opt(
            "SELECT d.*, s.stage_kind, CASE WHEN d.dispatch_state='declined' THEN d.claim_lease_until_ms ELSE s.lease_until_ms END AS normal_lease_until_ms \
         FROM imagery.image_execution_stage s \
         JOIN imagery.image_stage_cursor c USING(job_id,stage_id) \
         JOIN imagery.image_dispatch_attempt d ON d.job_id=s.job_id AND d.stage_id=s.stage_id \
         WHERE s.job_id=$1 AND s.stage_id=$2 AND d.dispatch_id=$3 AND (c.current_dispatch=d.dispatch_id OR (d.dispatch_state='declined' AND EXISTS(SELECT 1 FROM imagery.image_decline_checkpoint p WHERE p.dispatch_id=d.dispatch_id))) FOR UPDATE OF s,c,d",
            &[
                &job.as_bytes().as_slice(),
                &stage.as_bytes().as_slice(),
                &dispatch.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::Superseded)?;
    let stage = parse_stage(&row, owner)?;
    if !matches!(
        stage.current.state,
        DispatchState::Declined
            | DispatchState::Sent
            | DispatchState::Unknown
            | DispatchState::Succeeded
            | DispatchState::Failed
    ) {
        return Err(ExecutionError::Superseded);
    }
    Ok((stage, row.get("normal_lease_until_ms")))
}

/// Scans due retained dispatches using a stable dispatch-id cursor; claiming rechecks every fence.
pub(crate) async fn scan_recovery_in(
    tx: &Transaction<'_>,
    scope: &str,
    after: Option<DispatchId>,
    limit: u32,
    now_ms: i64,
) -> Result<Vec<RecoveryStage>, ExecutionError> {
    require_read_committed(tx).await?;
    if scope.is_empty()
        || scope.len() > 128
        || limit == 0
        || limit > MAXIMUM_RECOVERY_SCAN_LIMIT
        || now_ms <= 0
    {
        return Err(ExecutionError::InvalidRecord);
    }
    let after = after.map(|dispatch| dispatch.as_bytes().to_vec());
    let rows = tx
        .query(
            "SELECT d.*, s.owner_scope, s.stage_kind FROM imagery.image_execution_stage s \
         JOIN imagery.image_stage_cursor c USING(job_id,stage_id) \
         JOIN imagery.image_dispatch_attempt d ON d.job_id=s.job_id AND d.stage_id=s.stage_id \
         LEFT JOIN imagery.image_dispatch_recovery r ON r.dispatch_id=d.dispatch_id \
         WHERE s.capacity_scope=$1 AND ($2::bytea IS NULL OR d.dispatch_id>$2) \
         AND ((d.dispatch_state='declined' AND d.claim_lease_until_ms<=$4 AND EXISTS(SELECT 1 FROM imagery.image_decline_checkpoint p WHERE p.dispatch_id=d.dispatch_id)) OR (c.current_dispatch=d.dispatch_id AND s.lease_until_ms<=$4 AND d.dispatch_state IN ('sent','unknown','succeeded','failed'))) \
         AND (r.dispatch_id IS NULL OR (r.recovery_state<>'completed' \
              AND r.lease_until_ms<=$4 AND r.next_at_ms<=$4)) \
         ORDER BY d.dispatch_id LIMIT $3",
            &[&scope, &after, &i64::from(limit), &now_ms],
        )
        .await
        .map_err(map_db)?;
    rows.iter()
        .map(|row| {
            let owner = parse_owner(&row.get::<_, Vec<u8>>("owner_scope"))?;
            parse_stage(row, &owner)
        })
        .collect()
}

/// Claims one retained dispatch after the normal owner lease lapses, without modifying its claim.
pub(crate) async fn claim_recovery_in(
    tx: &Transaction<'_>,
    command: &ClaimRecoveryCommand,
) -> Result<RecoveryLease, ExecutionError> {
    require_read_committed(tx).await?;
    if command.now_ms <= 0 || command.lease_ms <= 0 {
        return Err(ExecutionError::InvalidRecord);
    }
    let lease_until_ms = command
        .now_ms
        .checked_add(command.lease_ms)
        .ok_or(ExecutionError::InvalidRecord)?;
    let (stage, normal_lease) = lock_current(
        tx,
        &command.owner,
        &command.job,
        &command.stage,
        &command.dispatch,
    )
    .await?;
    let normal = tx
        .query_one(
            "SELECT claim_token FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_id=$2",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .get::<_, Vec<u8>>(0);
    if command.claim == stage.claim || normal == command.claim.as_bytes() {
        return Err(ExecutionError::BasisConflict);
    }
    let existing = tx
        .query_opt(
            "SELECT * FROM imagery.image_dispatch_recovery WHERE dispatch_id=$1 FOR UPDATE",
            &[&command.dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?;
    let generation = if let Some(row) = existing {
        let generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
        if row.get::<_, Vec<u8>>("claim_token") == command.claim.as_bytes() {
            if row.get::<_, i64>("lease_ms") != command.lease_ms {
                return Err(ExecutionError::BasisConflict);
            }
            return Ok(RecoveryLease {
                stage,
                token: command.claim,
                generation,
                lease_until_ms: row.get("lease_until_ms"),
            });
        }
        if row.get::<_, String>("recovery_state") == "completed" {
            return Err(ExecutionError::Superseded);
        }
        if row.get::<_, i64>("lease_until_ms") > command.now_ms
            || row.get::<_, i64>("next_at_ms") > command.now_ms
        {
            return Err(ExecutionError::LostClaim);
        }
        generation
            .checked_add(1)
            .ok_or(ExecutionError::InvalidRecord)?
    } else {
        1
    };
    if normal_lease > command.now_ms {
        return Err(ExecutionError::LostClaim);
    }
    tx.execute(
        "INSERT INTO imagery.image_dispatch_recovery \
        (dispatch_id,job_id,stage_id,claim_token,claim_generation,lease_ms,lease_until_ms, \
         recovery_state,next_at_ms,updated_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,'claimed',0,$8) \
        ON CONFLICT(dispatch_id) DO UPDATE SET claim_token=EXCLUDED.claim_token, \
        claim_generation=EXCLUDED.claim_generation,lease_ms=EXCLUDED.lease_ms, \
        lease_until_ms=EXCLUDED.lease_until_ms,recovery_state='claimed',next_at_ms=0, \
        updated_at_ms=EXCLUDED.updated_at_ms",
        &[
            &command.dispatch.as_bytes().as_slice(),
            &command.job.as_bytes().as_slice(),
            &command.stage.as_bytes().as_slice(),
            &command.claim.as_bytes().as_slice(),
            &invalid(i64::try_from(generation))?,
            &command.lease_ms,
            &lease_until_ms,
            &command.now_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    bump_job_revision(tx, &command.job, command.now_ms).await?;
    Ok(RecoveryLease {
        stage,
        token: command.claim,
        generation,
        lease_until_ms,
    })
}

/// Validates current exclusive recovery ownership for a caller-owned publication transaction.
pub(crate) async fn validate_recovery_in(
    tx: &Transaction<'_>,
    lease: &RecoveryLease,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let owner = &lease.stage.owner;
    let job = &lease.stage.job;
    let stage = &lease.stage.stage;
    let dispatch = &lease.stage.current.dispatch;
    let token = &lease.token;
    let generation = lease.generation;
    require_read_committed(tx).await?;
    if now_ms <= 0 || generation == 0 {
        return Err(ExecutionError::InvalidRecord);
    }

    let (_, normal_lease) = lock_current(tx, owner, job, stage, dispatch).await?;
    let row = tx
        .query_opt(
            "SELECT * FROM imagery.image_dispatch_recovery WHERE dispatch_id=$1 FOR UPDATE",
            &[&dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::LostClaim)?;
    if normal_lease > now_ms
        || row.get::<_, String>("recovery_state") != "claimed"
        || row.get::<_, Vec<u8>>("claim_token") != token.as_bytes()
        || row.get::<_, i64>("claim_generation") != invalid(i64::try_from(generation))?
        || row.get::<_, i64>("lease_until_ms") <= now_ms
    {
        return Err(ExecutionError::LostClaim);
    }
    Ok(())
}

async fn require_completed_receipts(
    tx: &Transaction<'_>,
    stage: &RecoveryStage,
) -> Result<(), ExecutionError> {
    if !matches!(
        stage.current.state,
        DispatchState::Succeeded | DispatchState::Failed | DispatchState::Declined
    ) || matches!(
        stage.current.settlement,
        df_imagery::execution::DispatchSettlement::Held
            | df_imagery::execution::DispatchSettlement::UnknownOutstanding
    ) {
        return Err(ExecutionError::OperationConflict);
    }
    if stage.current.state != DispatchState::Succeeded {
        return Ok(());
    }
    if super::terminal::sealed_dispatch_in(tx, stage).await? {
        return Ok(());
    }
    let prepared_child = tx
        .query_opt(
            "SELECT 1 FROM imagery.image_execution_stage s \
        JOIN imagery.image_stage_cursor c USING(job_id,stage_id) \
        JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=c.current_dispatch \
        JOIN imagery.image_execution_stage actor ON actor.job_id=s.job_id AND actor.stage_id=$3 \
        WHERE s.job_id=$1 AND d.dispatch_state='prepared' \
        AND (s.reconciliation_dispatch=$2 OR s.parent_dispatch=$2 \
            OR (actor.stage_kind='inspection' AND s.stage_kind='local_transform' \
                AND s.parent_dispatch=actor.parent_dispatch)) LIMIT 1",
            &[
                &stage.job.as_bytes().as_slice(),
                &stage.current.dispatch.as_bytes().as_slice(),
                &stage.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .is_some();
    if prepared_child {
        return Err(ExecutionError::OperationConflict);
    }
    if matches!(
        stage.kind,
        df_imagery::execution::StageKind::Classification
            | df_imagery::execution::StageKind::Inspection
    ) || stage.kind.is_standalone()
    {
        let decision = tx
            .query_opt(
                "SELECT 1 FROM imagery.image_decision_checkpoint c \
            JOIN imagery.image_decision_observation o USING(dispatch_id,observation_version) \
            WHERE o.owner_scope=$1 AND o.job_id=$2 AND o.stage_id=$3 AND o.dispatch_id=$4 \
            AND c.usage_revision=o.usage_revision",
                &[
                    &owner_scope_bytes(&stage.owner),
                    &stage.job.as_bytes().as_slice(),
                    &stage.stage.as_bytes().as_slice(),
                    &stage.current.dispatch.as_bytes().as_slice(),
                ],
            )
            .await
            .map_err(map_db)?
            .is_some();
        return if decision {
            Ok(())
        } else {
            Err(ExecutionError::ObservationMissing)
        };
    }
    let candidate = tx
        .query_opt(
            "SELECT 1 FROM imagery.image_candidate_checkpoint p \
        JOIN imagery.image_candidate_observation o USING(dispatch_id,observation_version) \
        WHERE o.owner_scope=$1 AND o.job_id=$2 AND o.stage_id=$3 AND o.dispatch_id=$4 \
        AND p.usage_revision=o.usage_revision",
            &[
                &owner_scope_bytes(&stage.owner),
                &stage.job.as_bytes().as_slice(),
                &stage.stage.as_bytes().as_slice(),
                &stage.current.dispatch.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .is_some();
    if candidate {
        let child = tx
            .query_opt(
                "SELECT 1 FROM imagery.image_execution_stage s \
            LEFT JOIN imagery.image_stage_cursor c USING(job_id,stage_id) \
            LEFT JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=c.current_dispatch \
            WHERE s.owner_scope=$1 AND s.job_id=$2 AND s.parent_dispatch=$3 \
            AND s.stage_kind IN ('inspection','local_transform') \
            AND ((c.current_dispatch IS NULL AND s.stage_state='planned') \
                OR d.dispatch_state IN ('sent','unknown','succeeded','failed')) LIMIT 1",
                &[
                    &owner_scope_bytes(&stage.owner),
                    &stage.job.as_bytes().as_slice(),
                    &stage.current.dispatch.as_bytes().as_slice(),
                ],
            )
            .await
            .map_err(map_db)?
            .is_some();
        return if child {
            Ok(())
        } else {
            Err(ExecutionError::OperationConflict)
        };
    }
    let table: Option<String> = tx
        .query_one("SELECT to_regclass('imagery.image_job_result')::text", &[])
        .await
        .map_err(map_db)?
        .get(0);
    if table.is_none() {
        return Err(ExecutionError::OperationConflict);
    }
    let published = tx
        .query_opt(
            "SELECT 1 FROM imagery.image_job_result r \
        JOIN media.asset a ON a.asset_id=r.asset_id AND a.attempt=r.publication_attempt \
        WHERE r.owner_scope=$1 AND r.job_id=$2 AND r.stage_id=$3 AND r.dispatch_id=$4 \
        AND a.state='ready'",
            &[
                &owner_scope_bytes(&stage.owner),
                &stage.job.as_bytes().as_slice(),
                &stage.stage.as_bytes().as_slice(),
                &stage.current.dispatch.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .is_some();
    if !published {
        return Err(ExecutionError::OperationConflict);
    }
    Ok(())
}

/// Records exact terminal receipt acknowledgment or a durable next-poll time, releasing the lease.
pub(crate) async fn finish_recovery_in(
    tx: &Transaction<'_>,
    command: &FinishRecoveryCommand,
) -> Result<(), ExecutionError> {
    require_read_committed(tx).await?;
    if command.now_ms <= 0 {
        return Err(ExecutionError::InvalidRecord);
    }
    locked_job(tx, &command.owner, &command.job).await?;
    let row = tx
        .query_opt(
            "SELECT * FROM imagery.image_dispatch_recovery WHERE dispatch_id=$1 AND job_id=$2 AND stage_id=$3 FOR UPDATE",
            &[&command.dispatch.as_bytes().as_slice(), &command.job.as_bytes().as_slice(), &command.stage.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::LostClaim)?;
    if row.get::<_, Vec<u8>>("claim_token") != command.claim.as_bytes()
        || row.get::<_, i64>("claim_generation")
            != invalid(i64::try_from(command.claim_generation))?
    {
        return Err(ExecutionError::LostClaim);
    }
    let (state, next_at_ms) = match command.disposition {
        RecoveryDisposition::Completed => ("completed", 0),
        RecoveryDisposition::RetryAt { next_at_ms } => ("retry", next_at_ms),
    };
    let stored: String = row.get("recovery_state");
    if stored != "claimed" {
        if stored == state && row.get::<_, i64>("next_at_ms") == next_at_ms {
            return Ok(());
        }
        return Err(ExecutionError::OperationConflict);
    }
    let (stage, _) = lock_current(
        tx,
        &command.owner,
        &command.job,
        &command.stage,
        &command.dispatch,
    )
    .await?;
    if state == "retry" && next_at_ms <= command.now_ms {
        return Err(ExecutionError::InvalidRecord);
    }
    let lease = RecoveryLease {
        stage: stage.clone(),
        token: command.claim,
        generation: command.claim_generation,
        lease_until_ms: row.get("lease_until_ms"),
    };
    validate_recovery_in(tx, &lease, command.now_ms).await?;
    if command.disposition == RecoveryDisposition::Completed {
        require_completed_receipts(tx, &stage).await?;
    }
    tx.execute(
        "UPDATE imagery.image_dispatch_recovery SET recovery_state=$2, \
        lease_until_ms=0,next_at_ms=$3,updated_at_ms=$4 WHERE dispatch_id=$1",
        &[
            &command.dispatch.as_bytes().as_slice(),
            &state,
            &next_at_ms,
            &command.now_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    bump_job_revision(tx, &command.job, command.now_ms).await?;
    Ok(())
}
