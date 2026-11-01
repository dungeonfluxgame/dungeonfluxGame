//! Exact immutable decision references and the bounded publication pipeline ancestor graph.
use super::rows::database_error;
use df_imagery::execution::{
    CandidateReceipt, DecisionKind, DecisionReference, DispatchId, StageKind,
};
use df_imagery::publication::{FinalDecisionRef, MAXIMUM_RECIPE_BYTES, PublicationError};
use df_imagery::{ImageJob, owner_scope_bytes};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use tokio_postgres::Transaction;

// Compatibility pipeline v1 has one classification/image/correction/extraction and two inspections.
const MAXIMUM_PIPELINE_ANCESTORS: usize = 6;

pub(super) fn reference(value: DecisionReference) -> FinalDecisionRef {
    FinalDecisionRef {
        scope: value.scope,
        kind: value.kind,
        observation_version: value.observation_version,
        usage_revision: value.receipt.usage_revision,
        basis: value.basis,
        payload_sha256: value.payload_sha256,
    }
}

pub(super) fn encode(refs: &[FinalDecisionRef]) -> Result<String, PublicationError> {
    if refs.len() > MAXIMUM_PIPELINE_ANCESTORS {
        return Err(PublicationError::BindingConflict);
    }
    let value=Value::Array(refs.iter().map(|value|json!({
        "owner":owner_scope_bytes(&value.scope.owner),"job":value.scope.job.as_bytes(),
        "stage":value.scope.stage.as_bytes(),"dispatch":value.scope.dispatch.as_bytes(),
        "kind":value.kind.label(),"version":value.observation_version,"usage":value.usage_revision,
        "schema":value.basis.schema_revision,"validator":value.basis.validator_revision,
        "input_revision":value.basis.stage.input_revision,"input_digest":value.basis.stage.input_digest,
        "config_revision":value.basis.stage.config_revision,"config_digest":value.basis.stage.config_digest,
        "parent":value.basis.stage.parent_dispatch.map(|parent|*parent.as_bytes()),
        "payload_sha256":value.payload_sha256,
    })).collect()).to_string();
    if value.len() > MAXIMUM_RECIPE_BYTES {
        return Err(PublicationError::InvalidRecipe);
    }
    Ok(value)
}

pub(super) async fn read_refs_in(
    tx: &Transaction<'_>,
    job: &ImageJob,
    text: &str,
) -> Result<Vec<FinalDecisionRef>, PublicationError> {
    read_refs(tx, job, text, ReadAuthority::Locked).await
}

pub(super) async fn read_refs_snapshot_in(
    tx: &Transaction<'_>,
    job: &ImageJob,
    text: &str,
) -> Result<Vec<FinalDecisionRef>, PublicationError> {
    read_refs(tx, job, text, ReadAuthority::Snapshot).await
}

#[derive(Clone, Copy)]
enum ReadAuthority {
    Locked,
    Snapshot,
}

async fn read_decision(
    tx: &Transaction<'_>,
    job: &ImageJob,
    stage: &df_imagery::execution::ExecutionStageId,
    dispatch: &DispatchId,
    authority: ReadAuthority,
) -> Result<Option<DecisionReference>, PublicationError> {
    match authority {
        ReadAuthority::Locked => {
            crate::imagery_execution::read_decision_reference_in(
                tx, &job.owner, &job.job, stage, dispatch,
            )
            .await
        }
        ReadAuthority::Snapshot => {
            crate::imagery_execution::read_decision_reference_unlocked_in(
                tx, &job.owner, &job.job, stage, dispatch,
            )
            .await
        }
    }
    .map_err(PublicationError::Execution)
}

