//! Dispatch attempts: prepare, send, supplier correlation, completion,
//! settlement mirroring and cancellation.
//!
//! A new send never supersedes a prepared, sent or unknown current attempt:
//! the cursor names one current dispatch and only it may move. Terminal
//! replay compares the full outcome; unknown resolves late without a new
//! send. The job lock serializes stage, dispatch, cursor and checkpoint rows;
//! gate checks stay locked through commit.

use df_imagery::execution::{
    CancelCommand, CompleteCommand, CompletionOutcome, DispatchSettlement, DispatchState,
    DispatchView, ExecutionError, MarkSentCommand, PrepareCommand, SupplierCheckpoint,
    SupplierReport, book_call_id_for, completion_releases_capacity, sender_may_complete,
};
use df_imagery::{ImageJobId, ImageOwner};
use tokio_postgres::{Row, Transaction};

type FinanceColumns = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);

use super::common::{
    bump_job_revision, bytes16, invalid, locked_claim, locked_dispatch, locked_job, map_db,
    parse_checkpoint, parse_supplier_checkpoint, read_dispatch_view, release_capacity_slot,
    require_gate_revision, require_read_committed, reserve_active_in,
};

fn dispatch_matches_prepare(
    row: &Row,
    command: &PrepareCommand,
    local: bool,
) -> Result<bool, ExecutionError> {
    let attempt: Option<Vec<u8>> = row.get("finance_attempt");
    let operation: Option<Vec<u8>> = row.get("finance_operation");
    let effect: Option<Vec<u8>> = row.get("finance_effect");
    let stored = match (attempt, operation, effect) {
        (Some(attempt), Some(operation), Some(effect)) => Some((attempt, operation, effect)),
        (None, None, None) => None,
        _ => return Err(ExecutionError::InvalidRecord),
    };
    let wanted = command.mode.finance().map(|tuple| {
        (
            tuple.attempt.to_vec(),
            tuple.operation.to_vec(),
            tuple.effect.to_vec(),
        )
    });
    if stored != wanted {
        return Ok(false);
    }
    let row_local: bool = row.get("local_accounting");
    if row_local != local {
        return Ok(false);
    }
    let stored_token = bytes16(&row.get::<_, Vec<u8>>("claim_token"))?;
    let stored_generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
    if stored_token != *command.claim.as_bytes() || stored_generation != command.claim_generation {
        return Ok(false);
    }
    let stored_call: String = row.get("book_call_id");
    Ok(stored_call == book_call_id_for(&command.dispatch))
}

