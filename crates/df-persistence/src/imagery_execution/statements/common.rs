//! Shared statement helpers for the durable image execution journal.
//!
//! Everything here takes the caller's transaction and performs one atomic
//! call at READ COMMITTED; row locks carry the guarantees. No helper
//! sleeps, retries, spends money, or touches finance, catalog or provider
//! state on its own.
//!
//! Lock the job first to serialize stage, dispatch, cursor and checkpoint changes.
//! Gate checks retain a shared gate lock until commit; operator updates take the
//! gate before capacity bounds. Finance, fence and quota/catalog locks precede
//! imagery locks. Failed commits are Unknown and never authorize a resend.

use df_imagery::execution::{
    CheckpointAlpha, DispatchView, ExecutionError, ExecutionStageId, FinanceTuple,
    OutputCheckpoint, StageKind, StageState, SupplierCheckpoint,
};
use df_imagery::{ImageJobId, ImageOwner, owner_of_bytes, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};

pub(crate) fn record_execution_failure(error: &tokio_postgres::Error) {
    let sqlstate = error.code().map_or("none", |code| code.code());
    df_observe::record_audit_line(
        "IMAGERY",
        format!("IMAGERY execution_failed sqlstate={sqlstate}"),
    );
}

/// The one place a pre-commit database error becomes an execution error: the
/// SQLSTATE is recorded so a constraint violation is not mistaken for an
/// outage. A unique violation means a supplied id is already bound elsewhere;
/// every other failure changed nothing and stays `Unavailable`. Statement
/// text and values never reach diagnostics.
pub(crate) fn map_db(error: tokio_postgres::Error) -> ExecutionError {
    record_execution_failure(&error);
    match error.as_db_error().map(|db| db.code().code()) {
        Some("23505") => ExecutionError::IdentityConflict,
        _ => ExecutionError::Unavailable,
    }
}

/// Every execution statement runs at READ COMMITTED: row locks carry the
/// guarantees and no caller may inherit a stricter default. A transaction at
/// another level is refused before it reads or writes anything.
pub(crate) async fn require_read_committed(tx: &Transaction<'_>) -> Result<(), ExecutionError> {
    let setting: String = tx
        .query_one("SELECT current_setting('transaction_isolation', true)", &[])
        .await
        .map_err(map_db)?
        .get(0);
    if setting == "read committed" {
        return Ok(());
    }
    df_observe::record_audit_line(
        "IMAGERY",
        format!("IMAGERY execution_wrong_isolation setting={setting}"),
    );
    Err(ExecutionError::Unavailable)
}

pub(crate) fn invalid<T, E>(result: Result<T, E>) -> Result<T, ExecutionError> {
    result.map_err(|_| {
        df_observe::record_audit_line("IMAGERY", "IMAGERY execution_record_invalid".to_owned());
        ExecutionError::InvalidRecord
    })
}

pub(crate) fn owner_matches(stored_scope: &[u8], owner: &ImageOwner) -> bool {
    stored_scope == owner_scope_bytes(owner).as_slice()
}

pub(crate) fn check_owner(scope: &[u8], owner: &ImageOwner) -> Result<(), ExecutionError> {
    if owner_matches(scope, owner) {
        Ok(())
    } else {
        Err(ExecutionError::NotFound)
    }
}

pub(crate) fn parse_owner(scope: &[u8]) -> Result<ImageOwner, ExecutionError> {
    invalid(owner_of_bytes(scope).ok_or(()))
}

pub(crate) fn stage_kind_of(label: &str) -> Result<StageKind, ExecutionError> {
    invalid(StageKind::from_key(label).ok_or(()))
}

pub(crate) fn stage_state_of(label: &str) -> Result<StageState, ExecutionError> {
    invalid(StageState::from_key(label).ok_or(()))
}

pub(crate) fn dispatch_state_of(
    label: &str,
) -> Result<df_imagery::execution::DispatchState, ExecutionError> {
    invalid(df_imagery::execution::DispatchState::from_key(label).ok_or(()))
}

pub(crate) fn settlement_of(
    label: &str,
) -> Result<df_imagery::execution::DispatchSettlement, ExecutionError> {
    invalid(df_imagery::execution::DispatchSettlement::from_key(label).ok_or(()))
}

