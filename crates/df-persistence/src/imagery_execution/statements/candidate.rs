//! Intermediate image observations bound to exact immutable raw supplier bytes and usage.
//! The protected domain observation has measured pixels; storage verifies raw-byte identity.
use super::common::{
    alpha_of, bump_job_revision, bytes32, invalid, locked_dispatch, locked_job, map_db,
    read_dispatch_view, require_read_committed,
};
use df_imagery::execution::{
    CandidateDescriptor, CandidateObservation, CandidateReceipt, CompleteCommand, DispatchId,
    DispatchView, ExecutionError, ExecutionScope, ExecutionStageId, ObservationReceipt,
};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};

fn parse(row: &Row, scope: ExecutionScope) -> Result<CandidateReceipt, ExecutionError> {
    Ok(CandidateReceipt {
        scope,
        basis: super::decision::parse_basis(row)?,
        observation_version: invalid(u32::try_from(row.get::<_, i64>("observation_version")))?,
        receipt: ObservationReceipt {
            usage_revision: invalid(u32::try_from(row.get::<_, i32>("usage_revision")))?,
            observed_at_ms: row.get("observed_at_ms"),
        },
        descriptor: CandidateDescriptor {
            mime: row.get("mime"),
            width_px: invalid(u32::try_from(row.get::<_, i32>("width_px")))?,
            height_px: invalid(u32::try_from(row.get::<_, i32>("height_px")))?,
            alpha: alpha_of(&row.get::<_, String>("alpha"))?,
            byte_len: invalid(u64::try_from(row.get::<_, i64>("byte_len")))?,
            sha256: bytes32(&row.get::<_, Vec<u8>>("sha256"))?,
            validator_revision: invalid(u32::try_from(row.get::<_, i64>("validator_revision")))?,
        },
    })
}
async fn read(
    tx: &Transaction<'_>,
    scope: ExecutionScope,
    version: Option<u32>,
) -> Result<Option<CandidateReceipt>, ExecutionError> {
    let version = version.map(i64::from);
    let row=tx.query_opt("SELECT o.*,o.candidate_kind AS stage_kind FROM imagery.image_candidate_observation o \
        WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND dispatch_id=$4 \
        AND ($5::bigint IS NULL OR observation_version=$5) ORDER BY observation_version DESC LIMIT 1",
        &[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),
        &scope.dispatch.as_bytes().as_slice(),&version]).await.map_err(map_db)?;
    row.map(|row| parse(&row, scope)).transpose()
}
/// Reads exact or latest scoped measured candidate evidence; no final publication is implied.
pub(crate) async fn read_candidate_observation_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
    version: Option<u32>,
) -> Result<Option<CandidateReceipt>, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    locked_dispatch(tx, job, stage, dispatch).await?;
    read(
        tx,
        ExecutionScope {
            owner: owner.clone(),
            job: *job,
            stage: *stage,
            dispatch: *dispatch,
        },
        version,
    )
    .await
}
/// Reads the exact winning intermediate candidate for publication/source-lineage verification.
pub(crate) async fn read_candidate_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<CandidateReceipt>, ExecutionError> {
    require_read_committed(tx).await?;
    locked_job(tx, owner, job).await?;
    locked_dispatch(tx, job, stage, dispatch).await?;
    let selected=tx.query_opt("SELECT observation_version,usage_revision FROM imagery.image_candidate_checkpoint WHERE dispatch_id=$1",
        &[&dispatch.as_bytes().as_slice()]).await.map_err(map_db)?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let result = read(
        tx,
        ExecutionScope {
            owner: owner.clone(),
            job: *job,
            stage: *stage,
            dispatch: *dispatch,
        },
        Some(invalid(u32::try_from(
            selected.get::<_, i64>("observation_version"),
        ))?),
    )
    .await?;
    if result.as_ref().is_some_and(|receipt| {
        i64::from(receipt.receipt.usage_revision)
            != i64::from(selected.get::<_, i32>("usage_revision"))
    }) {
        return Err(ExecutionError::InvalidRecord);
    }
    Ok(result)
}
/// Retains protected measured metadata only after exact raw supplier bytes are durable.
pub(crate) async fn record_candidate_observation_in(
    tx: &Transaction<'_>,
    observation: &CandidateObservation,
) -> Result<CandidateReceipt, ExecutionError> {
    require_read_committed(tx).await?;
    let observed = observation.as_receipt();
    let scope = &observed.scope;
    locked_job(tx, &scope.owner, &scope.job).await?;
    let dispatch = locked_dispatch(tx, &scope.job, &scope.stage, &scope.dispatch).await?;
    if let Some(stored) = read(tx, scope.clone(), Some(observed.observation_version)).await? {
        return if stored == *observed {
            Ok(stored)
        } else {
            Err(ExecutionError::OperationConflict)
        };
    }
    let basis =
        super::decision::read_stage_basis_in(tx, &scope.owner, &scope.job, &scope.stage).await?;
    if basis != observed.basis {
        return Err(ExecutionError::BasisConflict);
    }
    if !matches!(
        dispatch.get::<_, String>("dispatch_state").as_str(),
        "sent" | "unknown" | "succeeded" | "failed"
    ) {
        return Err(ExecutionError::Superseded);
    }
    let d = &observed.descriptor;
    let matches = crate::imagery_runtime::usage_output_matches_in(
        tx,
        scope,
        df_imagery::cost::UsageRevision(u64::from(observed.receipt.usage_revision)),
        d.byte_len,
        &d.sha256,
    )
    .await
    .map_err(|_| {
        df_observe::record_audit_line("IMAGERY", "IMAGERY candidate_usage_read_failed".to_owned());
        ExecutionError::Unavailable
    })?;
    if !matches {
        return Err(ExecutionError::ObservationMissing);
    }
    tx.execute("INSERT INTO imagery.image_candidate_observation(dispatch_id,observation_version,owner_scope, \
        job_id,stage_id,candidate_kind,input_revision,input_digest,config_revision,config_digest,parent_dispatch, \
        mime,width_px,height_px,alpha,byte_len,sha256,validator_revision,usage_revision,observed_at_ms) \
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)",
        &[&scope.dispatch.as_bytes().as_slice(),&i64::from(observed.observation_version),&owner_scope_bytes(&scope.owner),
        &scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),&basis.kind.key(),&i64::from(basis.input_revision),
        &basis.input_digest.as_slice(),&invalid(i64::try_from(basis.config_revision))?,&basis.config_digest.as_slice(),
        &basis.parent_dispatch.map(|parent|parent.as_bytes().to_vec()),&d.mime,&invalid(i32::try_from(d.width_px))?,
        &invalid(i32::try_from(d.height_px))?,&d.alpha.key(),&invalid(i64::try_from(d.byte_len))?,&d.sha256.as_slice(),
        &i64::from(d.validator_revision),&invalid(i32::try_from(observed.receipt.usage_revision))?,&observed.receipt.observed_at_ms])
        .await.map_err(map_db)?;
    bump_job_revision(tx, &scope.job, observed.receipt.observed_at_ms).await?;
    Ok(observed.clone())
}
/// Selects prior real-byte evidence on the original dispatch, never inventing an object checkpoint.
pub(crate) async fn complete_candidate_in(
    tx: &Transaction<'_>,
    command: &CompleteCommand,
    observation_version: u32,
    usage_revision: u32,
) -> Result<DispatchView, ExecutionError> {
    let observed = read(
        tx,
        ExecutionScope {
            owner: command.owner.clone(),
            job: command.job,
            stage: command.stage,
            dispatch: command.dispatch,
        },
        Some(observation_version),
    )
    .await?
    .ok_or(ExecutionError::ObservationMissing)?;
    if observed.receipt.usage_revision != usage_revision {
        return Err(ExecutionError::OperationConflict);
    }
    let existing=tx.query_opt("SELECT observation_version,usage_revision FROM imagery.image_candidate_checkpoint WHERE dispatch_id=$1",
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
    tx.execute("INSERT INTO imagery.image_candidate_checkpoint(dispatch_id,observation_version,usage_revision,recorded_at_ms) \
        VALUES($1,$2,$3,$4)",&[&command.dispatch.as_bytes().as_slice(),&i64::from(observation_version),
        &invalid(i32::try_from(usage_revision))?,&command.now_ms]).await.map_err(map_db)?;
    tx.execute("UPDATE imagery.image_dispatch_attempt SET dispatch_state='succeeded',failure_cause=NULL,usage_revision=$2, \
        updated_at_ms=$3 WHERE dispatch_id=$1",&[&command.dispatch.as_bytes().as_slice(),
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

/// Caller holds the exact owned job lock; this snapshot read acquires no execution row locks.
pub(crate) async fn read_candidate_reference_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<CandidateReceipt>, ExecutionError> {
    require_read_committed(tx).await?;
    let row=tx.query_opt("SELECT o.*,o.candidate_kind AS stage_kind FROM imagery.image_candidate_checkpoint p \
        JOIN imagery.image_candidate_observation o USING(dispatch_id,observation_version) \
        JOIN imagery.image_stage_cursor c ON c.current_dispatch=o.dispatch_id AND c.job_id=o.job_id AND c.stage_id=o.stage_id \
        JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=o.dispatch_id \
        WHERE o.owner_scope=$1 AND o.job_id=$2 AND o.stage_id=$3 AND o.dispatch_id=$4 \
        AND d.dispatch_state='succeeded' AND d.usage_revision=o.usage_revision AND p.usage_revision=o.usage_revision",
        &[&owner_scope_bytes(owner),&job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&dispatch.as_bytes().as_slice()])
        .await.map_err(map_db)?;
    row.map(|row| {
        parse(
            &row,
            ExecutionScope {
                owner: owner.clone(),
                job: *job,
                stage: *stage,
                dispatch: *dispatch,
            },
        )
    })
    .transpose()
}
