//! Scoped measured candidate witnesses in an immutable negative delivery seal.
use super::super::{rows::database_error, selection};
use df_imagery::execution::{CandidateReceipt, DispatchId, ExecutionStageId};
use df_imagery::publication::{MAXIMUM_RECIPE_BYTES, PublicationError};
use df_imagery::{ImageJobId, ImageOwner};
use serde_json::{Value, json};
use tokio_postgres::Transaction;

const MAXIMUM_PIPELINE_CANDIDATES: usize = 3;

pub(super) fn encode(candidates: &[CandidateReceipt]) -> Result<String, PublicationError> {
    if candidates.len() > MAXIMUM_PIPELINE_CANDIDATES {
        return Err(PublicationError::InvalidRecord);
    }
    let value=candidates.iter().map(|candidate|Ok(json!({
        "stage":candidate.scope.stage.as_bytes(),"dispatch":candidate.scope.dispatch.as_bytes(),
        "binding":selection::candidate_binding(candidate)?,
    }))).collect::<Result<Vec<Value>,PublicationError>>()?;
    Ok(Value::Array(value).to_string())
}

pub(super) async fn read_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    text: &str,
) -> Result<Vec<CandidateReceipt>, PublicationError> {
    if text.len() > MAXIMUM_RECIPE_BYTES {
        return Err(PublicationError::InvalidRecord);
    }
    let value: Value = serde_json::from_str(text).map_err(|_| PublicationError::InvalidRecord)?;
    let values = value
        .as_array()
        .filter(|values| values.len() <= MAXIMUM_PIPELINE_CANDIDATES)
        .ok_or(PublicationError::InvalidRecord)?;
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let stage: Vec<u8> = serde_json::from_value(
            value
                .get("stage")
                .cloned()
                .ok_or(PublicationError::InvalidRecord)?,
        )
        .map_err(|_| PublicationError::InvalidRecord)?;
        let dispatch: Vec<u8> = serde_json::from_value(
            value
                .get("dispatch")
                .cloned()
                .ok_or(PublicationError::InvalidRecord)?,
        )
        .map_err(|_| PublicationError::InvalidRecord)?;
        let candidate = crate::imagery_execution::read_candidate_reference_in(
            tx,
            owner,
            job,
            &ExecutionStageId::from_bytes(&stage).map_err(|_| PublicationError::InvalidRecord)?,
            &DispatchId::from_bytes(&dispatch).map_err(|_| PublicationError::InvalidRecord)?,
        )
        .await
        .map_err(PublicationError::Execution)?
        .ok_or(PublicationError::BindingConflict)?;
        if result
            .iter()
            .any(|prior: &CandidateReceipt| prior.scope.dispatch == candidate.scope.dispatch)
        {
            return Err(PublicationError::BindingConflict);
        }
        result.push(candidate);
    }
    let expected: Value =
        serde_json::from_str(&encode(&result)?).map_err(|_| PublicationError::InvalidRecord)?;
    if expected != value {
        return Err(PublicationError::BindingConflict);
    }
    Ok(result)
}

pub(super) async fn stored_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<Vec<CandidateReceipt>>, PublicationError> {
    let row=tx.query_opt("SELECT candidates_binding FROM imagery.image_no_delivery WHERE owner_scope=$1 AND job_id=$2", &[&df_imagery::owner_scope_bytes(owner),&job.as_bytes().as_slice()]).await.map_err(database_error)?;
    match row {
        Some(row) => read_in(tx, owner, job, &row.get::<_, String>(0))
            .await
            .map(Some),
        None => Ok(None),
    }
}