pub(crate) fn alpha_of(label: &str) -> Result<CheckpointAlpha, ExecutionError> {
    invalid(CheckpointAlpha::from_key(label).ok_or(()))
}

pub(crate) fn cause_of(
    value: Option<String>,
) -> Result<Option<df_imagery::execution::FailureCause>, ExecutionError> {
    value
        .map(|text| invalid(df_imagery::execution::FailureCause::from_key(&text).ok_or(())))
        .transpose()
}

pub(crate) fn bytes16(bytes: &[u8]) -> Result<[u8; 16], ExecutionError> {
    invalid(bytes.try_into())
}

pub(crate) fn bytes32(bytes: &[u8]) -> Result<[u8; 32], ExecutionError> {
    invalid(bytes.try_into())
}

pub(crate) fn parse_stage(
    row: &Row,
    owner: &ImageOwner,
    job_revision: u64,
    planned: Option<df_imagery::execution::PublicationIdentity>,
) -> Result<df_imagery::execution::StageView, ExecutionError> {
    let job_bytes: Vec<u8> = row.get("job_id");
    let stage_bytes: Vec<u8> = row.get("stage_id");
    let kind_text: String = row.get("stage_kind");
    let state_text: String = row.get("stage_state");
    let generation: i64 = row.get("claim_generation");
    let lease_until_ms: i64 = row.get("lease_until_ms");
    if stage_kind_of(&kind_text)?.is_standalone() != planned.is_none() {
        return Err(ExecutionError::PlannedIdentityConflict);
    }
    Ok(df_imagery::execution::StageView {
        owner: owner.clone(),
        job: invalid(ImageJobId::from_bytes(&job_bytes))?,
        stage: invalid(ExecutionStageId::from_bytes(&stage_bytes))?,
        kind: stage_kind_of(&kind_text)?,
        state: stage_state_of(&state_text)?,
        claim_generation: invalid(u64::try_from(generation))?,
        lease_until_ms,
        local_accounting: row.get("local_accounting"),
        planned_publication: planned,
        job_revision,
    })
}

