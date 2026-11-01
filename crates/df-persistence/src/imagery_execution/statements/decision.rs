//! Durable private decision observations and exact winning version selection.
//! Raw usage precedes selection; this store verifies lineage, never semantic approval.

use super::common::{
    bump_job_revision, bytes32, check_owner, invalid, locked_dispatch, locked_job, map_db,
    read_dispatch_view, require_read_committed, stage_kind_of,
};
use df_imagery::execution::{
    CompleteCommand, DecisionBasis, DecisionKind, DecisionObservation, DecisionReceipt,
    DecisionScope, DispatchId, DispatchView, ExecutionError, ExecutionStageId,
    MAXIMUM_SQL_DECISION_PAYLOAD_BYTES, StageBasis,
};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};

pub(super) fn parse_basis(row: &Row) -> Result<StageBasis, ExecutionError> {
    let parent: Option<Vec<u8>> = row.get("parent_dispatch");
    Ok(StageBasis {
        kind: stage_kind_of(&row.get::<_, String>("stage_kind"))?,
        input_revision: invalid(u32::try_from(row.get::<_, i64>("input_revision")))?,
        input_digest: bytes32(&row.get::<_, Vec<u8>>("input_digest"))?,
        config_revision: invalid(u64::try_from(row.get::<_, i64>("config_revision")))?,
        config_digest: bytes32(&row.get::<_, Vec<u8>>("config_digest"))?,
        parent_dispatch: parent
            .map(|bytes| invalid(DispatchId::from_bytes(&bytes)))
            .transpose()?,
    })
}

/// Reads immutable canonical stage/dependency basis under its owned job lock.
pub(crate) async fn read_stage_basis_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<StageBasis, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    let row=tx.query_opt("SELECT owner_scope,stage_kind,input_revision,input_digest,config_revision, \
        config_digest,parent_dispatch FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_id=$2",
        &[&job.as_bytes().as_slice(),&stage.as_bytes().as_slice()]).await.map_err(map_db)?
        .ok_or(ExecutionError::NotFound)?;
    check_owner(&row.get::<_, Vec<u8>>("owner_scope"), owner)?;
    parse_basis(&row)
}

fn parse_observation(
    row: &Row,
    scope: DecisionScope,
) -> Result<DecisionObservation, ExecutionError> {
    let kind = DecisionKind::parse_label(&row.get::<_, String>("decision_kind"))
        .ok_or(ExecutionError::InvalidRecord)?;
    let stage = parse_basis(row)?;
    DecisionObservation::new(
        scope,
        kind,
        invalid(u32::try_from(row.get::<_, i64>("observation_version")))?,
        DecisionBasis {
            schema_revision: invalid(u32::try_from(row.get::<_, i64>("schema_revision")))?,
            validator_revision: invalid(u32::try_from(row.get::<_, i64>("validator_revision")))?,
            stage,
        },
        row.get("payload"),
        DecisionReceipt {
            usage_revision: invalid(u32::try_from(row.get::<_, i32>("usage_revision")))?,
            observed_at_ms: row.get("observed_at_ms"),
        },
        invalid(usize::try_from(MAXIMUM_SQL_DECISION_PAYLOAD_BYTES))?,
    )
    .map_err(|_| ExecutionError::InvalidRecord)
}

async fn read_observation(
    tx: &Transaction<'_>,
    scope: DecisionScope,
    version: Option<u32>,
) -> Result<Option<DecisionObservation>, ExecutionError> {
    let version = version.map(i64::from);
    let row=tx.query_opt("SELECT o.*,o.decision_kind AS stage_kind FROM imagery.image_decision_observation o \
        WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND dispatch_id=$4 \
        AND ($5::bigint IS NULL OR observation_version=$5) ORDER BY observation_version DESC LIMIT 1",
        &[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),
        &scope.dispatch.as_bytes().as_slice(),&version]).await.map_err(map_db)?;
    row.map(|row| parse_observation(&row, scope)).transpose()
}

/// Reads exact versioned source evidence without treating it as a completed or approved decision.
pub(crate) async fn read_decision_observation_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
    version: Option<u32>,
) -> Result<Option<DecisionObservation>, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    locked_dispatch(tx, job, stage, dispatch).await?;
    read_observation(
        tx,
        DecisionScope {
            owner: owner.clone(),
            job: *job,
            stage: *stage,
            dispatch: *dispatch,
        },
        version,
    )
    .await
}