/// Reads the stored output checkpoint for one dispatch, if any.
async fn stored_checkpoint(
    tx: &Transaction<'_>,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<Option<df_imagery::execution::OutputCheckpoint>, ExecutionError> {
    let row = tx
        .query_opt(
            "SELECT storage_key, object_version, byte_len, sha256, recipe_digest, mime, \
             width_px, height_px, alpha, usage_revision, book_call_id \
             FROM imagery.image_dispatch_checkpoint WHERE dispatch_id = $1",
            &[&dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?;
    row.map(|row| parse_checkpoint(&row, dispatch)).transpose()
}

/// Writes the checkpoint with winning-write semantics: the first committed
/// write stands and a conflicting replay is refused, never overwritten.
async fn write_winning_checkpoint(
    tx: &Transaction<'_>,
    dispatch: &df_imagery::execution::DispatchId,
    checkpoint: &df_imagery::execution::OutputCheckpoint,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let revision = invalid(i32::try_from(checkpoint.usage_revision))?;
    let object_version = &checkpoint.object_version;
    let byte_len = invalid(i64::try_from(checkpoint.byte_len))?;
    let width_px = checkpoint
        .width_px
        .map(i32::try_from)
        .transpose()
        .map_err(|_| ExecutionError::InvalidRecord)?;
    let height_px = checkpoint
        .height_px
        .map(i32::try_from)
        .transpose()
        .map_err(|_| ExecutionError::InvalidRecord)?;
    tx.execute(
        "INSERT INTO imagery.image_dispatch_checkpoint (dispatch_id, storage_key, \
         object_version, byte_len, sha256, recipe_digest, mime, width_px, height_px, \
         alpha, usage_revision, book_call_id, recorded_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
         ON CONFLICT (dispatch_id) DO NOTHING",
        &[
            &dispatch.as_bytes().as_slice(),
            &checkpoint.storage_key,
            &object_version,
            &byte_len,
            &checkpoint.sha256.as_slice(),
            &checkpoint.recipe_digest.as_slice(),
            &checkpoint.mime,
            &width_px,
            &height_px,
            &checkpoint.alpha.label(),
            &revision,
            &checkpoint.book_call_id,
            &now_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    let stored = stored_checkpoint(tx, dispatch)
        .await?
        .ok_or(ExecutionError::InvalidRecord)?;
    if stored != *checkpoint {
        return Err(ExecutionError::OperationConflict);
    }
    Ok(())
}

/// Frees the stage reserve once a dispatch reaches a capacity-releasing state.
/// Unknown keeps its liability and its reserve for reconciliation.
pub(super) async fn maybe_release_reserve(
    tx: &Transaction<'_>,
    job: &ImageJobId,
    stage: &df_imagery::execution::ExecutionStageId,
    state: DispatchState,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    if !completion_releases_capacity(state) {
        return Ok(());
    }
    let row = tx
        .query_one(
            "SELECT capacity_scope, capacity_reserved FROM imagery.image_execution_stage \
             WHERE job_id = $1 AND stage_id = $2",
            &[&job.as_bytes().as_slice(), &stage.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?;
    let reserved: bool = row.get("capacity_reserved");
    if !reserved {
        return Ok(());
    }
    let scope: String = row.get("capacity_scope");
    release_capacity_slot(tx, &scope, now_ms).await?;
    tx.execute(
        "UPDATE imagery.image_execution_stage SET capacity_reserved = false, \
         updated_at_ms = $3 WHERE job_id = $1 AND stage_id = $2",
        &[
            &job.as_bytes().as_slice(),
            &stage.as_bytes().as_slice(),
            &now_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    Ok(())
}

/// Prepares one send: a fresh dispatch id with the next sequence, immutable
/// retry lineage to the previous current dispatch, and the all-or-none
/// finance tuple (local work carries none). The finance owner commits its own
/// prepare first; this row is the durable execution half that transport reads.
/// An identical dispatch id replays without writing; a mismatched one
/// conflicts. A new id while the cursor names a prepared, sent or unknown
/// dispatch is refused, never superseded: a retry needs a proven terminal
/// predecessor plus a fresh finance admission and a new capacity reservation.
/// The cursor moves to the new dispatch in the same transaction.
pub(crate) async fn prepare_dispatch_in(
    tx: &Transaction<'_>,
    command: &PrepareCommand,
) -> Result<DispatchView, ExecutionError> {
    require_read_committed(tx).await?;
    let (_scope, _revision, deadline, state, _asset, _attempt, _job_bytes) =
        locked_job(tx, &command.owner, &command.job).await?;
    if let Some(existing) = tx
        .query_opt(
            "SELECT job_id, stage_id, dispatch_id, dispatch_seq, dispatch_state, \
             failure_cause, settlement, finance_attempt, finance_operation, finance_effect, \
             supplier_job_id, usage_revision, book_call_id, claim_token, claim_generation, \
             local_accounting, created_at_ms, updated_at_ms \
             FROM imagery.image_dispatch_attempt WHERE dispatch_id = $1",
            &[&command.dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
    {
        let row_job: Vec<u8> = existing.get("job_id");
        let row_stage: Vec<u8> = existing.get("stage_id");
        if row_job.as_slice() != command.job.as_bytes()
            || row_stage.as_slice() != command.stage.as_bytes()
        {
            return Err(ExecutionError::NotFound);
        }
        if dispatch_matches_prepare(&existing, command, command.mode.is_local())? {
            return read_dispatch_view(tx, &command.owner, &command.dispatch).await;
        }
        return Err(ExecutionError::IdentityConflict);
    }
    super::terminal::check_output_open_in(tx, &command.owner, &command.job).await?;
    if state == "retired" {
        return Err(ExecutionError::Retired);
    }
    if command.now_ms >= deadline {
        return Err(ExecutionError::Expired);
    }
    let stage_row = locked_claim(
        tx,
        &command.owner,
        &command.job,
        &command.stage,
        &command.claim,
        command.claim_generation,
        command.now_ms,
    )
    .await?;
    let local: bool = stage_row.get("local_accounting");
    match (&command.mode, local) {
        (df_imagery::execution::DispatchMode::Local, true) => {}
        (df_imagery::execution::DispatchMode::Hosted(_), false) => {}
        _ => return Err(ExecutionError::BasisConflict),
    }
    let scope_text: String = stage_row.get("capacity_scope");
    let claim_revision: i64 = stage_row.get("claim_config_revision");
    require_gate_revision(tx, &scope_text, invalid(u64::try_from(claim_revision))?).await?;
    let cursor = tx
        .query_opt(
            "SELECT current_dispatch FROM imagery.image_stage_cursor \
             WHERE job_id = $1 AND stage_id = $2 FOR UPDATE",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?;
    let parent: Option<Vec<u8>> = match cursor {
        Some(row) => {
            let current: Vec<u8> = row.get("current_dispatch");
            let current_row = tx
                .query_opt(
                    "SELECT dispatch_state FROM imagery.image_dispatch_attempt \
                     WHERE dispatch_id = $1",
                    &[&current],
                )
                .await
                .map_err(map_db)?
                .ok_or(ExecutionError::InvalidRecord)?;
            let current_state: String = current_row.get("dispatch_state");
            match current_state.as_str() {
                "prepared" | "sent" | "unknown" => {
                    return Err(ExecutionError::Superseded);
                }
                "declined" => {
                    let retry:Option<Vec<u8>>=tx.query_one("SELECT retry_dispatch_id FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_id=$2",&[&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice()]).await.map_err(map_db)?.get(0);
                    if retry.as_deref() != Some(current.as_slice()) {
                        return Err(ExecutionError::Superseded);
                    }
                    super::decline::require_retry_in(
                        tx,
                        &df_imagery::execution::ExecutionScope {
                            owner: command.owner.clone(),
                            job: command.job,
                            stage: command.stage,
                            dispatch: invalid(df_imagery::execution::DispatchId::from_bytes(
                                &current,
                            ))?,
                        },
                        command.now_ms,
                        true,
                    )
                    .await?;
                    Some(current)
                }
                "succeeded" | "failed" | "released" => Some(current),
                _ => return Err(ExecutionError::InvalidRecord),
            }
        }
        None => None,
    };
    let max_row = tx
        .query_one(
            "SELECT COALESCE(MAX(dispatch_seq), -1) FROM imagery.image_dispatch_attempt \
             WHERE job_id = $1 AND stage_id = $2",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?;
    let max_seen: i32 = max_row.get("coalesce");
    let sequence = df_imagery::execution::next_sequence(Some(max_seen));
    let reserved: bool = stage_row.get("capacity_reserved");
    if !reserved {
        reserve_active_in(tx, &scope_text, command.now_ms).await?;
        tx.execute(
            "UPDATE imagery.image_execution_stage SET capacity_reserved = true, \
             updated_at_ms = $3 WHERE job_id = $1 AND stage_id = $2",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
                &command.now_ms,
            ],
        )
        .await
        .map_err(map_db)?;
    }
    let finance = command.mode.finance();
    let (attempt, operation, effect): FinanceColumns = match finance {
        Some(tuple) => (
            Some(tuple.attempt.to_vec()),
            Some(tuple.operation.to_vec()),
            Some(tuple.effect.to_vec()),
        ),
        None => (None, None, None),
    };
    let book_call_id = book_call_id_for(&command.dispatch);
    let inserted = tx
        .execute(
            "INSERT INTO imagery.image_dispatch_attempt (dispatch_id, job_id, stage_id, \
             dispatch_seq, dispatch_state, settlement, finance_attempt, finance_operation, \
             finance_effect, usage_revision, book_call_id, lineage_parent, claim_token, \
             claim_generation, local_accounting, created_at_ms, updated_at_ms,claim_lease_until_ms) \
             VALUES ($1, $2, $3, $4, 'prepared', 'held', $5, $6, $7, 0, $8, $9, $10, $11, $12, $13, $13,$14) ON CONFLICT DO NOTHING",
            &[
                &command.dispatch.as_bytes().as_slice(),
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
                &sequence,
                &attempt,
                &operation,
                &effect,
                &book_call_id,
                &parent,
                &command.claim.as_bytes().as_slice(),
                &invalid(i64::try_from(command.claim_generation))?,
                &local,
                &command.now_ms,
                &stage_row.get::<_,i64>("lease_until_ms"),
            ],
        )
        .await;
    if inserted.map_err(map_db)? != 1 {
        return Err(ExecutionError::IdentityConflict);
    }
    tx.execute(
        "INSERT INTO imagery.image_stage_cursor \
         (job_id, stage_id, current_dispatch, refetch_dispatch, updated_at_ms) \
         VALUES ($1, $2, $3, NULL, $4) \
         ON CONFLICT (job_id, stage_id) DO UPDATE SET current_dispatch = EXCLUDED.current_dispatch, \
         updated_at_ms = EXCLUDED.updated_at_ms",
        &[
            &command.job.as_bytes().as_slice(),
            &command.stage.as_bytes().as_slice(),
            &command.dispatch.as_bytes().as_slice(),
            &command.now_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    tx.execute("UPDATE imagery.image_execution_stage SET retry_dispatch_id=NULL WHERE job_id=$1 AND stage_id=$2",&[&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice()]).await.map_err(map_db)?;
    bump_job_revision(tx, &command.job, command.now_ms).await?;
    read_dispatch_view(tx, &command.owner, &command.dispatch).await
}

/// Commits the durable Sent marker after the finance owner committed its own.
/// Transport launches only after this row commits. The cursor must name this
/// dispatch and its stored claim snapshot must match the presenter, so two
/// ids can never become sent under one claim. An already-sent dispatch
/// rereads as-is under the same claim; anything else refuses, so ambiguous
/// work is never resent from here.
pub(crate) async fn mark_sent_in(
    tx: &Transaction<'_>,
    command: &MarkSentCommand,
) -> Result<DispatchView, ExecutionError> {
    require_read_committed(tx).await?;
    let (_scope, _revision, deadline, state, _asset, _attempt, _job_bytes) =
        locked_job(tx, &command.owner, &command.job).await?;
    let row = locked_dispatch(tx, &command.job, &command.stage, &command.dispatch).await?;
    if row.get::<_, bool>("local_accounting") {
        super::sent::mark_local_sent_in(tx, command).await?;
        return read_dispatch_view(tx, &command.owner, &command.dispatch).await;
    }
    let stored_token = bytes16(&row.get::<_, Vec<u8>>("claim_token"))?;
    let stored_generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
    if stored_token != *command.claim.as_bytes() || stored_generation != command.claim_generation {
        return Err(ExecutionError::LostClaim);
    }
    let state_text: String = row.get("dispatch_state");
    let cursor = tx
        .query_opt(
            "SELECT current_dispatch FROM imagery.image_stage_cursor \
             WHERE job_id = $1 AND stage_id = $2 FOR UPDATE",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::Superseded)?;
    let current: Vec<u8> = cursor.get("current_dispatch");
    if !sender_may_complete(&bytes16(&current)?, command.dispatch.as_bytes()) {
        return Err(ExecutionError::Superseded);
    }
    if matches!(state_text.as_str(), "sent" | "succeeded") {
        return read_dispatch_view(tx, &command.owner, &command.dispatch).await;
    }
    super::terminal::check_output_open_in(tx, &command.owner, &command.job).await?;
    if state == "retired" {
        return Err(ExecutionError::Retired);
    }
    if command.now_ms >= deadline {
        return Err(ExecutionError::Expired);
    }
    if super::common::attached_interest_count(tx, &command.job).await? == 0 {
        return Err(ExecutionError::DemandLapsed);
    }
    let stage_row = locked_claim(
        tx,
        &command.owner,
        &command.job,
        &command.stage,
        &command.claim,
        command.claim_generation,
        command.now_ms,
    )
    .await?;
    let scope_text: String = stage_row.get("capacity_scope");
    let claim_revision: i64 = stage_row.get("claim_config_revision");
    require_gate_revision(tx, &scope_text, invalid(u64::try_from(claim_revision))?).await?;
    match state_text.as_str() {
        "prepared" => {
            tx.execute(
                "UPDATE imagery.image_dispatch_attempt SET dispatch_state = 'sent', \
                 updated_at_ms = $2 WHERE dispatch_id = $1",
                &[&command.dispatch.as_bytes().as_slice(), &command.now_ms],
            )
            .await
            .map_err(map_db)?;
            bump_job_revision(tx, &command.job, command.now_ms).await?;
            read_dispatch_view(tx, &command.owner, &command.dispatch).await
        }
        "sent" | "succeeded" => read_dispatch_view(tx, &command.owner, &command.dispatch).await,
        _ => Err(ExecutionError::Superseded),
    }
}

/// Correlates a supplier receipt with its dispatch. Recording the same id
/// twice is one idempotent update; a different id over a recorded one is a
/// conflict that writes nothing. Needs no live claim: late receipts still
/// correlate after a worker moves on.
pub(crate) async fn record_supplier_in(
    tx: &Transaction<'_>,
    command: &SupplierReport,
) -> Result<DispatchView, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, &command.owner, &command.job).await?;
    let row = locked_dispatch(tx, &command.job, &command.stage, &command.dispatch).await?;
    let stored: Option<String> = row.get("supplier_job_id");
    match stored {
        None => {
            tx.execute(
                "UPDATE imagery.image_dispatch_attempt SET supplier_job_id = $2, \
                 updated_at_ms = $3 WHERE dispatch_id = $1",
                &[
                    &command.dispatch.as_bytes().as_slice(),
                    &command.supplier_job_id,
                    &command.now_ms,
                ],
            )
            .await
            .map_err(map_db)?;
            bump_job_revision(tx, &command.job, command.now_ms).await?;
        }
        Some(existing) if existing == command.supplier_job_id => {}
        Some(_) => return Err(ExecutionError::SupplierConflict),
    }
    read_dispatch_view(tx, &command.owner, &command.dispatch).await
}

/// Retains one versioned supplier resume checkpoint for fal/BFL resume
/// without resending. An identical version replays; a conflicting one is
/// refused and the winning write stands.
pub(crate) async fn record_supplier_checkpoint_in(
    tx: &Transaction<'_>,
    checkpoint: &SupplierCheckpoint,
) -> Result<SupplierCheckpoint, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, &checkpoint.owner, &checkpoint.job).await?;
    locked_dispatch(tx, &checkpoint.job, &checkpoint.stage, &checkpoint.dispatch).await?;
    let version = invalid(i32::try_from(checkpoint.checkpoint_version))?;
    if let Some(row) = tx
        .query_opt(
            "SELECT dispatch_id, checkpoint_version, supplier_origin, supplier_model, \
             checkpoint_bytes, recorded_at_ms \
             FROM imagery.image_supplier_checkpoint \
             WHERE dispatch_id = $1 AND checkpoint_version = $2",
            &[&checkpoint.dispatch.as_bytes().as_slice(), &version],
        )
        .await
        .map_err(map_db)?
    {
        let stored = parse_supplier_checkpoint(
            &row,
            &checkpoint.owner,
            &checkpoint.job,
            &checkpoint.stage,
            &checkpoint.dispatch,
        )?;
        if stored.origin == checkpoint.origin
            && stored.model == checkpoint.model
            && stored.checkpoint_bytes == checkpoint.checkpoint_bytes
        {
            return Ok(stored);
        }
        return Err(ExecutionError::SupplierConflict);
    }
    tx.execute(
        "INSERT INTO imagery.image_supplier_checkpoint (dispatch_id, checkpoint_version, \
         supplier_origin, supplier_model, checkpoint_bytes, recorded_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
        &[
            &checkpoint.dispatch.as_bytes().as_slice(),
            &version,
            &checkpoint.origin,
            &checkpoint.model,
            &checkpoint.checkpoint_bytes.as_slice(),
            &checkpoint.now_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    let row = tx
        .query_opt(
            "SELECT dispatch_id, checkpoint_version, supplier_origin, supplier_model, \
             checkpoint_bytes, recorded_at_ms \
             FROM imagery.image_supplier_checkpoint \
             WHERE dispatch_id = $1 AND checkpoint_version = $2",
            &[&checkpoint.dispatch.as_bytes().as_slice(), &version],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::InvalidRecord)?;
    let stored = parse_supplier_checkpoint(
        &row,
        &checkpoint.owner,
        &checkpoint.job,
        &checkpoint.stage,
        &checkpoint.dispatch,
    )?;
    if stored.origin == checkpoint.origin
        && stored.model == checkpoint.model
        && stored.checkpoint_bytes == checkpoint.checkpoint_bytes
    {
        bump_job_revision(tx, &checkpoint.job, checkpoint.now_ms).await?;
        Ok(stored)
    } else {
        Err(ExecutionError::SupplierConflict)
    }
}

/// The latest retained supplier checkpoint for one dispatch, if any.
pub(crate) async fn read_supplier_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &df_imagery::execution::ExecutionStageId,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<Option<SupplierCheckpoint>, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    locked_dispatch(tx, job, stage, dispatch).await?;
    let row = tx
        .query_opt(
            "SELECT dispatch_id, checkpoint_version, supplier_origin, supplier_model, \
             checkpoint_bytes, recorded_at_ms \
             FROM imagery.image_supplier_checkpoint WHERE dispatch_id = $1 \
             ORDER BY checkpoint_version DESC LIMIT 1",
            &[&dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?;
    row.map(|row| parse_supplier_checkpoint(&row, owner, job, stage, dispatch))
        .transpose()
}

/// Completes one dispatch. Only the cursor's current dispatch may complete; a
/// late result for a superseded dispatch is refused, never finalized. An
/// exact terminal replay compares the full outcome and checkpoint:
/// identical replays, conflicting refuses. Unknown accepts a late correlated
/// receipt or recovered output without a new send and keeps its liability
/// until finance mirrors settlement. Completion is reconciliation, never new
/// work, so it proceeds past the job deadline and retirement. Terminal
/// outcomes retire the durable active reserve.
async fn complete_unknown_in(
    tx: &Transaction<'_>,
    command: &CompleteCommand,
    row: &tokio_postgres::Row,
) -> Result<DispatchView, ExecutionError> {
    let wanted = match &command.outcome {
        CompletionOutcome::Unknown => None,
        CompletionOutcome::UnknownWithCause { cause } => Some(cause.label()),
        _ => return Err(ExecutionError::InvalidRecord),
    };
    let state: String = row.get("dispatch_state");
    if state == "unknown" {
        let stored: Option<String> = row.get("failure_cause");
        if wanted.is_none() || stored.as_deref() == wanted {
            return read_dispatch_view(tx, &command.owner, &command.dispatch).await;
        }
        if stored.is_some() {
            return Err(ExecutionError::OperationConflict);
        }
    } else if !matches!(state.as_str(), "prepared" | "sent") {
        return Err(ExecutionError::OperationConflict);
    }
    tx.execute("UPDATE imagery.image_dispatch_attempt SET settlement=CASE WHEN dispatch_state='unknown' THEN settlement ELSE 'unknown_outstanding' END, \
        dispatch_state='unknown',failure_cause=$2,updated_at_ms=GREATEST(updated_at_ms,$3) WHERE dispatch_id=$1",
        &[&command.dispatch.as_bytes().as_slice(),&wanted,&command.now_ms]).await.map_err(map_db)?;
    bump_job_revision(tx, &command.job, command.now_ms).await?;
    read_dispatch_view(tx, &command.owner, &command.dispatch).await
}

pub(crate) async fn complete_dispatch_in(
    tx: &Transaction<'_>,
    command: &CompleteCommand,
) -> Result<DispatchView, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, &command.owner, &command.job).await?;
    if matches!(&command.outcome, CompletionOutcome::Succeeded { .. }) {
        let basis =
            super::decision::read_stage_basis_in(tx, &command.owner, &command.job, &command.stage)
                .await?;
        if matches!(
            basis.kind,
            df_imagery::execution::StageKind::Classification
                | df_imagery::execution::StageKind::Inspection
        ) || basis.kind.is_standalone()
        {
            return Err(ExecutionError::BasisConflict);
        }
    }
    let cursor = tx
        .query_opt(
            "SELECT current_dispatch FROM imagery.image_stage_cursor \
             WHERE job_id = $1 AND stage_id = $2 FOR UPDATE",
            &[
                &command.job.as_bytes().as_slice(),
                &command.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::Superseded)?;
    let current: Vec<u8> = cursor.get("current_dispatch");
    if !sender_may_complete(&bytes16(&current)?, command.dispatch.as_bytes()) {
        return Err(ExecutionError::Superseded);
    }
    let row = locked_dispatch(tx, &command.job, &command.stage, &command.dispatch).await?;
    let state_text: String = row.get("dispatch_state");
    let stored_token = bytes16(&row.get::<_, Vec<u8>>("claim_token"))?;
    let stored_generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
    // The original dispatch claim is a recovery capability, never a send permit.
    // A replacement worker may reconcile under its live stage claim instead.
    if state_text == "prepared"
        || stored_token != *command.claim.as_bytes()
        || stored_generation != command.claim_generation
    {
        locked_claim(
            tx,
            &command.owner,
            &command.job,
            &command.stage,
            &command.claim,
            command.claim_generation,
            command.now_ms,
        )
        .await?;
    }

    match state_text.as_str() {
        "declined" => match &command.outcome {
            CompletionOutcome::Declined {
                observation_version,
                usage_revision,
            } => {
                super::decline::complete_decline_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }
            CompletionOutcome::Unknown | CompletionOutcome::UnknownWithCause { .. } => {
                read_dispatch_view(tx, &command.owner, &command.dispatch).await
            }
            _ => Err(ExecutionError::OperationConflict),
        },
        "succeeded" => match &command.outcome {
            CompletionOutcome::Declined {
                observation_version,
                usage_revision,
            } => {
                super::decline::complete_decline_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }

            CompletionOutcome::Unusable {
                cause,
                usage_revision,
            } => super::failure::complete_unusable_in(tx, command, *cause, *usage_revision).await,
            CompletionOutcome::Candidate {
                observation_version,
                usage_revision,
            } => {
                super::candidate::complete_candidate_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }
            CompletionOutcome::Decision {
                observation_version,
                usage_revision,
            } => {
                super::decision::complete_decision_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }
            CompletionOutcome::Unknown | CompletionOutcome::UnknownWithCause { .. } => {
                read_dispatch_view(tx, &command.owner, &command.dispatch).await
            }
            CompletionOutcome::Succeeded {
                usage_revision,
                checkpoint,
            } => {
                let stored = stored_checkpoint(tx, &command.dispatch)
                    .await?
                    .ok_or(ExecutionError::InvalidRecord)?;
                if stored == *checkpoint && stored.usage_revision == *usage_revision {
                    read_dispatch_view(tx, &command.owner, &command.dispatch).await
                } else {
                    Err(ExecutionError::OperationConflict)
                }
            }
            CompletionOutcome::Failed { .. } => Err(ExecutionError::OperationConflict),
        },
        "failed" => match &command.outcome {
            CompletionOutcome::Declined {
                observation_version,
                usage_revision,
            } => {
                super::decline::complete_decline_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }

            CompletionOutcome::Unusable {
                cause,
                usage_revision,
            } => super::failure::complete_unusable_in(tx, command, *cause, *usage_revision).await,
            CompletionOutcome::Candidate { .. } => Err(ExecutionError::OperationConflict),
            CompletionOutcome::Decision { .. } => Err(ExecutionError::OperationConflict),
            CompletionOutcome::Unknown | CompletionOutcome::UnknownWithCause { .. } => {
                read_dispatch_view(tx, &command.owner, &command.dispatch).await
            }
            CompletionOutcome::Failed { cause } => {
                let stored: Option<String> = row.get("failure_cause");
                if stored.as_deref() == Some(cause.label()) {
                    read_dispatch_view(tx, &command.owner, &command.dispatch).await
                } else {
                    Err(ExecutionError::OperationConflict)
                }
            }
            CompletionOutcome::Succeeded { .. } => Err(ExecutionError::OperationConflict),
        },
        "released" => match &command.outcome {
            CompletionOutcome::Unknown | CompletionOutcome::UnknownWithCause { .. } => {
                read_dispatch_view(tx, &command.owner, &command.dispatch).await
            }
            _ => Err(ExecutionError::OperationConflict),
        },
        "unknown" => match &command.outcome {
            CompletionOutcome::Declined {
                observation_version,
                usage_revision,
            } => {
                super::decline::complete_decline_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }

            CompletionOutcome::Unusable {
                cause,
                usage_revision,
            } => super::failure::complete_unusable_in(tx, command, *cause, *usage_revision).await,
            CompletionOutcome::Candidate {
                observation_version,
                usage_revision,
            } => {
                super::candidate::complete_candidate_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }
            CompletionOutcome::Decision {
                observation_version,
                usage_revision,
            } => {
                super::decision::complete_decision_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }
            CompletionOutcome::Unknown | CompletionOutcome::UnknownWithCause { .. } => {
                complete_unknown_in(tx, command, &row).await
            }
            CompletionOutcome::Succeeded {
                usage_revision,
                checkpoint,
            } => {
                let stored_call: String = row.get("book_call_id");
                if checkpoint.book_call_id != stored_call
                    || checkpoint.usage_revision != *usage_revision
                {
                    return Err(ExecutionError::InvalidRecord);
                }
                let revision = invalid(i32::try_from(*usage_revision))?;
                tx.execute(
                    "UPDATE imagery.image_dispatch_attempt SET dispatch_state = 'succeeded', \
                     failure_cause = NULL, usage_revision = $2, updated_at_ms = $3 \
                     WHERE dispatch_id = $1",
                    &[
                        &command.dispatch.as_bytes().as_slice(),
                        &revision,
                        &command.now_ms,
                    ],
                )
                .await
                .map_err(map_db)?;
                write_winning_checkpoint(tx, &command.dispatch, checkpoint, command.now_ms).await?;
                maybe_release_reserve(
                    tx,
                    &command.job,
                    &command.stage,
                    DispatchState::Succeeded,
                    command.now_ms,
                )
                .await?;
                bump_job_revision(tx, &command.job, command.now_ms).await?;
                read_dispatch_view(tx, &command.owner, &command.dispatch).await
            }
            CompletionOutcome::Failed { cause } => {
                tx.execute(
                    "UPDATE imagery.image_dispatch_attempt SET dispatch_state = 'failed', \
                     failure_cause = $2, updated_at_ms = $3 WHERE dispatch_id = $1",
                    &[
                        &command.dispatch.as_bytes().as_slice(),
                        &cause.label(),
                        &command.now_ms,
                    ],
                )
                .await
                .map_err(map_db)?;
                maybe_release_reserve(
                    tx,
                    &command.job,
                    &command.stage,
                    DispatchState::Succeeded,
                    command.now_ms,
                )
                .await?;
                bump_job_revision(tx, &command.job, command.now_ms).await?;
                read_dispatch_view(tx, &command.owner, &command.dispatch).await
            }
        },
        "prepared" | "sent" => match &command.outcome {
            CompletionOutcome::Declined {
                observation_version,
                usage_revision,
            } => {
                super::decline::complete_decline_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }

            CompletionOutcome::Unusable {
                cause,
                usage_revision,
            } => super::failure::complete_unusable_in(tx, command, *cause, *usage_revision).await,
            CompletionOutcome::Candidate {
                observation_version,
                usage_revision,
            } => {
                super::candidate::complete_candidate_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }
            CompletionOutcome::Decision {
                observation_version,
                usage_revision,
            } => {
                super::decision::complete_decision_in(
                    tx,
                    command,
                    *observation_version,
                    *usage_revision,
                )
                .await
            }
            CompletionOutcome::Succeeded {
                usage_revision,
                checkpoint,
            } => {
                let stored_call: String = row.get("book_call_id");
                if checkpoint.book_call_id != stored_call {
                    return Err(ExecutionError::InvalidRecord);
                }
                let revision = invalid(i32::try_from(*usage_revision))?;
                if checkpoint.usage_revision != *usage_revision {
                    return Err(ExecutionError::InvalidRecord);
                }
                tx.execute(
                    "UPDATE imagery.image_dispatch_attempt SET dispatch_state = 'succeeded', \
                     failure_cause = NULL, usage_revision = $2, updated_at_ms = $3 \
                     WHERE dispatch_id = $1",
                    &[
                        &command.dispatch.as_bytes().as_slice(),
                        &revision,
                        &command.now_ms,
                    ],
                )
                .await
                .map_err(map_db)?;
                write_winning_checkpoint(tx, &command.dispatch, checkpoint, command.now_ms).await?;
                maybe_release_reserve(
                    tx,
                    &command.job,
                    &command.stage,
                    DispatchState::Succeeded,
                    command.now_ms,
                )
                .await?;
                bump_job_revision(tx, &command.job, command.now_ms).await?;
                read_dispatch_view(tx, &command.owner, &command.dispatch).await
            }
            CompletionOutcome::Failed { cause } => {
                tx.execute(
                    "UPDATE imagery.image_dispatch_attempt SET dispatch_state = 'failed', \
                     failure_cause = $2, updated_at_ms = $3 WHERE dispatch_id = $1",
                    &[
                        &command.dispatch.as_bytes().as_slice(),
                        &cause.label(),
                        &command.now_ms,
                    ],
                )
                .await
                .map_err(map_db)?;
                maybe_release_reserve(
                    tx,
                    &command.job,
                    &command.stage,
                    DispatchState::Failed,
                    command.now_ms,
                )
                .await?;
                bump_job_revision(tx, &command.job, command.now_ms).await?;
                read_dispatch_view(tx, &command.owner, &command.dispatch).await
            }
            CompletionOutcome::Unknown | CompletionOutcome::UnknownWithCause { .. } => {
                complete_unknown_in(tx, command, &row).await
            }
        },
        _ => Err(ExecutionError::InvalidRecord),
    }
}

/// Mirrors a proven finance settlement onto the dispatch row. The finance
/// owner decides; this row only mirrors after that call commits. Local
/// record-only work mirrors `settled` on the same path, never touching money.
pub(crate) async fn mirror_settlement_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &df_imagery::execution::ExecutionStageId,
    dispatch: &df_imagery::execution::DispatchId,
    settlement: DispatchSettlement,
    now_ms: i64,
) -> Result<DispatchView, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    let row = locked_dispatch(tx, job, stage, dispatch).await?;
    let stored: String = row.get("settlement");
    if stored == settlement.label() {
        return read_dispatch_view(tx, owner, dispatch).await;
    }
    let may_advance = matches!(stored.as_str(), "held" | "unknown_outstanding")
        && settlement != DispatchSettlement::Held
        && !(stored == "unknown_outstanding"
            && settlement == DispatchSettlement::UnknownOutstanding);
    if !may_advance {
        return Err(ExecutionError::OperationConflict);
    }
    if settlement == DispatchSettlement::UnknownOutstanding
        && row.get::<_, String>("dispatch_state") == "succeeded"
    {
        let usage_revision = invalid(u64::try_from(row.get::<_, i32>("usage_revision")))?;
        let scope = df_imagery::execution::ExecutionScope {
            owner: owner.clone(),
            job: *job,
            stage: *stage,
            dispatch: *dispatch,
        };
        let unknown = crate::imagery_runtime::usage_unknown_in(
            tx,
            &scope,
            df_imagery::cost::UsageRevision(usage_revision),
        )
        .await
        .map_err(|_| {
            df_observe::record_audit_line(
                "IMAGERY",
                "IMAGERY settlement_usage_read_failed".to_owned(),
            );
            ExecutionError::Unavailable
        })?;
        if !unknown {
            return Err(ExecutionError::ObservationMissing);
        }
    }
    tx.execute(
        "UPDATE imagery.image_dispatch_attempt SET settlement = $2, updated_at_ms = $3 \
         WHERE dispatch_id = $1",
        &[
            &dispatch.as_bytes().as_slice(),
            &settlement.label(),
            &now_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    bump_job_revision(tx, job, now_ms).await?;
    read_dispatch_view(tx, owner, dispatch).await
}

/// Cancels one prepared dispatch before any byte leaves. A sent, terminal or
/// unknown dispatch refuses: cancellation needs proof of unsent, and the
/// finance owner releases its hold first. The reserve retires with the row.
pub(crate) async fn cancel_dispatch_in(
    tx: &Transaction<'_>,
    command: &CancelCommand,
) -> Result<DispatchView, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, &command.owner, &command.job).await?;
    let row = locked_dispatch(tx, &command.job, &command.stage, &command.dispatch).await?;
    let state_text: String = row.get("dispatch_state");
    match state_text.as_str() {
        "prepared" => {}
        "released" => {
            let token = bytes16(&row.get::<_, Vec<u8>>("claim_token"))?;
            let generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
            if token != *command.claim.as_bytes() || generation != command.claim_generation {
                return Err(ExecutionError::LostClaim);
            }
            return read_dispatch_view(tx, &command.owner, &command.dispatch).await;
        }
        _ => return Err(ExecutionError::Superseded),
    }
    locked_claim(
        tx,
        &command.owner,
        &command.job,
        &command.stage,
        &command.claim,
        command.claim_generation,
        command.now_ms,
    )
    .await?;
    tx.execute(
        "UPDATE imagery.image_dispatch_attempt SET dispatch_state = 'released', \
         failure_cause = NULL, settlement = 'released', updated_at_ms = $2 \
         WHERE dispatch_id = $1",
        &[&command.dispatch.as_bytes().as_slice(), &command.now_ms],
    )
    .await
    .map_err(map_db)?;
    maybe_release_reserve(
        tx,
        &command.job,
        &command.stage,
        DispatchState::Released,
        command.now_ms,
    )
    .await?;
    bump_job_revision(tx, &command.job, command.now_ms).await?;
    read_dispatch_view(tx, &command.owner, &command.dispatch).await
}

/// Reads retained output metadata for either accounting mode under the owned job lock.
pub(crate) async fn read_output_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &df_imagery::execution::ExecutionStageId,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<Option<df_imagery::execution::OutputCheckpoint>, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    locked_dispatch(tx, job, stage, dispatch).await?;
    stored_checkpoint(tx, dispatch).await
}
