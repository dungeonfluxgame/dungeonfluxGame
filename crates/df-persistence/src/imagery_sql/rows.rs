//! Row mapping for the imagery journal: the closed-vocabulary columns read back into typed
//! values, and the one place a database error is recorded. Statement text and payload bytes
//! never reach diagnostics; only the SQLSTATE is recorded.

use df_assets::MediaRequestKey;
use df_imagery::{
    ImageInterest, ImageJob, ImageJobId, InterestState, JobState, JournalError, SharingIdentity,
    VersionedSnapshot, owner_of_bytes,
};
use df_types::OperationId;
use tokio_postgres::Row;

/// The one place a pre-commit database error becomes a journal error: the SQLSTATE is recorded
/// so a constraint violation or a deadlock is not mistaken for an outage. A unique violation
/// means a supplied id is already bound elsewhere; every other failure changed nothing and
/// stays `Unavailable`. No statement text or value is logged.
pub(crate) fn unavailable(error: tokio_postgres::Error) -> JournalError {
    record_failure(&error);
    unique_violation(&error).unwrap_or(JournalError::Unavailable)
}

fn unique_violation(error: &tokio_postgres::Error) -> Option<JournalError> {
    let sqlstate = error.as_db_error()?.code().code();
    (sqlstate == "23505").then_some(JournalError::IdentityConflict)
}

pub(crate) fn record_failure(error: &tokio_postgres::Error) {
    let sqlstate = error.code().map_or("none", |code| code.code());
    df_observe::record_audit_line("IMAGERY", format!("IMAGERY db_failed sqlstate={sqlstate}"));
}

fn invalid<T, E>(result: Result<T, E>) -> Result<T, JournalError> {
    result.map_err(|_| {
        df_observe::record_audit_line("IMAGERY", "IMAGERY record_invalid".to_owned());
        JournalError::InvalidRecord
    })
}

fn snapshot_of(revision: i64, bytes: Vec<u8>) -> Result<VersionedSnapshot, JournalError> {
    let revision = invalid(u32::try_from(revision))?;
    invalid(VersionedSnapshot::new(revision, bytes, usize::MAX))
}

pub(super) const JOB_COLUMNS: &str = "j.job_id, j.owner_scope, j.sharing_identity, \
    j.intent_revision, j.intent_bytes, j.config_revision, j.config_bytes, j.absolute_deadline_ms, \
    j.state, j.revision, j.resolved_recipe_key, j.recipe_revision, j.recipe_bytes, \
    j.created_at_ms, j.updated_at_ms";

