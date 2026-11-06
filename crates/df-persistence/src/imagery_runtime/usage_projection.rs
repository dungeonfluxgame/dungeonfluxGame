//! Bounded metadata-only journal snapshot. No supplier bytes, prompts, pricing or money reads.

use df_imagery::cost::{ImageCostProjection, UsageEvidence, UsageRevision, pinned_call_id};
use df_imagery::execution::{
    DispatchId, DispatchMode, ExecutionScope, ExecutionStageId, FinanceTuple, StageKind,
};
use df_imagery::runtime::RuntimeUsage;
use df_imagery::runtime::usage_projection::{
    ImageUsageSummary, StageUsageObservation, UsageAttempt, UsageBookProjectionInput,
    UsageProjectionInput, UsageProjectionLimits, project_book_usage, project_usage,
};
use df_imagery::{ImageJobId, ImageOwner, JournalError, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};

/// Owned snapshot; the caller releases the database transaction/connection before Book awaits.
pub struct UsageProjectionRows {
    pub job_revision: u64,
    pub attempts: Vec<UsageAttempt>,
    pub observations: Vec<RuntimeUsage>,
}

impl std::fmt::Debug for UsageProjectionRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsageProjectionRows")
            .field("job_revision", &self.job_revision)
            .field("attempts", &self.attempts.len())
            .field("observations", &self.observations.len())
            .finish()
    }
}

const ATTEMPTS: &str = concat!(
    "SELECT d.stage_id,d.dispatch_id,d.book_call_id,s.stage_kind,d.local_accounting,d.finance_attempt,d.finance_operation,d.finance_effect ",
    "FROM imagery.image_dispatch_attempt d JOIN imagery.image_execution_stage s ",
    "ON s.job_id=d.job_id AND s.stage_id=d.stage_id AND s.owner_scope=j.owner_scope ",
    "WHERE d.job_id=j.job_id ORDER BY d.stage_id,d.dispatch_id LIMIT $3"
);

const METADATA_VALID: &str = concat!(
    "octet_length(u.provider)<=$6::bigint AND octet_length(u.model)<=$6::bigint ",
    "AND octet_length(u.cause)<=$6::bigint AND octet_length(u.detail)<=$6::bigint ",
    "AND jsonb_array_length(u.maxima)<=$4::bigint AND jsonb_array_length(u.units)<=$4::bigint ",
    "AND octet_length(u.maxima::text)<=$5::bigint AND octet_length(u.units::text)<=$5::bigint"
);

fn query() -> String {
    format!(
        concat!(
            "SELECT j.revision AS job_revision,a.stage_id,a.dispatch_id,a.book_call_id,a.stage_kind,a.local_accounting,a.finance_attempt,a.finance_operation,a.finance_effect, ",
            "u.revision AS usage_revision,CASE WHEN {valid} THEN u.provider END AS provider, ",
            "CASE WHEN {valid} THEN u.model END AS model, ",
            "CASE WHEN {valid} THEN u.cause END AS cause, ",
            "CASE WHEN {valid} THEN u.detail END AS detail,u.evidence,u.usable,u.observed_at_ms, ",
            "(u.revision IS NULL OR ({valid})) AS metadata_valid, ",
            "CASE WHEN {valid} THEN u.maxima::text END AS maxima_text, ",
            "CASE WHEN {valid} THEN u.units::text END AS units_text ",
            "FROM imagery.image_job j LEFT JOIN LATERAL ({attempts}) a ON true ",
            "LEFT JOIN LATERAL (SELECT revision,provider,model,cause,detail,maxima,evidence,units,usable,observed_at_ms ",
            "FROM imagery.image_dispatch_usage WHERE owner_scope=j.owner_scope AND job_id=j.job_id ",
            "AND stage_id=a.stage_id AND dispatch_id=a.dispatch_id ORDER BY revision DESC LIMIT 1) u ON true ",
            "WHERE j.job_id=$1 AND j.owner_scope=$2 ORDER BY a.stage_id,a.dispatch_id"
        ),
        valid = METADATA_VALID,
        attempts = ATTEMPTS
    )
}

struct ReadBounds {
    attempts_plus_one: i64,
    unit_lines: i64,
    unit_json_bytes: i64,
    word_bytes: i64,
}

