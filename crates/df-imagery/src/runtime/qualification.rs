//! Protected policy proofs reconstructed from exact retained observations, outside DB locks.
use super::{analysis, inspection_policy, ports::*};
use crate::execution::{
    CandidateReceipt, DecisionKind, DecisionObservation, ExecutionScope, LocalInspectionCheckpoint,
    StageKind,
};
use crate::publication::FinalDecisionRef;
use crate::request::InspectionRequirement;
use crate::service::types::{CreatorAssessment, CreatorRoute};
use crate::{ImageJob, ImageOwner};
use inspection_policy::{
    InspectionAssessment, InspectionVerdict, PhotoCheckFailure, UnjudgedReason,
};
use sha2::{Digest, Sha256};

/// The selected interpretation and exact raw revision, with its real source candidate.
#[derive(Clone, Debug)]
pub struct PublicationDecisionEvidence {
    pub decision: DecisionObservation,
    pub usage: RuntimeUsage,
    pub source_candidate: Option<CandidateReceipt>,
}
/// Actual immutable ancestor observations; local preflight failures name no supplier dispatch.
#[derive(Clone, Debug, Default)]
pub struct PublicationEvidence {
    pub decisions: Vec<PublicationDecisionEvidence>,
    pub candidates: Vec<CandidateReceipt>,
    pub local_inspections: Vec<LocalInspectionCheckpoint>,
}

#[derive(Clone)]
struct Binding {
    owner: ImageOwner,
    job: crate::ImageJobId,
    intent_digest: [u8; 32],
    config_digest: [u8; 32],
    candidates: Vec<CandidateReceipt>,
    decisions: Vec<FinalDecisionRef>,
    locals: Vec<LocalInspectionCheckpoint>,
}
impl Binding {
    fn matches(
        &self,
        job: &ImageJob,
        refs: &[FinalDecisionRef],
        candidates: &[CandidateReceipt],
        locals: &[LocalInspectionCheckpoint],
    ) -> bool {
        self.owner == job.owner
            && self.job == job.job
            && self.intent_digest == digest(job.intent.bytes())
            && self.config_digest == digest(job.config.bytes())
            && self.decisions == refs
            && self.candidates == candidates
            && self.locals == locals
    }
}

