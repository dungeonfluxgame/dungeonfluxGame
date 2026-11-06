//! Protected cause matching and complete immutable terminal command binding.
use super::super::{decision_binding, local_inspections, selection};
use df_imagery::execution::{
    CandidateReceipt, DecisionKind, TerminalActor, TerminalAuthority, TerminalCommand,
    TerminalValidation,
};
use df_imagery::publication::{
    FinalDecisionRef, MAXIMUM_RECIPE_BYTES, NoDeliveryCause, NoDeliveryRequest, PublicationError,
};
use df_imagery::runtime::NoDeliveryReason;
use df_imagery::{ImageJob, owner_scope_bytes};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use tokio_postgres::Transaction;

pub(super) struct RetainedEvidence {
    pub reason: NoDeliveryReason,
    pub decisions: Vec<FinalDecisionRef>,
    pub candidates: Vec<CandidateReceipt>,
    pub locals: Vec<df_imagery::execution::LocalInspectionCheckpoint>,
    pub no_output_raw_sha256: Option<[u8; 32]>,
    pub legacy_output: Option<LegacyOutput>,
}

pub(super) struct LegacyOutput {
    pub write: df_assets::WriteId,
    pub version: df_assets::ObjectVersion,
    pub sha256: [u8; 32],
    pub recipe_digest: [u8; 32],
}

fn actor_value(actor: &TerminalActor) -> Value {
    let scope = actor.scope();
    let (kind, dispatch, claim, generation) = match actor {
        TerminalActor::Dispatch(scope) => {
            ("dispatch", Some(*scope.dispatch.as_bytes()), None, None)
        }
        TerminalActor::UnsentStage(claim) => (
            "unsent_stage",
            None,
            Some(*claim.token.as_bytes()),
            Some(claim.generation),
        ),
        TerminalActor::UnclaimedStage(_) => ("unclaimed_stage", None, None, None),
        TerminalActor::DeferredStage(_) => ("deferred_stage", None, None, None),
        TerminalActor::ReleasedStage(_) => ("released_stage", None, None, None),
    };
    json!({"kind":kind,"owner":owner_scope_bytes(&scope.owner),"job":scope.job.as_bytes(),"stage":scope.stage.as_bytes(),"dispatch":dispatch,"claim":claim,"generation":generation})
}

pub(super) fn command_value(command: &TerminalCommand, job: &ImageJob) -> Value {
    json!({"actor":actor_value(&command.actor),"tail":actor_value(&command.tail),"authority":authority_value(&command.authority),
        "expected_revision":command.expected_job_revision,"candidate_dispatch":command.candidate_dispatch.map(|value|*value.as_bytes()),
        "intent_revision":job.intent.revision(),"intent_digest":<[u8;32]>::from(Sha256::digest(job.intent.bytes())),
        "config_revision":job.config.revision(),"config_digest":<[u8;32]>::from(Sha256::digest(job.config.bytes())),
        "recipe_digest":job.recipe.as_ref().map(df_imagery::publication::recipe_digest)})
}

fn authority_value(authority: &TerminalAuthority) -> Value {
    match authority {
        TerminalAuthority::Claim(claim) => {
            json!({"kind":"claim","owner":owner_scope_bytes(&claim.scope.owner),"job":claim.scope.job.as_bytes(),"stage":claim.scope.stage.as_bytes(),"token":claim.token.as_bytes(),"generation":claim.generation})
        }
        TerminalAuthority::Recovery(lease) => {
            json!({"kind":"recovery","owner":owner_scope_bytes(&lease.stage.owner),"job":lease.stage.job.as_bytes(),"stage":lease.stage.stage.as_bytes(),"dispatch":lease.stage.current.dispatch.as_bytes(),"token":lease.token.as_bytes(),"generation":lease.generation})
        }
        TerminalAuthority::Unclaimed(proof) => {
            json!({"kind":"unclaimed_stage","owner":owner_scope_bytes(&proof.scope.owner),"job":proof.scope.job.as_bytes(),"stage":proof.scope.stage.as_bytes(),"revision":proof.job_revision})
        }
        TerminalAuthority::Deferred(proof) => {
            json!({"kind":"deferred_stage","owner":owner_scope_bytes(&proof.scope.owner),"job":proof.scope.job.as_bytes(),"stage":proof.scope.stage.as_bytes(),"revision":proof.job_revision,"generation":proof.claim_generation,"not_before_ms":proof.not_before_ms})
        }
        TerminalAuthority::Released(proof) => {
            json!({"kind":"released_stage","owner":owner_scope_bytes(&proof.scope.owner),"job":proof.scope.job.as_bytes(),"stage":proof.scope.stage.as_bytes(),"revision":proof.job_revision,"generation":proof.claim_generation,"candidate_dispatch":proof.candidate_dispatch.as_bytes()})
        }
    }
}

