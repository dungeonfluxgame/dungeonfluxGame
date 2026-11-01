//! Ready-backed contribution evidence; finance retains actual raw validity and liability authority.

use super::{decision_binding, rows::database_error, selection};
use df_imagery::execution::{DispatchId, ExecutionStageId, StageKind};
use df_imagery::publication::PublicationError;
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::Transaction;

/// An explicit economic disposition of one exact durable usage revision, without money mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageEconomicUse {
    SelectedCandidate {
        usage_revision: u32,
    },
    SupersededCandidate {
        usage_revision: u32,
    },
    UndeliveredCandidate {
        usage_revision: u32,
    },
    NoOutput {
        usage_revision: u32,
    },
    /// A useful decision remains subject to its actual raw response validity, including Unjudged.
    Decision {
        usage_revision: u32,
    },
}

impl StageEconomicUse {
    pub fn usage_revision(self) -> u32 {
        match self {
            Self::SelectedCandidate { usage_revision }
            | Self::SupersededCandidate { usage_revision }
            | Self::UndeliveredCandidate { usage_revision }
            | Self::NoOutput { usage_revision }
            | Self::Decision { usage_revision } => usage_revision,
        }
    }
}

/// Reads immutable actual Ready lineage without acquiring catalog or execution row locks.
/// The caller holds the exact job lock after its finance and raw-usage locks; absence is no proof.
pub async fn stage_economic_use_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<StageEconomicUse>, PublicationError> {
    super::rows::require_read_committed(tx).await?;
    if let Some(usage_revision) =
        super::no_delivery::no_output_use_in(tx, owner, job, stage, dispatch).await?
    {
        return Ok(Some(StageEconomicUse::NoOutput { usage_revision }));
    }
    if let Some(usage_revision) =
        super::no_delivery::candidate_use_in(tx, owner, job, stage, dispatch).await?
    {
        return Ok(Some(StageEconomicUse::UndeliveredCandidate {
            usage_revision,
        }));
    }
    if let Some(usage_revision) =
        super::no_delivery::legacy_output_use_in(tx, owner, job, stage, dispatch).await?
    {
        return Ok(Some(StageEconomicUse::UndeliveredCandidate {
            usage_revision,
        }));
    }
    let Some(ready) = tx.query_opt(
        "SELECT r.stage_id,r.dispatch_id,r.private_reconciliation,r.published_at_ms,s.candidate_stage_id,s.candidate_dispatch_id, \
        s.candidate_binding,s.decision_binding,s.resolved_recipe_digest, \
        EXISTS(SELECT 1 FROM imagery.image_final_selection x WHERE x.job_id=r.job_id) AS has_selection \
        FROM imagery.image_job_result r \
        JOIN media.asset a ON a.asset_id=r.asset_id AND a.attempt=r.publication_attempt \
        LEFT JOIN imagery.image_final_selection s ON s.job_id=r.job_id AND s.owner_scope=r.owner_scope \
        AND s.asset_id=r.asset_id AND s.publication_attempt=r.publication_attempt \
        AND s.final_stage_id=r.stage_id AND s.final_dispatch_id=r.dispatch_id \
        AND s.allocation_stage_id=r.allocation_stage_id AND s.resolved_recipe_digest=r.recipe_digest \
        WHERE r.owner_scope=$1 AND r.job_id=$2 AND a.state='ready' AND a.kind='image' \
        AND a.provenance='generated'",
        &[&owner_scope_bytes(owner),&job.as_bytes().as_slice()],
    ).await.map_err(database_error)? else { return Ok(None) };
    let canonical = crate::read_in(tx, owner, *job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    if ready.get::<_, bool>("has_selection")
        && ready
            .get::<_, Option<Vec<u8>>>("resolved_recipe_digest")
            .is_none()
    {
        return Err(PublicationError::BindingConflict);
    }
    let private = ready.get::<_, bool>("private_reconciliation");
    if private
        && canonical.state != df_imagery::JobState::Retired
        && canonical.absolute_deadline_ms > ready.get::<_, i64>("published_at_ms")
    {
        return Err(PublicationError::BindingConflict);
    }
    let candidate_stage: Option<Vec<u8>> = ready.get("candidate_stage_id");
    let candidate_dispatch: Option<Vec<u8>> = ready.get("candidate_dispatch_id");
    let (Some(candidate_stage), Some(candidate_dispatch)) = (candidate_stage, candidate_dispatch)
    else {
        if ready
            .get::<_, Option<Vec<u8>>>("resolved_recipe_digest")
            .is_some()
        {
            return Ok(None);
        }
        if df_imagery::runtime::requires_final_selection(&canonical)
            .map_err(|_| PublicationError::InvalidRecord)?
        {
            return Err(PublicationError::BindingConflict);
        }
        return legacy_ready_in(tx, owner, job, stage, dispatch, &ready).await;
    };
    let selected = crate::imagery_execution::read_candidate_reference_in(
        tx,
        owner,
        job,
        &ExecutionStageId::from_bytes(&candidate_stage)
            .map_err(|_| PublicationError::InvalidRecord)?,
        &DispatchId::from_bytes(&candidate_dispatch)
            .map_err(|_| PublicationError::InvalidRecord)?,
    )
    .await
    .map_err(PublicationError::Execution)?
    .ok_or(PublicationError::BindingConflict)?;
    if ready
        .get::<_, Option<Vec<u8>>>("candidate_binding")
        .as_deref()
        != Some(selection::candidate_binding(&selected)?.as_slice())
    {
        return Err(PublicationError::BindingConflict);
    }
    let refs = decision_binding::read_refs_snapshot_in(
        tx,
        &canonical,
        &ready.get::<_, String>("decision_binding"),
    )
    .await?;
    let candidates = decision_binding::verify_snapshot_in(tx, &canonical, &selected, &refs).await?;
    let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.scope.stage == *stage && candidate.scope.dispatch == *dispatch)
    else {
        return Ok(None);
    };
    let usage_revision = candidate.receipt.usage_revision;
    if private {
        Ok(Some(StageEconomicUse::UndeliveredCandidate {
            usage_revision,
        }))
    } else if matches!(
        candidate.basis.kind,
        StageKind::Generation | StageKind::Edit
    ) && candidates
        .iter()
        .any(|candidate| candidate.basis.kind == StageKind::Correction)
    {
        Ok(Some(StageEconomicUse::SupersededCandidate {
            usage_revision,
        }))
    } else {
        Ok(Some(StageEconomicUse::SelectedCandidate { usage_revision }))
    }
}