/// Only qualification constructs this binding; opaque recipe bytes never imply acceptance.
#[derive(Clone)]
pub struct QualifiedPipeline {
    binding: Binding,
    candidate: CandidateReceipt,
}
impl std::fmt::Debug for QualifiedPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QualifiedPipeline")
            .field("job", &self.binding.job)
            .finish_non_exhaustive()
    }
}
impl QualifiedPipeline {
    pub fn matches(
        &self,
        job: &ImageJob,
        candidate: &CandidateReceipt,
        refs: &[FinalDecisionRef],
        sources: &[CandidateReceipt],
        locals: &[LocalInspectionCheckpoint],
    ) -> bool {
        self.candidate == *candidate && self.binding.matches(job, refs, sources, locals)
    }
    pub fn candidate(&self) -> &CandidateReceipt {
        &self.candidate
    }
    pub fn decisions(&self) -> &[FinalDecisionRef] {
        &self.binding.decisions
    }
    pub fn source_candidates(&self) -> &[CandidateReceipt] {
        &self.binding.candidates
    }
    pub fn local_inspections(&self) -> &[LocalInspectionCheckpoint] {
        &self.binding.locals
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionValidity {
    Useful,
    Uncertain,
    Unjudged,
}
/// An actual useful decision, distinct from whether an image was delivered to a customer.
#[derive(Clone)]
pub struct QualifiedDecision {
    owner: ImageOwner,
    job: crate::ImageJobId,
    intent_digest: [u8; 32],
    config_digest: [u8; 32],
    reference: FinalDecisionRef,
    source: Option<CandidateReceipt>,
    raw_sha256: [u8; 32],
    validity: DecisionValidity,
    classification_route: Option<CreatorRoute>,
}
impl std::fmt::Debug for QualifiedDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QualifiedDecision")
            .field("validity", &self.validity)
            .finish_non_exhaustive()
    }
}
impl QualifiedDecision {
    pub fn matches(
        &self,
        job: &ImageJob,
        reference: &FinalDecisionRef,
        source: Option<&CandidateReceipt>,
    ) -> bool {
        self.owner == job.owner
            && self.job == job.job
            && self.intent_digest == digest(job.intent.bytes())
            && self.config_digest == digest(job.config.bytes())
            && self.reference == *reference
            && self.source.as_ref() == source
    }
    pub fn matches_metadata(
        &self,
        owner: &ImageOwner,
        job: &crate::ImageJobId,
        intent_digest: &[u8; 32],
        config_digest: &[u8; 32],
        reference: &FinalDecisionRef,
        source: Option<&CandidateReceipt>,
    ) -> bool {
        self.owner == *owner
            && self.job == *job
            && self.intent_digest == *intent_digest
            && self.config_digest == *config_digest
            && self.reference == *reference
            && self.source.as_ref() == source
    }
    pub fn scope(&self) -> &ExecutionScope {
        &self.reference.scope
    }
    pub fn usage_revision(&self) -> u32 {
        self.reference.usage_revision
    }
    pub fn raw_sha256(&self) -> &[u8; 32] {
        &self.raw_sha256
    }
    pub fn validity(&self) -> DecisionValidity {
        self.validity
    }
    pub fn is_useful(&self) -> bool {
        self.validity != DecisionValidity::Unjudged
    }
    pub fn classification_route(&self) -> Option<CreatorRoute> {
        self.classification_route
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoDeliveryReason {
    InspectionRejected,
    RequiredInspectionUnjudged,
    CreatorNeedsRevision,
    CreatorUncertain,
    Retired,
    Expired,
    RequiredInspectionPreparation,
    RequiredInspectionDisabled,
    SupplierNoOutput,
    SupplierDeclined,
    SourceDenied,
    MissingCredential,
    UnsupportedProfile,
}
impl NoDeliveryReason {
    pub fn parse_label(label: &str) -> Option<Self> {
        Some(match label {
            "inspection_rejected" => Self::InspectionRejected,
            "required_inspection_unjudged" => Self::RequiredInspectionUnjudged,
            "creator_needs_revision" => Self::CreatorNeedsRevision,
            "creator_uncertain" => Self::CreatorUncertain,
            "retired" => Self::Retired,
            "expired" => Self::Expired,
            "required_inspection_preparation" => Self::RequiredInspectionPreparation,
            "required_inspection_disabled" => Self::RequiredInspectionDisabled,
            "supplier_no_output" => Self::SupplierNoOutput,
            "supplier_declined" => Self::SupplierDeclined,
            "source_denied" => Self::SourceDenied,
            "missing_credential" => Self::MissingCredential,
            "unsupported_profile" => Self::UnsupportedProfile,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::InspectionRejected => "inspection_rejected",
            Self::RequiredInspectionUnjudged => "required_inspection_unjudged",
            Self::CreatorNeedsRevision => "creator_needs_revision",
            Self::CreatorUncertain => "creator_uncertain",
            Self::Retired => "retired",
            Self::Expired => "expired",
            Self::RequiredInspectionPreparation => "required_inspection_preparation",
            Self::RequiredInspectionDisabled => "required_inspection_disabled",
            Self::SupplierNoOutput => "supplier_no_output",
            Self::SupplierDeclined => "supplier_declined",
            Self::SourceDenied => "source_denied",
            Self::MissingCredential => "missing_credential",
            Self::UnsupportedProfile => "unsupported_profile",
        }
    }
}
#[derive(Clone)]
pub struct QualifiedNoDelivery {
    binding: Binding,
    candidate: Option<CandidateReceipt>,
    actor: ExecutionScope,
    reason: NoDeliveryReason,
    expected_revision: u64,
}
impl std::fmt::Debug for QualifiedNoDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QualifiedNoDelivery")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}
impl QualifiedNoDelivery {
    pub fn matches(
        &self,
        job: &ImageJob,
        actor: &ExecutionScope,
        refs: &[FinalDecisionRef],
        sources: &[CandidateReceipt],
        locals: &[LocalInspectionCheckpoint],
    ) -> bool {
        self.expected_revision == job.revision
            && self.actor == *actor
            && self.binding.matches(job, refs, sources, locals)
    }
    pub fn actor(&self) -> &ExecutionScope {
        &self.actor
    }
    pub fn reason(&self) -> NoDeliveryReason {
        self.reason
    }
    pub fn expected_revision(&self) -> u64 {
        self.expected_revision
    }
    pub fn decisions(&self) -> &[FinalDecisionRef] {
        &self.binding.decisions
    }
    pub fn source_candidates(&self) -> &[CandidateReceipt] {
        &self.binding.candidates
    }
    pub fn local_inspections(&self) -> &[LocalInspectionCheckpoint] {
        &self.binding.locals
    }
    pub fn candidate(&self) -> Option<&CandidateReceipt> {
        self.candidate.as_ref()
    }
}
#[derive(Clone, Debug)]
pub struct QualifiedDemandEnded {
    owner: ImageOwner,
    job: crate::ImageJobId,
    revision: u64,
    intent_digest: [u8; 32],
    config_digest: [u8; 32],
    reason: NoDeliveryReason,
}
impl QualifiedDemandEnded {
    pub fn matches(&self, job: &ImageJob, now_ms: i64) -> bool {
        self.owner == job.owner
            && self.job == job.job
            && self.revision == job.revision
            && self.intent_digest == digest(job.intent.bytes())
            && self.config_digest == digest(job.config.bytes())
            && (job.state == crate::JobState::Retired || now_ms >= job.absolute_deadline_ms)
    }
    pub fn expected_revision(&self) -> u64 {
        self.revision
    }
    pub fn reason(&self) -> NoDeliveryReason {
        self.reason
    }
}

pub fn qualify_demand_ended(
    job: &ImageJob,
    now_ms: i64,
) -> Result<QualifiedDemandEnded, RuntimeError> {
    let reason = if job.state == crate::JobState::Retired {
        NoDeliveryReason::Retired
    } else if now_ms >= job.absolute_deadline_ms {
        NoDeliveryReason::Expired
    } else {
        return Err(RuntimeError::InvalidRecord);
    };
    Ok(QualifiedDemandEnded {
        owner: job.owner.clone(),
        job: job.job,
        revision: job.revision,
        intent_digest: digest(job.intent.bytes()),
        config_digest: digest(job.config.bytes()),
        reason,
    })
}

/// A required inspection failed before admission; no supplier dispatch or money is implied.
#[derive(Clone, Debug)]
pub struct QualifiedRequiredPreflight {
    owner: ImageOwner,
    job: crate::ImageJobId,
    revision: u64,
    intent_digest: [u8; 32],
    config_digest: [u8; 32],
    claim: crate::execution::StageClaim,
    candidate: CandidateReceipt,
    source_digest: [u8; 32],
    reason: NoDeliveryReason,
}
impl QualifiedRequiredPreflight {
    pub fn matches(
        &self,
        job: &ImageJob,
        claim: &crate::execution::StageClaim,
        candidate: &CandidateReceipt,
        source_digest: &[u8; 32],
    ) -> bool {
        self.owner == job.owner
            && self.job == job.job
            && self.revision == job.revision
            && self.intent_digest == digest(job.intent.bytes())
            && self.config_digest == digest(job.config.bytes())
            && self.claim == *claim
            && self.candidate == *candidate
            && self.source_digest == *source_digest
    }
    pub fn claim(&self) -> &crate::execution::StageClaim {
        &self.claim
    }
    pub fn candidate(&self) -> &CandidateReceipt {
        &self.candidate
    }
    pub fn source_digest(&self) -> &[u8; 32] {
        &self.source_digest
    }
    pub fn expected_revision(&self) -> u64 {
        self.revision
    }
    pub fn reason(&self) -> NoDeliveryReason {
        self.reason
    }
}
pub fn qualify_required_preflight(
    job: &ImageJob,
    claim: &crate::execution::StageClaim,
    candidate: &CandidateReceipt,
    source_digest: [u8; 32],
    reason: crate::execution::LocalInspectionReason,
    now_ms: i64,
) -> Result<QualifiedRequiredPreflight, RuntimeError> {
    let intent = decoded(job)?;
    let spec = intent
        .operation
        .pipeline()
        .and_then(|pipeline| pipeline.inspection.as_ref())
        .ok_or(RuntimeError::InvalidRecord)?;
    if spec.requirement != InspectionRequirement::Required
        || claim.scope.owner != job.owner
        || claim.scope.job != job.job
        || claim.generation == 0
        || candidate.scope.owner != job.owner
        || candidate.scope.job != job.job
        || now_ms <= 0
    {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(QualifiedRequiredPreflight {
        owner: job.owner.clone(),
        job: job.job,
        revision: job.revision,
        intent_digest: digest(job.intent.bytes()),
        config_digest: digest(job.config.bytes()),
        claim: claim.clone(),
        candidate: candidate.clone(),
        source_digest,
        reason: match reason {
            crate::execution::LocalInspectionReason::Preparation => {
                NoDeliveryReason::RequiredInspectionPreparation
            }
            crate::execution::LocalInspectionReason::Disabled => {
                NoDeliveryReason::RequiredInspectionDisabled
            }
        },
    })
}

/// Replays raw supplier/schema evidence; callers run this on their bounded owned CPU worker.
pub fn qualify_economic_decision(
    job: &ImageJob,
    evidence: &PublicationDecisionEvidence,
) -> Result<QualifiedDecision, RuntimeError> {
    if crate::service::analysis_codec::is_analysis_intent(job.intent.bytes()) {
        let validity = super::standalone::validate_standalone_evidence(job, evidence)?;
        return Ok(QualifiedDecision {
            owner: job.owner.clone(),
            job: job.job,
            intent_digest: digest(job.intent.bytes()),
            config_digest: digest(job.config.bytes()),
            reference: FinalDecisionRef::from_observation(&evidence.decision),
            source: None,
            raw_sha256: digest(
                evidence
                    .usage
                    .raw_output
                    .as_deref()
                    .ok_or(RuntimeError::InvalidRecord)?,
            ),
            validity,
            classification_route: None,
        });
    }
    let intent = decoded(job)?;
    let config = super::retained_config(job)?;
    let meaning = replay_decision(job, &intent, &config, evidence)?;
    let (validity, route) = match meaning {
        Meaning::Classification(assessment) => (
            if assessment.route == CreatorRoute::Uncertain {
                DecisionValidity::Uncertain
            } else {
                DecisionValidity::Useful
            },
            Some(assessment.route),
        ),
        Meaning::Inspection(assessment) => (
            if matches!(assessment.verdict, InspectionVerdict::Unjudged(_)) {
                DecisionValidity::Unjudged
            } else {
                DecisionValidity::Useful
            },
            None,
        ),
    };
    Ok(QualifiedDecision {
        owner: job.owner.clone(),
        job: job.job,
        intent_digest: digest(job.intent.bytes()),
        config_digest: digest(job.config.bytes()),
        reference: FinalDecisionRef::from_observation(&evidence.decision),
        source: evidence.source_candidate.clone(),
        raw_sha256: digest(
            evidence
                .usage
                .raw_output
                .as_deref()
                .ok_or(RuntimeError::InvalidRecord)?,
        ),
        validity,
        classification_route: route,
    })
}

/// Qualified output requires exact configured decisions; BestEffort never becomes a fabricated Pass.
pub fn qualify_publication(
    job: &ImageJob,
    evidence: &PublicationEvidence,
    candidate: &CandidateReceipt,
) -> Result<QualifiedPipeline, RuntimeError> {
    let intent = decoded(job)?;
    let config = super::retained_config(job)?;
    let facts = collect(job, evidence)?;
    if !facts.binding.candidates.contains(candidate) {
        return Err(RuntimeError::InvalidRecord);
    }
    if intent
        .operation
        .pipeline()
        .is_some_and(|pipeline| pipeline.creator.is_some())
    {
        let (_, assessment) = facts
            .classification
            .as_ref()
            .ok_or(RuntimeError::InvalidRecord)?;
        if !matches!(assessment.route, CreatorRoute::General | CreatorRoute::Bfl) {
            return Err(RuntimeError::ProviderRefused);
        }
    }
    if let Some(spec) = intent
        .operation
        .pipeline()
        .and_then(|pipeline| pipeline.inspection.as_ref())
    {
        if let Some(selected) = inspection_for(&facts, candidate) {
            match selected.2.verdict {
                InspectionVerdict::Pass => {}
                InspectionVerdict::Unjudged(_)
                    if spec.requirement == InspectionRequirement::BestEffort => {}
                InspectionVerdict::Failed(PhotoCheckFailure::VisibleText)
                    if visible_text_exception(&facts, &selected.1, spec) => {}
                _ => return Err(RuntimeError::ProviderRefused),
            }
        } else if spec.requirement != InspectionRequirement::BestEffort
            || !facts.binding.locals.iter().any(|local| {
                local.candidate_scope.dispatch == candidate.scope.dispatch
                    || candidate.basis.kind == StageKind::Extraction
                        && candidate.basis.parent_dispatch == Some(local.candidate_scope.dispatch)
            })
        {
            return Err(RuntimeError::InvalidRecord);
        }
    }
    let selected_backend = if facts
        .classification
        .as_ref()
        .is_some_and(|(_, assessment)| assessment.route == CreatorRoute::Bfl)
    {
        crate::BackendId::Bfl
    } else {
        super::dispatch::backend(&config, &intent)
    };
    if selected_backend == crate::BackendId::Fal
        && matches!(
            intent.output.delivered().alpha(),
            crate::AlphaHandling::PreserveAlpha
        )
        && candidate.basis.kind != StageKind::Extraction
    {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(QualifiedPipeline {
        binding: facts.binding,
        candidate: candidate.clone(),
    })
}

/// A first rejected image with a correction remaining cannot become terminal evidence.
pub fn qualify_no_delivery(
    job: &ImageJob,
    evidence: &PublicationEvidence,
    candidate: Option<&CandidateReceipt>,
) -> Result<QualifiedNoDelivery, RuntimeError> {
    let intent = decoded(job)?;
    let facts = collect(job, evidence)?;
    let (actor, reason) = if let Some(candidate) = candidate {
        if !facts.binding.candidates.contains(candidate) {
            return Err(RuntimeError::InvalidRecord);
        }
        let spec = intent
            .operation
            .pipeline()
            .and_then(|pipeline| pipeline.inspection.as_ref())
            .ok_or(RuntimeError::InvalidRecord)?;
        let selected = inspection_for(&facts, candidate).ok_or(RuntimeError::InvalidRecord)?;
        let reason = match selected.2.verdict {
            InspectionVerdict::Failed(PhotoCheckFailure::VisibleText)
                if visible_text_exception(&facts, &selected.1, spec) =>
            {
                return Err(RuntimeError::InvalidRecord);
            }
            InspectionVerdict::Failed(_)
                if selected.1.basis.kind == StageKind::Correction
                    || spec.maximum_corrections == 0 =>
            {
                NoDeliveryReason::InspectionRejected
            }
            InspectionVerdict::Unjudged(_)
                if spec.requirement == InspectionRequirement::Required =>
            {
                NoDeliveryReason::RequiredInspectionUnjudged
            }
            _ => return Err(RuntimeError::InvalidRecord),
        };
        (selected.0.clone(), reason)
    } else {
        let (actor, assessment) = facts
            .classification
            .as_ref()
            .ok_or(RuntimeError::InvalidRecord)?;
        let reason = match assessment.route {
            CreatorRoute::Revise => NoDeliveryReason::CreatorNeedsRevision,
            CreatorRoute::Uncertain => NoDeliveryReason::CreatorUncertain,
            _ => return Err(RuntimeError::InvalidRecord),
        };
        (actor.clone(), reason)
    };
    Ok(QualifiedNoDelivery {
        binding: facts.binding,
        candidate: candidate.cloned(),
        actor,
        reason,
        expected_revision: job.revision,
    })
}

enum Meaning {
    Classification(CreatorAssessment),
    Inspection(InspectionAssessment),
}
struct Facts {
    binding: Binding,
    classification: Option<(ExecutionScope, CreatorAssessment)>,
    inspections: Vec<(ExecutionScope, CandidateReceipt, InspectionAssessment)>,
}
fn collect(job: &ImageJob, evidence: &PublicationEvidence) -> Result<Facts, RuntimeError> {
    let intent = decoded(job)?;
    let config = super::retained_config(job)?;
    let config_digest = digest(job.config.bytes());
    let mut candidates = evidence.candidates.clone();
    candidates.sort_by_key(|candidate| candidate.scope.dispatch);
    if candidates
        .windows(2)
        .any(|pair| pair[0].scope.dispatch == pair[1].scope.dispatch)
        || candidates.iter().any(|candidate| {
            candidate.scope.owner != job.owner
                || candidate.scope.job != job.job
                || candidate.basis.config_digest != config_digest
                || !matches!(
                    candidate.basis.kind,
                    StageKind::Generation
                        | StageKind::Edit
                        | StageKind::Correction
                        | StageKind::Extraction
                )
        })
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let mut decisions = Vec::new();
    let mut classification = None;
    let mut inspections = Vec::new();
    let mut bytes = 0u64;
    for retained in &evidence.decisions {
        bytes = bytes
            .checked_add(
                u64::try_from(retained.usage.raw_output.as_ref().map_or(0, Vec::len))
                    .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .ok_or(RuntimeError::InvalidRecord)?;
        if bytes > config.transforms.max_decoded_bytes {
            return Err(RuntimeError::InvalidConfiguration);
        }
        if decisions.iter().any(|reference: &FinalDecisionRef| {
            reference.scope.dispatch == retained.decision.scope.dispatch
        }) {
            return Err(RuntimeError::InvalidRecord);
        }
        match replay_decision(job, &intent, &config, retained)? {
            Meaning::Classification(assessment) => {
                if classification.is_some() {
                    return Err(RuntimeError::InvalidRecord);
                }
                classification = Some((retained.decision.scope.clone(), assessment));
            }
            Meaning::Inspection(assessment) => {
                let source = retained
                    .source_candidate
                    .as_ref()
                    .ok_or(RuntimeError::InvalidRecord)?;
                if !candidates.contains(source) {
                    return Err(RuntimeError::InvalidRecord);
                }
                inspections.push((retained.decision.scope.clone(), source.clone(), assessment));
            }
        }
        decisions.push(FinalDecisionRef::from_observation(&retained.decision));
    }
    let mut locals = evidence.local_inspections.clone();
    locals.sort_by_key(|local| local.scope.stage);
    if !locals.is_empty() {
        let spec = intent
            .operation
            .pipeline()
            .and_then(|pipeline| pipeline.inspection.as_ref())
            .ok_or(RuntimeError::InvalidRecord)?;
        if spec.requirement != InspectionRequirement::BestEffort {
            return Err(RuntimeError::InvalidRecord);
        }
        for local in &locals {
            let source = candidates
                .iter()
                .find(|candidate| candidate.scope == local.candidate_scope)
                .ok_or(RuntimeError::InvalidRecord)?;
            if local.scope.owner != job.owner
                || local.scope.job != job.job
                || local.basis.kind != StageKind::Inspection
                || local.basis.parent_dispatch != Some(source.scope.dispatch)
                || local.schema_revision != 1
                || local.validator_revision != 1
                || local.basis.config_digest != config_digest
                || local.candidate_observation_version != source.observation_version
                || local.candidate_usage_revision != source.receipt.usage_revision
                || inspections
                    .iter()
                    .any(|(_, candidate, _)| candidate.scope == source.scope)
            {
                return Err(RuntimeError::InvalidRecord);
            }
        }
        if locals
            .windows(2)
            .any(|pair| pair[0].scope.stage == pair[1].scope.stage)
        {
            return Err(RuntimeError::InvalidRecord);
        }
    }
    Ok(Facts {
        binding: Binding {
            owner: job.owner.clone(),
            job: job.job,
            intent_digest: digest(job.intent.bytes()),
            config_digest,
            candidates,
            decisions,
            locals,
        },
        classification,
        inspections,
    })
}

fn replay_decision(
    job: &ImageJob,
    intent: &crate::intent_codec::DecodedIntent,
    config: &crate::ImageConfig,
    retained: &PublicationDecisionEvidence,
) -> Result<Meaning, RuntimeError> {
    let decision = &retained.decision;
    let usage = &retained.usage;
    if decision.scope.owner != job.owner
        || decision.scope.job != job.job
        || usage.owner != job.owner
        || usage.job != job.job
        || usage.stage != decision.scope.stage
        || usage.dispatch != decision.scope.dispatch
        || usage.revision.0 != u64::from(decision.receipt.usage_revision)
        || decision.basis.schema_revision != 1
        || decision.basis.validator_revision != 1
        || decision.basis.stage.kind != decision.kind.stage_kind()
        || decision.basis.stage.config_digest != digest(job.config.bytes())
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let raw = usage
        .raw_output
        .as_deref()
        .ok_or(RuntimeError::InvalidRecord)?;
    match decision.kind {
        DecisionKind::Classification => {
            let pipeline = intent
                .operation
                .pipeline()
                .ok_or(RuntimeError::InvalidRecord)?;
            let creator = pipeline
                .creator
                .as_ref()
                .ok_or(RuntimeError::InvalidRecord)?;
            if retained.source_candidate.is_some()
                || config.analysis.classification.as_ref() != Some(&creator.route)
            {
                return Err(RuntimeError::InvalidRecord);
            }
            let (status, value) = analysis::restore_response(raw, raw.len())?;
            if status != 200 {
                return Err(RuntimeError::InvalidRecord);
            }
            let assessment = analysis::classify(&value, creator, pipeline.rating)?;
            if analysis::assessment_payload(assessment) != decision.payload() {
                return Err(RuntimeError::InvalidRecord);
            }
            Ok(Meaning::Classification(assessment))
        }
        DecisionKind::PhotoInspection
        | DecisionKind::HeadLocations
        | DecisionKind::Segmentation
        | DecisionKind::StagingIdentity
        | DecisionKind::StagingFacing => Err(RuntimeError::InvalidRecord),
        DecisionKind::Inspection => {
            let spec = intent
                .operation
                .pipeline()
                .and_then(|pipeline| pipeline.inspection.as_ref())
                .ok_or(RuntimeError::InvalidRecord)?;
            if config.analysis.inspection.as_ref() != Some(&spec.route) {
                return Err(RuntimeError::InvalidRecord);
            }
            let source = retained
                .source_candidate
                .as_ref()
                .ok_or(RuntimeError::InvalidRecord)?;
            if source.scope.owner != job.owner
                || source.scope.job != job.job
                || decision.basis.stage.parent_dispatch != Some(source.scope.dispatch)
            {
                return Err(RuntimeError::InvalidRecord);
            }
            let value = analysis::parse_response(decision.payload(), decision.payload().len())?;
            let stored = InspectionAssessment::from_value(&value, spec)
                .map_err(|_| RuntimeError::InvalidRecord)?;
            let actual = replay_inspection(raw, spec)?;
            if stored != actual {
                return Err(RuntimeError::InvalidRecord);
            }
            Ok(Meaning::Inspection(stored))
        }
    }
}
fn inspection_for<'a>(
    facts: &'a Facts,
    candidate: &CandidateReceipt,
) -> Option<&'a (ExecutionScope, CandidateReceipt, InspectionAssessment)> {
    facts.inspections.iter().find(|(scope, source, _)| {
        source.scope.dispatch == candidate.scope.dispatch
            || candidate.basis.kind == StageKind::Extraction
                && candidate.basis.parent_dispatch == Some(scope.dispatch)
    })
}
fn visible_text_exception(
    facts: &Facts,
    candidate: &CandidateReceipt,
    spec: &crate::ImageInspection,
) -> bool {
    candidate.basis.kind == StageKind::Correction
        && spec.maximum_corrections == 1
        && facts.inspections.iter().any(|(scope, _, assessment)| {
            Some(scope.dispatch) == candidate.basis.parent_dispatch
                && matches!(assessment.verdict, InspectionVerdict::Failed(_))
        })
}
fn decoded(job: &ImageJob) -> Result<crate::intent_codec::DecodedIntent, RuntimeError> {
    crate::intent_codec::decode_intent(
        job.intent.bytes(),
        &crate::service::PROTOCOL_LIMITS,
        crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| RuntimeError::InvalidRecord)
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub(super) fn retain_inspection_failure(reason: UnjudgedReason) -> Vec<u8> {
    serde_json::json!({"schema_revision":1,"failure":reason.label()})
        .to_string()
        .into_bytes()
}
pub(super) fn replay_inspection(
    raw: &[u8],
    spec: &crate::ImageInspection,
) -> Result<InspectionAssessment, RuntimeError> {
    if let Ok((status, body)) = analysis::restore_response_bytes(raw, raw.len()) {
        if status != 200 {
            return Ok(InspectionAssessment::unjudged(
                spec,
                UnjudgedReason::Provider,
            ));
        }
        let value = match analysis::parse_response(&body, body.len()) {
            Ok(value) => value,
            Err(_) => {
                return Ok(InspectionAssessment::unjudged(
                    spec,
                    UnjudgedReason::InvalidResponse,
                ));
            }
        };
        return Ok(crate::backends::analysis::photo::answer(&value)
            .and_then(|answer| inspection_policy::interpret(answer, spec))
            .unwrap_or_else(|_| {
                InspectionAssessment::unjudged(spec, UnjudgedReason::InvalidResponse)
            }));
    }
    let envelope = analysis::parse_response(raw, raw.len())?;
    let object = envelope
        .as_object()
        .filter(|object| object.len() == 2)
        .ok_or(RuntimeError::InvalidRecord)?;
    if object
        .get("schema_revision")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let reason = match object.get("failure").and_then(serde_json::Value::as_str) {
        Some("preparation") => UnjudgedReason::Preparation,
        Some("transport") => UnjudgedReason::Transport,
        Some("provider") => UnjudgedReason::Provider,
        Some("invalid_response") => UnjudgedReason::InvalidResponse,
        Some("deadline") => UnjudgedReason::Deadline,
        Some("disabled") => UnjudgedReason::Disabled,
        Some("cancelled") => UnjudgedReason::Cancelled,
        _ => return Err(RuntimeError::InvalidRecord),
    };
    Ok(InspectionAssessment::unjudged(spec, reason))
}
