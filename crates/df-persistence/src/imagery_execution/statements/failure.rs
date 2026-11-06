//! Definitive unusable outcomes pin actual durable usage; unknown evidence never becomes failure.
use super::common::{bump_job_revision, invalid, locked_dispatch, map_db, read_dispatch_view};
use df_imagery::execution::{
    CompleteCommand, DispatchState, DispatchView, ExecutionError, ExecutionScope, FailureCause,
};
use tokio_postgres::Transaction;

pub(super) async fn complete_unusable_in(
    tx: &Transaction<'_>,
    command: &CompleteCommand,
    cause: FailureCause,
    usage_revision: u32,
) -> Result<DispatchView, ExecutionError> {
    if usage_revision == 0 {
        return Err(ExecutionError::ObservationMissing);
    }
    let row = locked_dispatch(tx, &command.job, &command.stage, &command.dispatch).await?;
    let state: String = row.get("dispatch_state");
    let stored_usage: i32 = row.get("usage_revision");
    if state == "failed" {
        let stored_cause: Option<String> = row.get("failure_cause");
        if stored_cause.as_deref() != Some(cause.key()) {
            return Err(ExecutionError::OperationConflict);
        }
        if stored_usage == invalid(i32::try_from(usage_revision))? {
            return read_dispatch_view(tx, &command.owner, &command.dispatch).await;
        }
        if stored_usage != 0 {
            return Err(ExecutionError::OperationConflict);
        }
    } else if !matches!(state.as_str(), "sent" | "unknown") {
        return Err(ExecutionError::OperationConflict);
    }
    let scope = ExecutionScope {
        owner: command.owner.clone(),
        job: command.job,
        stage: command.stage,
        dispatch: command.dispatch,
    };
    let known = crate::imagery_runtime::usage_definitive_failure_in(
        tx,
        &scope,
        df_imagery::cost::UsageRevision(u64::from(usage_revision)),
    )
    .await
    .map_err(|_| {
        df_observe::record_audit_line("IMAGERY", "IMAGERY failure_usage_read_failed".to_owned());
        ExecutionError::Unavailable
    })?;
    if !known {
        return Err(ExecutionError::ObservationMissing);
    }
    tx.execute(
        "UPDATE imagery.image_dispatch_attempt SET dispatch_state='failed',failure_cause=$2, \
        usage_revision=$3,updated_at_ms=$4 WHERE dispatch_id=$1",
        &[
            &command.dispatch.as_bytes().as_slice(),
            &cause.key(),
            &invalid(i32::try_from(usage_revision))?,
            &command.now_ms,
        ],
    )
    .await
    .map_err(map_db)?;
    super::dispatch::maybe_release_reserve(
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