pub(super) fn request_binding(
    request: &NoDeliveryRequest<'_>,
    job: &ImageJob,
) -> Result<Vec<u8>, PublicationError> {
    let command = request.terminal;
    // A committed seal increments revision; replay compares its original immutable proof before CAS.
    let mut original = job.clone();
    original.revision = command.expected_job_revision;
    let (reason, decisions, candidates, locals, selected, preflight) = match request.cause {
        NoDeliveryCause::Semantic(proof) => {
            let TerminalActor::Dispatch(actor) = &command.actor else {
                return Err(PublicationError::BindingConflict);
            };
            if proof.expected_revision() != command.expected_job_revision
                || !proof.matches(
                    &original,
                    actor,
                    proof.decisions(),
                    proof.source_candidates(),
                    proof.local_inspections(),
                )
            {
                return Err(PublicationError::BindingConflict);
            }
            (
                proof.reason(),
                decision_binding::encode(proof.decisions())?,
                super::candidates::encode(proof.source_candidates())?,
                local_inspections::encode(proof.local_inspections())?,
                proof
                    .candidate()
                    .map(selection::candidate_binding)
                    .transpose()?,
                Value::Null,
            )
        }
        NoDeliveryCause::DemandEnded(proof) => {
            if proof.expected_revision() != command.expected_job_revision
                || !proof.matches(&original, command.now_ms)
            {
                return Err(PublicationError::BindingConflict);
            }
            (
                proof.reason(),
                "[]".into(),
                "[]".into(),
                "[]".into(),
                None,
                Value::Null,
            )
        }
        NoDeliveryCause::RequiredPreflight(proof) => {
            let TerminalActor::UnsentStage(claim) = &command.actor else {
                return Err(PublicationError::BindingConflict);
            };
            if proof.expected_revision() != command.expected_job_revision
                || !proof.matches(&original, claim, proof.candidate(), proof.source_digest())
            {
                return Err(PublicationError::BindingConflict);
            }
            (
                proof.reason(),
                "[]".into(),
                super::candidates::encode(std::slice::from_ref(proof.candidate()))?,
                "[]".into(),
                Some(selection::candidate_binding(proof.candidate())?),
                json!(proof.source_digest()),
            )
        }
        NoDeliveryCause::NoOutput(proof) => {
            let TerminalActor::Dispatch(actor) = &command.actor else {
                return Err(PublicationError::BindingConflict);
            };
            if proof.scope() != actor || proof.expected_revision() != command.expected_job_revision
            {
                return Err(PublicationError::BindingConflict);
            }
            let basis = proof.basis();
            (
                NoDeliveryReason::SupplierNoOutput,
                "[]".into(),
                "[]".into(),
                "[]".into(),
                None,
                json!({"raw_sha256":proof.raw_sha256(),"usage_revision":proof.usage_revision(),
                    "kind":basis.kind.key(),"input_revision":basis.input_revision,"input_digest":basis.input_digest,
                    "config_revision":basis.config_revision,"config_digest":basis.config_digest,"parent":basis.parent_dispatch.map(|value|*value.as_bytes())}),
            )
        }
        NoDeliveryCause::PreflightRejected(proof) => {
            let TerminalActor::UnsentStage(claim) = &command.actor else {
                return Err(PublicationError::BindingConflict);
            };
            let intent_digest = Sha256::digest(job.intent.bytes()).into();
            let config_digest = Sha256::digest(job.config.bytes()).into();
            if proof.claim() != claim
                || !proof.matches_metadata(
                    &job.owner,
                    &job.job,
                    command.expected_job_revision,
                    &intent_digest,
                    &config_digest,
                )
            {
                return Err(PublicationError::BindingConflict);
            }
            let basis = proof.basis();
            (
                preflight_reason(proof.reason()),
                "[]".into(),
                super::candidates::encode(
                    &proof.candidate().cloned().into_iter().collect::<Vec<_>>(),
                )?,
                "[]".into(),
                proof
                    .candidate()
                    .map(selection::candidate_binding)
                    .transpose()?,
                json!({"reason":proof.reason().key(),"kind":basis.kind.key(),"input_revision":basis.input_revision,"input_digest":basis.input_digest,"config_revision":basis.config_revision,"config_digest":basis.config_digest,"parent":basis.parent_dispatch.map(|value|*value.as_bytes())}),
            )
        }
        NoDeliveryCause::SubmitDeclined(proof) => {
            let TerminalActor::Dispatch(scope) = &command.actor else {
                return Err(PublicationError::BindingConflict);
            };
            if &proof.scope != scope {
                return Err(PublicationError::BindingConflict);
            }
            (
                NoDeliveryReason::SupplierDeclined,
                "[]".into(),
                "[]".into(),
                "[]".into(),
                None,
                decline_value(proof),
            )
        }
    };
    let authority = authority_value(&command.authority);
    let bytes=json!({"domain":"df-imagery/no-delivery-v1","actor":actor_value(&command.actor),"tail":actor_value(&command.tail),"authority":authority,
        "expected_revision":command.expected_job_revision,"candidate_dispatch":command.candidate_dispatch.map(|value|*value.as_bytes()),"reason":reason.key(),
        "intent_revision":job.intent.revision(),"intent_digest":<[u8;32]>::from(Sha256::digest(job.intent.bytes())),
        "config_revision":job.config.revision(),"config_digest":<[u8;32]>::from(Sha256::digest(job.config.bytes())),
        "recipe_digest":job.recipe.as_ref().map(df_imagery::publication::recipe_digest),
        "decisions":decisions,"candidates":candidates,"locals":locals,"selected":selected,"cause_details":preflight,
    }).to_string().into_bytes();
    if bytes.len() > MAXIMUM_RECIPE_BYTES {
        return Err(PublicationError::InvalidRecord);
    }
    Ok(bytes)
}

