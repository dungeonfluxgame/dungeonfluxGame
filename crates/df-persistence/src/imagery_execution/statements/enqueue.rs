//! Enqueue, capacity bounds and execution gates for the journal.
//!
//! Enqueue reserves a pending slot sized from the job's retained
//! intent/config bytes; claim later moves it to active concurrency. Lock
//! order: job first; gate verification precedes capacity mutation.

use df_imagery::execution::{
    CapacityBounds, EnqueueCommand, EnqueueOutcome, ExecutionError, ExecutionGate,
};
use tokio_postgres::Transaction;

use super::common::{
    attached_interest_count, bump_job_revision, check_owner, invalid, locked_job, map_db,
    parse_stage, require_gate_enabled, require_read_committed, reserve_pending_in, stage_kind_of,
};

fn stage_basis_matches(
    row: &tokio_postgres::Row,
    command: &EnqueueCommand,
    planned: Option<df_imagery::execution::PublicationIdentity>,
) -> Result<bool, ExecutionError> {
    let kind_text: String = row.get("stage_kind");
    let input_revision: i64 = row.get("input_revision");
    let input_digest: Vec<u8> = row.get("input_digest");
    let config_revision: i64 = row.get("config_revision");
    let config_digest: Vec<u8> = row.get("config_digest");
    let scope_text: String = row.get("capacity_scope");
    let local: bool = row.get("local_accounting");
    let parent: Option<Vec<u8>> = row.get("parent_dispatch");
    let same = parent.as_deref()
        == command
            .parent_dispatch
            .as_ref()
            .map(|parent| parent.as_bytes().as_slice())
        && stage_kind_of(&kind_text)? == command.kind
        && input_revision == i64::from(command.input_revision)
        && input_digest.as_slice() == command.input_digest
        && config_revision == invalid(i64::try_from(command.config_revision))?
        && config_digest.as_slice() == command.config_digest
        && scope_text == command.capacity.as_str()
        && local == command.local_accounting
        && planned == command.planned_publication;
    Ok(same)
}

