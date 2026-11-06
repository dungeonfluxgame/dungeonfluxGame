//! Immutable checkpoint row parsing; result interpretation remains outside the store.
use super::encoding;
use crate::imagery_execution::statements::common::{bytes32, invalid, map_db};
use df_imagery::execution::*;
use df_imagery::{ImageJobId, ImageOwner, VersionedSnapshot, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};

fn kind(row: &Row) -> Result<DecisionKind, ExecutionError> {
    DecisionKind::from_key(&row.get::<_, String>("decision_kind"))
        .ok_or(ExecutionError::InvalidRecord)
}
fn scope(row: &Row, owner: &ImageOwner) -> Result<StageScope, ExecutionError> {
    Ok(StageScope {
        owner: owner.clone(),
        job: invalid(ImageJobId::from_bytes(&row.get::<_, Vec<u8>>("job_id")))?,
        stage: invalid(ExecutionStageId::from_bytes(
            &row.get::<_, Vec<u8>>("stage_id"),
        ))?,
    })
}
fn result(row: &Row) -> Result<VersionedSnapshot, ExecutionError> {
    let bytes: Vec<u8> = row.get("result");
    let length = bytes.len();
    invalid(VersionedSnapshot::new(
        invalid(u32::try_from(row.get::<_, i64>("result_revision")))?,
        bytes,
        length,
    ))
}
pub(super) fn reference(
    row: &Row,
    owner: &ImageOwner,
) -> Result<AnalysisCheckpointReference, ExecutionError> {
    Ok(AnalysisCheckpointReference {
        scope: scope(row, owner)?,
        kind: kind(row)?,
        checkpoint_version: invalid(u32::try_from(row.get::<_, i64>("checkpoint_version")))?,
        schema_revision: invalid(u32::try_from(row.get::<_, i64>("schema_revision")))?,
        validator_revision: invalid(u32::try_from(row.get::<_, i64>("validator_revision")))?,
        result_sha256: bytes32(&row.get::<_, Vec<u8>>("result_sha256"))?,
        intent_digest: bytes32(&row.get::<_, Vec<u8>>("intent_digest"))?,
        config_digest: bytes32(&row.get::<_, Vec<u8>>("config_digest"))?,
        source_digest: bytes32(&row.get::<_, Vec<u8>>("source_digest"))?,
    })
}
pub(super) fn final_checkpoint(
    row: &Row,
    owner: &ImageOwner,
) -> Result<AnalysisCheckpoint, ExecutionError> {
    let r = reference(row, owner)?;
    Ok(AnalysisCheckpoint {
        scope: r.scope,
        kind: r.kind,
        checkpoint_version: r.checkpoint_version,
        schema_revision: r.schema_revision,
        validator_revision: r.validator_revision,
        result: result(row)?,
        result_sha256: r.result_sha256,
        intent_digest: r.intent_digest,
        config_digest: r.config_digest,
        source_digest: r.source_digest,
        stages: encoding::parse_stages(&row.get::<_, String>("stages_text"))?,
        dependencies: encoding::parse_dependencies(&row.get::<_, String>("dependencies_text"))?,
    })
}
pub(super) fn local_reference(
    row: &Row,
    owner: &ImageOwner,
) -> Result<LocalAnalysisReference, ExecutionError> {
    let r = reference(row, owner)?;
    let parent: Option<Vec<u8>> = row.get("parent_dispatch");
    Ok(LocalAnalysisReference {
        scope: r.scope,
        kind: r.kind,
        checkpoint_version: r.checkpoint_version,
        schema_revision: r.schema_revision,
        validator_revision: r.validator_revision,
        basis: StageBasis {
            kind: r.kind.stage_kind(),
            input_revision: invalid(u32::try_from(row.get::<_, i64>("input_revision")))?,
            input_digest: bytes32(&row.get::<_, Vec<u8>>("input_digest"))?,
            config_revision: invalid(u64::try_from(row.get::<_, i64>("config_revision")))?,
            config_digest: r.config_digest,
            parent_dispatch: parent
                .map(|b| invalid(DispatchId::from_bytes(&b)))
                .transpose()?,
        },
        result_sha256: r.result_sha256,
        intent_digest: r.intent_digest,
        config_digest: r.config_digest,
        source_digest: r.source_digest,
    })
}
pub(super) fn local_checkpoint(
    row: &Row,
    owner: &ImageOwner,
) -> Result<LocalAnalysisCheckpoint, ExecutionError> {
    let r = local_reference(row, owner)?;
    Ok(LocalAnalysisCheckpoint {
        scope: r.scope,
        kind: r.kind,
        checkpoint_version: r.checkpoint_version,
        schema_revision: r.schema_revision,
        validator_revision: r.validator_revision,
        basis: r.basis,
        result: result(row)?,
        result_sha256: r.result_sha256,
        intent_digest: r.intent_digest,
        config_digest: r.config_digest,
        source_digest: r.source_digest,
        dependencies: encoding::parse_dependencies(&row.get::<_, String>("dependencies_text"))?,
    })
}
pub(super) async fn read_final(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<AnalysisCheckpoint>, ExecutionError> {
    tx.query_opt(
        "SELECT c.*,c.stages::text AS stages_text,c.dependencies::text AS dependencies_text \
        FROM imagery.image_analysis_checkpoint c WHERE c.owner_scope=$1 AND c.job_id=$2",
        &[&owner_scope_bytes(owner), &job.as_bytes().as_slice()],
    )
    .await
    .map_err(map_db)?
    .map(|r| final_checkpoint(&r, owner))
    .transpose()
}
pub(super) async fn read_local(
    tx: &Transaction<'_>,
    scope: &StageScope,
) -> Result<Option<LocalAnalysisCheckpoint>, ExecutionError> {
    tx.query_opt(
        "SELECT c.*,c.dependencies::text AS dependencies_text FROM imagery.image_local_analysis c \
        WHERE c.owner_scope=$1 AND c.job_id=$2 AND c.stage_id=$3",
        &[
            &owner_scope_bytes(&scope.owner),
            &scope.job.as_bytes().as_slice(),
            &scope.stage.as_bytes().as_slice(),
        ],
    )
    .await
    .map_err(map_db)?
    .map(|r| local_checkpoint(&r, &scope.owner))
    .transpose()
}