fn read_bounds(limits: UsageProjectionLimits) -> Result<ReadBounds, JournalError> {
    limits.validate().map_err(|_| JournalError::InvalidRecord)?;
    let attempts = limits
        .max_attempts
        .checked_add(1)
        .ok_or(JournalError::InvalidRecord)?;
    // JSONB's canonical text needs at most six bytes per category byte plus keys/u64 framing.
    let per_line = limits
        .max_word_bytes
        .checked_mul(6)
        .and_then(|bytes| bytes.checked_add(96))
        .ok_or(JournalError::InvalidRecord)?;
    let json_bytes = limits
        .max_unit_lines
        .checked_mul(per_line)
        .and_then(|bytes| bytes.checked_add(2))
        .ok_or(JournalError::InvalidRecord)?;
    Ok(ReadBounds {
        attempts_plus_one: i64::try_from(attempts).map_err(|_| JournalError::InvalidRecord)?,
        unit_lines: i64::try_from(limits.max_unit_lines)
            .map_err(|_| JournalError::InvalidRecord)?,
        unit_json_bytes: i64::try_from(json_bytes).map_err(|_| JournalError::InvalidRecord)?,
        word_bytes: i64::try_from(limits.max_word_bytes)
            .map_err(|_| JournalError::InvalidRecord)?,
    })
}

/// One SQL snapshot of the exact scoped job revision, attempts and latest raw44 metadata.
/// Bounded excess is InvalidRecord; no history, payload bytes or private configuration is selected.
pub async fn read_usage_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: ImageJobId,
    limits: UsageProjectionLimits,
) -> Result<Option<UsageProjectionRows>, JournalError> {
    let bounds = read_bounds(limits)?;
    let rows = tx
        .query(
            &query(),
            &[
                &job.as_bytes().as_slice(),
                &owner_scope_bytes(owner),
                &bounds.attempts_plus_one,
                &bounds.unit_lines,
                &bounds.unit_json_bytes,
                &bounds.word_bytes,
            ],
        )
        .await
        .map_err(|_| JournalError::Unavailable)?;
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    if rows.len() > limits.max_attempts {
        return Err(JournalError::InvalidRecord);
    }
    let revision = first
        .try_get::<_, i64>("job_revision")
        .map_err(|_| JournalError::InvalidRecord)?;
    let job_revision = u64::try_from(revision)
        .ok()
        .filter(|revision| *revision > 0)
        .ok_or(JournalError::InvalidRecord)?;
    let mut result = UsageProjectionRows {
        job_revision,
        attempts: Vec::new(),
        observations: Vec::new(),
    };
    for row in &rows {
        decode_row(row, owner, job, limits, &mut result)?;
    }
    project_usage(UsageProjectionInput {
        owner,
        job,
        attempts: &result.attempts,
        observations: &result.observations,
        receipts: &[],
        limits,
    })
    .map_err(|_| JournalError::InvalidRecord)?;
    Ok(Some(result))
}

fn decode_row(
    row: &Row,
    owner: &ImageOwner,
    job: ImageJobId,
    limits: UsageProjectionLimits,
    result: &mut UsageProjectionRows,
) -> Result<(), JournalError> {
    if !row
        .try_get::<_, bool>("metadata_valid")
        .map_err(|_| JournalError::InvalidRecord)?
    {
        return Err(JournalError::InvalidRecord);
    }
    let dispatch = row
        .try_get::<_, Option<Vec<u8>>>("dispatch_id")
        .map_err(|_| JournalError::InvalidRecord)?;
    let Some(dispatch) = dispatch else {
        return Ok(());
    };
    let dispatch = DispatchId::from_bytes(&dispatch).map_err(|_| JournalError::InvalidRecord)?;
    let stage = ExecutionStageId::from_bytes(
        &row.try_get::<_, Vec<u8>>("stage_id")
            .map_err(|_| JournalError::InvalidRecord)?,
    )
    .map_err(|_| JournalError::InvalidRecord)?;
    let kind = StageKind::from_key(
        &row.try_get::<_, String>("stage_kind")
            .map_err(|_| JournalError::InvalidRecord)?,
    )
    .ok_or(JournalError::InvalidRecord)?;
    if row
        .try_get::<_, String>("book_call_id")
        .map_err(|_| JournalError::InvalidRecord)?
        != pinned_call_id(dispatch.as_bytes())
    {
        return Err(JournalError::InvalidRecord);
    }
    result.attempts.push(UsageAttempt {
        scope: ExecutionScope {
            owner: owner.clone(),
            job,
            stage,
            dispatch,
        },
        kind,
        mode: decode_mode(
            row.try_get("local_accounting")
                .map_err(|_| JournalError::InvalidRecord)?,
            row.try_get("finance_attempt")
                .map_err(|_| JournalError::InvalidRecord)?,
            row.try_get("finance_operation")
                .map_err(|_| JournalError::InvalidRecord)?,
            row.try_get("finance_effect")
                .map_err(|_| JournalError::InvalidRecord)?,
        )?,
    });
    let revision = row
        .try_get::<_, Option<i64>>("usage_revision")
        .map_err(|_| JournalError::InvalidRecord)?;
    if let Some(revision) = revision {
        if result.observations.len() >= limits.max_observations {
            return Err(JournalError::InvalidRecord);
        }
        result
            .observations
            .push(decode_usage(row, owner, job, stage, dispatch, revision)?);
    }
    Ok(())
}