/// Sets the durable operator bounds for one capacity scope, preserving the
/// current counters. Lowering a bound below current usage is allowed: new
/// work refuses while existing liability stays intact.
pub(crate) async fn set_capacity_bounds_in(
    tx: &Transaction<'_>,
    scope: &str,
    bounds: &CapacityBounds,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    require_read_committed(tx).await?;
    if scope.is_empty() || scope.len() > 128 || now_ms <= 0 {
        return Err(ExecutionError::InvalidRecord);
    }
    tx.execute(
        "INSERT INTO imagery.image_execution_capacity \
         (capacity_scope, bound, in_flight, pending_jobs, pending_bytes, \
          pending_jobs_bound, pending_bytes_bound, updated_at_ms,decision_payload_bytes_bound,urgency_promotion_ms) \
         VALUES ($1, $2, 0, 0, 0, $3, $4, $5,$6,$7) \
         ON CONFLICT (capacity_scope) DO UPDATE SET bound = EXCLUDED.bound, \
         pending_jobs_bound = EXCLUDED.pending_jobs_bound, \
         pending_bytes_bound = EXCLUDED.pending_bytes_bound, \
         decision_payload_bytes_bound=EXCLUDED.decision_payload_bytes_bound, \
         urgency_promotion_ms=EXCLUDED.urgency_promotion_ms, \
         updated_at_ms = EXCLUDED.updated_at_ms",
        &[
            &scope,
            &bounds.max_active,
            &bounds.max_pending_jobs,
            &bounds.max_pending_bytes,
            &now_ms,
            &bounds.max_decision_payload_bytes,
            &bounds.urgency_promotion_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    Ok(())
}

/// Sets the actively applied execution gate for one scope. A committed
/// disable stops claims, prepares and sends; reconciliation continues.
pub(crate) async fn set_execution_gate_in(
    tx: &Transaction<'_>,
    scope: &str,
    applied_revision: u64,
    enabled: bool,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    require_read_committed(tx).await?;
    if scope.is_empty() || scope.len() > 128 || applied_revision == 0 || now_ms <= 0 {
        return Err(ExecutionError::InvalidRecord);
    }
    let revision = invalid(i64::try_from(applied_revision))?;
    let changed = tx
        .execute(
            "INSERT INTO imagery.image_execution_gate \
         (gate_scope, applied_revision, enabled, updated_at_ms) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (gate_scope) DO UPDATE SET applied_revision = EXCLUDED.applied_revision, \
         enabled = EXCLUDED.enabled, updated_at_ms = EXCLUDED.updated_at_ms \
         WHERE imagery.image_execution_gate.applied_revision < EXCLUDED.applied_revision \
         OR (imagery.image_execution_gate.applied_revision = EXCLUDED.applied_revision \
             AND imagery.image_execution_gate.enabled = EXCLUDED.enabled)",
            &[&scope, &revision, &enabled, &now_ms],
        )
        .await
        .map_err(map_db)?;
    if changed != 1 {
        let (current_revision, _) = super::common::read_gate_in(tx, scope).await?;
        return Err(ExecutionError::ConfigFenced { current_revision });
    }
    Ok(())
}

/// Reads the actively applied execution gate for one scope.
pub(crate) async fn read_execution_gate_in(
    tx: &Transaction<'_>,
    scope: &str,
) -> Result<ExecutionGate, ExecutionError> {
    require_read_committed(tx).await?;
    let (applied_revision, enabled) = super::common::read_gate_in(tx, scope).await?;
    Ok(ExecutionGate {
        scope: scope.to_owned(),
        applied_revision,
        enabled,
    })
}

/// Plans one logical stage with its durable pending reserve. An identical
/// enqueue replays as `Existing` without reserving again; a different
/// immutable basis conflicts. The job lock serializes the insert with replay.
pub(crate) async fn enqueue_stage_in(
    tx: &Transaction<'_>,
    command: &EnqueueCommand,
) -> Result<EnqueueOutcome, ExecutionError> {
    enqueue_with_authority_in(tx, command, None).await
}

pub(super) async fn enqueue_reconciliation_stage_in(
    tx: &Transaction<'_>,
    command: &EnqueueCommand,
    parent: &df_imagery::execution::RecoveryLease,
) -> Result<EnqueueOutcome, ExecutionError> {
    enqueue_with_authority_in(tx, command, Some(parent)).await
}

async fn enqueue_with_authority_in(
    tx: &Transaction<'_>,
    command: &EnqueueCommand,
    recovery: Option<&df_imagery::execution::RecoveryLease>,
) -> Result<EnqueueOutcome, ExecutionError> {
    require_read_committed(tx).await?;
    let (_scope, revision, deadline, state, planned_asset, planned_attempt, job_bytes) =
        locked_job(tx, &command.owner, &command.job).await?;
    if command.kind.is_standalone() != command.planned_publication.is_none() {
        return Err(ExecutionError::PlannedIdentityConflict);
    }
    let stored = super::common::optional_planned_pair(planned_asset, planned_attempt)?;
    let mixed:bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM imagery.image_execution_stage WHERE job_id=$1 \
        AND (stage_kind IN ('photo_inspection','head_locations','segmentation','staging_identity','staging_facing'))<>$2)",
        &[&command.job.as_bytes().as_slice(),&command.kind.is_standalone()]).await.map_err(map_db)?.get(0);
    if mixed || stored.is_some() && stored != command.planned_publication {
        return Err(ExecutionError::PlannedIdentityConflict);
    }
    if stored.is_none() {
        if let Some(publication) = command.planned_publication {
            tx.execute("UPDATE imagery.image_job SET planned_publication_asset=$2,planned_publication_attempt=$3,updated_at_ms=$4 WHERE job_id=$1",
                &[&command.job.as_bytes().as_slice(),&publication.asset.as_slice(),&publication.attempt.as_slice(),&command.now_ms])
                .await.map_err(map_db)?;
        }
    }
    let planned = command.planned_publication;
    if let Some(row) = tx
        .query_opt(
            "SELECT job_id, stage_id, owner_scope, stage_kind, stage_state, claim_generation, \
             lease_until_ms, input_revision, input_digest, config_revision, config_digest, \
             capacity_scope, local_accounting,parent_dispatch FROM imagery.image_execution_stage \
             WHERE job_id = $1 AND (stage_id = $2 OR ($3::bytea IS NOT NULL \
                 AND parent_dispatch=$3 AND stage_kind=$4)) ORDER BY (stage_id=$2) DESC,stage_id LIMIT 1 FOR UPDATE",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
                &command
                    .parent_dispatch
                    .map(|parent| parent.as_bytes().to_vec()),
                &command.kind.label(),
            ],
        )
        .await
        .map_err(map_db)?
    {
        let stored: Vec<u8> = row.get("owner_scope");
        check_owner(&stored, &command.owner)?;
        if !stage_basis_matches(&row, command, planned)? {
            return Err(ExecutionError::BasisConflict);
        }
        let view = parse_stage(&row, &command.owner, revision, planned)?;
        return Ok(EnqueueOutcome::Existing(view));
    }
    super::terminal::check_output_open_in(tx, &command.owner, &command.job).await?;
    if let Some(parent) = recovery {
        super::recovery::validate_recovery_in(tx, parent, command.now_ms).await?;
        if command.kind != df_imagery::execution::StageKind::LocalTransform
            || !command.local_accounting
            || command.owner != parent.stage.owner
            || command.job != parent.stage.job
            || command.parent_dispatch.is_none()
        {
            return Err(ExecutionError::BasisConflict);
        }
    } else {
        if state == "retired" {
            return Err(ExecutionError::Retired);
        }
        if command.now_ms >= deadline {
            return Err(ExecutionError::Expired);
        }
        if attached_interest_count(tx, &command.job).await? == 0 {
            return Err(ExecutionError::DemandLapsed);
        }
    }
    let parent_stage = if let Some(parent) = command.parent_dispatch {
        let parent_row = tx
            .query_opt(
                "SELECT d.stage_id FROM imagery.image_dispatch_attempt d \
            JOIN imagery.image_stage_cursor c ON c.current_dispatch=d.dispatch_id \
            WHERE d.job_id=$1 AND d.dispatch_id=$2 AND d.dispatch_state='succeeded'",
                &[
                    &command.job.as_bytes().as_slice(),
                    &parent.as_bytes().as_slice(),
                ],
            )
            .await
            .map_err(map_db)?
            .ok_or(ExecutionError::BasisConflict)?;
        let parent_stage: Vec<u8> = parent_row.get("stage_id");
        if parent_stage.as_slice() == command.stage.as_bytes() {
            return Err(ExecutionError::BasisConflict);
        }
        Some(parent_stage)
    } else {
        None
    };
    if recovery.is_none() {
        require_gate_enabled(tx, command.capacity.as_str()).await?;
    }
    reserve_pending_in(tx, command.capacity.as_str(), job_bytes, command.now_ms).await?;
    let capacity_scope = command.capacity.as_str();
    let config_revision = invalid(i64::try_from(command.config_revision))?;
    let input_revision = i64::from(command.input_revision);
    let kind_label = command.kind.label();
    let owner_scope = df_imagery::owner_scope_bytes(&command.owner);
    let inserted = tx
        .execute(
            "INSERT INTO imagery.image_execution_stage (job_id, stage_id, owner_scope, \
             stage_kind, stage_state, claim_token, claim_generation, lease_until_ms, \
             input_revision, input_digest, config_revision, config_digest, capacity_scope, \
             capacity_reserved, local_accounting, created_at_ms, updated_at_ms,parent_stage_id,parent_dispatch) \
             VALUES ($1, $2, $3, $4, 'planned', $2, 0, 0, $5, $6, $7, $8, $9, false, $10, $11, $11,$12,$13)",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
                &owner_scope,
                &kind_label,
                &input_revision,
                &command.input_digest.as_slice(),
                &config_revision,
                &command.config_digest.as_slice(),
                &capacity_scope,
                &command.local_accounting,
                &command.now_ms,
                &parent_stage,
                &command.parent_dispatch.map(|parent|parent.as_bytes().to_vec()),
            ],
        )
        .await;
    inserted.map_err(map_db)?;
    if matches!(
        command.kind,
        df_imagery::execution::StageKind::Generation
            | df_imagery::execution::StageKind::Edit
            | df_imagery::execution::StageKind::Extraction
            | df_imagery::execution::StageKind::LocalTransform
    ) {
        tx.execute(
            "UPDATE imagery.image_job SET planned_output_stage = $2 \
            WHERE job_id = $1 AND planned_output_stage IS NULL",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?;
    }
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
    let view = parse_stage(&row, &command.owner, revision, planned)?;
    Ok(EnqueueOutcome::Enqueued(view))
}

