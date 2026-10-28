//! The imagery journal transactions: each function takes the caller's transaction and performs
//! one atomic call. The advisory lock over the canonical owner and operation serializes scoped
//! acceptance so concurrent creates leave exactly one job, receipt and interest; job rows are
//! locked before interest rows and at most one job row per call, so opposite-order deadlocks
//! cannot form. Snapshots are compared whole; the fingerprint never stands in for the payload.

use df_imagery::{
    AcceptCommand, AcceptOutcome, AcceptTarget, FreezeCommand, FreezeOutcome, ImageInterest,
    ImageJob, ImageJobId, InterestState, JobState, JournalError, OperationView, WithdrawOutcome,
    Withdrawal, is_expired, owner_scope_bytes,
};
use df_types::OperationId;
use tokio_postgres::Transaction;

use super::rows::{
    JOB_COLUMNS, parse_interest_row, parse_job, parse_lookup_interest, parse_lookup_receipt,
    unavailable,
};

/// Second seed input of the acceptance lock; the text carries owner and operation bytes.
const ACCEPT_LOCK_SEED: i64 = 2_026_102_737;

/// Every journal statement runs at READ COMMITTED: row locks carry the guarantees and no
/// caller may inherit a stricter default. A transaction at another level is refused before it
/// reads or writes anything.
async fn require_read_committed(tx: &Transaction<'_>) -> Result<(), JournalError> {
    let setting: String = tx
        .query_one("SELECT current_setting('transaction_isolation', true)", &[])
        .await
        .map_err(unavailable)?
        .get(0);
    if setting == "read committed" {
        return Ok(());
    }
    record_failure_for_isolation(&setting);
    Err(JournalError::Unavailable)
}

fn record_failure_for_isolation(setting: &str) {
    df_observe::record_audit_line(
        "IMAGERY",
        format!("IMAGERY wrong_isolation setting={setting}"),
    );
}

/// Serializes scoped operation acceptance: concurrent accepts of one operation queue here, so
/// the receipt lookup below decides replay versus creation exactly once.
async fn lock_operation(
    tx: &Transaction<'_>,
    owner_scope: &[u8],
    operation: &OperationId,
) -> Result<(), JournalError> {
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended(encode($1, 'hex') || '|' || \
         encode($2, 'hex'), $3))",
        &[
            &owner_scope,
            &operation.as_bytes().as_slice(),
            &ACCEPT_LOCK_SEED,
        ],
    )
    .await
    .map_err(unavailable)?;
    Ok(())
}

