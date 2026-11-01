//! Canonical private metadata encoding for immutable analytical provenance.
use df_imagery::execution::*;
use df_imagery::{ImageJobId, OperationHandle, owner_of_bytes, owner_scope_bytes};
use serde_json::{Value, json};

fn scope(scope: &StageScope) -> Value {
    json!({"owner":owner_scope_bytes(&scope.owner),"job":scope.job.as_bytes(),"stage":scope.stage.as_bytes()})
}
fn basis(basis: &StageBasis) -> Value {
    json!({"kind":basis.kind.label(),"input_revision":basis.input_revision,"input_digest":basis.input_digest,
        "config_revision":basis.config_revision,"config_digest":basis.config_digest,
        "parent":basis.parent_dispatch.map(|id|*id.as_bytes())})
}
pub(super) fn checkpoint_reference(reference: &AnalysisCheckpointReference) -> Value {
    json!({"scope":scope(&reference.scope),"kind":reference.kind.label(),"version":reference.checkpoint_version,
        "schema":reference.schema_revision,"validator":reference.validator_revision,"result":reference.result_sha256,
        "intent":reference.intent_digest,"config":reference.config_digest,"sources":reference.source_digest})
}
pub(super) fn dependencies(values: &[AnalysisDependencyRef]) -> String {
    Value::Array(values.iter().map(|value|json!({"owner":owner_scope_bytes(&value.handle.owner),
        "operation":value.handle.operation.as_bytes(),"checkpoint":checkpoint_reference(&value.checkpoint)})).collect()).to_string()
}
fn local(reference: &LocalAnalysisReference) -> Value {
    json!({"scope":scope(&reference.scope),"kind":reference.kind.label(),"version":reference.checkpoint_version,
        "schema":reference.schema_revision,"validator":reference.validator_revision,"basis":basis(&reference.basis),
        "result":reference.result_sha256,"intent":reference.intent_digest,"config":reference.config_digest,
        "sources":reference.source_digest})
}
fn decision(reference: &DecisionReference) -> Value {
    let stage = StageScope {
        owner: reference.scope.owner.clone(),
        job: reference.scope.job,
        stage: reference.scope.stage,
    };
    json!({"scope":scope(&stage),"dispatch":reference.scope.dispatch.as_bytes(),"kind":reference.kind.label(),
        "version":reference.observation_version,"schema":reference.basis.schema_revision,
        "validator":reference.basis.validator_revision,"basis":basis(&reference.basis.stage),
        "usage":reference.receipt.usage_revision,"observed":reference.receipt.observed_at_ms,
        "payload":reference.payload_sha256})
}
pub(super) fn stages(values: &[AnalysisStageReference]) -> String {
    Value::Array(
        values
            .iter()
            .map(|value| match value {
                AnalysisStageReference::Supplier(reference) => {
                    json!({"supplier":decision(reference)})
                }
                AnalysisStageReference::Local(reference) => json!({"local":local(reference)}),
            })
            .collect(),
    )
    .to_string()
}
fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, ExecutionError> {
    value.get(key).ok_or(ExecutionError::InvalidRecord)
}
fn bytes<const N: usize>(value: &Value, key: &str) -> Result<[u8; N], ExecutionError> {
    let bytes: Vec<u8> = serde_json::from_value(field(value, key)?.clone())
        .map_err(|_| ExecutionError::InvalidRecord)?;
    bytes.try_into().map_err(|_| ExecutionError::InvalidRecord)
}
fn number(value: &Value, key: &str) -> Result<u64, ExecutionError> {
    field(value, key)?
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or(ExecutionError::InvalidRecord)
}
fn revision(value: &Value, key: &str) -> Result<u32, ExecutionError> {
    u32::try_from(number(value, key)?).map_err(|_| ExecutionError::InvalidRecord)
}
fn kind(value: &Value) -> Result<DecisionKind, ExecutionError> {
    field(value, "kind")?
        .as_str()
        .and_then(DecisionKind::parse_label)
        .ok_or(ExecutionError::InvalidRecord)
}
fn parse_scope(value: &Value) -> Result<StageScope, ExecutionError> {
    let owner: Vec<u8> = serde_json::from_value(field(value, "owner")?.clone())
        .map_err(|_| ExecutionError::InvalidRecord)?;
    Ok(StageScope {
        owner: owner_of_bytes(&owner).ok_or(ExecutionError::InvalidRecord)?,
        job: ImageJobId::from_bytes(&bytes::<16>(value, "job")?)
            .map_err(|_| ExecutionError::InvalidRecord)?,
        stage: ExecutionStageId::from_bytes(&bytes::<16>(value, "stage")?)
            .map_err(|_| ExecutionError::InvalidRecord)?,
    })
}
fn parse_basis(value: &Value) -> Result<StageBasis, ExecutionError> {
    let parent = field(value, "parent")?;
    Ok(StageBasis {
        kind: field(value, "kind")?
            .as_str()
            .and_then(StageKind::parse_label)
            .ok_or(ExecutionError::InvalidRecord)?,
        input_revision: revision(value, "input_revision")?,
        input_digest: bytes(value, "input_digest")?,
        config_revision: number(value, "config_revision")?,
        config_digest: bytes(value, "config_digest")?,
        parent_dispatch: if parent.is_null() {
            None
        } else {
            let data: Vec<u8> = serde_json::from_value(parent.clone())
                .map_err(|_| ExecutionError::InvalidRecord)?;
            Some(DispatchId::from_bytes(&data).map_err(|_| ExecutionError::InvalidRecord)?)
        },
    })
}
fn parse_checkpoint_reference(
    value: &Value,
) -> Result<AnalysisCheckpointReference, ExecutionError> {
    Ok(AnalysisCheckpointReference {
        scope: parse_scope(field(value, "scope")?)?,
        kind: kind(value)?,
        checkpoint_version: revision(value, "version")?,
        schema_revision: revision(value, "schema")?,
        validator_revision: revision(value, "validator")?,
        result_sha256: bytes(value, "result")?,
        intent_digest: bytes(value, "intent")?,
        config_digest: bytes(value, "config")?,
        source_digest: bytes(value, "sources")?,
    })
}
pub(super) fn parse_dependencies(text: &str) -> Result<Vec<AnalysisDependencyRef>, ExecutionError> {
    let value: Value = serde_json::from_str(text).map_err(|_| ExecutionError::InvalidRecord)?;
    value
        .as_array()
        .ok_or(ExecutionError::InvalidRecord)?
        .iter()
        .map(|value| {
            let owner: Vec<u8> = serde_json::from_value(field(value, "owner")?.clone())
                .map_err(|_| ExecutionError::InvalidRecord)?;
            Ok(AnalysisDependencyRef {
                handle: OperationHandle {
                    owner: owner_of_bytes(&owner).ok_or(ExecutionError::InvalidRecord)?,
                    operation: df_types::OperationId::from_bytes(&bytes::<16>(value, "operation")?)
                        .map_err(|_| ExecutionError::InvalidRecord)?,
                },
                checkpoint: parse_checkpoint_reference(field(value, "checkpoint")?)?,
            })
        })
        .collect()
}
fn parse_local(value: &Value) -> Result<LocalAnalysisReference, ExecutionError> {
    Ok(LocalAnalysisReference {
        scope: parse_scope(field(value, "scope")?)?,
        kind: kind(value)?,
        checkpoint_version: revision(value, "version")?,
        schema_revision: revision(value, "schema")?,
        validator_revision: revision(value, "validator")?,
        basis: parse_basis(field(value, "basis")?)?,
        result_sha256: bytes(value, "result")?,
        intent_digest: bytes(value, "intent")?,
        config_digest: bytes(value, "config")?,
        source_digest: bytes(value, "sources")?,
    })
}
fn parse_decision(value: &Value) -> Result<DecisionReference, ExecutionError> {
    let scope = parse_scope(field(value, "scope")?)?;
    Ok(DecisionReference {
        scope: ExecutionScope {
            owner: scope.owner,
            job: scope.job,
            stage: scope.stage,
            dispatch: DispatchId::from_bytes(&bytes::<16>(value, "dispatch")?)
                .map_err(|_| ExecutionError::InvalidRecord)?,
        },
        kind: kind(value)?,
        observation_version: revision(value, "version")?,
        basis: DecisionBasis {
            schema_revision: revision(value, "schema")?,
            validator_revision: revision(value, "validator")?,
            stage: parse_basis(field(value, "basis")?)?,
        },
        receipt: ObservationReceipt {
            usage_revision: revision(value, "usage")?,
            observed_at_ms: i64::try_from(number(value, "observed")?)
                .map_err(|_| ExecutionError::InvalidRecord)?,
        },
        payload_sha256: bytes(value, "payload")?,
    })
}
pub(super) fn parse_stages(text: &str) -> Result<Vec<AnalysisStageReference>, ExecutionError> {
    let value: Value = serde_json::from_str(text).map_err(|_| ExecutionError::InvalidRecord)?;
    value
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or(ExecutionError::InvalidRecord)?
        .iter()
        .map(|value| {
            if value.as_object().map(|o| o.len()) != Some(1) {
                return Err(ExecutionError::InvalidRecord);
            }
            if let Some(local) = value.get("local") {
                parse_local(local).map(AnalysisStageReference::Local)
            } else {
                parse_decision(field(value, "supplier")?).map(AnalysisStageReference::Supplier)
            }
        })
        .collect()
}
