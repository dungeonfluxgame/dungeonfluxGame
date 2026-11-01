//! Paid creator classification and its exactly-once generation dependency.

use super::{analysis, dispatch, ports::*, worker::NativeImageRuntime};
use crate::cost::{ImageCostProjection, UsageEvidence};
use crate::execution::*;
use crate::intent_codec::DecodedIntent;
use crate::service::types::{CreatorAssessment, CreatorRoute};
use crate::{ImageConfig, ImageJob, ImageJournal, ImageOperation, JobState, ServiceClock};
use sha2::{Digest, Sha256};

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
    pub(super) async fn classified_config(
        &self,
        job: &ImageJob,
        stage: &StageView,
        config: &ImageConfig,
    ) -> Result<(ImageConfig, Option<DecisionObservation>), RuntimeError> {
        let mut basis = self
            .execution
            .read_stage_basis(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let mut seen = std::collections::BTreeSet::new();
        let (dispatch, parent) = loop {
            let Some(parent) = basis.parent_dispatch else {
                return Ok((config.clone(), None));
            };
            if !seen.insert(parent) {
                return Err(RuntimeError::InvalidRecord);
            }
            let dispatch = self
                .execution
                .read_dispatch(&job.owner, &parent)
                .await
                .map_err(RuntimeError::Execution)?;
            if dispatch.job != job.job {
                return Err(RuntimeError::InvalidRecord);
            }
            basis = self
                .execution
                .read_stage_basis(&job.owner, &job.job, &dispatch.stage)
                .await
                .map_err(RuntimeError::Execution)?;
            if basis.kind == StageKind::Classification {
                break (dispatch, parent);
            }
        };
        let observation = self
            .execution
            .read_decision_checkpoint(&job.owner, &job.job, &dispatch.stage, &parent)
            .await
            .map_err(RuntimeError::Execution)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let value = analysis::parse_response(observation.payload(), observation.payload().len())?;
        let route = value
            .get("route")
            .and_then(serde_json::Value::as_str)
            .and_then(CreatorRoute::parse_label)
            .ok_or(RuntimeError::InvalidRecord)?;
        let mut resolved = config.clone();
        match route {
            CreatorRoute::General => {}
            CreatorRoute::Bfl => {
                let intent = crate::intent_codec::decode_intent(
                    job.intent.bytes(),
                    &crate::service::PROTOCOL_LIMITS,
                    crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
                )
                .map_err(|_| RuntimeError::InvalidRecord)?;
                let purpose = &mut resolved.purposes[intent.purpose.purpose.index()];
                if dispatch::backend(config, &intent) != crate::BackendId::Bfl {
                    purpose.account_override = config.creator_bfl_account.clone();
                    purpose.model_override = None;
                }
                purpose.backend_override = Some(crate::BackendId::Bfl);
            }
            CreatorRoute::Revise | CreatorRoute::Uncertain => {
                return Err(RuntimeError::ProviderRefused);
            }
        }
        Ok((resolved, Some(observation)))
    }

    pub(super) async fn execution_intent(
        &self,
        job: &ImageJob,
        stage: &StageView,
        original: &DecodedIntent,
        classification: Option<&DecisionObservation>,
    ) -> Result<DecodedIntent, RuntimeError> {
        let mut operation = original.operation.clone();
        let prompt = match &mut operation {
            ImageOperation::Generate(spec) => &mut spec.prompt,
            ImageOperation::Edit(spec) => &mut spec.prompt,
            ImageOperation::Transform(_) => return Ok(clone_intent(original, operation)),
        };
        let mut pipeline = prompt.pipeline().cloned();
        let mut text = prompt.text().to_owned();
        if let Some(classification) = classification {
            let payload =
                analysis::parse_response(classification.payload(), classification.payload().len())?;
            let category = payload
                .get("category")
                .and_then(serde_json::Value::as_str)
                .and_then(crate::CreatorCategory::parse_label)
                .ok_or(RuntimeError::InvalidRecord)?;
            if matches!(
                category,
                crate::CreatorCategory::General
                    | crate::CreatorCategory::AdultNonsexualAnatomyDetail
            ) {
                if let Some(pipeline) = &mut pipeline {
                    if let Some(general) = pipeline
                        .creator
                        .as_ref()
                        .and_then(|creator| creator.general_prompt.as_ref())
                    {
                        text = general.text().to_owned();
                    }
                    pipeline.rating = crate::ImageContentRating::Restrained;
                }
            }
        }
        if stage.kind == StageKind::Correction {
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
            let decision = self
                .execution
                .read_decision_checkpoint(&job.owner, &job.job, &parent.stage, &parent.dispatch)
                .await
                .map_err(RuntimeError::Execution)?
                .ok_or(RuntimeError::InvalidRecord)?;
            let spec = pipeline
                .as_ref()
                .and_then(|pipeline| pipeline.inspection.as_ref())
                .ok_or(RuntimeError::InvalidRecord)?;
            let value = analysis::parse_response(decision.payload(), decision.payload().len())?;
            let assessment =
                super::inspection_policy::InspectionAssessment::from_value(&value, spec)
                    .map_err(|_| RuntimeError::InvalidRecord)?;
            let super::inspection_policy::InspectionVerdict::Failed(failure) = assessment.verdict
            else {
                return Err(RuntimeError::InvalidRecord);
            };
            text = format!("{}\n\n{}", text, failure.correction());
        }
        let mut selected = crate::TextPrompt::new(&text, prompt.template_version)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        if let Some(pipeline) = pipeline {
            selected = selected
                .with_pipeline(pipeline)
                .map_err(|_| RuntimeError::InvalidRecord)?;
        }
        *prompt = selected;
        Ok(clone_intent(original, operation))
    }

    pub(super) async fn run_classification(
        &self,
        job: ImageJob,
        stage: StageView,
        intent: &DecodedIntent,
        config: &ImageConfig,
    ) -> Result<(), RuntimeError> {
        let creator = intent
            .operation
            .pipeline()
            .and_then(|pipeline| pipeline.creator.as_ref())
            .ok_or(RuntimeError::InvalidRecord)?;
        if config.analysis.classification.as_ref() != Some(&creator.route) {
            return Err(RuntimeError::SourceDenied);
        }
        if job.state != JobState::Planning {
            return Err(RuntimeError::InvalidRecord);
        }
        let now = self.clock.now_ms();
        let lease_ms = super::lease::claim_lease_ms(config, job.absolute_deadline_ms, now)?;
        let timeout = super::lease::call_budget(config, lease_ms)?;
        let bounds = self
            .execution
            .read_capacity_bounds(self.settings.capacity.as_str())
            .await
            .map_err(RuntimeError::Execution)?;
        let maximum = usize::try_from(bounds.max_decision_payload_bytes)
            .map_err(|_| RuntimeError::InvalidConfiguration)?;
        let credential = self
            .credentials
            .analysis_credential(&creator.route.account)?;
        let quota = super::quota_gate::admit(
            &self.credentials,
            self.settings.config.quota,
            creator.route.endpoint.as_str(),
            &credential,
            None,
        )?;
        let prepared =
            analysis::PreparedClassification::prepare(creator, &credential, timeout, maximum)?;
        let inputs = dispatch::resolve_inputs(intent, &self.sources).await?;
        let claim = ClaimToken::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let stage = self
            .execution
            .claim(
                &ClaimCommand::new(
                    job.owner.clone(),
                    job.job,
                    stage.stage,
                    self.settings.active_config_revision,
                    claim,
                    lease_ms,
                    now,
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        self.emit(crate::telemetry::EventKind::Claimed, &job, &stage, None);
        let quota = super::quota_gate::claim(quota)?;
        let job = self
            .journal
            .read_job(&job.owner, job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let dispatch = DispatchId::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let mut basis = super::retained_cost_basis(&job, config, StageKind::Classification)?;
        basis.prompt_bytes = prepared.body_bytes;
        basis.reference_count = 0;
        basis.generated = None;
        let route = self.cost_policy.stage_route(&job, config, &basis)?;
        let mode = self
            .admission
            .admit(&DispatchAdmission {
                job: &job,
                stage: &stage,
                dispatch,
                claim,
                config,
                quote: Some(&route),
                sources: &inputs.receipts,
            })
            .await?;
        if mode.is_local() != stage.local_accounting {
            return Err(RuntimeError::InvalidRecord);
        }
        self.execution
            .prepare(
                &PrepareCommand::new(
                    StageClaim {
                        scope: StageScope {
                            owner: job.owner.clone(),
                            job: job.job,
                            stage: stage.stage,
                        },
                        token: claim,
                        generation: stage.claim_generation,
                    },
                    dispatch,
                    mode,
                    self.clock.now_ms(),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        let context = PublicationContext {
            job,
            stage,
            dispatch,
            claim,
            now_ms: self.clock.now_ms(),
            usage_revision: 0,
            recovery: None,
        };
        self.emit(
            crate::telemetry::EventKind::Admitted,
            &context.job,
            &context.stage,
            Some(dispatch),
        );
        let scope = ExecutionScope {
            owner: context.job.owner.clone(),
            job: context.job.job,
            stage: context.stage.stage,
            dispatch,
        };
        self.usage.record_sources(&scope, &inputs.receipts).await?;
        self.costs
            .start_pinned(&crate::cost::pinned_call_id(dispatch.as_bytes()), &route)
            .await
            .map_err(RuntimeError::Cost)?;
        if self
            .admission
            .mark_sent(
                &DispatchAdmission {
                    job: &context.job,
                    stage: &context.stage,
                    dispatch,
                    claim: context.claim,
                    config,
                    quote: Some(&route),
                    sources: &inputs.receipts,
                },
                mode,
            )
            .await?
            != SendAuthorization::New
        {
            return self.classification_unknown(&context, &route, mode).await;
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
        let execution_started = std::time::Instant::now();
        let response = prepared.execute().await;
        let execution_elapsed = execution_started.elapsed();
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                let mut event = crate::telemetry::ImageEvent::new(
                    crate::telemetry::EventKind::Unknown,
                    self.event_context(&context.job, &context.stage, Some(dispatch)),
                )
                .elapsed(execution_elapsed)
                .failed(crate::telemetry::FailureKind::ProviderUnknown);
                event.measurements.supplier_dispatched = Some(true);
                event.measurements.raw_checkpointed = Some(false);
                self.telemetry.observe(event);
                return self.classification_unknown(&context, &route, mode).await;
            }
        };
        let parsed = analysis::parse_response(&response.body, maximum);
        let (evidence, units) = parsed
            .as_ref()
            .map(analysis::reported_usage)
            .unwrap_or((UsageEvidence::Unknown, Vec::new()));
        let rating = intent
            .operation
            .pipeline()
            .ok_or(RuntimeError::InvalidRecord)?
            .rating;
        let assessment = if response.status == 200 {
            parsed.and_then(|value| analysis::classify(&value, creator, rating))
        } else {
            Err(RuntimeError::ProviderRefused)
        };
        super::quota_gate::observe(
            quota,
            super::quota_gate::raw_answer(response.status, &response.body, assessment.is_ok()),
            &self.telemetry,
            self.event_context(&context.job, &context.stage, Some(dispatch)),
        );
        let raw = self
            .retain_usage(
                &context,
                &route,
                evidence,
                &units,
                Some(&analysis::retain_response(response.status, &response.body)),
                assessment.is_ok(),
            )
            .await?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::SupplierCompleted,
            self.event_context(&context.job, &context.stage, Some(dispatch)),
        )
        .elapsed(execution_elapsed);
        event.measurements.supplier_dispatched = Some(true);
        event.measurements.raw_checkpointed = Some(true);
        event.measurements.provider_status = Some(response.status);
        self.telemetry.observe(event);
        self.finish_classification(&context, mode, &raw, maximum, assessment)
            .await
    }

    async fn classification_unknown(
        &self,
        context: &PublicationContext,
        route: &crate::cost::CostRoute,
        mode: DispatchMode,
    ) -> Result<(), RuntimeError> {
        let raw = self
            .retain_usage(context, route, UsageEvidence::Unknown, &[], None, false)
            .await?;
        self.complete(context, CompletionOutcome::Unknown).await?;
        self.admission
            .outcome(&context.job, context.dispatch, mode, None)
            .await?;
        self.project_usage(&raw).await?;
        Err(RuntimeError::ProviderUnknown)
    }

    pub(super) async fn finish_classification(
        &self,
        context: &PublicationContext,
        mode: DispatchMode,
        raw: &RuntimeUsage,
        maximum: usize,
        assessment: Result<CreatorAssessment, RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let usage_revision =
            u32::try_from(raw.revision.0).map_err(|_| RuntimeError::InvalidRecord)?;
        let assessment = match assessment {
            Ok(assessment) => assessment,
            Err(_) => {
                let definitive = raw.evidence != UsageEvidence::Unknown;
                self.complete(
                    context,
                    if definitive {
                        CompletionOutcome::Unusable {
                            cause: FailureCause::InvalidOutput,
                            usage_revision,
                        }
                    } else {
                        CompletionOutcome::Unknown
                    },
                )
                .await?;
                if definitive {
                    self.finish_no_output(context, raw, mode).await?;
                } else {
                    self.admission
                        .outcome(&context.job, context.dispatch, mode, None)
                        .await?;
                    self.project_usage(raw).await?;
                }
                return Err(if definitive {
                    RuntimeError::ProviderRefused
                } else {
                    RuntimeError::ProviderUnknown
                });
            }
        };
        let basis = self
            .execution
            .read_stage_basis(&context.job.owner, &context.job.job, &context.stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let observation = DecisionObservation::new(
            ExecutionScope {
                owner: context.job.owner.clone(),
                job: context.job.job,
                stage: context.stage.stage,
                dispatch: context.dispatch,
            },
            DecisionKind::Classification,
            1,
            DecisionBasis {
                schema_revision: 1,
                validator_revision: 1,
                stage: basis,
            },
            analysis::assessment_payload(assessment),
            ObservationReceipt {
                usage_revision,
                observed_at_ms: i64::try_from(raw.at_ms)
                    .map_err(|_| RuntimeError::InvalidRecord)?,
            },
            maximum,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        self.execution
            .record_decision_observation(&observation)
            .await
            .map_err(RuntimeError::Execution)?;
        self.decision_checkpointed(
            context,
            &observation,
            raw,
            if matches!(assessment.route, CreatorRoute::General | CreatorRoute::Bfl) {
                crate::telemetry::AnalysisVerdict::Accepted
            } else if assessment.route == CreatorRoute::Uncertain {
                crate::telemetry::AnalysisVerdict::Unjudged
            } else {
                crate::telemetry::AnalysisVerdict::Rejected
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
        self.settle_decision(context, mode, raw, observation, None)
            .await?;
        self.project_usage(raw).await?;
        let intent = crate::intent_codec::decode_intent(
            context.job.intent.bytes(),
            &crate::service::PROTOCOL_LIMITS,
            crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let config = super::retained_config(&context.job)?;
        self.enqueue_classified_image(context, &intent, &config, assessment)
            .await
    }

    pub(super) fn decision_checkpointed(
        &self,
        context: &PublicationContext,
        decision: &DecisionObservation,
        usage: &RuntimeUsage,
        verdict: crate::telemetry::AnalysisVerdict,
    ) {
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::DecisionCheckpointed,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        event.context.config_revision = Some(decision.basis.stage.config_revision);
        event.context.parent_attempt = decision.basis.stage.parent_dispatch;
        event.measurements.raw_checkpointed = Some(true);
        event.measurements.supplier_dispatched = Some(true);
        event.measurements.usage_revision = Some(usage.revision);
        event.measurements.analysis_verdict = Some(verdict);
        self.telemetry.observe(event);
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::AnalysisCompleted,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        event.context.config_revision = Some(decision.basis.stage.config_revision);
        event.context.parent_attempt = decision.basis.stage.parent_dispatch;
        event.measurements.analysis_verdict = Some(verdict);
        self.telemetry.observe(event);
    }
    pub(super) async fn record_settlement(
        &self,
        job: &ImageJob,
        dispatch: DispatchId,
        stage: ExecutionStageId,
    ) {
        let context = crate::telemetry::EventContext {
            job: Some(job.job),
            stage: Some(stage),
            attempt: Some(dispatch),
            gate_revision: Some(self.settings.active_config_revision),
            ..Default::default()
        };
        let event = match self.execution.read_dispatch(&job.owner, &dispatch).await {
            Ok(actual) => {
                let mut event = crate::telemetry::ImageEvent::new(
                    crate::telemetry::EventKind::SettlementRecorded,
                    context,
                );
                event.measurements.settlement = Some(actual.settlement);
                event
            }
            Err(_) => {
                crate::telemetry::ImageEvent::new(crate::telemetry::EventKind::Failed, context)
                    .failed(crate::telemetry::FailureKind::Journal)
            }
        };
        self.telemetry.observe(event);
    }

    pub(super) async fn settle_decision(
        &self,
        context: &PublicationContext,
        mode: DispatchMode,
        raw: &RuntimeUsage,
        decision: DecisionObservation,
        source_candidate: Option<CandidateReceipt>,
    ) -> Result<(), RuntimeError> {
        let job = context.job.clone();
        let evidence = super::PublicationDecisionEvidence {
            decision,
            usage: raw.clone(),
            source_candidate,
        };
        let qualified =
            tokio::task::spawn_blocking(move || super::qualify_economic_decision(&job, &evidence))
                .await
                .map_err(|_| RuntimeError::WorkerFailed)??;
        self.admission
            .outcome_decision(&DispatchDecisionSettlement {
                job: &context.job,
                mode,
                usage: raw,
                qualified: &qualified,
            })
            .await?;
        self.record_settlement(&context.job, context.dispatch, context.stage.stage)
            .await;
        Ok(())
    }

    async fn enqueue_classified_image(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        assessment: CreatorAssessment,
    ) -> Result<(), RuntimeError> {
        if !matches!(assessment.route, CreatorRoute::General | CreatorRoute::Bfl) {
            return self.finish_rejected_output(context, None).await;
        }
        let job = self
            .journal
            .read_job(&context.job.owner, context.job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        if job.state == JobState::Retired || self.clock.now_ms() >= job.absolute_deadline_ms {
            return Ok(());
        }
        let kind = match intent.operation {
            ImageOperation::Generate(_) => StageKind::Generation,
            ImageOperation::Edit(_) => StageKind::Edit,
            _ => return Err(RuntimeError::InvalidRecord),
        };
        let parent_basis = self
            .execution
            .read_stage_basis(&job.owner, &job.job, &context.stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let command = EnqueueCommand::new(
            StageScope {
                owner: job.owner.clone(),
                job: job.job,
                stage: ExecutionStageId::from_bytes(&self.ids.next_bytes())
                    .map_err(|_| RuntimeError::InvalidRecord)?,
            },
            StageBasis {
                kind,
                input_revision: job.intent.revision(),
                input_digest: Sha256::digest(job.intent.bytes()).into(),
                config_revision: parent_basis.config_revision,
                config_digest: parent_basis.config_digest,
                parent_dispatch: Some(context.dispatch),
            },
            super::planned_publication(&context.stage)?,
            self.settings.capacity.clone(),
            config.accounting.mode == crate::AccountingMode::RecordOnly,
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
            self.event_context(&job, outcome.view(), None),
        );
        event.context.parent_attempt = command.parent_dispatch;
        event.context.config_revision = Some(command.config_revision);
        self.telemetry.observe(event);
        Ok(())
    }
}

fn clone_intent(original: &DecodedIntent, operation: ImageOperation) -> DecodedIntent {
    DecodedIntent {
        owner: original.owner.clone(),
        purpose: original.purpose,
        operation,
        execution: original.execution,
        freshness: original.freshness,
        audience: original.audience.clone(),
        expected_source_revision: original.expected_source_revision,
        output: original.output.clone(),
    }
}