pub(super) async fn verify_in(
    tx: &Transaction<'_>,
    request: &NoDeliveryRequest<'_>,
    job: &ImageJob,
    validation: &TerminalValidation,
) -> Result<RetainedEvidence, PublicationError> {
    if !matches!(request.cause, NoDeliveryCause::DemandEnded(_))
        && request.terminal.actor != request.terminal.tail
    {
        return Err(PublicationError::BindingConflict);
    }
    match request.cause {
        NoDeliveryCause::Semantic(proof) => {
            let refs = decision_binding::read_refs_in(
                tx,
                job,
                &decision_binding::encode(proof.decisions())?,
            )
            .await?;
            let candidates = if let Some(candidate) = proof.candidate() {
                if validation.candidate.as_ref() != Some(candidate) {
                    return Err(PublicationError::BindingConflict);
                }
                decision_binding::verify_in(tx, job, candidate, &refs).await?
            } else {
                if validation.candidate.is_some()
                    || refs.len() != 1
                    || refs[0].kind != DecisionKind::Classification
                {
                    return Err(PublicationError::BindingConflict);
                }
                vec![]
            };
            local_inspections::verify_in(tx, job, proof.local_inspections(), &candidates).await?;
            let TerminalActor::Dispatch(actor) = &request.terminal.actor else {
                return Err(PublicationError::BindingConflict);
            };
            if !proof.matches(job, actor, &refs, &candidates, proof.local_inspections()) {
                return Err(PublicationError::BindingConflict);
            }
            Ok(RetainedEvidence {
                reason: proof.reason(),
                decisions: refs,
                candidates,
                locals: proof.local_inspections().to_vec(),
                no_output_raw_sha256: None,
                legacy_output: None,
            })
        }
        NoDeliveryCause::DemandEnded(proof) => {
            if !proof.matches(job, request.terminal.now_ms) {
                return Err(PublicationError::BindingConflict);
            }
            let (candidates, decisions) = retained_ancestors(tx, job, validation).await?;
            Ok(RetainedEvidence {
                reason: proof.reason(),
                decisions,
                candidates,
                locals: vec![],
                no_output_raw_sha256: None,
                legacy_output: None,
            })
        }
        NoDeliveryCause::RequiredPreflight(proof) => {
            let TerminalActor::UnsentStage(claim) = &request.terminal.actor else {
                return Err(PublicationError::BindingConflict);
            };
            let candidate = validation
                .candidate
                .as_ref()
                .ok_or(PublicationError::BindingConflict)?;
            let source_digest = crate::imagery_runtime::sources_digest_in(tx, &candidate.scope)
                .await
                .map_err(|_| PublicationError::Unavailable)?
                .ok_or(PublicationError::BindingConflict)?;
            if !proof.matches(job, claim, candidate, &source_digest) {
                return Err(PublicationError::BindingConflict);
            }
            let (candidates, decisions) = retained_ancestors(tx, job, validation).await?;
            Ok(RetainedEvidence {
                reason: proof.reason(),
                decisions,
                candidates,
                locals: vec![],
                no_output_raw_sha256: None,
                legacy_output: None,
            })
        }
        NoDeliveryCause::NoOutput(proof) => {
            let actual = validation
                .actor
                .dispatch
                .as_ref()
                .ok_or(PublicationError::BindingConflict)?;
            let row=tx.query_opt("SELECT evidence,usable,raw_output_sha256 FROM imagery.image_dispatch_usage WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND dispatch_id=$4 AND revision=$5", &[
                &owner_scope_bytes(&job.owner),&job.job.as_bytes().as_slice(),&actual.stage.as_bytes().as_slice(),&actual.dispatch.as_bytes().as_slice(),&i64::from(actual.usage_revision),
            ]).await.map_err(super::super::rows::database_error)?.ok_or(PublicationError::BindingConflict)?;
            let raw: Option<[u8; 32]> = row
                .get::<_, Option<Vec<u8>>>("raw_output_sha256")
                .map(|value| {
                    value
                        .try_into()
                        .map_err(|_| PublicationError::InvalidRecord)
                })
                .transpose()?;
            if row.get::<_, String>("evidence") == "unknown"
                || row.get::<_, bool>("usable")
                || !proof.matches(job, actual, &validation.actor.basis, raw.as_ref())
            {
                return Err(PublicationError::BindingConflict);
            }
            let (candidates, decisions) = retained_ancestors(tx, job, validation).await?;
            Ok(RetainedEvidence {
                reason: NoDeliveryReason::SupplierNoOutput,
                decisions,
                candidates,
                locals: vec![],
                no_output_raw_sha256: raw,
                legacy_output: None,
            })
        }
        NoDeliveryCause::PreflightRejected(proof) => {
            let TerminalActor::UnsentStage(claim) = &request.terminal.actor else {
                return Err(PublicationError::BindingConflict);
            };
            let row=tx.query_one("SELECT intent_digest,config_digest FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2",&[&owner_scope_bytes(&job.owner),&job.job.as_bytes().as_slice()]).await.map_err(super::super::rows::database_error)?;
            let intent: [u8; 32] = row
                .get::<_, Vec<u8>>("intent_digest")
                .try_into()
                .map_err(|_| PublicationError::InvalidRecord)?;
            let config: [u8; 32] = row
                .get::<_, Vec<u8>>("config_digest")
                .try_into()
                .map_err(|_| PublicationError::InvalidRecord)?;
            if proof.claim() != claim
                || proof.basis() != &validation.actor.basis
                || proof.candidate() != validation.candidate.as_ref()
                || !proof.matches_metadata(&job.owner, &job.job, job.revision, &intent, &config)
            {
                return Err(PublicationError::BindingConflict);
            }
            let (candidates, decisions) = retained_ancestors(tx, job, validation).await?;
            Ok(RetainedEvidence {
                reason: preflight_reason(proof.reason()),
                decisions,
                candidates,
                locals: vec![],
                no_output_raw_sha256: None,
                legacy_output: None,
            })
        }
        NoDeliveryCause::SubmitDeclined(proof) => {
            let TerminalActor::Dispatch(scope) = &request.terminal.actor else {
                return Err(PublicationError::BindingConflict);
            };
            if scope != &proof.scope
                || crate::imagery_execution::read_decline_reference_in(tx, scope)
                    .await
                    .map_err(PublicationError::Execution)?
                    .as_ref()
                    != Some(proof)
            {
                return Err(PublicationError::BindingConflict);
            }
            let row=tx.query_one("SELECT retry_dispatch_id FROM imagery.image_execution_stage WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 FOR UPDATE",&[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice()]).await.map_err(super::super::rows::database_error)?;
            if row.get::<_, Option<Vec<u8>>>(0).is_some() {
                return Err(PublicationError::BindingConflict);
            }
            let (candidates, decisions) = retained_ancestors(tx, job, validation).await?;
            Ok(RetainedEvidence {
                reason: NoDeliveryReason::SupplierDeclined,
                decisions,
                candidates,
                locals: vec![],
                no_output_raw_sha256: None,
                legacy_output: None,
            })
        }
    }
}

