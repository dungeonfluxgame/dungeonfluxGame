//! Metadata encoding for protected submit-rejection receipts; source bytes remain only in raw44.
use super::common::{bytes32, invalid, map_db, parse_dispatch};
use df_imagery::execution::*;
use df_imagery::owner_scope_bytes;
use serde_json::{Value, json};
use tokio_postgres::{Row, Transaction};
fn metadata(receipt: &DeclineReceipt) -> String {
    json!({"schema_revision":2,"contract":receipt.contract.label(),"status":receipt.status,"body_sha256":receipt.body_sha256,"body_bytes":receipt.body_bytes,"transport_request_id_sha256":receipt.transport_request_id_sha256,"retry_after_values":receipt.retry_after_values,"backoff_ms":receipt.retry.backoff_ms}).to_string()
}
fn digest(value: &Value) -> Result<[u8; 32], ExecutionError> {
    let bytes: Vec<u8> =
        serde_json::from_value(value.clone()).map_err(|_| ExecutionError::InvalidRecord)?;
    bytes32(&bytes)
}
fn parse(row: &Row, scope: ExecutionScope) -> Result<DeclineReceipt, ExecutionError> {
    let value: Value = serde_json::from_str(&row.get::<_, String>("metadata"))
        .map_err(|_| ExecutionError::InvalidRecord)?;
    if value["schema_revision"].as_u64() != Some(2) {
        return Err(ExecutionError::InvalidRecord);
    }
    let d = parse_dispatch(row, &scope.owner)?;
    Ok(DeclineReceipt {
        scope,
        basis: super::decision::parse_basis(row)?,
        finance: d.finance,
        observation_version: invalid(u32::try_from(row.get::<_, i64>("observation_version")))?,
        receipt: ObservationReceipt {
            usage_revision: invalid(u32::try_from(row.get::<_, i32>("selected_usage_revision")))?,
            observed_at_ms: row.get("observed_at_ms"),
        },
        contract: SubmitDeclineContract::parse_label(
            value["contract"]
                .as_str()
                .ok_or(ExecutionError::InvalidRecord)?,
        )
        .ok_or(ExecutionError::InvalidRecord)?,
        status: invalid(u16::try_from(
            value["status"]
                .as_u64()
                .ok_or(ExecutionError::InvalidRecord)?,
        ))?,
        body_sha256: digest(&value["body_sha256"])?,
        body_bytes: value["body_bytes"]
            .as_u64()
            .ok_or(ExecutionError::InvalidRecord)?,
        raw_sha256: bytes32(&row.get::<_, Vec<u8>>("raw_sha256"))?,
        raw_bytes: invalid(u64::try_from(row.get::<_, i64>("raw_bytes")))?,
        route: DeclineRoute {
            provider: row.get("provider"),
            model: row.get("model"),
            cause: row.get("cause"),
            detail: row.get("detail"),
        },
        retry_not_before_ms: row.get("retry_not_before_ms"),
        transport_request_id_sha256: if value["transport_request_id_sha256"].is_null() {
            None
        } else {
            Some(digest(&value["transport_request_id_sha256"])?)
        },
        retry_after_values: serde_json::from_value(value["retry_after_values"].clone())
            .map_err(|_| ExecutionError::InvalidRecord)?,
        retry: DeclineRetryPolicy {
            max_attempts: invalid(u32::try_from(row.get::<_, i64>("max_attempts")))?,
            retry_deadline_ms: row.get("retry_deadline_ms"),
            backoff_ms: value["backoff_ms"]
                .as_i64()
                .ok_or(ExecutionError::InvalidRecord)?,
            config_snapshot_revision: invalid(u32::try_from(
                row.get::<_, i64>("config_snapshot_revision"),
            ))?,
        },
    })
}
async fn read(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
    version: Option<u32>,
    selected: bool,
) -> Result<Option<DeclineReceipt>, ExecutionError> {
    let version = version.map(i64::from);
    let selected_guard = if selected {
        "AND d.dispatch_state='declined' AND d.supplier_job_id IS NULL AND NOT EXISTS(SELECT 1 FROM imagery.image_supplier_checkpoint accepted WHERE accepted.dispatch_id=d.dispatch_id) AND d.usage_revision=o.usage_revision AND EXISTS(SELECT 1 FROM imagery.image_decline_checkpoint p WHERE p.dispatch_id=o.dispatch_id AND p.observation_version=o.observation_version AND p.usage_revision=o.usage_revision) AND EXISTS(SELECT 1 FROM imagery.image_dispatch_usage u WHERE u.dispatch_id=o.dispatch_id AND u.owner_scope=o.owner_scope AND u.job_id=o.job_id AND u.stage_id=o.stage_id AND u.revision=o.usage_revision AND u.evidence='unknown' AND NOT u.usable AND u.units=u.maxima AND u.observed_at_ms=o.observed_at_ms AND u.raw_output_sha256=o.raw_sha256 AND octet_length(u.raw_output)::bigint=o.raw_bytes AND u.provider=o.provider AND u.model=o.model AND u.cause=o.cause AND u.detail=o.detail)"
    } else {
        ""
    };
    let sql = format!(
        "SELECT d.*,s.stage_kind,s.input_revision,s.input_digest,s.config_revision,s.config_digest,s.parent_dispatch,o.*,o.usage_revision AS selected_usage_revision FROM imagery.image_decline_observation o JOIN imagery.image_execution_stage s ON s.job_id=o.job_id AND s.stage_id=o.stage_id JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=o.dispatch_id WHERE o.owner_scope=$1 AND o.job_id=$2 AND o.stage_id=$3 AND o.dispatch_id=$4 AND ($5::bigint IS NULL OR o.observation_version=$5) {selected_guard} ORDER BY o.observation_version DESC LIMIT 1"
    );
    tx.query_opt(
        &sql,
        &[
            &owner_scope_bytes(&scope.owner),
            &scope.job.as_bytes().as_slice(),
            &scope.stage.as_bytes().as_slice(),
            &scope.dispatch.as_bytes().as_slice(),
            &version,
        ],
    )
    .await
    .map_err(map_db)?
    .map(|row| parse(&row, scope.clone()))
    .transpose()
}