/// Matches an outside-transaction semantic proof to the exact current decision and raw revision.
/// The caller holds its finance/raw/job locks; this function fetches no private raw or payload blob.
pub async fn decision_economic_use_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
    proof: &df_imagery::runtime::QualifiedDecision,
) -> Result<Option<StageEconomicUse>, PublicationError> {
    super::rows::require_read_committed(tx).await?;
    let Some(reference) = crate::imagery_execution::read_decision_reference_unlocked_in(
        tx, owner, job, stage, dispatch,
    )
    .await
    .map_err(PublicationError::Execution)?
    else {
        return Ok(None);
    };
    let source = if reference.kind == df_imagery::execution::DecisionKind::Inspection {
        let parent = reference
            .basis
            .stage
            .parent_dispatch
            .ok_or(PublicationError::BindingConflict)?;
        let view = crate::imagery_execution::read_dispatch_in(tx, owner, &parent)
            .await
            .map_err(PublicationError::Execution)?;
        if view.job != *job {
            return Err(PublicationError::BindingConflict);
        }
        Some(
            crate::imagery_execution::read_candidate_reference_in(
                tx,
                owner,
                job,
                &view.stage,
                &parent,
            )
            .await
            .map_err(PublicationError::Execution)?
            .ok_or(PublicationError::BindingConflict)?,
        )
    } else {
        None
    };
    let reference = decision_binding::reference(reference);
    // 0040 generates these SHA-256 columns from the same locked canonical snapshot bytes.
    let snapshot=tx.query_opt("SELECT intent_digest,config_digest FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2", &[&owner_scope_bytes(owner),&job.as_bytes().as_slice()]).await.map_err(database_error)?.ok_or(PublicationError::NotFound)?;
    let intent_digest: [u8; 32] = snapshot
        .get::<_, Vec<u8>>("intent_digest")
        .try_into()
        .map_err(|_| PublicationError::InvalidRecord)?;
    let config_digest: [u8; 32] = snapshot
        .get::<_, Vec<u8>>("config_digest")
        .try_into()
        .map_err(|_| PublicationError::InvalidRecord)?;
    if !proof.matches_metadata(
        owner,
        job,
        &intent_digest,
        &config_digest,
        &reference,
        source.as_ref(),
    ) {
        return Err(PublicationError::BindingConflict);
    }
    let raw=tx.query_opt("SELECT raw_output_sha256 FROM imagery.image_dispatch_usage WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND dispatch_id=$4 AND revision=$5", &[&owner_scope_bytes(owner),&job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&dispatch.as_bytes().as_slice(),&i64::from(reference.usage_revision)]).await.map_err(database_error)?.ok_or(PublicationError::BindingConflict)?;
    if raw.get::<_, Option<Vec<u8>>>(0).as_deref() != Some(proof.raw_sha256().as_slice()) {
        return Err(PublicationError::BindingConflict);
    }
    Ok(Some(StageEconomicUse::Decision {
        usage_revision: reference.usage_revision,
    }))
}

async fn legacy_ready_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
    ready: &tokio_postgres::Row,
) -> Result<Option<StageEconomicUse>, PublicationError> {
    if ready.get::<_, Vec<u8>>("stage_id") != stage.as_bytes()
        || ready.get::<_, Vec<u8>>("dispatch_id") != dispatch.as_bytes()
    {
        return Ok(None);
    }
    let row=tx.query_opt("SELECT d.usage_revision FROM imagery.image_dispatch_attempt d \
        JOIN imagery.image_execution_stage s USING(job_id,stage_id) \
        JOIN imagery.image_stage_cursor c ON c.job_id=d.job_id AND c.stage_id=d.stage_id AND c.current_dispatch=d.dispatch_id \
        JOIN imagery.image_dispatch_checkpoint p ON p.dispatch_id=d.dispatch_id AND p.usage_revision=d.usage_revision \
        WHERE s.owner_scope=$1 AND d.job_id=$2 AND d.stage_id=$3 AND d.dispatch_id=$4 \
        AND d.dispatch_state='succeeded' AND s.stage_kind IN ('generation','edit','correction','extraction')",
        &[&owner_scope_bytes(owner),&job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&dispatch.as_bytes().as_slice()])
        .await.map_err(database_error)?;
    row.map(|row| {
        let usage_revision =
            u32::try_from(row.get::<_, i32>(0)).map_err(|_| PublicationError::InvalidRecord)?;
        Ok(if ready.get::<_, bool>("private_reconciliation") {
            StageEconomicUse::UndeliveredCandidate { usage_revision }
        } else {
            StageEconomicUse::SelectedCandidate { usage_revision }
        })
    })
    .transpose()
}
