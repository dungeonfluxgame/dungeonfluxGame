//! Immutable scoped terminal receipt encoding, independent of supplier settlement state.
use super::super::rows::database_error;
use super::evidence::RetainedEvidence;
use df_imagery::execution::{
    ExecutionScope, ExecutionStageId, StageScope, TerminalActor, TerminalAllocation,
    TerminalStageSnapshot, TerminalValidation,
};
use df_imagery::publication::{
    NoDeliveryAllocation, NoDeliveryReceipt, NoDeliveryStage, PublicationError,
};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};

fn stage(
    row: &Row,
    owner: &ImageOwner,
    job: &ImageJobId,
    prefix: &str,
) -> Result<NoDeliveryStage, PublicationError> {
    let scope = StageScope {
        owner: owner.clone(),
        job: *job,
        stage: ExecutionStageId::from_bytes(
            &row.get::<_, Vec<u8>>(format!("{prefix}_stage_id").as_str()),
        )
        .map_err(|_| PublicationError::InvalidRecord)?,
    };
    match row
        .get::<_, String>(format!("{prefix}_kind").as_str())
        .as_str()
    {
        "unsent_stage" => Ok(NoDeliveryStage::UnsentStage(scope)),
        "unclaimed_stage" => Ok(NoDeliveryStage::UnclaimedStage(scope)),
        "deferred_stage" => Ok(NoDeliveryStage::DeferredStage(scope)),
        "released_stage" => Ok(NoDeliveryStage::ReleasedStage(scope)),
        "dispatch" => Ok(NoDeliveryStage::Dispatch {
            scope: ExecutionScope {
                owner: scope.owner,
                job: scope.job,
                stage: scope.stage,
                dispatch: df_imagery::execution::DispatchId::from_bytes(
                    &row.get::<_, Option<Vec<u8>>>(format!("{prefix}_dispatch_id").as_str())
                        .ok_or(PublicationError::InvalidRecord)?,
                )
                .map_err(|_| PublicationError::InvalidRecord)?,
            },
            usage_revision: u32::try_from(
                row.get::<_, Option<i32>>(format!("{prefix}_usage_revision").as_str())
                    .ok_or(PublicationError::InvalidRecord)?,
            )
            .map_err(|_| PublicationError::InvalidRecord)?,
        }),
        _ => Err(PublicationError::InvalidRecord),
    }
}

fn allocation(row: &Row) -> Result<NoDeliveryAllocation, PublicationError> {
    match row.get::<_, String>("allocation_kind").as_str() {
        "no_allocation" => Ok(NoDeliveryAllocation::NoAllocation),
        "existing" => Ok(NoDeliveryAllocation::Existing {
            stage: ExecutionStageId::from_bytes(
                &row.get::<_, Option<Vec<u8>>>("allocation_stage_id")
                    .ok_or(PublicationError::InvalidRecord)?,
            )
            .map_err(|_| PublicationError::InvalidRecord)?,
            asset: df_assets::AssetId::from_bytes(
                &row.get::<_, Option<Vec<u8>>>("asset_id")
                    .ok_or(PublicationError::InvalidRecord)?,
            )
            .map_err(|_| PublicationError::InvalidRecord)?,
            attempt: df_assets::AttemptId::from_bytes(
                &row.get::<_, Option<Vec<u8>>>("publication_attempt")
                    .ok_or(PublicationError::InvalidRecord)?,
            )
            .map_err(|_| PublicationError::InvalidRecord)?,
        }),
        _ => Err(PublicationError::InvalidRecord),
    }
}

pub(super) async fn read_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<(NoDeliveryReceipt, Vec<u8>)>, PublicationError> {
    let Some(row) = tx
        .query_opt(
            "SELECT * FROM imagery.image_no_delivery WHERE owner_scope=$1 AND job_id=$2",
            &[&owner_scope_bytes(owner), &job.as_bytes().as_slice()],
        )
        .await
        .map_err(database_error)?
    else {
        return Ok(None);
    };
    Ok(Some((
        parse(&row, owner, job)?,
        row.get("evidence_binding"),
    )))
}

pub(super) async fn read_receipt_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<NoDeliveryReceipt>, PublicationError> {
    let row=tx.query_opt("SELECT actor_kind,actor_stage_id,actor_dispatch_id,actor_usage_revision,tail_kind,tail_stage_id,tail_dispatch_id,tail_usage_revision,candidate_stage_id,candidate_dispatch_id,allocation_kind,allocation_stage_id,asset_id,publication_attempt,reason,expected_job_revision,sealed_job_revision,sealed_at_ms FROM imagery.image_no_delivery WHERE owner_scope=$1 AND job_id=$2", &[&owner_scope_bytes(owner),&job.as_bytes().as_slice()]).await.map_err(database_error)?;
    row.map(|row| parse(&row, owner, job)).transpose()
}

