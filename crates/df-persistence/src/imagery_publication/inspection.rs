//! Winning inspection metadata comes from the actual immutable Ready selection, never a new judge.
use super::{rows, selection};
use df_imagery::execution::{
    DecisionKind, DecisionObservation, LocalInspectionCheckpoint, StageKind,
};
use df_imagery::publication::PublicationError;
use df_imagery::{ImageJob, ImageJobId, ImageOwner};
use tokio_postgres::Transaction;

#[derive(Clone, Debug)]
pub enum PublishedInspectionEvidence {
    Decision(DecisionObservation),
    Local(LocalInspectionCheckpoint),
}

/// Private journal input for the authorized service's bounded pure assessment projection.
#[derive(Clone, Debug)]
pub struct PublishedInspection {
    pub job: ImageJob,
    pub evidence: PublishedInspectionEvidence,
}

/// Reads only the inspection actually selected by nonprivate Ready; caller authorizes current access.
pub async fn read_published_inspection_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<PublishedInspection>, PublicationError> {
    let Some((result, _)) = rows::stored_result(tx, owner, job).await? else {
        return Ok(None);
    };
    if result.private_reconciliation {
        return Ok(None);
    }
    let Some(selected) = selection::read_in(tx, owner, job).await? else {
        return Ok(None);
    };
    if selected.final_stage != result.stage || selected.final_dispatch != result.dispatch {
        return Err(PublicationError::BindingConflict);
    }
    let candidate = selected
        .candidate
        .as_ref()
        .ok_or(PublicationError::BindingConflict)?;
    let references = selected
        .decisions
        .iter()
        .filter(|reference| {
            reference.kind == DecisionKind::Inspection
                && (reference.basis.stage.parent_dispatch == Some(candidate.scope.dispatch)
                    || candidate.basis.kind == StageKind::Extraction
                        && candidate.basis.parent_dispatch == Some(reference.scope.dispatch))
        })
        .collect::<Vec<_>>();
    let evidence = match references.as_slice() {
        [reference] => {
            let observation = crate::imagery_execution::read_decision_checkpoint_in(
                tx,
                owner,
                job,
                &reference.scope.stage,
                &reference.scope.dispatch,
            )
            .await
            .map_err(PublicationError::Execution)?
            .ok_or(PublicationError::BindingConflict)?;
            if df_imagery::publication::FinalDecisionRef::from_observation(&observation)
                != **reference
            {
                return Err(PublicationError::BindingConflict);
            }
            PublishedInspectionEvidence::Decision(observation)
        }
        [] => {
            let locals = selected
                .local_inspections
                .iter()
                .filter(|local| {
                    local.candidate_scope == candidate.scope
                        || candidate.basis.kind == StageKind::Extraction
                            && candidate.basis.parent_dispatch
                                == Some(local.candidate_scope.dispatch)
                })
                .collect::<Vec<_>>();
            match locals.as_slice() {
                [local] => PublishedInspectionEvidence::Local((**local).clone()),
                [] => return Ok(None),
                _ => return Err(PublicationError::BindingConflict),
            }
        }
        _ => return Err(PublicationError::BindingConflict),
    };
    let job = crate::read_in(tx, owner, *job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    Ok(Some(PublishedInspection { job, evidence }))
}