fn preflight_reason(reason: df_imagery::runtime::PreflightCause) -> NoDeliveryReason {
    match reason {
        df_imagery::runtime::PreflightCause::MissingCredential => {
            NoDeliveryReason::MissingCredential
        }
        df_imagery::runtime::PreflightCause::UnsupportedProfile => {
            NoDeliveryReason::UnsupportedProfile
        }
    }
}

fn decline_value(proof: &df_imagery::execution::DeclineReceipt) -> Value {
    let basis = &proof.basis;
    json!({"scope":actor_value(&TerminalActor::Dispatch(proof.scope.clone())),"kind":basis.kind.key(),"input_revision":basis.input_revision,"input_digest":basis.input_digest,"config_revision":basis.config_revision,"config_digest":basis.config_digest,"parent":basis.parent_dispatch.map(|value|*value.as_bytes()),
        "finance":proof.finance.map(|finance|json!({"attempt":finance.attempt,"operation":finance.operation,"effect":finance.effect})),
        "observation_version":proof.observation_version,"usage_revision":proof.receipt.usage_revision,"observed_at_ms":proof.receipt.observed_at_ms,
        "contract":proof.contract.key(),"status":proof.status,"body_sha256":proof.body_sha256,"body_bytes":proof.body_bytes,"raw_sha256":proof.raw_sha256,"raw_bytes":proof.raw_bytes,
        "provider":proof.route.provider,"model":proof.route.model,"cause":proof.route.cause,"detail":proof.route.detail,"retry_not_before_ms":proof.retry_not_before_ms,
        "transport_request_id_sha256":proof.transport_request_id_sha256,"retry_after_values":proof.retry_after_values,
        "max_attempts":proof.retry.max_attempts,"retry_deadline_ms":proof.retry.retry_deadline_ms,"backoff_ms":proof.retry.backoff_ms,"config_snapshot_revision":proof.retry.config_snapshot_revision})
}

