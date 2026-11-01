//! Exact no-dispatch inspection metadata remains separate from supplier decision references.

use df_imagery::execution::{CandidateReceipt, ExecutionStageId, LocalInspectionCheckpoint};
use df_imagery::publication::{MAXIMUM_RECIPE_BYTES, PublicationError};
use df_imagery::{ImageJob, owner_scope_bytes};
use serde_json::{Value, json};
use tokio_postgres::Transaction;

const MAXIMUM_LOCAL_INSPECTIONS: usize = 2;

pub(super) fn encode(values: &[LocalInspectionCheckpoint]) -> Result<String, PublicationError> {
    if values.len() > MAXIMUM_LOCAL_INSPECTIONS {
        return Err(PublicationError::BindingConflict);
    }
    let value=Value::Array(values.iter().map(|value|json!({
        "owner":owner_scope_bytes(&value.scope.owner),"job":value.scope.job.as_bytes(),
        "stage":value.scope.stage.as_bytes(),"kind":value.basis.kind.label(),
        "input_revision":value.basis.input_revision,"input_digest":value.basis.input_digest,
        "config_revision":value.basis.config_revision,"config_digest":value.basis.config_digest,
        "parent":value.basis.parent_dispatch.map(|parent|*parent.as_bytes()),
        "candidate_stage":value.candidate_scope.stage.as_bytes(),"candidate_dispatch":value.candidate_scope.dispatch.as_bytes(),
        "candidate_version":value.candidate_observation_version,"candidate_usage":value.candidate_usage_revision,
        "source_digest":value.source_digest,"reason":value.reason.label(),"observed_at_ms":value.observed_at_ms,
        "schema":value.schema_revision,"validator":value.validator_revision,
    })).collect()).to_string();
    if value.len() > MAXIMUM_RECIPE_BYTES {
        return Err(PublicationError::InvalidRecord);
    }
    Ok(value)
}

pub(super) async fn read_in(
    tx: &Transaction<'_>,
    job: &ImageJob,
    text: &str,
) -> Result<Vec<LocalInspectionCheckpoint>, PublicationError> {
    if text.len() > MAXIMUM_RECIPE_BYTES {
        return Err(PublicationError::InvalidRecord);
    }
    let value: Value = serde_json::from_str(text).map_err(|_| PublicationError::InvalidRecord)?;
    let values = value
        .as_array()
        .filter(|values| values.len() <= MAXIMUM_LOCAL_INSPECTIONS)
        .ok_or(PublicationError::InvalidRecord)?;
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let bytes: Vec<u8> = serde_json::from_value(
            value
                .get("stage")
                .cloned()
                .ok_or(PublicationError::InvalidRecord)?,
        )
        .map_err(|_| PublicationError::InvalidRecord)?;
        let stage =
            ExecutionStageId::from_bytes(&bytes).map_err(|_| PublicationError::InvalidRecord)?;
        let checkpoint =
            crate::imagery_execution::read_local_inspection_in(tx, &job.owner, &job.job, &stage)
                .await
                .map_err(PublicationError::Execution)?
                .ok_or(PublicationError::BindingConflict)?;
        if result
            .iter()
            .any(|prior: &LocalInspectionCheckpoint| prior.scope.stage == stage)
        {
            return Err(PublicationError::BindingConflict);
        }
        result.push(checkpoint);
    }
    let expected: Value =
        serde_json::from_str(&encode(&result)?).map_err(|_| PublicationError::InvalidRecord)?;
    if expected != value {
        return Err(PublicationError::BindingConflict);
    }
    Ok(result)
}

pub(super) async fn verify_in(
    tx: &Transaction<'_>,
    job: &ImageJob,
    values: &[LocalInspectionCheckpoint],
    candidates: &[CandidateReceipt],
) -> Result<(), PublicationError> {
    let exact = read_in(tx, job, &encode(values)?).await?;
    if exact != values
        || values.iter().any(|value| {
            value.scope.owner != job.owner
                || value.scope.job != job.job
                || !candidates.iter().any(|candidate| {
                    candidate.scope == value.candidate_scope
                        && candidate.observation_version == value.candidate_observation_version
                        && candidate.receipt.usage_revision == value.candidate_usage_revision
                })
        })
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(())
}