/// Reserves a strictly newer startup epoch with a compare-and-set against the durable gate.
pub(crate) async fn advance_execution_gate_in(
    tx: &Transaction<'_>,
    scope: &str,
    expected_revision: u64,
    enabled: bool,
    now_ms: i64,
) -> Result<ExecutionGate, ExecutionError> {
    require_read_committed(tx).await?;
    if scope.is_empty() || scope.len() > 128 || now_ms <= 0 {
        return Err(ExecutionError::InvalidRecord);
    }
    let expected = invalid(i64::try_from(expected_revision))?;
    let next = expected
        .checked_add(1)
        .ok_or(ExecutionError::InvalidRecord)?;
    let query = if expected == 0 {
        "INSERT INTO imagery.image_execution_gate(gate_scope,applied_revision,enabled,updated_at_ms) \
            VALUES($1,$2,$3,$4) ON CONFLICT(gate_scope) DO NOTHING"
    } else {
        "UPDATE imagery.image_execution_gate SET applied_revision=$2,enabled=$3,updated_at_ms=$4 \
            WHERE gate_scope=$1 AND applied_revision=$5"
    };
    let changed = if expected == 0 {
        tx.execute(query, &[&scope, &next, &enabled, &now_ms]).await
    } else {
        tx.execute(query, &[&scope, &next, &enabled, &now_ms, &expected])
            .await
    }
    .map_err(map_db)?;
    if changed != 1 {
        let current_revision = match super::common::read_gate_in(tx, scope).await {
            Ok((revision, _)) => revision,
            Err(ExecutionError::ConfigFenced {
                current_revision: 0,
            }) => 0,
            Err(error) => return Err(error),
        };
        return Err(ExecutionError::ConfigFenced { current_revision });
    }
    Ok(ExecutionGate {
        scope: scope.to_owned(),
        applied_revision: invalid(u64::try_from(next))?,
        enabled,
    })
}
