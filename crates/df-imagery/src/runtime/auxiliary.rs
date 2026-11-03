//! Separately admitted inspection and alpha stages over retained candidate bytes.
use super::{
    analysis, dispatch, inspection_policy, ports::*, qualification, worker::NativeImageRuntime,
};
use crate::backends::analysis::{InspectionInput, PreparedInspection};
use crate::cost::{ImageCostProjection, UsageEvidence, UsageRevision};
use crate::execution::*;
use crate::intent_codec::DecodedIntent;
use crate::{ImageConfig, ImageJob, ImageJournal, ServiceClock};
use inspection_policy::{
    InspectionAssessment, InspectionVerdict, PhotoCheckFailure, UnjudgedReason,
};

struct AuxiliaryAdmission<'a> {
    job: ImageJob,
    stage: StageView,
    config: &'a ImageConfig,
    route: &'a crate::cost::CostRoute,
    sources: &'a [SourceReceipt],
    lease_ms: i64,
    claim: Option<ClaimToken>,
    quota: Option<super::quota::QuotaAdmission>,
}
struct AdmittedAuxiliary {
    context: PublicationContext,
    mode: DispatchMode,
    quota: Option<super::quota::QuotaPermit>,
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
    pub(super) async fn source_candidate(
        &self,
        job: &ImageJob,
        stage: &StageView,
    ) -> Result<CandidateReceipt, RuntimeError> {
        let basis = self
            .execution
            .read_stage_basis(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let parent = basis.parent_dispatch.ok_or(RuntimeError::InvalidRecord)?;
        let parent = self
            .execution
            .read_dispatch(&job.owner, &parent)
            .await
            .map_err(RuntimeError::Execution)?;
        let parent_basis = self
            .execution
            .read_stage_basis(&job.owner, &job.job, &parent.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let source = if parent_basis.kind == StageKind::Inspection {
            let candidate = parent_basis
                .parent_dispatch
                .ok_or(RuntimeError::InvalidRecord)?;
            self.execution
                .read_dispatch(&job.owner, &candidate)
                .await
                .map_err(RuntimeError::Execution)?
        } else {
            parent
        };
        if source.job != job.job || source.state != DispatchState::Succeeded {
            return Err(RuntimeError::InvalidRecord);
        }
        self.execution
            .read_candidate_checkpoint(&job.owner, &job.job, &source.stage, &source.dispatch)
            .await
            .map_err(RuntimeError::Execution)?
            .ok_or(RuntimeError::InvalidRecord)
    }

    async fn claim_auxiliary(
        &self,
        job: &ImageJob,
        stage: &StageView,
        lease_ms: i64,
    ) -> Result<(StageView, ClaimToken), RuntimeError> {
        let claim = ClaimToken::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let claimed = self
            .execution
            .claim(
                &ClaimCommand::new(
                    job.owner.clone(),
                    job.job,
                    stage.stage,
                    self.settings.active_config_revision,
                    claim,
                    lease_ms,
                    self.clock.now_ms(),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        self.emit(crate::telemetry::EventKind::Claimed, job, &claimed, None);
        Ok((claimed, claim))
    }

    async fn admit_auxiliary(
        &self,
        input: AuxiliaryAdmission<'_>,
    ) -> Result<AdmittedAuxiliary, RuntimeError> {
        let (stage, claim) = if let Some(claim) = input.claim {
            (input.stage, claim)
        } else {
            self.claim_auxiliary(&input.job, &input.stage, input.lease_ms)
                .await?
        };
        let quota = super::quota_gate::claim(input.quota)?;
        let dispatch = DispatchId::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let job = self
            .journal
            .read_job(&input.job.owner, input.job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let mut context = PublicationContext {
            job,
            stage,
            dispatch,
            claim,
            now_ms: self.clock.now_ms(),
            usage_revision: 0,
            recovery: None,
        };
        let admission = DispatchAdmission {
            job: &context.job,
            stage: &context.stage,
            dispatch,
            claim,
            config: input.config,
            quote: Some(input.route),
            sources: input.sources,
        };
        let mode = self.admission.admit(&admission).await?;
        if mode.is_local() != context.stage.local_accounting {
            return Err(RuntimeError::InvalidRecord);
        }
        self.execution
            .prepare(
                &PrepareCommand::new(
                    StageClaim {
                        scope: StageScope {
                            owner: context.job.owner.clone(),
                            job: context.job.job,
                            stage: context.stage.stage,
                        },
                        token: claim,
                        generation: context.stage.claim_generation,
                    },
                    dispatch,
                    mode,
                    self.clock.now_ms(),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        // The dispatch row was created after the context was built; later writes must not stamp
        // an earlier time than its creation.
        context.now_ms = self.clock.now_ms();
        self.emit(
            crate::telemetry::EventKind::Admitted,
            &context.job,
            &context.stage,
            Some(dispatch),
        );
        self.usage
            .record_sources(
                &ExecutionScope {
                    owner: context.job.owner.clone(),
                    job: context.job.job,
                    stage: context.stage.stage,
                    dispatch,
                },
                input.sources,
            )
            .await?;
        self.costs
            .start_pinned(
                &crate::cost::pinned_call_id(dispatch.as_bytes()),
                input.route,
            )
            .await
            .map_err(RuntimeError::Cost)?;
        if self.admission.mark_sent(&admission, mode).await? != SendAuthorization::New {
            self.complete(&context, CompletionOutcome::Unknown).await?;
            return Err(RuntimeError::ProviderUnknown);
        }
        self.execution
            .mark_sent(
                &MarkSentCommand::new(
                    context.job.owner.clone(),
                    context.job.job,
                    context.stage.stage,
                    dispatch,
                    claim,
                    context.stage.claim_generation,
                    self.clock.now_ms(),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        self.emit(
            crate::telemetry::EventKind::Sent,
            &context.job,
            &context.stage,
            Some(dispatch),
        );
        Ok(AdmittedAuxiliary {
            context,
            mode,
            quota,
        })
    }

    pub(super) async fn run_extraction_stage(
        &self,
        job: ImageJob,
        stage: StageView,
        intent: &DecodedIntent,
        config: &ImageConfig,
    ) -> Result<(), RuntimeError> {
        let candidate = self.source_candidate(&job, &stage).await?;
        let sources = self
            .usage
            .read_sources(&candidate.scope)
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        let inputs = dispatch::resolve_inputs(intent, &self.sources).await?;
        if inputs.receipts != sources {
            return Err(RuntimeError::SourceDenied);
        }
        let raw = self
            .usage
            .read_usage_revision(
                &candidate.scope,
                UsageRevision(u64::from(candidate.receipt.usage_revision)),
            )
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        let source = raw.raw_output.ok_or(RuntimeError::InvalidRecord)?;
        let lease_ms = stage_lease_ms(&job, config, self.clock.now_ms())?;
        let mut basis = super::retained_cost_basis(&job, config, StageKind::Extraction)?;
        basis.backend = Some(crate::BackendId::Fal);
        basis.model = config
            .backend(crate::BackendId::Fal)
            .alpha_model
            .clone()
            .unwrap_or(
                crate::ModelName::new(crate::backends::api::fal::BACKGROUND_REMOVAL)
                    .map_err(|_| RuntimeError::InvalidConfiguration)?,
            );
        basis.prompt_bytes = 0;
        basis.reference_count = 1;
        basis.generated = None;
        basis.generation_quality = None;
        let route = self.cost_policy.stage_route(&job, config, &basis)?;
        let credentials = dispatch::pin_backend_credentials(
            crate::BackendId::Fal,
            &config.backend(crate::BackendId::Fal).account,
            &self.credentials,
        )?;
        let quota =
            dispatch::admit_image_quota(config, intent, self.settings.config.quota, &credentials)?;
        let admitted = self
            .admit_auxiliary(AuxiliaryAdmission {
                job,
                stage,
                config,
                route: &route,
                sources: &sources,
                lease_ms,
                claim: None,
                quota,
            })
            .await?;
        let context = admitted.context;
        let timeout = super::lease::call_budget(config, lease_ms)?;
        let execution_started = std::time::Instant::now();
        let executed = dispatch::execute_alpha(
            dispatch::DispatchInput {
                context: &context,
                intent,
                config,
                timeout,
                resume: false,
                telemetry: &self.telemetry,
                event_context: self.event_context(
                    &context.job,
                    &context.stage,
                    Some(context.dispatch),
                ),
            },
            &self.execution,
            Some(source),
            &credentials,
        )
        .await;
        let execution_elapsed = execution_started.elapsed();
        let mut event = crate::telemetry::ImageEvent::new(
            if executed.is_ok() {
                crate::telemetry::EventKind::SupplierCompleted
            } else {
                crate::telemetry::EventKind::Unknown
            },
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        )
        .elapsed(execution_elapsed);
        event.measurements.supplier_dispatched = Some(true);
        event.measurements.raw_checkpointed = Some(false);
        if let Ok(image) = &executed {
            event.measurements.output_bytes = u64::try_from(image.bytes.len()).ok();
        }
        self.telemetry.observe(event);
        super::quota_gate::observe(
            admitted.quota,
            match &executed {
                Ok(_) => super::quota::QuotaAnswer::Succeeded,
                Err(failure) if failure.error == RuntimeError::SupplierQuotaExhausted => {
                    super::quota::QuotaAnswer::Exhausted
                }
                Err(_) => super::quota::QuotaAnswer::Inconclusive,
            },
            &self.telemetry,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        self.finish_extraction(&context, intent, config, &route, admitted.mode, executed)
            .await
    }

    pub(super) async fn finish_extraction(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        route: &crate::cost::CostRoute,
        mode: DispatchMode,
        executed: Result<dispatch::ExecutedImage, dispatch::DispatchFailure>,
    ) -> Result<(), RuntimeError> {
        match executed {
            Ok(image) => {
                let raw = self
                    .retain_usage(
                        context,
                        route,
                        image.evidence,
                        &image.units,
                        Some(&image.bytes),
                        true,
                    )
                    .await?;
                self.finish_candidate(context, intent, config, &raw, mode)
                    .await
            }
            Err(failure) => {
                let observed = failure
                    .usage
                    .or_else(|| failure.api.as_deref().map(dispatch::retain_api_failure));
                let raw = self
                    .retain_usage(
                        context,
                        route,
                        observed
                            .as_ref()
                            .map_or(UsageEvidence::Unknown, |usage| usage.evidence),
                        observed
                            .as_ref()
                            .map_or(&[], |usage| usage.units.as_slice()),
                        observed.as_ref().map(|usage| usage.raw.as_slice()),
                        false,
                    )
                    .await?;
                self.complete(
                    context,
                    failure
                        .error
                        .failure_cause()
                        .map_or(CompletionOutcome::Unknown, |cause| {
                            CompletionOutcome::UnknownWithCause { cause }
                        }),
                )
                .await?;
                self.admission
                    .outcome(&context.job, context.dispatch, mode, None)
                    .await?;
                self.project_usage(&raw).await?;
                Err(failure.error)
            }
        }
    }

    pub(super) async fn run_inspection_stage(
        &self,
        job: ImageJob,
        stage: StageView,
        intent: &DecodedIntent,
        config: &ImageConfig,
    ) -> Result<(), RuntimeError> {
        let spec = intent
            .operation
            .pipeline()
            .and_then(|pipeline| pipeline.inspection.as_ref())
            .ok_or(RuntimeError::InvalidRecord)?;
        if config.analysis.inspection.as_ref() != Some(&spec.route) {
            return Err(RuntimeError::SourceDenied);
        }
        let candidate = self.source_candidate(&job, &stage).await?;
        let sources = self
            .usage
            .read_sources(&candidate.scope)
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        let inputs = dispatch::resolve_inputs(intent, &self.sources).await?;
        if inputs.receipts != sources {
            return Err(RuntimeError::SourceDenied);
        }
        let raw = self
            .usage
            .read_usage_revision(
                &candidate.scope,
                UsageRevision(u64::from(candidate.receipt.usage_revision)),
            )
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        let bytes = raw.raw_output.ok_or(RuntimeError::InvalidRecord)?;
        let maximum = usize::try_from(
            self.execution
                .read_capacity_bounds(self.settings.capacity.as_str())
                .await
                .map_err(RuntimeError::Execution)?
                .max_decision_payload_bytes,
        )
        .map_err(|_| RuntimeError::InvalidConfiguration)?;
        let lease_ms = stage_lease_ms(&job, config, self.clock.now_ms())?;
        let timeout = super::lease::call_budget(config, lease_ms)?;
        let credential = match self.credentials.analysis_credential(&spec.route.account) {
            Ok(credential) => credential,
            Err(RuntimeError::CredentialUnavailable) => {
                let (stage, claim) = self.claim_auxiliary(&job, &stage, lease_ms).await?;
                return self
                    .inspection_preflight_failed(
                        &job,
                        &stage,
                        claim,
                        &candidate,
                        &sources,
                        LocalInspectionReason::Disabled,
                    )
                    .await;
            }
            Err(error) => return Err(error),
        };
        let quota = super::quota_gate::admit(
            &self.credentials,
            self.settings.config.quota,
            spec.route.endpoint.as_str(),
            &credential,
            None,
        )?;
        let (stage, claim) = self.claim_auxiliary(&job, &stage, lease_ms).await?;
        let owned_spec = spec.clone();
        let references = inputs.images;
        let memory = config.transforms.max_decoded_bytes;
        let preparation = config.analysis.preparation;
        let prepared = tokio::task::spawn_blocking(move || {
            PreparedInspection::prepare(
                InspectionInput {
                    spec: &owned_spec,
                    candidate: &bytes,
                    references: &references,
                    timeout,
                    maximum_response_bytes: maximum,
                    max_decoded_bytes: memory,
                    preparation: &preparation,
                },
                &credential,
            )
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?;
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(_) => {
                return self
                    .inspection_preflight_failed(
                        &job,
                        &stage,
                        claim,
                        &candidate,
                        &sources,
                        LocalInspectionReason::Preparation,
                    )
                    .await;
            }
        };
        let mut basis = super::retained_cost_basis(&job, config, StageKind::Inspection)?;
        basis.prompt_bytes = prepared.text_bytes;
        basis.reference_count = prepared.picture_count;
        basis.generated = None;
        let route = self.cost_policy.stage_route(&job, config, &basis)?;
        let admitted = self
            .admit_auxiliary(AuxiliaryAdmission {
                job,
                stage,
                config,
                route: &route,
                sources: &sources,
                lease_ms,
                claim: Some(claim),
                quota,
            })
            .await?;
        let context = admitted.context;
        let execution_started = std::time::Instant::now();
        let response = prepared.execute().await;
        let execution_elapsed = execution_started.elapsed();
        let (retained, evidence, units, quota_answer) = match response {
            Ok(response) => {
                let usage = analysis::parse_response(&response.body, maximum)
                    .as_ref()
                    .map(analysis::reported_usage)
                    .unwrap_or((UsageEvidence::Unknown, Vec::new()));
                (
                    analysis::retain_response(response.status, &response.body),
                    usage.0,
                    usage.1,
                    Some((response.status, response.body)),
                )
            }
            Err(error) => (
                qualification::retain_inspection_failure(match error {
                    RuntimeError::Expired => UnjudgedReason::Deadline,
                    RuntimeError::Cancelled => UnjudgedReason::Cancelled,
                    _ => UnjudgedReason::Transport,
                }),
                UsageEvidence::Unknown,
                Vec::new(),
                None,
            ),
        };
        let assessment = qualification::replay_inspection(&retained, spec)?;
        let useful = !matches!(assessment.verdict, InspectionVerdict::Unjudged(_));
        super::quota_gate::observe(
            admitted.quota,
            quota_answer
                .as_ref()
                .map_or(super::quota::QuotaAnswer::Inconclusive, |(status, body)| {
                    super::quota_gate::raw_answer(*status, body, useful)
                }),
            &self.telemetry,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        let raw = self
            .retain_usage(&context, &route, evidence, &units, Some(&retained), useful)
            .await?;
        let mut event = crate::telemetry::ImageEvent::new(
            if quota_answer.is_some() {
                crate::telemetry::EventKind::SupplierCompleted
            } else {
                crate::telemetry::EventKind::Unknown
            },
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        )
        .elapsed(execution_elapsed);
        event.measurements.supplier_dispatched = Some(true);
        event.measurements.raw_checkpointed = Some(true);
        event.measurements.provider_status = quota_answer.as_ref().map(|(status, _)| *status);
        self.telemetry.observe(event);
        self.finish_inspection(
            &context,
            &candidate,
            admitted.mode,
            &raw,
            maximum,
            assessment,
        )
        .await
    }

    async fn inspection_preflight_failed(
        &self,
        job: &ImageJob,
        stage: &StageView,
        claim: ClaimToken,
        candidate: &CandidateReceipt,
        sources: &[SourceReceipt],
        reason: LocalInspectionReason,
    ) -> Result<(), RuntimeError> {
        let checkpoint_started = std::time::Instant::now();
        let job = self
            .journal
            .read_job(&job.owner, job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let stage_claim = StageClaim {
            scope: StageScope {
                owner: job.owner.clone(),
                job: job.job,
                stage: stage.stage,
            },
            token: claim,
            generation: stage.claim_generation,
        };
        let source_digest = super::source_receipts_digest(sources)?;
        let intent = crate::intent_codec::decode_intent(
            job.intent.bytes(),
            &crate::service::PROTOCOL_LIMITS,
            crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let spec = intent
            .operation
            .pipeline()
            .and_then(|pipeline| pipeline.inspection.as_ref())
            .ok_or(RuntimeError::InvalidRecord)?;
        if spec.requirement == crate::InspectionRequirement::Required {
            return self
                .finish_required_preflight(&job, &stage_claim, candidate, source_digest, reason)
                .await;
        }
        let command = LocalInspectionCommand::new(
            &job,
            stage_claim,
            candidate.scope.dispatch,
            source_digest,
            reason,
            self.clock.now_ms(),
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        self.execution
            .record_local_inspection(&command)
            .await
            .map_err(RuntimeError::Execution)?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::LocalAnalysisCompleted,
            self.event_context(&job, stage, None),
        )
        .elapsed(checkpoint_started.elapsed());
        event.measurements.supplier_dispatched = Some(false);
        event.measurements.analysis_verdict = Some(crate::telemetry::AnalysisVerdict::Unjudged);
        event.context.parent_attempt = Some(candidate.scope.dispatch);
        event.context.config_revision = Some(candidate.basis.config_revision);
        self.telemetry.observe(event);
        let config = super::retained_config(&job)?;
        let selection = self.compose_selection(&job, candidate, None).await?;
        let kind = if dispatch::extracts_alpha(&config, &intent) {
            StageKind::Extraction
        } else {
            StageKind::LocalTransform
        };
        self.enqueue_dependency_for(
            &job,
            stage,
            candidate,
            kind,
            if kind == StageKind::LocalTransform {
                Some(&selection)
            } else {
                None
            },
        )
        .await?;
        Ok(())
    }

    pub(super) async fn finish_inspection(
        &self,
        context: &PublicationContext,
        candidate: &CandidateReceipt,
        mode: DispatchMode,
        raw: &RuntimeUsage,
        maximum: usize,
        assessment: InspectionAssessment,
    ) -> Result<(), RuntimeError> {
        let basis = self
            .execution
            .read_stage_basis(&context.job.owner, &context.job.job, &context.stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let usage_revision =
            u32::try_from(raw.revision.0).map_err(|_| RuntimeError::InvalidRecord)?;
        let decision = DecisionObservation::new(
            ExecutionScope {
                owner: context.job.owner.clone(),
                job: context.job.job,
                stage: context.stage.stage,
                dispatch: context.dispatch,
            },
            DecisionKind::Inspection,
            1,
            DecisionBasis {
                schema_revision: 1,
                validator_revision: 1,
                stage: basis,
            },
            assessment.to_value().to_string().into_bytes(),
            ObservationReceipt {
                usage_revision,
                observed_at_ms: i64::try_from(raw.at_ms)
                    .map_err(|_| RuntimeError::InvalidRecord)?,
            },
            maximum,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        self.execution
            .record_decision_observation(&decision)
            .await
            .map_err(RuntimeError::Execution)?;
        self.decision_checkpointed(
            context,
            &decision,
            raw,
            match assessment.verdict {
                InspectionVerdict::Pass => crate::telemetry::AnalysisVerdict::Accepted,
                InspectionVerdict::Failed(_) => crate::telemetry::AnalysisVerdict::Rejected,
                InspectionVerdict::Unjudged(_) => crate::telemetry::AnalysisVerdict::Unjudged,
            },
        );
        self.complete(
            context,
            CompletionOutcome::Decision {
                observation_version: 1,
                usage_revision,
            },
        )
        .await?;
        self.settle_decision(
            context,
            mode,
            raw,
            decision.clone(),
            Some(candidate.clone()),
        )
        .await?;
        self.project_usage_or_pending(raw).await?;
        let intent = crate::intent_codec::decode_intent(
            context.job.intent.bytes(),
            &crate::service::PROTOCOL_LIMITS,
            crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let config = super::retained_config(&context.job)?;
        self.advance_inspection(context, &intent, &config, candidate, &decision, &assessment)
            .await
    }

    pub(super) async fn advance_inspection(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        candidate: &CandidateReceipt,
        decision: &DecisionObservation,
        assessment: &InspectionAssessment,
    ) -> Result<(), RuntimeError> {
        let spec = intent
            .operation
            .pipeline()
            .and_then(|pipeline| pipeline.inspection.as_ref())
            .ok_or(RuntimeError::InvalidRecord)?;
        let accepted = matches!(assessment.verdict, InspectionVerdict::Pass)
            || matches!(assessment.verdict, InspectionVerdict::Unjudged(_))
                && spec.requirement == crate::request::InspectionRequirement::BestEffort
            || matches!(
                assessment.verdict,
                InspectionVerdict::Failed(PhotoCheckFailure::VisibleText)
            ) && candidate.basis.kind == StageKind::Correction
                && spec.maximum_corrections == 1;
        if accepted {
            if context.recovery.is_some()
                && (context.job.state == crate::JobState::Retired
                    || self.clock.now_ms() >= context.job.absolute_deadline_ms)
            {
                return self
                    .reconcile_selected_output(context, intent, config, candidate)
                    .await;
            }
            if dispatch::extracts_alpha(config, intent) {
                self.enqueue_inspected_alpha(context, candidate, decision)
                    .await?;
                return Ok(());
            }
            let selected = self
                .compose_selection(&context.job, candidate, Some(decision))
                .await?;
            self.enqueue_candidate_dependency(
                context,
                candidate,
                StageKind::LocalTransform,
                Some(&selected),
            )
            .await?;
            return Ok(());
        }
        if matches!(assessment.verdict, InspectionVerdict::Failed(_))
            && candidate.basis.kind != StageKind::Correction
            && spec.maximum_corrections > 0
        {
            self.enqueue_correction(context, candidate, decision)
                .await?;
            return Ok(());
        }
        self.finish_rejected_output(context, Some(candidate)).await
    }

    async fn enqueue_inspected_alpha(
        &self,
        context: &PublicationContext,
        candidate: &CandidateReceipt,
        decision: &DecisionObservation,
    ) -> Result<(), RuntimeError> {
        let command = EnqueueCommand::new(
            StageScope {
                owner: context.job.owner.clone(),
                job: context.job.job,
                stage: ExecutionStageId::from_bytes(&self.ids.next_bytes())
                    .map_err(|_| RuntimeError::InvalidRecord)?,
            },
            StageBasis {
                kind: StageKind::Extraction,
                input_revision: 1,
                input_digest: candidate.descriptor.sha256,
                config_revision: candidate.basis.config_revision,
                config_digest: candidate.basis.config_digest,
                parent_dispatch: Some(decision.scope.dispatch),
            },
            super::planned_publication(&context.stage)?,
            self.settings.capacity.clone(),
            context.stage.local_accounting,
            self.clock.now_ms(),
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let outcome = self
            .execution
            .enqueue(&command)
            .await
            .map_err(RuntimeError::Execution)?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::Queued,
            self.event_context(&context.job, outcome.view(), None),
        );
        event.context.parent_attempt = command.parent_dispatch;
        event.context.config_revision = Some(command.config_revision);
        self.telemetry.observe(event);
        Ok(())
    }

    async fn enqueue_correction(
        &self,
        context: &PublicationContext,
        candidate: &CandidateReceipt,
        decision: &DecisionObservation,
    ) -> Result<(), RuntimeError> {
        let mut hash = sha2::Sha256::new();
        use sha2::Digest;
        hash.update(b"df-imagery/correction-input-v1");
        hash.update(candidate.descriptor.sha256);
        hash.update(decision.payload());
        hash.update(context.job.intent.bytes());
        let input_digest = hash.finalize().into();
        let stage = ExecutionStageId::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let command = EnqueueCommand::new(
            StageScope {
                owner: context.job.owner.clone(),
                job: context.job.job,
                stage,
            },
            StageBasis {
                kind: StageKind::Correction,
                input_revision: 1,
                input_digest,
                config_revision: candidate.basis.config_revision,
                config_digest: candidate.basis.config_digest,
                parent_dispatch: Some(decision.scope.dispatch),
            },
            super::planned_publication(&context.stage)?,
            self.settings.capacity.clone(),
            context.stage.local_accounting,
            self.clock.now_ms(),
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let outcome = self
            .execution
            .enqueue(&command)
            .await
            .map_err(RuntimeError::Execution)?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::Queued,
            self.event_context(&context.job, outcome.view(), None),
        );
        event.context.parent_attempt = command.parent_dispatch;
        event.context.config_revision = Some(command.config_revision);
        self.telemetry.observe(event);
        Ok(())
    }
}

fn stage_lease_ms(job: &ImageJob, config: &ImageConfig, now_ms: i64) -> Result<i64, RuntimeError> {
    super::lease::claim_lease_ms(config, job.absolute_deadline_ms, now_ms)
}