fn parse(
    row: &Row,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<NoDeliveryReceipt, PublicationError> {
    let candidate = match (
        row.get::<_, Option<Vec<u8>>>("candidate_stage_id"),
        row.get::<_, Option<Vec<u8>>>("candidate_dispatch_id"),
    ) {
        (Some(stage), Some(dispatch)) => Some(ExecutionScope {
            owner: owner.clone(),
            job: *job,
            stage: ExecutionStageId::from_bytes(&stage)
                .map_err(|_| PublicationError::InvalidRecord)?,
            dispatch: df_imagery::execution::DispatchId::from_bytes(&dispatch)
                .map_err(|_| PublicationError::InvalidRecord)?,
        }),
        (None, None) => None,
        _ => return Err(PublicationError::InvalidRecord),
    };
    let receipt = NoDeliveryReceipt {
        owner: owner.clone(),
        job: *job,
        actor: stage(row, owner, job, "actor")?,
        tail: stage(row, owner, job, "tail")?,
        candidate,
        allocation: allocation(row)?,
        reason: df_imagery::runtime::NoDeliveryReason::parse_label(&row.get::<_, String>("reason"))
            .ok_or(PublicationError::InvalidRecord)?,
        expected_job_revision: u64::try_from(row.get::<_, i64>("expected_job_revision"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        job_revision: u64::try_from(row.get::<_, i64>("sealed_job_revision"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        sealed_at_ms: row.get("sealed_at_ms"),
    };
    Ok(receipt)
}

struct StageFields {
    kind: &'static str,
    stage: Vec<u8>,
    dispatch: Option<Vec<u8>>,
    usage: Option<i32>,
}
fn fields(
    stage: &TerminalStageSnapshot,
    actor: &TerminalActor,
) -> Result<StageFields, PublicationError> {
    let (kind, dispatch, usage) = match (actor, &stage.dispatch) {
        (TerminalActor::Dispatch(_), Some(dispatch)) => (
            "dispatch",
            Some(dispatch.dispatch.as_bytes().to_vec()),
            Some(
                i32::try_from(dispatch.usage_revision)
                    .map_err(|_| PublicationError::InvalidRecord)?,
            ),
        ),
        (TerminalActor::UnsentStage(_), None) => ("unsent_stage", None, None),
        (TerminalActor::UnclaimedStage(_), None) => ("unclaimed_stage", None, None),
        (TerminalActor::DeferredStage(_), None) => ("deferred_stage", None, None),
        (TerminalActor::ReleasedStage(_), None) => ("released_stage", None, None),
        _ => return Err(PublicationError::BindingConflict),
    };
    Ok(StageFields {
        kind,
        stage: stage.stage.stage.as_bytes().to_vec(),
        dispatch,
        usage,
    })
}

pub(super) async fn insert_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::TerminalCommand,
    validation: &TerminalValidation,
    retained: &RetainedEvidence,
    binding: &[u8],
) -> Result<NoDeliveryReceipt, PublicationError> {
    let scope = command.actor.scope();
    let actor = fields(&validation.actor, &command.actor)?;
    let tail = fields(&validation.tail, &command.tail)?;
    let (allocation_kind, allocation_stage, asset, attempt) = match validation.allocation {
        TerminalAllocation::NoAllocation => ("no_allocation", None, None, None),
        TerminalAllocation::Existing {
            stage,
            asset,
            attempt,
        } => (
            "existing",
            Some(stage.as_bytes().to_vec()),
            Some(asset.to_vec()),
            Some(attempt.to_vec()),
        ),
    };
    let candidate_stage = validation
        .candidate
        .as_ref()
        .map(|candidate| candidate.scope.stage.as_bytes().to_vec());
    let candidate_dispatch = validation
        .candidate
        .as_ref()
        .map(|candidate| candidate.scope.dispatch.as_bytes().to_vec());
    let expected = i64::try_from(command.expected_job_revision)
        .map_err(|_| PublicationError::InvalidRecord)?;
    let revision = expected
        .checked_add(1)
        .ok_or(PublicationError::InvalidRecord)?;
    let decisions = super::super::decision_binding::encode(&retained.decisions)?;
    let candidates = super::candidates::encode(&retained.candidates)?;
    let locals = super::super::local_inspections::encode(&retained.locals)?;
    let no_output_raw = retained.no_output_raw_sha256.map(|value| value.to_vec());
    let legacy_write = retained
        .legacy_output
        .as_ref()
        .map(|output| output.write.as_bytes().to_vec());
    let legacy_version = retained
        .legacy_output
        .as_ref()
        .map(|output| output.version.as_str());
    let legacy_sha = retained
        .legacy_output
        .as_ref()
        .map(|output| output.sha256.to_vec());
    let legacy_recipe = retained
        .legacy_output
        .as_ref()
        .map(|output| output.recipe_digest.to_vec());
    tx.execute("INSERT INTO imagery.image_no_delivery(job_id,owner_scope,actor_kind,actor_stage_id,actor_dispatch_id,actor_usage_revision,tail_kind,tail_stage_id,tail_dispatch_id,tail_usage_revision,candidate_stage_id,candidate_dispatch_id,allocation_kind,allocation_stage_id,asset_id,publication_attempt,expected_job_revision,sealed_job_revision,reason,evidence_binding,decision_binding,candidates_binding,local_inspection_binding,sealed_at_ms,no_output_raw_sha256,legacy_output_write_id,legacy_output_version,legacy_output_sha256,legacy_output_recipe_digest) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29)", &[
        &scope.job.as_bytes().as_slice(),&owner_scope_bytes(&scope.owner),&actor.kind,&actor.stage,&actor.dispatch,&actor.usage,&tail.kind,&tail.stage,&tail.dispatch,&tail.usage,
        &candidate_stage,&candidate_dispatch,&allocation_kind,&allocation_stage,&asset,&attempt,&expected,&revision,&retained.reason.label(),&binding,&decisions,&candidates,&locals,&command.now_ms,&no_output_raw,&legacy_write,&legacy_version,&legacy_sha,&legacy_recipe,
    ]).await.map_err(database_error)?;
    read_in(tx, &scope.owner, &scope.job)
        .await?
        .map(|(receipt, _)| receipt)
        .ok_or(PublicationError::InvalidRecord)
}