async fn read_refs(
    tx: &Transaction<'_>,
    job: &ImageJob,
    text: &str,
    authority: ReadAuthority,
) -> Result<Vec<FinalDecisionRef>, PublicationError> {
    if text.len() > MAXIMUM_RECIPE_BYTES {
        return Err(PublicationError::InvalidRecipe);
    }
    let value: Value = serde_json::from_str(text).map_err(|_| PublicationError::InvalidRecord)?;
    let values = value
        .as_array()
        .filter(|values| values.len() <= MAXIMUM_PIPELINE_ANCESTORS)
        .ok_or(PublicationError::InvalidRecord)?;
    let mut refs = Vec::with_capacity(values.len());
    for value in values {
        let bytes: Vec<u8> = serde_json::from_value(
            value
                .get("dispatch")
                .cloned()
                .ok_or(PublicationError::InvalidRecord)?,
        )
        .map_err(|_| PublicationError::InvalidRecord)?;
        let dispatch =
            DispatchId::from_bytes(&bytes).map_err(|_| PublicationError::InvalidRecord)?;
        let stored = crate::imagery_execution::read_dispatch_in(tx, &job.owner, &dispatch)
            .await
            .map_err(PublicationError::Execution)?;
        if stored.job != job.job {
            return Err(PublicationError::BindingConflict);
        }
        let actual = read_decision(tx, job, &stored.stage, &dispatch, authority)
            .await?
            .ok_or(PublicationError::BindingConflict)?;
        refs.push(reference(actual));
    }
    let expected: Value =
        serde_json::from_str(&encode(&refs)?).map_err(|_| PublicationError::InvalidRecord)?;
    if expected != value {
        return Err(PublicationError::BindingConflict);
    }
    Ok(refs)
}

#[derive(Clone)]
enum Ancestor {
    Candidate(CandidateReceipt),
    Decision(FinalDecisionRef),
}
impl Ancestor {
    fn basis(&self) -> &df_imagery::execution::StageBasis {
        match self {
            Self::Candidate(candidate) => &candidate.basis,
            Self::Decision(decision) => &decision.basis.stage,
        }
    }
}