async fn job_exists(tx: &Transaction<'_>, job: &ImageJobId) -> Result<bool, JournalError> {
    let row = tx
        .query_opt(
            "SELECT 1 FROM imagery.image_job WHERE job_id = $1",
            &[&job.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    Ok(row.is_some())
}

async fn interest_exists(
    tx: &Transaction<'_>,
    interest: &df_imagery::ImageInterestId,
) -> Result<bool, JournalError> {
    let row = tx
        .query_opt(
            "SELECT 1 FROM imagery.image_interest WHERE interest_id = $1",
            &[&interest.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    Ok(row.is_some())
}

fn snapshot_columns(
    snapshot: &df_imagery::VersionedSnapshot,
) -> Result<(i64, Vec<u8>), JournalError> {
    Ok((i64::from(snapshot.revision()), snapshot.bytes().to_vec()))
}

pub(crate) async fn accept_in(
    tx: &Transaction<'_>,
    command: &AcceptCommand,
) -> Result<AcceptOutcome, JournalError> {
    require_read_committed(tx).await?;
    if command.now_ms <= 0 {
        return Err(JournalError::InvalidRecord);
    }
    let owner_scope = owner_scope_bytes(&command.identity.owner);
    lock_operation(tx, &owner_scope, &command.identity.operation).await?;
    if let Some(view) = lookup_in(tx, &command.identity.owner, command.identity.operation).await? {
        if view.receipt.fingerprint == command.identity.fingerprint
            && view.job.intent == command.intent
        {
            return Ok(AcceptOutcome::Replayed(view));
        }
        return Err(JournalError::OperationConflict);
    }
    match &command.target {
        AcceptTarget::Create {
            job,
            absolute_deadline_ms,
        } => create_in(tx, command, &owner_scope, job, *absolute_deadline_ms).await,
        AcceptTarget::Join { job } => join_in(tx, command, &owner_scope, job).await,
    }
}

async fn create_in(
    tx: &Transaction<'_>,
    command: &AcceptCommand,
    owner_scope: &[u8],
    job: &ImageJobId,
    absolute_deadline_ms: i64,
) -> Result<AcceptOutcome, JournalError> {
    if absolute_deadline_ms <= 0 || absolute_deadline_ms <= command.now_ms {
        return Err(JournalError::InvalidRecord);
    }
    if job_exists(tx, job).await? || interest_exists(tx, &command.interest).await? {
        return Err(JournalError::IdentityConflict);
    }
    let (intent_revision, intent_bytes) = snapshot_columns(&command.intent)?;
    let (config_revision, config_bytes) = snapshot_columns(&command.config)?;
    tx.execute(
        "INSERT INTO imagery.image_job (job_id, owner_scope, sharing_identity, intent_revision, \
         intent_bytes, config_revision, config_bytes, absolute_deadline_ms, state, revision, \
         created_at_ms, updated_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'planning', 1, $9, $9)",
        &[
            &job.as_bytes().as_slice(),
            &owner_scope,
            &command.sharing.as_bytes().as_slice(),
            &intent_revision,
            &intent_bytes,
            &config_revision,
            &config_bytes,
            &absolute_deadline_ms,
            &command.now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    tx.execute(
        "INSERT INTO imagery.image_operation (owner_scope, operation_id, fingerprint, job_id, \
         interest_id, created_at_ms) VALUES ($1, $2, $3, $4, $5, $6)",
        &[
            &owner_scope,
            &command.identity.operation.as_bytes().as_slice(),
            &command.identity.fingerprint.as_slice(),
            &job.as_bytes().as_slice(),
            &command.interest.as_bytes().as_slice(),
            &command.now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    tx.execute(
        "INSERT INTO imagery.image_interest (interest_id, owner_scope, job_id, operation_id, \
         state, created_at_ms) VALUES ($1, $2, $3, $4, 'attached', $5)",
        &[
            &command.interest.as_bytes().as_slice(),
            &owner_scope,
            &job.as_bytes().as_slice(),
            &command.identity.operation.as_bytes().as_slice(),
            &command.now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    let stored = ImageJob {
        job: *job,
        owner: command.identity.owner.clone(),
        sharing: command.sharing,
        intent: command.intent.clone(),
        config: command.config.clone(),
        absolute_deadline_ms,
        state: JobState::Planning,
        revision: 1,
        resolved_key: None,
        recipe: None,
        created_at_ms: command.now_ms,
        updated_at_ms: command.now_ms,
    };
    let interest = ImageInterest {
        interest: command.interest,
        owner: command.identity.owner.clone(),
        job: *job,
        operation: command.identity.operation,
        state: InterestState::Attached,
        created_at_ms: command.now_ms,
        withdrawn_at_ms: None,
    };
    Ok(AcceptOutcome::Created(OperationView {
        receipt: df_imagery::OperationReceipt {
            owner: command.identity.owner.clone(),
            operation: command.identity.operation,
            fingerprint: command.identity.fingerprint,
            job: *job,
            interest: command.interest,
        },
        job: stored,
        interest,
    }))
}

async fn read_locked(
    tx: &Transaction<'_>,
    owner_scope: &[u8],
    job: &ImageJobId,
) -> Result<Option<ImageJob>, JournalError> {
    let query = format!(
        "SELECT {JOB_COLUMNS} FROM imagery.image_job j WHERE j.job_id = $1 AND j.owner_scope = \
         $2 FOR UPDATE"
    );
    let row = tx
        .query_opt(&query, &[&job.as_bytes().as_slice(), &owner_scope])
        .await
        .map_err(unavailable)?;
    row.map(|row| parse_job(&row)).transpose()
}

async fn join_in(
    tx: &Transaction<'_>,
    command: &AcceptCommand,
    owner_scope: &[u8],
    job_id: &ImageJobId,
) -> Result<AcceptOutcome, JournalError> {
    let Some(mut job) = read_locked(tx, owner_scope, job_id).await? else {
        return Err(JournalError::NotFound);
    };
    if job.state == JobState::Retired {
        return Err(JournalError::JoinRefused(df_imagery::JoinRefusal::Retired));
    }
    if is_expired(job.absolute_deadline_ms, command.now_ms) {
        return Err(JournalError::JoinRefused(df_imagery::JoinRefusal::Expired));
    }
    if job.sharing != command.sharing {
        return Err(JournalError::JoinRefused(
            df_imagery::JoinRefusal::SharingMismatch,
        ));
    }
    if job.intent != command.intent {
        return Err(JournalError::JoinRefused(
            df_imagery::JoinRefusal::IntentMismatch,
        ));
    }
    if job.config != command.config {
        return Err(JournalError::JoinRefused(
            df_imagery::JoinRefusal::ConfigMismatch,
        ));
    }
    if interest_exists(tx, &command.interest).await? {
        return Err(JournalError::IdentityConflict);
    }
    tx.execute(
        "INSERT INTO imagery.image_operation (owner_scope, operation_id, fingerprint, job_id, \
         interest_id, created_at_ms) VALUES ($1, $2, $3, $4, $5, $6)",
        &[
            &owner_scope,
            &command.identity.operation.as_bytes().as_slice(),
            &command.identity.fingerprint.as_slice(),
            &job_id.as_bytes().as_slice(),
            &command.interest.as_bytes().as_slice(),
            &command.now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    tx.execute(
        "INSERT INTO imagery.image_interest (interest_id, owner_scope, job_id, operation_id, \
         state, created_at_ms) VALUES ($1, $2, $3, $4, 'attached', $5)",
        &[
            &command.interest.as_bytes().as_slice(),
            &owner_scope,
            &job_id.as_bytes().as_slice(),
            &command.identity.operation.as_bytes().as_slice(),
            &command.now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    tx.execute(
        "UPDATE imagery.image_job SET revision = revision + 1, updated_at_ms = $2 \
         WHERE job_id = $1",
        &[&job_id.as_bytes().as_slice(), &command.now_ms],
    )
    .await
    .map_err(unavailable)?;
    job.revision += 1;
    job.updated_at_ms = command.now_ms;
    let interest = ImageInterest {
        interest: command.interest,
        owner: command.identity.owner.clone(),
        job: *job_id,
        operation: command.identity.operation,
        state: InterestState::Attached,
        created_at_ms: command.now_ms,
        withdrawn_at_ms: None,
    };
    Ok(AcceptOutcome::Joined(OperationView {
        receipt: df_imagery::OperationReceipt {
            owner: command.identity.owner.clone(),
            operation: command.identity.operation,
            fingerprint: command.identity.fingerprint,
            job: *job_id,
            interest: command.interest,
        },
        job,
        interest,
    }))
}

/// The receipt with its current job and interest in one statement, so a concurrent freeze or
/// withdrawal cannot slip between separate reads.
pub(crate) async fn lookup_in(
    tx: &Transaction<'_>,
    owner: &df_imagery::ImageOwner,
    operation: OperationId,
) -> Result<Option<OperationView>, JournalError> {
    require_read_committed(tx).await?;
    let owner_scope = owner_scope_bytes(owner);
    let query = format!(
        "SELECT {JOB_COLUMNS}, o.owner_scope AS op_owner, o.operation_id AS op_operation, \
         o.fingerprint AS op_fingerprint, o.job_id AS op_job, o.interest_id AS op_interest, \
         i.interest_id AS int_id, i.owner_scope AS int_owner, i.job_id AS int_job, \
         i.operation_id AS int_operation, i.state AS int_state, i.created_at_ms AS int_created, \
         i.withdrawn_at_ms AS int_withdrawn \
         FROM imagery.image_operation o \
         JOIN imagery.image_job j ON j.job_id = o.job_id AND j.owner_scope = o.owner_scope \
         JOIN imagery.image_interest i ON i.interest_id = o.interest_id \
         WHERE o.owner_scope = $1 AND o.operation_id = $2"
    );
    let row = tx
        .query_opt(&query, &[&owner_scope, &operation.as_bytes().as_slice()])
        .await
        .map_err(unavailable)?;
    let Some(row) = row else { return Ok(None) };
    let job = parse_job(&row)?;
    if job.owner != *owner {
        return Err(JournalError::InvalidRecord);
    }
    let receipt = parse_lookup_receipt(&row, owner, operation, job.job)?;
    let interest =
        parse_lookup_interest(&row, owner, job.job, operation, receipt.interest)?;
    Ok(Some(OperationView {
        receipt,
        job,
        interest,
    }))
}

pub(crate) async fn read_in(
    tx: &Transaction<'_>,
    owner: &df_imagery::ImageOwner,
    job: &ImageJobId,
) -> Result<Option<ImageJob>, JournalError> {
    require_read_committed(tx).await?;
    let owner_scope = owner_scope_bytes(owner);
    let query = format!(
        "SELECT {JOB_COLUMNS} FROM imagery.image_job j WHERE j.job_id = $1 AND j.owner_scope = $2"
    );
    let row = tx
        .query_opt(&query, &[&job.as_bytes().as_slice(), &owner_scope])
        .await
        .map_err(unavailable)?;
    row.map(|row| parse_job(&row)).transpose()
}

pub(crate) async fn freeze_in(
    tx: &Transaction<'_>,
    command: &FreezeCommand,
) -> Result<FreezeOutcome, JournalError> {
    require_read_committed(tx).await?;
    if command.now_ms <= 0 {
        return Err(JournalError::InvalidRecord);
    }
    let owner_scope = owner_scope_bytes(&command.owner);
    let Some(mut job) = read_locked(tx, &owner_scope, &command.job).await? else {
        return Err(JournalError::NotFound);
    };
    // Frozen facts decide before any refusal: an identical frozen key and recipe rereads as
    // frozen even after retirement or expiry, without reviving the job, while a different
    // key or recipe keeps the first decision as a conflict.
    if let (Some(key), Some(recipe)) = (&job.resolved_key, &job.recipe) {
        if *key == command.key && *recipe == command.recipe {
            return Ok(FreezeOutcome::AlreadyFrozen { job });
        }
        return Err(JournalError::RecipeConflict);
    }
    if job.state == JobState::Retired {
        return Err(JournalError::Retired);
    }
    if is_expired(job.absolute_deadline_ms, command.now_ms) {
        return Err(JournalError::Expired);
    }
    if job.revision != command.expected_revision {
        return Err(JournalError::Stale {
            current_revision: job.revision,
        });
    }
    let (recipe_revision, recipe_bytes) = snapshot_columns(&command.recipe)?;
    tx.execute(
        "UPDATE imagery.image_job SET state = 'recipe_frozen', resolved_recipe_key = $2, \
         recipe_revision = $3, recipe_bytes = $4, revision = revision + 1, updated_at_ms = $5 \
         WHERE job_id = $1",
        &[
            &command.job.as_bytes().as_slice(),
            &command.key.as_bytes().as_slice(),
            &recipe_revision,
            &recipe_bytes,
            &command.now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    job.state = JobState::RecipeFrozen;
    job.resolved_key = Some(command.key);
    job.recipe = Some(command.recipe.clone());
    job.revision += 1;
    job.updated_at_ms = command.now_ms;
    Ok(FreezeOutcome::Frozen { job })
}

async fn count_attached(tx: &Transaction<'_>, job: &ImageJobId) -> Result<u64, JournalError> {
    let row = tx
        .query_one(
            "SELECT COUNT(*) FROM imagery.image_interest WHERE job_id = $1 AND state = 'attached'",
            &[&job.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    let count: i64 = row.get(0);
    u64::try_from(count).map_err(|_| JournalError::InvalidRecord)
}

pub(crate) async fn withdraw_in(
    tx: &Transaction<'_>,
    owner: &df_imagery::ImageOwner,
    interest_id: &df_imagery::ImageInterestId,
    now_ms: i64,
) -> Result<WithdrawOutcome, JournalError> {
    require_read_committed(tx).await?;
    if now_ms <= 0 {
        return Err(JournalError::InvalidRecord);
    }
    let owner_scope = owner_scope_bytes(owner);
    let peek = tx
        .query_opt(
            "SELECT job_id FROM imagery.image_interest WHERE interest_id = $1 AND owner_scope = $2",
            &[&interest_id.as_bytes().as_slice(), &owner_scope],
        )
        .await
        .map_err(unavailable)?;
    let Some(peek) = peek else {
        return Err(JournalError::NotFound);
    };
    let job_bytes: Vec<u8> = peek.get(0);
    let job_id = ImageJobId::from_bytes(&job_bytes).map_err(|_| JournalError::InvalidRecord)?;
    // The job row first, then the interest row: the same order every call locks in.
    let Some(mut job) = read_locked(tx, &owner_scope, &job_id).await? else {
        return Err(JournalError::InvalidRecord);
    };
    let locked = tx
        .query_opt(
            "SELECT interest_id, owner_scope, job_id, operation_id, state, created_at_ms, \
             withdrawn_at_ms FROM imagery.image_interest WHERE interest_id = $1 FOR UPDATE",
            &[&interest_id.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    let Some(locked) = locked else {
        return Err(JournalError::InvalidRecord);
    };
    let mut interest = parse_interest_row(&locked, owner, job_id)?;
    if interest.state == InterestState::Withdrawn {
        let attached_remaining = count_attached(tx, &job_id).await?;
        return Ok(WithdrawOutcome::AlreadyWithdrawn(Withdrawal {
            interest,
            attached_remaining,
            job_revision: job.revision,
        }));
    }
    tx.execute(
        "UPDATE imagery.image_interest SET state = 'withdrawn', withdrawn_at_ms = $2 \
         WHERE interest_id = $1",
        &[&interest_id.as_bytes().as_slice(), &now_ms],
    )
    .await
    .map_err(unavailable)?;
    let attached_remaining = count_attached(tx, &job_id).await?;
    let state = if attached_remaining == 0 {
        JobState::Retired
    } else {
        job.state
    };
    tx.execute(
        "UPDATE imagery.image_job SET state = $2, revision = revision + 1, updated_at_ms = $3 \
         WHERE job_id = $1",
        &[&job_id.as_bytes().as_slice(), &state.label(), &now_ms],
    )
    .await
    .map_err(unavailable)?;
    interest.state = InterestState::Withdrawn;
    interest.withdrawn_at_ms = Some(now_ms);
    job.state = state;
    job.revision += 1;
    job.updated_at_ms = now_ms;
    Ok(WithdrawOutcome::Withdrawn(Withdrawal {
        interest,
        attached_remaining,
        job_revision: job.revision,
    }))
}