async fn has_raw(tx: &Transaction<'_>, r: &DeclineReceipt) -> Result<bool, ExecutionError> {
    let s = &r.scope;
    Ok(tx.query_opt("SELECT 1 FROM imagery.image_dispatch_usage u WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND dispatch_id=$4 AND revision=$5 AND evidence='unknown' AND NOT usable AND units=maxima AND octet_length(raw_output)::bigint=$6 AND raw_output_sha256=$7 AND provider=$8 AND model=$9 AND cause=$10 AND detail=$11 AND observed_at_ms=$12",&[&owner_scope_bytes(&s.owner),&s.job.as_bytes().as_slice(),&s.stage.as_bytes().as_slice(),&s.dispatch.as_bytes().as_slice(),&i64::from(r.receipt.usage_revision),&invalid(i64::try_from(r.raw_bytes))?,&r.raw_sha256.as_slice(),&r.route.provider,&r.route.model,&r.route.cause,&r.route.detail,&r.receipt.observed_at_ms]).await.map_err(map_db)?.is_some())
}

/// Exact selected historical receipt; this read never acquires locks after the finance owner.
pub(crate) async fn read_decline_reference_in(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
) -> Result<Option<DeclineReceipt>, ExecutionError> {
    super::common::require_read_committed(tx).await?;
    read(tx, scope, None, true).await
}
pub(crate) async fn record_decline_observation_in(
    tx: &Transaction<'_>,
    observation: &DeclineObservation,
) -> Result<DeclineReceipt, ExecutionError> {
    use super::common::{locked_dispatch, locked_job, read_dispatch_view, require_read_committed};
    require_read_committed(tx).await?;
    let r = observation.as_receipt();
    let s = &r.scope;
    locked_job(tx, &s.owner, &s.job).await?;
    if let Some(stored) = read(tx, s, Some(r.observation_version), false).await? {
        return if stored == *r {
            Ok(stored)
        } else {
            Err(ExecutionError::OperationConflict)
        };
    }
    let d = locked_dispatch(tx, &s.job, &s.stage, &s.dispatch).await?;
    let view = read_dispatch_view(tx, &s.owner, &s.dispatch).await?;
    if !matches!(view.state, DispatchState::Sent | DispatchState::Unknown)
        || view.finance != r.finance
        || d.get::<_, Option<String>>("supplier_job_id").is_some()
    {
        return Err(ExecutionError::Superseded);
    }
    if tx
        .query_opt(
            "SELECT 1 FROM imagery.image_supplier_checkpoint WHERE dispatch_id=$1 LIMIT 1",
            &[&s.dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .is_some()
    {
        return Err(ExecutionError::Superseded);
    }
    if super::decision::read_stage_basis_in(tx, &s.owner, &s.job, &s.stage).await? != r.basis {
        return Err(ExecutionError::BasisConflict);
    }
    let binding = tx
        .query_one(
            "SELECT config_revision,config_digest FROM imagery.image_job WHERE job_id=$1",
            &[&s.job.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?;
    if binding.get::<_, i64>("config_revision") != i64::from(r.retry.config_snapshot_revision)
        || binding.get::<_, Vec<u8>>("config_digest") != r.basis.config_digest
    {
        return Err(ExecutionError::BasisConflict);
    }
    let raw = has_raw(tx, r).await?;
    if !raw {
        return Err(ExecutionError::ObservationMissing);
    }
    tx.execute("INSERT INTO imagery.image_decline_observation(dispatch_id,observation_version,owner_scope,job_id,stage_id,usage_revision,raw_sha256,raw_bytes,provider,model,cause,detail,metadata,retry_not_before_ms,max_attempts,retry_deadline_ms,config_snapshot_revision,observed_at_ms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)",&[&s.dispatch.as_bytes().as_slice(),&i64::from(r.observation_version),&owner_scope_bytes(&s.owner),&s.job.as_bytes().as_slice(),&s.stage.as_bytes().as_slice(),&invalid(i32::try_from(r.receipt.usage_revision))?,&r.raw_sha256.as_slice(),&invalid(i64::try_from(r.raw_bytes))?,&r.route.provider,&r.route.model,&r.route.cause,&r.route.detail,&metadata(r),&r.retry_not_before_ms,&i64::from(r.retry.max_attempts),&r.retry.retry_deadline_ms,&i64::from(r.retry.config_snapshot_revision),&r.receipt.observed_at_ms]).await.map_err(map_db)?;
    super::common::bump_job_revision(tx, &s.job, r.receipt.observed_at_ms).await?;
    Ok(r.clone())
}
pub(super) async fn complete_decline_in(
    tx: &Transaction<'_>,
    command: &CompleteCommand,
    version: u32,
    usage_revision: u32,
) -> Result<DispatchView, ExecutionError> {
    use super::common::{bump_job_revision, locked_dispatch, read_dispatch_view};
    let scope = ExecutionScope {
        owner: command.owner.clone(),
        job: command.job,
        stage: command.stage,
        dispatch: command.dispatch,
    };
    let r = read(tx, &scope, Some(version), false)
        .await?
        .ok_or(ExecutionError::ObservationMissing)?;
    if !has_raw(tx, &r).await? {
        return Err(ExecutionError::ObservationMissing);
    }
    if command.now_ms < r.receipt.observed_at_ms {
        return Err(ExecutionError::InvalidRecord);
    }
    if r.receipt.usage_revision != usage_revision {
        return Err(ExecutionError::OperationConflict);
    }
    let row = locked_dispatch(tx, &scope.job, &scope.stage, &scope.dispatch).await?;
    let state: String = row.get("dispatch_state");
    if state == "declined" {
        let stored = read_decline_reference_in(tx, &scope)
            .await?
            .ok_or(ExecutionError::ObservationMissing)?;
        return if stored == r {
            read_dispatch_view(tx, &scope.owner, &scope.dispatch).await
        } else {
            Err(ExecutionError::OperationConflict)
        };
    }
    if !matches!(state.as_str(), "sent" | "unknown")
        || !matches!(
            row.get::<_, String>("settlement").as_str(),
            "held" | "unknown_outstanding"
        )
    {
        return Err(ExecutionError::OperationConflict);
    }
    let retry_reserve = retry_reservation(tx, command, &r).await?;
    tx.execute("INSERT INTO imagery.image_decline_checkpoint(dispatch_id,observation_version,usage_revision) VALUES($1,$2,$3)",&[&scope.dispatch.as_bytes().as_slice(),&i64::from(version),&invalid(i32::try_from(usage_revision))?]).await.map_err(map_db)?;
    tx.execute("UPDATE imagery.image_dispatch_attempt SET dispatch_state='declined',settlement='unknown_outstanding',failure_cause='rate_limited',usage_revision=$2,updated_at_ms=GREATEST(updated_at_ms,$3) WHERE dispatch_id=$1",&[&scope.dispatch.as_bytes().as_slice(),&invalid(i32::try_from(usage_revision))?,&command.now_ms]).await.map_err(map_db)?;
    super::dispatch::maybe_release_reserve(
        tx,
        &scope.job,
        &scope.stage,
        DispatchState::Failed,
        command.now_ms,
    )
    .await?;
    if let Some((capacity, job_bytes)) = retry_reserve {
        match super::common::reserve_pending_in(tx, &capacity, job_bytes, command.now_ms).await {
            Ok(()) => {
                tx.execute("UPDATE imagery.image_execution_stage SET stage_state='planned',retry_dispatch_id=$3,queued_not_before_ms=$4,queued_notice=NULL,claim_token=stage_id,claim_generation=claim_generation+1,lease_until_ms=0,claim_config_revision=0,claim_lease_ms=0,updated_at_ms=GREATEST(updated_at_ms,$5) WHERE job_id=$1 AND stage_id=$2",&[&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),&scope.dispatch.as_bytes().as_slice(),&r.retry_not_before_ms,&command.now_ms]).await.map_err(map_db)?;
            }
            Err(ExecutionError::CapacityFull) => {}
            Err(error) => return Err(error),
        }
    }
    bump_job_revision(tx, &scope.job, command.now_ms).await?;
    read_dispatch_view(tx, &scope.owner, &scope.dispatch).await
}
/// Both claim and prepare recheck count/deadlines against actual dispatch history.
pub(super) async fn require_retry_in(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
    now_ms: i64,
    require_due: bool,
) -> Result<DeclineReceipt, ExecutionError> {
    if tx.query_opt("SELECT 1 FROM imagery.image_stage_cursor WHERE job_id=$1 AND stage_id=$2 AND current_dispatch=$3",&[&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),&scope.dispatch.as_bytes().as_slice()]).await.map_err(map_db)?.is_none(){return Err(ExecutionError::Superseded)}
    let r = read_decline_reference_in(tx, scope)
        .await?
        .ok_or(ExecutionError::ObservationMissing)?;
    check_history(tx, scope, &r, now_ms, require_due).await?;
    Ok(r)
}
async fn check_history(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
    r: &DeclineReceipt,
    now_ms: i64,
    require_due: bool,
) -> Result<(), ExecutionError> {
    let row=tx.query_one("SELECT count(*) AS attempts,min(created_at_ms) AS first_at FROM imagery.image_dispatch_attempt WHERE job_id=$1 AND stage_id=$2",&[&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice()]).await.map_err(map_db)?;
    let deadline = row
        .get::<_, Option<i64>>("first_at")
        .ok_or(ExecutionError::InvalidRecord)?
        .checked_add(r.retry.retry_deadline_ms)
        .ok_or(ExecutionError::InvalidRecord)?;
    if row.get::<_, i64>("attempts") >= i64::from(r.retry.max_attempts)
        || now_ms >= deadline
        || r.retry_not_before_ms >= deadline
    {
        return Err(ExecutionError::Expired);
    }
    if require_due && now_ms < r.retry_not_before_ms {
        return Err(ExecutionError::Deferred {
            not_before_ms: r.retry_not_before_ms,
        });
    }
    Ok(())
}
/// Gate locks precede capacity locks, including when an operator disables retries during completion.
async fn retry_reservation(
    tx: &Transaction<'_>,
    command: &CompleteCommand,
    r: &DeclineReceipt,
) -> Result<Option<(String, i64)>, ExecutionError> {
    use super::common::{attached_interest_count, locked_job, require_gate_revision};
    let scope = &r.scope;
    let (_, _, deadline, state, _, _, job_bytes) = locked_job(tx, &scope.owner, &scope.job).await?;
    if state == "retired"
        || command.now_ms >= deadline
        || r.retry_not_before_ms >= deadline
        || attached_interest_count(tx, &scope.job).await? == 0
    {
        return Ok(None);
    }
    match super::terminal::check_output_open_in(tx, &scope.owner, &scope.job).await {
        Ok(()) => {}
        Err(ExecutionError::OutputSealed) => return Ok(None),
        Err(e) => return Err(e),
    }
    match check_history(tx, scope, r, command.now_ms, false).await {
        Ok(()) => {}
        Err(ExecutionError::Expired) => return Ok(None),
        Err(e) => return Err(e),
    }
    let row=tx.query_one("SELECT capacity_scope,claim_config_revision FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_id=$2",&[&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice()]).await.map_err(map_db)?;
    let capacity: String = row.get("capacity_scope");
    match require_gate_revision(
        tx,
        &capacity,
        invalid(u64::try_from(row.get::<_, i64>("claim_config_revision")))?,
    )
    .await
    {
        Ok(()) => Ok(Some((capacity, job_bytes))),
        Err(ExecutionError::Disabled | ExecutionError::ConfigFenced { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) async fn requeue_declined_in(
    tx: &Transaction<'_>,
    command: &DeclineRetryCommand,
) -> Result<StageView, ExecutionError> {
    use super::common::*;
    require_read_committed(tx).await?;
    let c = &command.claim;
    let s = &c.scope;
    locked_job(tx, &s.owner, &s.job).await?;
    let d = locked_dispatch(tx, &s.job, &s.stage, &command.dispatch).await?;
    if d.get::<_, Vec<u8>>("claim_token") != c.token.as_bytes()
        || d.get::<_, i64>("claim_generation") != invalid(i64::try_from(c.generation))?
    {
        return Err(ExecutionError::LostClaim);
    }
    let row=tx.query_one("SELECT s.*,c.current_dispatch FROM imagery.image_execution_stage s JOIN imagery.image_stage_cursor c USING(job_id,stage_id) WHERE s.job_id=$1 AND s.stage_id=$2 FOR UPDATE OF s,c",&[&s.job.as_bytes().as_slice(),&s.stage.as_bytes().as_slice()]).await.map_err(map_db)?;
    if row.get::<_, Vec<u8>>("current_dispatch") != command.dispatch.as_bytes() {
        return Err(ExecutionError::Superseded);
    }
    let retry: Option<Vec<u8>> = row.get("retry_dispatch_id");
    if retry.as_deref() == Some(command.dispatch.as_bytes()) {
        return super::reads::read_stage_in(tx, &s.owner, &s.job, &s.stage).await;
    }
    Err(ExecutionError::Expired)
}