fn decode_mode(
    local: bool,
    attempt: Option<Vec<u8>>,
    operation: Option<Vec<u8>>,
    effect: Option<Vec<u8>>,
) -> Result<DispatchMode, JournalError> {
    match (local, attempt, operation, effect) {
        (true, None, None, None) => Ok(DispatchMode::Local),
        (false, Some(attempt), Some(operation), Some(effect)) => {
            let tuple = FinanceTuple::new(
                attempt
                    .as_slice()
                    .try_into()
                    .map_err(|_| JournalError::InvalidRecord)?,
                operation
                    .as_slice()
                    .try_into()
                    .map_err(|_| JournalError::InvalidRecord)?,
                effect
                    .as_slice()
                    .try_into()
                    .map_err(|_| JournalError::InvalidRecord)?,
            )
            .map_err(|_| JournalError::InvalidRecord)?;
            Ok(DispatchMode::Hosted(tuple))
        }
        _ => Err(JournalError::InvalidRecord),
    }
}

fn decode_usage(
    row: &Row,
    owner: &ImageOwner,
    job: ImageJobId,
    stage: ExecutionStageId,
    dispatch: DispatchId,
    revision: i64,
) -> Result<RuntimeUsage, JournalError> {
    let evidence = match row
        .try_get::<_, String>("evidence")
        .map_err(|_| JournalError::InvalidRecord)?
        .as_str()
    {
        "reported" => UsageEvidence::Reported,
        "estimated" => UsageEvidence::Estimated,
        "unknown" => UsageEvidence::Unknown,
        _ => return Err(JournalError::InvalidRecord),
    };
    let revision = u64::try_from(revision)
        .ok()
        .filter(|revision| *revision > 0)
        .ok_or(JournalError::InvalidRecord)?;
    Ok(RuntimeUsage {
        owner: owner.clone(),
        job,
        stage,
        dispatch,
        revision: UsageRevision(revision),
        evidence,
        route: df_imagery::cost::CostRoute {
            provider: row
                .try_get("provider")
                .map_err(|_| JournalError::InvalidRecord)?,
            model: row
                .try_get("model")
                .map_err(|_| JournalError::InvalidRecord)?,
            cause: row
                .try_get("cause")
                .map_err(|_| JournalError::InvalidRecord)?,
            detail: row
                .try_get("detail")
                .map_err(|_| JournalError::InvalidRecord)?,
            maxima: super::usage::decode_units(
                &row.try_get::<_, String>("maxima_text")
                    .map_err(|_| JournalError::InvalidRecord)?,
            )
            .map_err(|_| JournalError::InvalidRecord)?,
        },
        units: super::usage::decode_units(
            &row.try_get::<_, String>("units_text")
                .map_err(|_| JournalError::InvalidRecord)?,
        )
        .map_err(|_| JournalError::InvalidRecord)?,
        usable: row
            .try_get("usable")
            .map_err(|_| JournalError::InvalidRecord)?,
        at_ms: u64::try_from(
            row.try_get::<_, i64>("observed_at_ms")
                .map_err(|_| JournalError::InvalidRecord)?,
        )
        .map_err(|_| JournalError::InvalidRecord)?,
        raw_output: None,
    })
}

/// Called only after releasing the database transaction and its pool slot.
pub async fn project_rows<C: ImageCostProjection + ?Sized>(
    rows: UsageProjectionRows,
    owner: &ImageOwner,
    job: ImageJobId,
    book: &C,
    limits: UsageProjectionLimits,
) -> Result<ImageUsageSummary, JournalError> {
    let pending = project_usage(UsageProjectionInput {
        owner,
        job,
        attempts: &rows.attempts,
        observations: &rows.observations,
        receipts: &[],
        limits,
    })
    .map_err(|_| JournalError::InvalidRecord)?;
    let mut receipts = Vec::new();
    for stage in &pending.stages {
        if !matches!(&stage.observation, StageUsageObservation::Recorded(_)) {
            continue;
        }
        if let Some(receipt) = book
            .read_receipt(&pinned_call_id(stage.dispatch.as_bytes()))
            .await
            .map_err(|_| JournalError::Unavailable)?
        {
            receipts.push(receipt);
        }
    }
    project_book_usage(UsageBookProjectionInput {
        owner,
        job,
        attempts: &rows.attempts,
        observations: &rows.observations,
        book_receipts: &receipts,
        limits,
    })
    .map_err(|_| JournalError::InvalidRecord)
}

#[cfg(test)]
#[path = "usage_projection_tests.rs"]
mod tests;