/// Reads only the immutable terminal selection; a raw observation alone is never success.
pub(crate) async fn read_decision_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<DecisionObservation>, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    locked_dispatch(tx, job, stage, dispatch).await?;
    let selected=tx.query_opt("SELECT observation_version FROM imagery.image_decision_checkpoint WHERE dispatch_id=$1",
        &[&dispatch.as_bytes().as_slice()]).await.map_err(map_db)?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    read_observation(
        tx,
        DecisionScope {
            owner: owner.clone(),
            job: *job,
            stage: *stage,
            dispatch: *dispatch,
        },
        Some(invalid(u32::try_from(
            selected.get::<_, i64>("observation_version"),
        ))?),
    )
    .await
}

/// Retains an exact observation after the raw usage receipt, with first-write immutable replay.
pub(crate) async fn record_decision_observation_in(
    tx: &Transaction<'_>,
    observation: &DecisionObservation,
) -> Result<DecisionObservation, ExecutionError> {
    require_read_committed(tx).await?;
    let scope = &observation.scope;
    locked_job(tx, &scope.owner, &scope.job).await?;
    let dispatch = locked_dispatch(tx, &scope.job, &scope.stage, &scope.dispatch).await?;
    if let Some(stored) =
        read_observation(tx, scope.clone(), Some(observation.observation_version)).await?
    {
        return if stored == *observation {
            Ok(stored)
        } else {
            Err(ExecutionError::OperationConflict)
        };
    }
    let stage = read_stage_basis_in(tx, &scope.owner, &scope.job, &scope.stage).await?;
    if stage != observation.basis.stage || stage.kind != observation.kind.stage_kind() {
        return Err(ExecutionError::BasisConflict);
    }
    let state: String = dispatch.get("dispatch_state");
    if !matches!(state.as_str(), "sent" | "unknown" | "succeeded" | "failed") {
        return Err(ExecutionError::Superseded);
    }
    let raw = crate::imagery_runtime::usage_recorded_in(
        tx,
        &scope.owner,
        &scope.job,
        &scope.stage,
        &scope.dispatch,
        df_imagery::cost::UsageRevision(u64::from(observation.receipt.usage_revision)),
    )
    .await
    .map_err(|_| {
        df_observe::record_audit_line("IMAGERY", "IMAGERY decision_usage_read_failed".to_owned());
        ExecutionError::Unavailable
    })?;
    if !raw {
        return Err(ExecutionError::ObservationMissing);
    }
    let bound:i64=tx.query_one("SELECT c.decision_payload_bytes_bound FROM imagery.image_execution_capacity c \
        JOIN imagery.image_execution_stage s ON s.capacity_scope=c.capacity_scope WHERE s.job_id=$1 AND s.stage_id=$2",
        &[&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice()]).await.map_err(map_db)?.get(0);
    if observation.payload().len() > invalid(usize::try_from(bound))? {
        return Err(ExecutionError::DecisionPayloadTooLarge {
            maximum_bytes: invalid(u64::try_from(bound))?,
        });
    }
    tx.execute("INSERT INTO imagery.image_decision_observation(dispatch_id,observation_version,owner_scope, \
        job_id,stage_id,decision_kind,schema_revision,validator_revision,input_revision,input_digest, \
        config_revision,config_digest,parent_dispatch,payload,usage_revision,observed_at_ms,payload_sha256) \
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
        &[&scope.dispatch.as_bytes().as_slice(),&i64::from(observation.observation_version),&owner_scope_bytes(&scope.owner),
        &scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),&observation.kind.label(),
        &i64::from(observation.basis.schema_revision),&i64::from(observation.basis.validator_revision),
        &i64::from(stage.input_revision),&stage.input_digest.as_slice(),&invalid(i64::try_from(stage.config_revision))?,
        &stage.config_digest.as_slice(),&stage.parent_dispatch.map(|parent|parent.as_bytes().to_vec()),
        &observation.payload(),&invalid(i32::try_from(observation.receipt.usage_revision))?,&observation.receipt.observed_at_ms,
        &observation.payload_sha256().as_slice()])
        .await.map_err(map_db)?;
    bump_job_revision(tx, &scope.job, observation.receipt.observed_at_ms).await?;
    Ok(observation.clone())
}