pub(super) fn parse_job(row: &Row) -> Result<ImageJob, JournalError> {
    let job_bytes: Vec<u8> = row.get("job_id");
    let scope_bytes: Vec<u8> = row.get("owner_scope");
    let sharing_bytes: Vec<u8> = row.get("sharing_identity");
    let intent_bytes: Vec<u8> = row.get("intent_bytes");
    let config_bytes: Vec<u8> = row.get("config_bytes");
    let state_text: String = row.get("state");
    let revision_int: i64 = row.get("revision");
    let key_bytes: Option<Vec<u8>> = row.get("resolved_recipe_key");
    let recipe_revision: Option<i64> = row.get("recipe_revision");
    let recipe_bytes: Option<Vec<u8>> = row.get("recipe_bytes");
    let owner = invalid(owner_of_bytes(&scope_bytes).ok_or(()))?;
    let state = invalid(JobState::parse_label(&state_text).ok_or(()))?;
    let resolved_key = key_bytes
        .map(|bytes| {
            let array: [u8; 32] = invalid(bytes.as_slice().try_into())?;
            Ok(MediaRequestKey::from_bytes(array))
        })
        .transpose()?;
    let recipe = match (recipe_revision, recipe_bytes) {
        (Some(revision), Some(bytes)) => Some(snapshot_of(revision, bytes)?),
        (None, None) => None,
        _ => return Err(JournalError::InvalidRecord),
    };
    match state {
        JobState::Planning => {
            if resolved_key.is_some() || recipe.is_some() {
                return Err(JournalError::InvalidRecord);
            }
        }
        JobState::RecipeFrozen => {
            if resolved_key.is_none() || recipe.is_none() {
                return Err(JournalError::InvalidRecord);
            }
        }
        JobState::Retired => {
            if resolved_key.is_none() != recipe.is_none() {
                return Err(JournalError::InvalidRecord);
            }
        }
    }
    Ok(ImageJob {
        job: invalid(ImageJobId::from_bytes(&job_bytes))?,
        owner,
        sharing: invalid(SharingIdentity::from_bytes(&sharing_bytes))?,
        intent: snapshot_of(row.get("intent_revision"), intent_bytes)?,
        config: snapshot_of(row.get("config_revision"), config_bytes)?,
        absolute_deadline_ms: row.get("absolute_deadline_ms"),
        state,
        revision: invalid(u64::try_from(revision_int))?,
        resolved_key,
        recipe,
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

pub(super) fn parse_lookup_receipt(
    row: &Row,
    owner: &df_imagery::ImageOwner,
    operation: OperationId,
    job: ImageJobId,
) -> Result<df_imagery::OperationReceipt, JournalError> {
    let actual_owner: Vec<u8> = row.get("op_owner");
    let actual_operation: Vec<u8> = row.get("op_operation");
    let actual_job: Vec<u8> = row.get("op_job");
    let fingerprint_bytes: Vec<u8> = row.get("op_fingerprint");
    let interest_bytes: Vec<u8> = row.get("op_interest");
    let expected_scope = df_imagery::owner_scope_bytes(owner);
    if actual_owner != expected_scope {
        return Err(JournalError::InvalidRecord);
    }
    if actual_operation != operation.as_bytes().as_slice() {
        return Err(JournalError::InvalidRecord);
    }
    if actual_job != job.as_bytes().as_slice() {
        return Err(JournalError::InvalidRecord);
    }
    let fingerprint: [u8; 32] = invalid(fingerprint_bytes.as_slice().try_into())?;
    Ok(df_imagery::OperationReceipt {
        owner: owner.clone(),
        operation,
        fingerprint,
        job,
        interest: invalid(df_imagery::ImageInterestId::from_bytes(&interest_bytes))?,
    })
}

/// The interest row of the joined lookup read, where every interest column carries an `int_`
/// prefix because the receipt columns share the same table names. The stored owner, job and
/// receipt linkage are validated against the actual tuples; a mismatch is an invalid record,
/// never a silent substitution of the requested values.
pub(super) fn parse_lookup_interest(
    row: &Row,
    owner: &df_imagery::ImageOwner,
    job: ImageJobId,
    operation: OperationId,
    interest: df_imagery::ImageInterestId,
) -> Result<ImageInterest, JournalError> {
    let actual_owner: Vec<u8> = row.get("int_owner");
    let actual_job: Vec<u8> = row.get("int_job");
    if actual_owner != df_imagery::owner_scope_bytes(owner) {
        return Err(JournalError::InvalidRecord);
    }
    if actual_job != job.as_bytes().as_slice() {
        return Err(JournalError::InvalidRecord);
    }
    let columns = InterestColumns {
        id: "int_id",
        operation: "int_operation",
        state: "int_state",
        created: "int_created",
        withdrawn: "int_withdrawn",
    };
    let expected = ExpectedLink {
        operation,
        interest,
    };
    parse_interest_parts(row, owner, job, &columns, Some(expected))
}

/// The interest row read from its own table with plain column names. The stored owner scope
/// and job are validated against the expected linkage; a mismatch is an invalid record.
pub(super) fn parse_interest_row(
    row: &Row,
    owner: &df_imagery::ImageOwner,
    job: ImageJobId,
) -> Result<ImageInterest, JournalError> {
    let actual_owner: Vec<u8> = row.get("owner_scope");
    let actual_job: Vec<u8> = row.get("job_id");
    if actual_owner != df_imagery::owner_scope_bytes(owner) {
        return Err(JournalError::InvalidRecord);
    }
    if actual_job != job.as_bytes().as_slice() {
        return Err(JournalError::InvalidRecord);
    }
    let columns = InterestColumns {
        id: "interest_id",
        operation: "operation_id",
        state: "state",
        created: "created_at_ms",
        withdrawn: "withdrawn_at_ms",
    };
    parse_interest_parts(row, owner, job, &columns, None)
}

/// Column names for one interest row read. The joined lookup prefixes every interest
/// column because the receipt columns share the same table names.
struct InterestColumns {
    id: &'static str,
    operation: &'static str,
    state: &'static str,
    created: &'static str,
    withdrawn: &'static str,
}

/// The exact receipt linkage a joined lookup must match: the operation and interest the
/// receipt row already named. A mismatch is an invalid record.
#[derive(Clone, Copy)]
struct ExpectedLink {
    operation: OperationId,
    interest: df_imagery::ImageInterestId,
}

fn parse_interest_parts(
    row: &Row,
    owner: &df_imagery::ImageOwner,
    job: ImageJobId,
    columns: &InterestColumns,
    expected_link: Option<ExpectedLink>,
) -> Result<ImageInterest, JournalError> {
    let interest_bytes: Vec<u8> = row.get(columns.id);
    let operation_bytes: Vec<u8> = row.get(columns.operation);
    let state_text: String = row.get(columns.state);
    let interest = invalid(df_imagery::ImageInterestId::from_bytes(&interest_bytes))?;
    let operation = invalid(OperationId::from_bytes(&operation_bytes))?;
    if expected_link.is_some_and(|expected| {
        operation != expected.operation || interest != expected.interest
    }) {
        return Err(JournalError::InvalidRecord);
    }
    Ok(ImageInterest {
        interest,
        owner: owner.clone(),
        job,
        operation,
        state: invalid(InterestState::parse_label(&state_text).ok_or(()))?,
        created_at_ms: row.get(columns.created),
        withdrawn_at_ms: row.get(columns.withdrawn),
    })
}