async fn read_ancestor(
    tx: &Transaction<'_>,
    job: &ImageJob,
    dispatch: DispatchId,
    refs: &[FinalDecisionRef],
    authority: ReadAuthority,
) -> Result<Ancestor, PublicationError> {
    let view = crate::imagery_execution::read_dispatch_in(tx, &job.owner, &dispatch)
        .await
        .map_err(PublicationError::Execution)?;
    if view.job != job.job || view.state != df_imagery::execution::DispatchState::Succeeded {
        return Err(PublicationError::BindingConflict);
    }
    let query = match authority {
        ReadAuthority::Locked => {
            "SELECT current_dispatch FROM imagery.image_stage_cursor WHERE job_id=$1 AND stage_id=$2 FOR UPDATE"
        }
        ReadAuthority::Snapshot => {
            "SELECT current_dispatch FROM imagery.image_stage_cursor WHERE job_id=$1 AND stage_id=$2"
        }
    };
    let current = tx
        .query_one(
            query,
            &[
                &job.job.as_bytes().as_slice(),
                &view.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(database_error)?
        .get::<_, Vec<u8>>(0);
    if current != dispatch.as_bytes() {
        return Err(PublicationError::BindingConflict);
    }
    let kind = match authority {
        ReadAuthority::Locked => {
            crate::imagery_execution::read_stage_basis_in(tx, &job.owner, &job.job, &view.stage)
                .await
                .map_err(PublicationError::Execution)?
                .kind
        }
        ReadAuthority::Snapshot => {
            let row = tx.query_one("SELECT stage_kind FROM imagery.image_execution_stage WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3", &[&owner_scope_bytes(&job.owner),&job.job.as_bytes().as_slice(),&view.stage.as_bytes().as_slice()]).await.map_err(database_error)?;
            StageKind::parse_label(&row.get::<_, String>(0))
                .ok_or(PublicationError::InvalidRecord)?
        }
    };
    match kind {
        StageKind::Generation | StageKind::Edit | StageKind::Correction | StageKind::Extraction => {
            let candidate = match authority {
                ReadAuthority::Locked => {
                    crate::imagery_execution::read_candidate_checkpoint_in(
                        tx,
                        &job.owner,
                        &job.job,
                        &view.stage,
                        &dispatch,
                    )
                    .await
                }
                ReadAuthority::Snapshot => {
                    crate::imagery_execution::read_candidate_reference_in(
                        tx,
                        &job.owner,
                        &job.job,
                        &view.stage,
                        &dispatch,
                    )
                    .await
                }
            }
            .map_err(PublicationError::Execution)?
            .ok_or(PublicationError::BindingConflict)?;
            Ok(Ancestor::Candidate(candidate))
        }
        StageKind::Classification | StageKind::Inspection => {
            let decision = reference(
                read_decision(tx, job, &view.stage, &dispatch, authority)
                    .await?
                    .ok_or(PublicationError::BindingConflict)?,
            );
            if !refs.contains(&decision) {
                return Err(PublicationError::BindingConflict);
            }
            Ok(Ancestor::Decision(decision))
        }
        StageKind::LocalTransform
        | StageKind::PhotoInspection
        | StageKind::HeadLocations
        | StageKind::Segmentation
        | StageKind::StagingIdentity
        | StageKind::StagingFacing => Err(PublicationError::BindingConflict),
    }
}

pub(super) async fn verify_in(
    tx: &Transaction<'_>,
    job: &ImageJob,
    selected: &CandidateReceipt,
    refs: &[FinalDecisionRef],
) -> Result<Vec<CandidateReceipt>, PublicationError> {
    verify(tx, job, selected, refs, ReadAuthority::Locked).await
}

pub(super) async fn verify_snapshot_in(
    tx: &Transaction<'_>,
    job: &ImageJob,
    selected: &CandidateReceipt,
    refs: &[FinalDecisionRef],
) -> Result<Vec<CandidateReceipt>, PublicationError> {
    verify(tx, job, selected, refs, ReadAuthority::Snapshot).await
}

async fn verify(
    tx: &Transaction<'_>,
    job: &ImageJob,
    selected: &CandidateReceipt,
    refs: &[FinalDecisionRef],
    authority: ReadAuthority,
) -> Result<Vec<CandidateReceipt>, PublicationError> {
    encode(refs)?;
    let declared = refs
        .iter()
        .map(|value| value.scope.dispatch)
        .collect::<BTreeSet<_>>();
    if declared.len() != refs.len()
        || refs
            .iter()
            .any(|value| value.scope.owner != job.owner || value.scope.job != job.job)
    {
        return Err(PublicationError::BindingConflict);
    }
    let mut nodes = BTreeMap::<DispatchId, Ancestor>::new();
    let mut selected_chain = BTreeSet::new();
    for (index, root) in std::iter::once(selected.scope.dispatch)
        .chain(refs.iter().map(|value| value.scope.dispatch))
        .enumerate()
    {
        let mut path = BTreeSet::new();
        let mut current = Some(root);
        while let Some(dispatch) = current {
            if !path.insert(dispatch) {
                return Err(PublicationError::BindingConflict);
            }
            if !nodes.contains_key(&dispatch) {
                if nodes.len() == MAXIMUM_PIPELINE_ANCESTORS {
                    return Err(PublicationError::BindingConflict);
                }
                nodes.insert(
                    dispatch,
                    read_ancestor(tx, job, dispatch, refs, authority).await?,
                );
            }
            let node = nodes
                .get(&dispatch)
                .ok_or(PublicationError::InvalidRecord)?;
            current = node.basis().parent_dispatch;
        }
        if index == 0 {
            selected_chain = path;
        } else if path.is_disjoint(&selected_chain) {
            return Err(PublicationError::BindingConflict);
        }
    }
    match nodes.get(&selected.scope.dispatch) {
        Some(Ancestor::Candidate(actual)) if actual == selected => {}
        _ => return Err(PublicationError::BindingConflict),
    }
    let mut sources = BTreeMap::new();
    let mut image_count = 0;
    let mut classification_count = 0;
    let mut inspection_count = 0;
    let mut correction_count = 0;
    let mut extraction_count = 0;
    for node in nodes.values() {
        match node.basis().kind {
            StageKind::Generation | StageKind::Edit => image_count += 1,
            StageKind::Classification => classification_count += 1,
            StageKind::Inspection => inspection_count += 1,
            StageKind::Correction => correction_count += 1,
            StageKind::Extraction => extraction_count += 1,
            StageKind::LocalTransform
            | StageKind::PhotoInspection
            | StageKind::HeadLocations
            | StageKind::Segmentation
            | StageKind::StagingIdentity
            | StageKind::StagingFacing => return Err(PublicationError::BindingConflict),
        }
        if let Ancestor::Candidate(candidate) = node {
            sources.insert(candidate.scope.dispatch, candidate.clone());
        }
        if let Ancestor::Decision(decision) = node
            && decision.kind == DecisionKind::Inspection
        {
            let parent = decision
                .basis
                .stage
                .parent_dispatch
                .ok_or(PublicationError::BindingConflict)?;
            let Some(Ancestor::Candidate(_)) = nodes.get(&parent) else {
                return Err(PublicationError::BindingConflict);
            };
        }
    }
    if image_count != 1
        || classification_count > 1
        || inspection_count > 2
        || correction_count > 1
        || extraction_count > 1
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(sources.into_values().collect())
}
