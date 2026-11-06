//! Protected standalone completion proofs. Only replay of actual retained observations constructs
//! them.
use super::*;

/// A standalone analysis result proven by replaying its retained supplier observation.
pub struct QualifiedAnalysis {
    checkpoint: AnalysisCheckpoint,
    job: ImageJob,
    audience: crate::Audience,
    sources: Vec<SourceReceipt>,
}
impl QualifiedAnalysis {
    pub fn checkpoint(&self) -> &AnalysisCheckpoint {
        &self.checkpoint
    }
    pub fn job(&self) -> &ImageJob {
        &self.job
    }
    pub fn audience(&self) -> &crate::Audience {
        &self.audience
    }
    pub fn sources(&self) -> &[SourceReceipt] {
        &self.sources
    }
}
/// A standalone analysis computed locally from its retained analysis dependencies and checked
/// against them, with the final selection it implies.
pub struct QualifiedLocalAnalysis {
    checkpoint: LocalAnalysisCheckpoint,
    final_checkpoint: AnalysisCheckpoint,
    job: ImageJob,
    audience: crate::Audience,
    sources: Vec<SourceReceipt>,
}
impl QualifiedLocalAnalysis {
    pub fn final_checkpoint(&self) -> &AnalysisCheckpoint {
        &self.final_checkpoint
    }
    pub fn checkpoint(&self) -> &LocalAnalysisCheckpoint {
        &self.checkpoint
    }
    pub fn job(&self) -> &ImageJob {
        &self.job
    }
    pub fn audience(&self) -> &crate::Audience {
        &self.audience
    }
    pub fn sources(&self) -> &[SourceReceipt] {
        &self.sources
    }
}
/// A selected typed decision is read without inventing an image publication or raster identity.
pub fn analysis_result(
    checkpoint: &AnalysisCheckpoint,
) -> Result<ImageAnalysisResult, RuntimeError> {
    let digest: [u8; 32] = Sha256::digest(checkpoint.result.bytes()).into();
    if checkpoint.result.revision() != 1 || digest != checkpoint.result_sha256 {
        return Err(RuntimeError::InvalidRecord);
    }
    analysis_codec::decode_result(checkpoint.result.bytes(), checkpoint.result.bytes().len())
        .map_err(|_| RuntimeError::InvalidRecord)
}
impl<J, E, S, P, F, C, I, K, B, T, Q, U> NativeImageRuntime<J, E, S, P, F, C, I, K, B, T, Q, U>
where
    J: ImageJournal,
    E: ImageExecutionJournal,
    S: ImageSources,
    P: ImagePublisher,
    F: ImageDispatchAdmission,
    C: ServiceClock,
    I: RuntimeIds,
    K: ImageCredentials,
    B: ImageCostProjection,
    T: ImageRuntimeTelemetry,
    Q: ImageRuntimeCostPolicy,
    U: ImageUsageJournal,
{
    pub(super) async fn complete_local_analysis(
        &self,
        job: ImageJob,
        stage: StageView,
        claim: ClaimToken,
        inputs: &Inputs,
        result: ImageAnalysisResult,
        maximum: usize,
    ) -> Result<Option<crate::publication::PublishedImage>, RuntimeError> {
        let checkpoint_started = std::time::Instant::now();
        let intent = analysis_codec::decode_analysis(job.intent.bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        if !crate::service::analysis::allows_local_decision(&intent.operation) {
            return Err(RuntimeError::InvalidRecord);
        }
        let operation = intent.operation.clone();
        let dependencies = inputs.dependencies.clone();
        let actual = tokio::task::spawn_blocking(move || {
            backend::prepare_local_analysis(
                &operation,
                dependencies.segmentation.as_ref(),
                dependencies.locations.as_ref(),
                dependencies.identity.as_ref(),
            )
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)??
        .ok_or(RuntimeError::InvalidRecord)?;
        if actual != result {
            return Err(RuntimeError::InvalidRecord);
        }
        crate::service::analysis::validate_result_binding(&intent.operation, &result)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let basis = self
            .execution
            .read_stage_basis(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let bytes = analysis_codec::encode_result(&result, maximum)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let checkpoint = LocalAnalysisCheckpoint {
            scope: StageScope {
                owner: job.owner.clone(),
                job: job.job,
                stage: stage.stage,
            },
            kind: decision_kind(&intent.operation),
            checkpoint_version: 1,
            schema_revision: 1,
            validator_revision: 1,
            basis,
            result_sha256: Sha256::digest(&bytes).into(),
            result: crate::VersionedSnapshot::new(1, bytes, maximum)
                .map_err(|_| RuntimeError::InvalidRecord)?,
            intent_digest: Sha256::digest(job.intent.bytes()).into(),
            config_digest: Sha256::digest(job.config.bytes()).into(),
            source_digest: super::super::source_receipts_digest(&inputs.receipts)?,
            dependencies: inputs.dependency_refs.clone(),
        };
        let final_checkpoint = local_selection(&checkpoint);
        let proof = QualifiedLocalAnalysis {
            checkpoint,
            final_checkpoint,
            job: job.clone(),
            audience: intent.audience.clone(),
            sources: inputs.receipts.clone(),
        };
        let checkpoint = self
            .execution
            .record_local_analysis(
                &proof,
                &StageClaim {
                    scope: StageScope {
                        owner: job.owner.clone(),
                        job: job.job,
                        stage: stage.stage,
                    },
                    token: claim,
                    generation: stage.claim_generation,
                },
                self.clock.now_ms(),
            )
            .await
            .map_err(RuntimeError::Execution)?;
        if checkpoint != proof.checkpoint {
            return Err(RuntimeError::InvalidRecord);
        }
        let selected = proof.final_checkpoint.clone();
        let proof = QualifiedAnalysis {
            checkpoint: selected,
            job: job.clone(),
            audience: intent.audience,
            sources: inputs.receipts.clone(),
        };
        self.execution
            .select_analysis_checkpoint(&proof, self.clock.now_ms())
            .await
            .map_err(RuntimeError::Execution)?;
        let mut context = self.event_context(&job, &stage, None);
        context.attempt = None;
        context.catalog_attempt = None;
        context.backend = None;
        context.purpose = Some(intent.purpose.purpose);
        context.analysis_kind = Some(telemetry_kind(&intent.operation));
        context.config_revision = Some(checkpoint.basis.config_revision);
        context.parent_attempt = checkpoint.basis.parent_dispatch;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::LocalAnalysisCompleted,
            context,
        )
        .elapsed(checkpoint_started.elapsed());
        event.measurements.supplier_dispatched = Some(false);
        event.measurements.raw_checkpointed = Some(false);
        event.measurements.analysis_verdict = Some(verdict(&result));
        self.telemetry.observe(event);
        Ok(None)
    }
    pub(super) async fn select_analysis_output(
        &self,
        context: &PublicationContext,
        result: &ImageAnalysisResult,
        maximum: usize,
    ) -> Result<(), RuntimeError> {
        let intent = analysis_codec::decode_analysis(context.job.intent.bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        crate::service::analysis::validate_result_binding(&intent.operation, result)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let (dependencies, dependency_refs) = self.analysis_dependencies(&intent).await?;
        let mut sources: Option<Vec<SourceReceipt>> = None;
        let expected_stages = match &intent.operation {
            ImageAnalysisOperation::Segmentation(spec) => spec.prompts.len(),
            _ => 1,
        };
        let mut cursor = Some(context.dispatch);
        let mut retained = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(dispatch) = cursor {
            if !seen.insert(dispatch) || seen.len() > expected_stages {
                return Err(RuntimeError::InvalidRecord);
            }
            let current = self
                .execution
                .read_dispatch(&context.job.owner, &dispatch)
                .await
                .map_err(RuntimeError::Execution)?;
            if current.job != context.job.job {
                return Err(RuntimeError::InvalidRecord);
            }
            let observation = self
                .execution
                .read_decision_checkpoint(
                    &context.job.owner,
                    &context.job.job,
                    &current.stage,
                    &dispatch,
                )
                .await
                .map_err(RuntimeError::Execution)?
                .ok_or(RuntimeError::InvalidRecord)?;
            let usage = self
                .usage
                .read_usage_revision(
                    &observation.scope,
                    crate::cost::UsageRevision(u64::from(observation.receipt.usage_revision)),
                )
                .await?
                .ok_or(RuntimeError::InvalidRecord)?;
            let consumed = self
                .usage
                .read_sources(&observation.scope)
                .await?
                .ok_or(RuntimeError::InvalidRecord)?;
            if consumed
                .iter()
                .map(|receipt| &receipt.pin)
                .ne(analysis_codec::sources(&intent.operation))
                || sources.as_ref().is_some_and(|sources| sources != &consumed)
            {
                return Err(RuntimeError::SourceDenied);
            }
            if sources.is_none() {
                sources = Some(consumed);
            }
            cursor = observation.basis.stage.parent_dispatch;
            retained.push(PublicationDecisionEvidence {
                decision: observation,
                usage,
                source_candidate: None,
            });
        }
        retained.reverse();
        let job = context.job.clone();
        let expected = result.clone();
        let sources = sources.ok_or(RuntimeError::InvalidRecord)?;
        let stages = tokio::task::spawn_blocking(move || {
            qualify_chain(&job, &retained, &expected, &dependencies)
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)??;
        let bytes = analysis_codec::encode_result(result, maximum)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let checkpoint = AnalysisCheckpoint {
            scope: StageScope {
                owner: context.job.owner.clone(),
                job: context.job.job,
                stage: context.stage.stage,
            },
            kind: decision_kind(&intent.operation),
            checkpoint_version: 1,
            schema_revision: 1,
            validator_revision: 1,
            result_sha256: Sha256::digest(&bytes).into(),
            result: crate::VersionedSnapshot::new(1, bytes, maximum)
                .map_err(|_| RuntimeError::InvalidRecord)?,
            intent_digest: Sha256::digest(context.job.intent.bytes()).into(),
            config_digest: Sha256::digest(context.job.config.bytes()).into(),
            source_digest: super::super::source_receipts_digest(&sources)?,
            stages,
            dependencies: dependency_refs,
        };
        let proof = QualifiedAnalysis {
            checkpoint,
            job: context.job.clone(),
            audience: intent.audience,
            sources,
        };
        self.execution
            .select_analysis_checkpoint(&proof, self.clock.now_ms())
            .await
            .map_err(RuntimeError::Execution)?;
        Ok(())
    }
}
fn qualify_chain(
    job: &ImageJob,
    retained: &[PublicationDecisionEvidence],
    expected: &ImageAnalysisResult,
    dependencies: &Dependencies,
) -> Result<Vec<AnalysisStageReference>, RuntimeError> {
    let intent = analysis_codec::decode_analysis(job.intent.bytes())
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let required = match &intent.operation {
        ImageAnalysisOperation::Segmentation(spec) => spec.prompts.len(),
        _ => 1,
    };
    if retained.len() != required {
        return Err(RuntimeError::InvalidRecord);
    }
    let mut previous = None;
    let mut parent = None;
    let mut stages = Vec::with_capacity(retained.len());
    for (index, evidence) in retained.iter().enumerate() {
        validate_standalone_evidence(job, evidence)?;
        if evidence.decision.basis.stage.parent_dispatch != parent {
            return Err(RuntimeError::InvalidRecord);
        }
        let (response, actual_dependencies, prior, prompt_index) = decode_raw(
            evidence
                .usage
                .raw_output
                .as_deref()
                .ok_or(RuntimeError::InvalidRecord)?,
        )?;
        if actual_dependencies != *dependencies || prior != previous || prompt_index != index {
            return Err(RuntimeError::InvalidRecord);
        }
        let (result, _) = decode_payload(evidence.decision.payload())?;
        if matches!(intent.operation, ImageAnalysisOperation::Segmentation(_))
            && response.segmentation.is_none()
        {
            return Err(RuntimeError::InvalidRecord);
        }
        previous = Some(result);
        parent = Some(evidence.decision.scope.dispatch);
        let decision = &evidence.decision;
        stages.push(AnalysisStageReference::Supplier(DecisionReference {
            scope: decision.scope.clone(),
            kind: decision.kind,
            observation_version: decision.observation_version,
            basis: decision.basis.clone(),
            receipt: decision.receipt,
            payload_sha256: *decision.payload_sha256(),
        }));
    }
    if previous.as_ref() != Some(expected) {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(stages)
}
pub(super) fn verdict(result: &ImageAnalysisResult) -> crate::telemetry::AnalysisVerdict {
    use crate::telemetry::AnalysisVerdict;
    match result {
        ImageAnalysisResult::PhotoInspection { assessment, .. } => match assessment.verdict {
            PhotoInspectionVerdict::Pass => AnalysisVerdict::Accepted,
            PhotoInspectionVerdict::Failed(_) => AnalysisVerdict::Rejected,
            PhotoInspectionVerdict::Unjudged(_) => AnalysisVerdict::Unjudged,
        },
        ImageAnalysisResult::HeadLocations { heads, .. } if heads.is_empty() => {
            AnalysisVerdict::Empty
        }
        ImageAnalysisResult::Segmentation { masks, .. }
            if masks.iter().all(|set| set.instances.is_empty()) =>
        {
            AnalysisVerdict::Empty
        }
        ImageAnalysisResult::StagingIdentity { matches, .. }
            if matches.iter().all(|matched| matched.candidate.is_none()) =>
        {
            AnalysisVerdict::Empty
        }
        ImageAnalysisResult::StagingFacing { facings, .. } if facings.is_empty() => {
            AnalysisVerdict::Empty
        }
        _ => AnalysisVerdict::Accepted,
    }
}

fn local_selection(checkpoint: &LocalAnalysisCheckpoint) -> AnalysisCheckpoint {
    AnalysisCheckpoint {
        scope: checkpoint.scope.clone(),
        kind: checkpoint.kind,
        checkpoint_version: 1,
        schema_revision: checkpoint.schema_revision,
        validator_revision: checkpoint.validator_revision,
        result: checkpoint.result.clone(),
        result_sha256: checkpoint.result_sha256,
        intent_digest: checkpoint.intent_digest,
        config_digest: checkpoint.config_digest,
        source_digest: checkpoint.source_digest,
        stages: vec![AnalysisStageReference::Local(
            LocalAnalysisReference::from_checkpoint(checkpoint),
        )],
        dependencies: checkpoint.dependencies.clone(),
    }
}