pub(crate) fn parse_dispatch(
    row: &Row,
    owner: &ImageOwner,
) -> Result<DispatchView, ExecutionError> {
    let dispatch_bytes: Vec<u8> = row.get("dispatch_id");
    let job_bytes: Vec<u8> = row.get("job_id");
    let stage_bytes: Vec<u8> = row.get("stage_id");
    let state_text: String = row.get("dispatch_state");
    let cause_text: Option<String> = row.get("failure_cause");
    let settlement_text: String = row.get("settlement");
    let finance_attempt: Option<Vec<u8>> = row.get("finance_attempt");
    let finance_operation: Option<Vec<u8>> = row.get("finance_operation");
    let finance_effect: Option<Vec<u8>> = row.get("finance_effect");
    let finance = match (finance_attempt, finance_operation, finance_effect) {
        (Some(attempt), Some(operation), Some(effect)) => {
            let tuple =
                FinanceTuple::new(bytes16(&attempt)?, bytes16(&operation)?, bytes16(&effect)?)
                    .map_err(|_| {
                        df_observe::record_audit_line(
                            "IMAGERY",
                            "IMAGERY execution_record_invalid".to_owned(),
                        );
                        ExecutionError::InvalidRecord
                    })?;
            Some(tuple)
        }
        (None, None, None) => None,
        _ => return Err(ExecutionError::InvalidRecord),
    };
    let usage: i32 = row.get("usage_revision");
    let generation: i64 = row.get("claim_generation");
    let sequence: i32 = row.get("dispatch_seq");
    let parent: Option<Vec<u8>> = row.get("lineage_parent");
    Ok(DispatchView {
        owner: owner.clone(),
        job: invalid(ImageJobId::from_bytes(&job_bytes))?,
        stage: invalid(ExecutionStageId::from_bytes(&stage_bytes))?,
        dispatch: invalid(df_imagery::execution::DispatchId::from_bytes(
            &dispatch_bytes,
        ))?,
        sequence: invalid(u32::try_from(sequence))?,
        state: dispatch_state_of(&state_text)?,
        cause: cause_of(cause_text)?,
        settlement: settlement_of(&settlement_text)?,
        lineage_parent: parent
            .map(|bytes| invalid(df_imagery::execution::DispatchId::from_bytes(&bytes)))
            .transpose()?,
        finance,
        local_accounting: row.get("local_accounting"),
        supplier_job_id: row.get("supplier_job_id"),
        usage_revision: invalid(u32::try_from(usage))?,
        book_call_id: row.get("book_call_id"),
        claim_generation: invalid(u64::try_from(generation))?,
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

pub(crate) fn parse_checkpoint(
    row: &Row,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<OutputCheckpoint, ExecutionError> {
    let storage_key: String = row.get("storage_key");
    let object_version: String = row.get("object_version");
    let byte_len: i64 = row.get("byte_len");
    let sha256: Vec<u8> = row.get("sha256");
    let recipe_digest: Vec<u8> = row.get("recipe_digest");
    let mime: String = row.get("mime");
    let width_px: Option<i32> = row.get("width_px");
    let height_px: Option<i32> = row.get("height_px");
    let alpha_text: String = row.get("alpha");
    let usage_revision: i32 = row.get("usage_revision");
    let book_call_id: String = row.get("book_call_id");
    if book_call_id != df_imagery::execution::book_call_id_for(dispatch) {
        return Err(ExecutionError::InvalidRecord);
    }
    let width_px = width_px
        .map(u32::try_from)
        .transpose()
        .map_err(|_| ExecutionError::InvalidRecord)?;
    let height_px = height_px
        .map(u32::try_from)
        .transpose()
        .map_err(|_| ExecutionError::InvalidRecord)?;
    let checkpoint = OutputCheckpoint::new(
        dispatch,
        df_imagery::execution::OutputObject {
            storage_key,
            object_version,
            byte_len: invalid(u64::try_from(byte_len))?,
            sha256: bytes32(&sha256)?,
        },
        df_imagery::execution::OutputImage {
            mime,
            width_px,
            height_px,
            alpha: alpha_of(&alpha_text)?,
        },
        bytes32(&recipe_digest)?,
        invalid(u32::try_from(usage_revision))?,
    )
    .map_err(|_| ExecutionError::InvalidRecord)?;
    if checkpoint.book_call_id != book_call_id {
        return Err(ExecutionError::InvalidRecord);
    }
    Ok(checkpoint)
}

pub(crate) fn parse_supplier_checkpoint(
    row: &Row,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<SupplierCheckpoint, ExecutionError> {
    let version: i32 = row.get("checkpoint_version");
    let origin: String = row.get("supplier_origin");
    let model: String = row.get("supplier_model");
    let bytes: Vec<u8> = row.get("checkpoint_bytes");
    let recorded_at_ms: i64 = row.get("recorded_at_ms");
    Ok(SupplierCheckpoint {
        owner: owner.clone(),
        job: *job,
        stage: *stage,
        dispatch: *dispatch,
        checkpoint_version: invalid(u32::try_from(version))?,
        origin,
        model,
        checkpoint_bytes: bytes,
        now_ms: recorded_at_ms,
    })
}

/// Reads the owned job row with its lock, revision, deadline, frozen state,
/// planned identity and retained intent/config byte counts. A wrong owner
/// learns nothing: the row reads as absent.
pub(crate) async fn locked_job(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<
    (
        Vec<u8>,
        u64,
        i64,
        String,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        i64,
    ),
    ExecutionError,
> {
    let row = tx
        .query_opt(
            "SELECT owner_scope, revision, absolute_deadline_ms, state, \
             planned_publication_asset, planned_publication_attempt, \
             octet_length(intent_bytes) AS intent_len, \
             octet_length(config_bytes) AS config_len \
             FROM imagery.image_job WHERE job_id = $1 FOR UPDATE",
            &[&job.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::NotFound)?;
    let scope: Vec<u8> = row.get("owner_scope");
    if !owner_matches(&scope, owner) {
        return Err(ExecutionError::NotFound);
    }
    let revision: i64 = row.get("revision");
    let deadline: i64 = row.get("absolute_deadline_ms");
    let state: String = row.get("state");
    let intent_len: i32 = row.get("intent_len");
    let config_len: i32 = row.get("config_len");
    Ok((
        scope,
        invalid(u64::try_from(revision))?,
        deadline,
        state,
        row.get("planned_publication_asset"),
        row.get("planned_publication_attempt"),
        i64::from(intent_len) + i64::from(config_len),
    ))
}

/// Bumps the existing job revision in the same transaction so service watch
/// observes every execution change. Returns the new revision.
pub(crate) async fn bump_job_revision(
    tx: &Transaction<'_>,
    job: &ImageJobId,
    now_ms: i64,
) -> Result<u64, ExecutionError> {
    let row = tx
        .query_one(
            "UPDATE imagery.image_job SET revision = revision + 1, updated_at_ms = $2 \
             WHERE job_id = $1 RETURNING revision",
            &[&job.as_bytes().as_slice(), &now_ms],
        )
        .await
        .map_err(map_db)?;
    let revision: i64 = row.get("revision");
    invalid(u64::try_from(revision))
}

pub(crate) fn planned_pair(
    asset: Option<Vec<u8>>,
    attempt: Option<Vec<u8>>,
) -> Result<([u8; 16], [u8; 16]), ExecutionError> {
    match (asset, attempt) {
        (Some(asset), Some(attempt)) => Ok((bytes16(&asset)?, bytes16(&attempt)?)),
        _ => Err(ExecutionError::InvalidRecord),
    }
}

/// Parses a genuine optional media identity, rejecting a partially populated database pair.
pub(crate) fn optional_planned_pair(
    asset: Option<Vec<u8>>,
    attempt: Option<Vec<u8>>,
) -> Result<Option<df_imagery::execution::PublicationIdentity>, ExecutionError> {
    match (asset, attempt) {
        (None, None) => Ok(None),
        (Some(asset), Some(attempt)) => invalid(df_imagery::execution::PublicationIdentity::new(
            bytes16(&asset)?,
            bytes16(&attempt)?,
        ))
        .map(Some),
        _ => Err(ExecutionError::InvalidRecord),
    }
}

/// Attached interests keep a job alive: with none, demand lapsed and no new
/// claim may start. Reads only; the job lock the caller holds serializes it.
pub(crate) async fn attached_interest_count(
    tx: &Transaction<'_>,
    job: &ImageJobId,
) -> Result<i64, ExecutionError> {
    let row = tx
        .query_one(
            "SELECT COUNT(*) FROM imagery.image_interest \
             WHERE job_id = $1 AND state = 'attached'",
            &[&job.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?;
    Ok(row.get("count"))
}

/// Reserves one pending slot with the job's retained intent/config bytes.
/// The conditional update carries the limit under competing transactions: a
/// missing scope row or a full queue both refuse without reserving.
pub(crate) async fn reserve_pending_in(
    tx: &Transaction<'_>,
    scope: &str,
    job_bytes: i64,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let reserved = tx
        .execute(
            "UPDATE imagery.image_execution_capacity SET pending_jobs = pending_jobs + 1, \
             pending_bytes = pending_bytes + $2, updated_at_ms = $3 \
             WHERE capacity_scope = $1 AND pending_jobs < pending_jobs_bound \
             AND pending_bytes + $2 <= pending_bytes_bound",
            &[&scope, &job_bytes, &now_ms],
        )
        .await
        .map_err(map_db)?;
    if reserved != 1 {
        return Err(ExecutionError::CapacityFull);
    }
    Ok(())
}

/// Returns one pending reserve. The caller holds the locks that prove the
/// reserve exists, so a missing row is a corrupt record, never silent.
pub(crate) async fn release_pending_in(
    tx: &Transaction<'_>,
    scope: &str,
    job_bytes: i64,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let released = tx
        .execute(
            "UPDATE imagery.image_execution_capacity SET pending_jobs = pending_jobs - 1, \
             pending_bytes = pending_bytes - $2, updated_at_ms = $3 \
             WHERE capacity_scope = $1",
            &[&scope, &job_bytes, &now_ms],
        )
        .await
        .map_err(map_db)?;
    if released != 1 {
        return Err(ExecutionError::InvalidRecord);
    }
    Ok(())
}

/// Moves one pending reserve to active concurrency for a first claim. The
/// conditional update carries the active bound under competing transactions.
pub(crate) async fn move_pending_to_active_in(
    tx: &Transaction<'_>,
    scope: &str,
    job_bytes: i64,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let moved = tx
        .execute(
            "UPDATE imagery.image_execution_capacity SET pending_jobs = pending_jobs - 1, \
             pending_bytes = pending_bytes - $2, in_flight = in_flight + 1, \
             updated_at_ms = $3 WHERE capacity_scope = $1 AND in_flight < bound \
             AND pending_jobs<=pending_jobs_bound AND pending_bytes<=pending_bytes_bound",
            &[&scope, &job_bytes, &now_ms],
        )
        .await
        .map_err(map_db)?;
    if moved != 1 {
        return Err(ExecutionError::CapacityFull);
    }
    Ok(())
}

/// Reserves one active slot for a post-terminal retry: a successful or
/// cancelled stage never reclaims its retired slot without this new
/// reservation against the operator bound.
pub(crate) async fn reserve_active_in(
    tx: &Transaction<'_>,
    scope: &str,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let reserved = tx
        .execute(
            "UPDATE imagery.image_execution_capacity SET in_flight = in_flight + 1, \
             updated_at_ms = $2 WHERE capacity_scope = $1 AND in_flight < bound \
             AND pending_jobs<=pending_jobs_bound AND pending_bytes<=pending_bytes_bound",
            &[&scope, &now_ms],
        )
        .await
        .map_err(map_db)?;
    if reserved != 1 {
        return Err(ExecutionError::CapacityFull);
    }
    Ok(())
}

/// Returns one held active slot to its scope. Idempotent per caller
/// discipline: the stage flag decides whether a slot is still held.
pub(crate) async fn release_capacity_slot(
    tx: &Transaction<'_>,
    scope: &str,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    tx.execute(
        "UPDATE imagery.image_execution_capacity SET in_flight = in_flight - 1, \
         updated_at_ms = $2 WHERE capacity_scope = $1 AND in_flight > 0",
        &[&scope, &now_ms],
    )
    .await
    .map_err(map_db)?;
    Ok(())
}

/// Reads the actively applied execution gate for one scope. No gate row
/// means no operator revision was ever applied: the fence reports revision
/// zero so every revisioned claim refuses until the operator sets one.
pub(crate) async fn read_gate_in(
    tx: &Transaction<'_>,
    scope: &str,
) -> Result<(u64, bool), ExecutionError> {
    let row = tx
        .query_opt(
            "SELECT applied_revision, enabled FROM imagery.image_execution_gate \
             WHERE gate_scope = $1 FOR SHARE",
            &[&scope],
        )
        .await
        .map_err(map_db)?;
    match row {
        Some(row) => {
            let revision: i64 = row.get("applied_revision");
            Ok((invalid(u64::try_from(revision))?, row.get("enabled")))
        }
        None => Err(ExecutionError::ConfigFenced {
            current_revision: 0,
        }),
    }
}

/// Refuses when the scope's execution gate is missing or disabled. New work
/// stops; already-sent liability and reconciliation are unaffected.
pub(crate) async fn require_gate_enabled(
    tx: &Transaction<'_>,
    scope: &str,
) -> Result<(), ExecutionError> {
    let (_, enabled) = read_gate_in(tx, scope).await?;
    if enabled {
        Ok(())
    } else {
        Err(ExecutionError::Disabled)
    }
}

/// Checks a claim against the actively applied gate revision and enablement:
/// this compares durable operator state, never the job's immutable config
/// snapshot against the caller itself.
pub(crate) async fn require_gate_revision(
    tx: &Transaction<'_>,
    scope: &str,
    expected: u64,
) -> Result<(), ExecutionError> {
    let (applied, enabled) = read_gate_in(tx, scope).await?;
    if !enabled {
        return Err(ExecutionError::Disabled);
    }
    if applied != expected {
        return Err(ExecutionError::ConfigFenced {
            current_revision: applied,
        });
    }
    Ok(())
}

/// The locked stage with its claim, checked against a presenting worker: the
/// owner must match, the stage must be admitted, and the presented token and
/// generation must own a live lease. Anything else is `NotFound`, `Stale` or
/// `LostClaim`; nothing is written by this check.
pub(crate) async fn locked_claim(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    claim: &df_imagery::execution::ClaimToken,
    generation: u64,
    now_ms: i64,
) -> Result<Row, ExecutionError> {
    let row = tx
        .query_opt(
            "SELECT job_id, stage_id, owner_scope, stage_kind, stage_state, \
             claim_token, claim_generation, claim_config_revision, lease_until_ms, capacity_scope, \
             capacity_reserved, local_accounting \
             FROM imagery.image_execution_stage \
             WHERE job_id = $1 AND stage_id = $2 FOR UPDATE",
            &[&job.as_bytes().as_slice(), &stage.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::NotFound)?;
    let stored_scope: Vec<u8> = row.get("owner_scope");
    check_owner(&stored_scope, owner)?;
    let stage_state: String = row.get("stage_state");
    if stage_state != "admitted" {
        let job_row = tx
            .query_one(
                "SELECT revision FROM imagery.image_job WHERE job_id = $1",
                &[&job.as_bytes().as_slice()],
            )
            .await
            .map_err(map_db)?;
        let revision: i64 = job_row.get("revision");
        return Err(ExecutionError::Stale {
            current_revision: invalid(u64::try_from(revision))?,
        });
    }
    let stored_token = bytes16(&row.get::<_, Vec<u8>>("claim_token"))?;
    let stored_generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
    let stored_lease: i64 = row.get("lease_until_ms");
    if !df_imagery::execution::lease_is_live(stored_lease, now_ms)
        || !df_imagery::execution::claim_owns(
            claim.as_bytes(),
            generation,
            &stored_token,
            stored_generation,
        )
    {
        return Err(ExecutionError::LostClaim);
    }
    Ok(row)
}

/// Locks one dispatch of this job and stage. A wrong dispatch id learns
/// nothing; a dispatch of another stage reads as absent.
pub(crate) async fn locked_dispatch(
    tx: &Transaction<'_>,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<Row, ExecutionError> {
    let row = tx
        .query_opt(
            "SELECT job_id, stage_id, dispatch_id, dispatch_seq, dispatch_state, \
             failure_cause, settlement, finance_attempt, finance_operation, finance_effect, \
             supplier_job_id, usage_revision, book_call_id, claim_token, claim_generation, \
             local_accounting, created_at_ms, updated_at_ms \
             FROM imagery.image_dispatch_attempt WHERE dispatch_id = $1 FOR UPDATE",
            &[&dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::NotFound)?;
    let row_job: Vec<u8> = row.get("job_id");
    let row_stage: Vec<u8> = row.get("stage_id");
    if row_job.as_slice() != job.as_bytes() || row_stage.as_slice() != stage.as_bytes() {
        return Err(ExecutionError::NotFound);
    }
    Ok(row)
}

/// Reads one dispatch row with its owner for a view.
pub(crate) async fn read_dispatch_view(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    dispatch: &df_imagery::execution::DispatchId,
) -> Result<DispatchView, ExecutionError> {
    let row = tx
        .query_opt(
            "SELECT d.dispatch_id, d.job_id, d.stage_id, d.dispatch_seq, d.dispatch_state, \
             d.failure_cause, d.settlement, d.finance_attempt, d.finance_operation, \
             d.finance_effect, d.supplier_job_id, d.usage_revision, d.book_call_id, \
             d.lineage_parent, d.claim_token, d.claim_generation, d.local_accounting, \
             d.created_at_ms, d.updated_at_ms, s.owner_scope AS owner_scope \
             FROM imagery.image_dispatch_attempt d \
             JOIN imagery.image_execution_stage s \
             ON s.job_id = d.job_id AND s.stage_id = d.stage_id \
             WHERE d.dispatch_id = $1",
            &[&dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::NotFound)?;
    let scope: Vec<u8> = row.get("owner_scope");
    check_owner(&scope, owner)?;
    parse_dispatch(&row, owner)
}