/// Selects previously retained evidence, preserving the original dispatch and any unknown liability.
pub(crate) async fn complete_decision_in(
    tx: &Transaction<'_>,
    command: &CompleteCommand,
    observation_version: u32,
    usage_revision: u32,
) -> Result<DispatchView, ExecutionError> {
    let observation = read_observation(
        tx,
        DecisionScope {
            owner: command.owner.clone(),
            job: command.job,
            stage: command.stage,
            dispatch: command.dispatch,
        },
        Some(observation_version),
    )
    .await?
    .ok_or(ExecutionError::ObservationMissing)?;
    if observation.receipt.usage_revision != usage_revision {
        return Err(ExecutionError::OperationConflict);
    }
    let existing=tx.query_opt("SELECT observation_version,usage_revision FROM imagery.image_decision_checkpoint WHERE dispatch_id=$1",
        &[&command.dispatch.as_bytes().as_slice()]).await.map_err(map_db)?;
    if let Some(existing) = existing {
        return if existing.get::<_, i64>("observation_version") == i64::from(observation_version)
            && existing.get::<_, i32>("usage_revision") == invalid(i32::try_from(usage_revision))?
        {
            read_dispatch_view(tx, &command.owner, &command.dispatch).await
        } else {
            Err(ExecutionError::OperationConflict)
        };
    }
    let state = locked_dispatch(tx, &command.job, &command.stage, &command.dispatch)
        .await?
        .get::<_, String>("dispatch_state");
    if !matches!(state.as_str(), "sent" | "unknown") {
        return Err(ExecutionError::OperationConflict);
    }
    tx.execute("INSERT INTO imagery.image_decision_checkpoint(dispatch_id,observation_version,usage_revision,recorded_at_ms) \
        VALUES($1,$2,$3,$4)", &[&command.dispatch.as_bytes().as_slice(),&i64::from(observation_version),
        &invalid(i32::try_from(usage_revision))?,&command.now_ms]).await.map_err(map_db)?;
    tx.execute("UPDATE imagery.image_dispatch_attempt SET dispatch_state='succeeded',failure_cause=NULL,usage_revision=$2, \
        updated_at_ms=$3 WHERE dispatch_id=$1", &[&command.dispatch.as_bytes().as_slice(),
        &invalid(i32::try_from(usage_revision))?,&command.now_ms]).await.map_err(map_db)?;
    super::dispatch::maybe_release_reserve(
        tx,
        &command.job,
        &command.stage,
        df_imagery::execution::DispatchState::Succeeded,
        command.now_ms,
    )
    .await?;
    bump_job_revision(tx, &command.job, command.now_ms).await?;
    read_dispatch_view(tx, &command.owner, &command.dispatch).await
}

/// Reads selected immutable metadata only, checking the exact successful current stage cursor.
pub(crate) async fn read_decision_reference_unlocked_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<df_imagery::execution::DecisionReference>, ExecutionError> {
    require_read_committed(tx).await?;
    let row=tx.query_opt("SELECT o.decision_kind,o.decision_kind AS stage_kind,o.observation_version, \
        o.schema_revision,o.validator_revision,o.input_revision,o.input_digest,o.config_revision, \
        o.config_digest,o.parent_dispatch,o.usage_revision,o.observed_at_ms,o.payload_sha256 \
        FROM imagery.image_decision_checkpoint p \
        JOIN imagery.image_decision_observation o USING(dispatch_id,observation_version) \
        JOIN imagery.image_stage_cursor c ON c.current_dispatch=o.dispatch_id AND c.job_id=o.job_id AND c.stage_id=o.stage_id \
        JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=o.dispatch_id \
        WHERE o.owner_scope=$1 AND o.job_id=$2 AND o.stage_id=$3 AND o.dispatch_id=$4 \
        AND d.dispatch_state='succeeded' AND d.usage_revision=o.usage_revision AND p.usage_revision=o.usage_revision",
        &[&owner_scope_bytes(owner),&job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&dispatch.as_bytes().as_slice()])
        .await.map_err(map_db)?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(df_imagery::execution::DecisionReference {
        scope: DecisionScope {
            owner: owner.clone(),
            job: *job,
            stage: *stage,
            dispatch: *dispatch,
        },
        kind: DecisionKind::parse_label(&row.get::<_, String>("decision_kind"))
            .ok_or(ExecutionError::InvalidRecord)?,
        observation_version: invalid(u32::try_from(row.get::<_, i64>("observation_version")))?,
        basis: DecisionBasis {
            schema_revision: invalid(u32::try_from(row.get::<_, i64>("schema_revision")))?,
            validator_revision: invalid(u32::try_from(row.get::<_, i64>("validator_revision")))?,
            stage: parse_basis(&row)?,
        },
        receipt: DecisionReceipt {
            usage_revision: invalid(u32::try_from(row.get::<_, i32>("usage_revision")))?,
            observed_at_ms: row.get("observed_at_ms"),
        },
        payload_sha256: bytes32(&row.get::<_, Vec<u8>>("payload_sha256"))?,
    }))
}

pub(crate) async fn read_decision_reference_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<df_imagery::execution::DecisionReference>, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    locked_dispatch(tx, job, stage, dispatch).await?;
    read_decision_reference_unlocked_in(tx, owner, job, stage, dispatch).await
}