pub(super) async fn retained_ancestors(
    tx: &Transaction<'_>,
    job: &ImageJob,
    validation: &TerminalValidation,
) -> Result<(Vec<CandidateReceipt>, Vec<FinalDecisionRef>), PublicationError> {
    let roots = validation
        .actor
        .dispatch
        .iter()
        .chain(validation.tail.dispatch.iter())
        .map(|view| view.dispatch)
        .chain(
            validation
                .candidate
                .iter()
                .map(|candidate| candidate.scope.dispatch),
        );
    let mut visited = BTreeSet::new();
    let mut candidates = BTreeMap::new();
    let mut decisions = BTreeMap::new();
    for root in roots {
        let mut path = BTreeSet::new();
        let mut current = Some(root);
        while let Some(dispatch) = current {
            if !path.insert(dispatch) {
                return Err(PublicationError::BindingConflict);
            }
            let view = crate::imagery_execution::read_dispatch_in(tx, &job.owner, &dispatch)
                .await
                .map_err(PublicationError::Execution)?;
            if view.job != job.job {
                return Err(PublicationError::BindingConflict);
            }
            let basis = crate::imagery_execution::read_stage_basis_in(
                tx,
                &job.owner,
                &job.job,
                &view.stage,
            )
            .await
            .map_err(PublicationError::Execution)?;
            current = basis.parent_dispatch;
            if !visited.insert(dispatch) {
                continue;
            }
            if visited.len() > 7 {
                return Err(PublicationError::BindingConflict);
            }
            if let Some(candidate) = crate::imagery_execution::read_candidate_reference_in(
                tx,
                &job.owner,
                &job.job,
                &view.stage,
                &dispatch,
            )
            .await
            .map_err(PublicationError::Execution)?
            {
                candidates.insert(dispatch, candidate);
            }
            if let Some(decision) = crate::imagery_execution::read_decision_reference_unlocked_in(
                tx,
                &job.owner,
                &job.job,
                &view.stage,
                &dispatch,
            )
            .await
            .map_err(PublicationError::Execution)?
            {
                decisions.insert(dispatch, decision_binding::reference(decision));
            }
        }
    }
    Ok((
        candidates.into_values().collect(),
        decisions.into_values().collect(),
    ))
}
