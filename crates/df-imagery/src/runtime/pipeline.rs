//! Intermediate candidates and honest local finalization over one retained publication anchor.
use super::{dispatch, ports::*, worker::NativeImageRuntime};
use crate::cost::{ImageCostProjection, UsageRevision};
use crate::execution::*;
use crate::intent_codec::DecodedIntent;
use crate::publication::{
    FinalDecisionRef, FinalOutputSelection, FrozenOutput, PublishedImage, recipe_digest,
};
use crate::{BackendId, ImageConfig, ImageJob, ImageJournal, JobState, ServiceClock};
use df_assets::{AssetVariant, ImageAlphaRequirement, ImageIntent, ImageProofLimits};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::time::Duration;

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
    pub(super) async fn finish_candidate(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        raw: &RuntimeUsage,
        mode: DispatchMode,
    ) -> Result<(), RuntimeError> {
        let bytes = raw.raw_output.clone().ok_or(RuntimeError::InvalidRecord)?;
        let mime = match image::guess_format(&bytes) {
            Ok(image::ImageFormat::Png) => "image/png",
            Ok(image::ImageFormat::Jpeg) => "image/jpeg",
            Ok(image::ImageFormat::WebP) => "image/webp",
            _ => return Err(RuntimeError::ProviderRefused),
        };
        let size = intent
            .output
            .generated()
            .ok_or(RuntimeError::InvalidRecord)?;
        let (width_px, height_px) = if dispatch::backend(config, intent) == BackendId::CodexCli {
            // The Codex image tool chooses its own picture size and the later local transform
            // resizes it to the delivered profile, so the candidate keeps its real dimensions.
            image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .ok()
                .and_then(|reader| reader.into_dimensions().ok())
                .unwrap_or((size.width_px(), size.height_px()))
        } else {
            (size.width_px(), size.height_px())
        };
        let image_intent =
            ImageIntent::new(mime, width_px, height_px, ImageAlphaRequirement::Preserve)
                .map_err(|_| RuntimeError::InvalidRecord)?;
        let limits = ImageProofLimits::new(
            config.transforms.max_output_bytes,
            size.pixel_count()
                .max(u64::from(width_px) * u64::from(height_px)),
            config.transforms.max_decoded_bytes,
        )
        .map_err(|_| RuntimeError::InvalidConfiguration)?;
        let scope = ExecutionScope {
            owner: context.job.owner.clone(),
            job: context.job.job,
            stage: context.stage.stage,
            dispatch: context.dispatch,
        };
        let basis = self
            .execution
            .read_stage_basis(&scope.owner, &scope.job, &scope.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let receipt = ObservationReceipt {
            usage_revision: u32::try_from(raw.revision.0)
                .map_err(|_| RuntimeError::InvalidRecord)?,
            observed_at_ms: i64::try_from(raw.at_ms).map_err(|_| RuntimeError::InvalidRecord)?,
        };
        let observed = tokio::task::spawn_blocking(move || {
            CandidateObservation::inspect(scope, basis, 1, receipt, &bytes, limits, image_intent)
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?
        .map_err(|_| RuntimeError::ProviderRefused)?;
        let candidate = self
            .execution
            .record_candidate_observation(&observed)
            .await
            .map_err(RuntimeError::Execution)?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::CandidateCheckpointed,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        event.context.config_revision = Some(candidate.basis.config_revision);
        event.context.parent_attempt = candidate.basis.parent_dispatch;
        event.measurements.raw_checkpointed = Some(true);
        event.measurements.supplier_dispatched = Some(true);
        event.measurements.usage_revision = Some(raw.revision);
        event.measurements.output_bytes = Some(candidate.descriptor.byte_len);
        event.measurements.width_px = Some(candidate.descriptor.width_px);
        event.measurements.height_px = Some(candidate.descriptor.height_px);
        self.telemetry.observe(event);
        self.complete(
            context,
            CompletionOutcome::Candidate {
                observation_version: 1,
                usage_revision: receipt.usage_revision,
            },
        )
        .await?;
        // The candidate is technically valid; customer usability is settled from actual delivery.
        let _held = mode;
        self.project_usage_or_pending(raw).await?;
        self.advance_candidate(context, intent, config, &candidate)
            .await
    }

    pub(super) async fn advance_candidate(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        candidate: &CandidateReceipt,
    ) -> Result<(), RuntimeError> {
        if context.recovery.is_some()
            && (context.job.state == JobState::Retired
                || self.clock.now_ms() >= context.job.absolute_deadline_ms)
        {
            return self
                .reconcile_selected_output(context, intent, config, candidate)
                .await;
        }
        if candidate.basis.kind != StageKind::Extraction
            && intent
                .operation
                .pipeline()
                .is_some_and(|pipeline| pipeline.inspection.is_some())
        {
            self.enqueue_candidate_dependency(context, candidate, StageKind::Inspection, None)
                .await?;
            return Ok(());
        }
        if candidate.basis.kind != StageKind::Extraction && dispatch::extracts_alpha(config, intent)
        {
            self.enqueue_candidate_dependency(context, candidate, StageKind::Extraction, None)
                .await?;
            return Ok(());
        }
        let selected = self
            .compose_selection(&context.job, candidate, None)
            .await?;
        self.enqueue_candidate_dependency(
            context,
            candidate,
            StageKind::LocalTransform,
            Some(&selected),
        )
        .await?;
        Ok(())
    }

    pub(super) async fn enqueue_candidate_dependency(
        &self,
        context: &PublicationContext,
        candidate: &CandidateReceipt,
        kind: StageKind,
        selection: Option<&FinalOutputSelection>,
    ) -> Result<EnqueueOutcome, RuntimeError> {
        self.enqueue_dependency_for(&context.job, &context.stage, candidate, kind, selection)
            .await
    }

    pub(super) async fn enqueue_dependency_for(
        &self,
        job: &ImageJob,
        publication_stage: &StageView,
        candidate: &CandidateReceipt,
        kind: StageKind,
        selection: Option<&FinalOutputSelection>,
    ) -> Result<EnqueueOutcome, RuntimeError> {
        let job = self
            .journal
            .read_job(&job.owner, job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        if job.state == JobState::Retired {
            return Err(RuntimeError::Cancelled);
        }
        if self.clock.now_ms() >= job.absolute_deadline_ms {
            return Err(RuntimeError::Expired);
        }
        let (input_revision, input_digest) =
            selection.map_or((1, dependency_digest(candidate, kind)), |selection| {
                (
                    selection.resolved_recipe.revision(),
                    recipe_digest(&selection.resolved_recipe),
                )
            });
        let stage = ExecutionStageId::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let command = EnqueueCommand::new(
            StageScope {
                owner: job.owner.clone(),
                job: job.job,
                stage,
            },
            StageBasis {
                kind,
                input_revision,
                input_digest,
                config_revision: candidate.basis.config_revision,
                config_digest: candidate.basis.config_digest,
                parent_dispatch: Some(candidate.scope.dispatch),
            },
            super::planned_publication(&publication_stage)?,
            self.settings.capacity.clone(),
            kind == StageKind::LocalTransform || publication_stage.local_accounting,
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
        event.context.config_revision = Some(command.config_revision);
        event.context.parent_attempt = command.parent_dispatch;
        self.telemetry.observe(event);
        Ok(outcome)
    }

    pub(super) async fn compose_selection(
        &self,
        job: &ImageJob,
        candidate: &CandidateReceipt,
        inspection: Option<&DecisionObservation>,
    ) -> Result<FinalOutputSelection, RuntimeError> {
        let plan = job.recipe.as_ref().ok_or(RuntimeError::InvalidRecord)?;
        let frozen = FrozenOutput::decode(plan).map_err(RuntimeError::Publication)?;
        let discovered;
        let inspection = if inspection.is_none() && candidate.basis.kind != StageKind::Extraction {
            discovered = if let Some(stage) = self
                .execution
                .read_child_stage(
                    &job.owner,
                    &job.job,
                    &candidate.scope.dispatch,
                    StageKind::Inspection,
                )
                .await
                .map_err(RuntimeError::Execution)?
            {
                if let Some(dispatch) = self
                    .execution
                    .read_current_dispatch(&job.owner, &job.job, &stage.stage)
                    .await
                    .map_err(RuntimeError::Execution)?
                {
                    self.execution
                        .read_decision_checkpoint(
                            &job.owner,
                            &job.job,
                            &stage.stage,
                            &dispatch.dispatch,
                        )
                        .await
                        .map_err(RuntimeError::Execution)?
                } else {
                    None
                }
            } else {
                None
            };
            discovered.as_ref()
        } else {
            inspection
        };
        let mut cursor = Some(inspection.map_or(candidate.scope.dispatch, |inspection| {
            inspection.scope.dispatch
        }));
        let mut seen = BTreeSet::new();
        let mut decisions = Vec::new();
        let mut candidates = Vec::new();
        while let Some(dispatch) = cursor {
            if !seen.insert(dispatch) {
                return Err(RuntimeError::InvalidRecord);
            }
            let current = self
                .execution
                .read_dispatch(&job.owner, &dispatch)
                .await
                .map_err(RuntimeError::Execution)?;
            if current.job != job.job || current.state != DispatchState::Succeeded {
                return Err(RuntimeError::InvalidRecord);
            }
            let basis = self
                .execution
                .read_stage_basis(&job.owner, &job.job, &current.stage)
                .await
                .map_err(RuntimeError::Execution)?;
            match basis.kind {
                StageKind::Classification | StageKind::Inspection => {
                    let observed = self
                        .execution
                        .read_decision_checkpoint(&job.owner, &job.job, &current.stage, &dispatch)
                        .await
                        .map_err(RuntimeError::Execution)?
                        .ok_or(RuntimeError::InvalidRecord)?;
                    decisions.push(observed);
                }
                StageKind::Generation
                | StageKind::Edit
                | StageKind::Correction
                | StageKind::Extraction => {
                    let observed = self
                        .execution
                        .read_candidate_checkpoint(&job.owner, &job.job, &current.stage, &dispatch)
                        .await
                        .map_err(RuntimeError::Execution)?
                        .ok_or(RuntimeError::InvalidRecord)?;
                    candidates.push(observed);
                }
                StageKind::LocalTransform
                | StageKind::PhotoInspection
                | StageKind::HeadLocations
                | StageKind::Segmentation
                | StageKind::StagingIdentity
                | StageKind::StagingFacing => return Err(RuntimeError::InvalidRecord),
            }
            if seen.len() > crate::publication::MAXIMUM_RECIPE_BYTES / 16 {
                return Err(RuntimeError::InvalidRecord);
            }
            cursor = basis.parent_dispatch;
        }
        decisions.reverse();
        candidates.sort_by_key(|candidate| candidate.scope.dispatch);
        let mut local_inspections = Vec::new();
        for source in &candidates {
            if let Some(stage) = self
                .execution
                .read_child_stage(
                    &job.owner,
                    &job.job,
                    &source.scope.dispatch,
                    StageKind::Inspection,
                )
                .await
                .map_err(RuntimeError::Execution)?
            {
                if let Some(local) = self
                    .execution
                    .read_local_inspection(&job.owner, &job.job, &stage.stage)
                    .await
                    .map_err(RuntimeError::Execution)?
                {
                    local_inspections.push(local);
                }
            }
        }
        local_inspections.sort_by_key(|local| local.scope.stage);
        let sources = self
            .usage
            .read_sources(&candidate.scope)
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        let mut execution = frozen.execution_recipe().to_vec();
        execution.extend_from_slice(b"df-imagery/final-selection-v1");
        let evidence=serde_json::json!({
            "original_plan_digest":recipe_digest(plan),
            "selected":candidate.scope.dispatch.as_bytes(),
            "candidates":candidates.iter().map(candidate_value).collect::<Vec<_>>(),
            "decisions":decisions.iter().map(|decision|serde_json::json!({
                "stage":decision.scope.stage.as_bytes(),"dispatch":decision.scope.dispatch.as_bytes(),
                "kind":decision.kind.label(),"observation_version":decision.observation_version,
                "usage_revision":decision.receipt.usage_revision,"schema_revision":decision.basis.schema_revision,
                "validator_revision":decision.basis.validator_revision,"payload":decision.payload(),
            })).collect::<Vec<_>>(),
            "local_inspections":local_inspections.iter().map(|local|serde_json::json!({
                "stage":local.scope.stage.as_bytes(),"candidate_stage":local.candidate_scope.stage.as_bytes(),
                "candidate_dispatch":local.candidate_scope.dispatch.as_bytes(),"source_digest":local.source_digest,
                "candidate_observation_version":local.candidate_observation_version,"candidate_usage_revision":local.candidate_usage_revision,
                "schema_revision":local.schema_revision,"validator_revision":local.validator_revision,"reason":local.reason.label(),
                "observed_at_ms":local.observed_at_ms,"input_revision":local.basis.input_revision,
                "input_digest":local.basis.input_digest,"config_revision":local.basis.config_revision,"config_digest":local.basis.config_digest,
            })).collect::<Vec<_>>(),
            "sources":sources.iter().map(|source|serde_json::json!({"asset":source.pin.asset.as_bytes(),
                "digest":source.pin.digest.as_bytes(),"attempt":source.attempt.as_bytes(),"write":source.write.as_bytes(),
                "version":source.version.as_str(),"suppression_generation":source.suppression_generation,
                "retention":source.retention.label()})).collect::<Vec<_>>(),
        }).to_string().into_bytes();
        execution.extend_from_slice(
            &u32::try_from(evidence.len())
                .map_err(|_| RuntimeError::InvalidRecord)?
                .to_be_bytes(),
        );
        execution.extend_from_slice(&evidence);
        let resolved = FrozenOutput::new(
            frozen
                .intent(AssetVariant::Master)
                .ok_or(RuntimeError::InvalidRecord)?,
            frozen.intent(AssetVariant::Display),
            execution,
        )
        .map_err(RuntimeError::Publication)?
        .snapshot()
        .map_err(RuntimeError::Publication)?;
        let source_stage = self
            .execution
            .read_stage(&job.owner, &job.job, &candidate.scope.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let anchor = self
            .execution
            .resolve(&job.owner, &super::planned_publication(&source_stage)?)
            .await
            .map_err(RuntimeError::Execution)?;
        Ok(FinalOutputSelection {
            allocation_stage: anchor.stage,
            candidate: Some(candidate.clone()),
            resolved_recipe: resolved,
            local_inspections,
            decisions: decisions
                .iter()
                .map(FinalDecisionRef::from_observation)
                .collect(),
        })
    }

    pub(super) async fn run_final_stage(
        &self,
        job: ImageJob,
        stage: StageView,
        intent: &DecodedIntent,
        config: &ImageConfig,
    ) -> Result<PublishedImage, RuntimeError> {
        if self
            .execution
            .read_current_dispatch(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?
            .is_some()
        {
            return Err(RuntimeError::ProviderUnknown);
        }
        let basis = self
            .execution
            .read_stage_basis(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let parent = basis.parent_dispatch.ok_or(RuntimeError::InvalidRecord)?;
        let parent_dispatch = self
            .execution
            .read_dispatch(&job.owner, &parent)
            .await
            .map_err(RuntimeError::Execution)?;
        let candidate = self
            .execution
            .read_candidate_checkpoint(&job.owner, &job.job, &parent_dispatch.stage, &parent)
            .await
            .map_err(RuntimeError::Execution)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let selection = self.compose_selection(&job, &candidate, None).await?;
        if basis.input_digest != recipe_digest(&selection.resolved_recipe) {
            return Err(RuntimeError::InvalidRecord);
        }
        let now = self.clock.now_ms();
        let lease_ms = super::lease::claim_lease_ms(config, job.absolute_deadline_ms, now)?;
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
        let dispatch = DispatchId::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let job = self
            .journal
            .read_job(&job.owner, job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let context = PublicationContext {
            job,
            stage,
            dispatch,
            claim,
            now_ms: now,
            usage_revision: 0,
            recovery: None,
        };
        let result = async {
            let sources = self
                .usage
                .read_sources(&candidate.scope)
                .await?
                .ok_or(RuntimeError::InvalidRecord)?;
            let input = DispatchAdmission {
                job: &context.job,
                stage: &context.stage,
                dispatch,
                claim,
                config,
                quote: None,
                sources: &sources,
            };
            let mode = self.admission.admit(&input).await?;
            if mode != DispatchMode::Local {
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
            self.usage
                .record_sources(
                    &ExecutionScope {
                        owner: context.job.owner.clone(),
                        job: context.job.job,
                        stage: context.stage.stage,
                        dispatch,
                    },
                    &sources,
                )
                .await?;
            if self.admission.mark_sent(&input, mode).await? != SendAuthorization::New {
                return Err(RuntimeError::ProviderUnknown);
            }
            self.finish_final_stage(&context, intent, config, &candidate, &selection)
                .await
        }
        .await;
        if result
            .as_ref()
            .is_err_and(|error| *error == RuntimeError::SourceDenied)
        {
            self.close_denied_sources(&context).await?;
        }
        result
    }

    pub(super) async fn settle_image_candidate(
        &self,
        job: &ImageJob,
        candidate: &CandidateReceipt,
    ) -> Result<bool, RuntimeError> {
        let Some(disposition) = self.publisher.image_economic_use(&candidate.scope).await? else {
            return Ok(false);
        };
        if disposition.usage_revision != candidate.receipt.usage_revision {
            return Err(RuntimeError::InvalidRecord);
        }
        let usage = self
            .usage
            .read_usage_revision(
                &candidate.scope,
                UsageRevision(u64::from(disposition.usage_revision)),
            )
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        let dispatch = self
            .execution
            .read_dispatch(&job.owner, &candidate.scope.dispatch)
            .await
            .map_err(RuntimeError::Execution)?;
        let mode = dispatch
            .finance
            .map_or(DispatchMode::Local, DispatchMode::Hosted);
        self.admission
            .outcome(
                job,
                candidate.scope.dispatch,
                mode,
                if usage.evidence == crate::cost::UsageEvidence::Unknown {
                    None
                } else {
                    Some(disposition.usable)
                },
            )
            .await?;
        self.record_settlement(job, dispatch.dispatch, dispatch.stage)
            .await;
        self.project_usage_or_pending(&usage).await?;
        Ok(true)
    }

    pub(super) async fn settle_image_chain(
        &self,
        job: &ImageJob,
        selected: &CandidateReceipt,
    ) -> Result<(), RuntimeError> {
        let mut cursor = Some(selected.scope.dispatch);
        let mut seen = BTreeSet::new();
        while let Some(dispatch) = cursor {
            if !seen.insert(dispatch) {
                return Err(RuntimeError::InvalidRecord);
            }
            let current = self
                .execution
                .read_dispatch(&job.owner, &dispatch)
                .await
                .map_err(RuntimeError::Execution)?;
            if current.job != job.job {
                return Err(RuntimeError::InvalidRecord);
            }
            let basis = self
                .execution
                .read_stage_basis(&job.owner, &job.job, &current.stage)
                .await
                .map_err(RuntimeError::Execution)?;
            if matches!(
                basis.kind,
                StageKind::Generation
                    | StageKind::Edit
                    | StageKind::Correction
                    | StageKind::Extraction
            ) {
                let candidate = self
                    .execution
                    .read_candidate_checkpoint(&job.owner, &job.job, &current.stage, &dispatch)
                    .await
                    .map_err(RuntimeError::Execution)?
                    .ok_or(RuntimeError::InvalidRecord)?;
                if !self.settle_image_candidate(job, &candidate).await? {
                    return Err(RuntimeError::InvalidRecord);
                }
            }
            if seen.len() > crate::publication::MAXIMUM_RECIPE_BYTES / 16 {
                return Err(RuntimeError::InvalidRecord);
            }
            cursor = basis.parent_dispatch;
        }
        Ok(())
    }

    pub(super) async fn finish_final_stage(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        candidate: &CandidateReceipt,
        selection: &FinalOutputSelection,
    ) -> Result<PublishedImage, RuntimeError> {
        let result =
            Box::pin(self.finish_final_stage_inner(context, intent, config, candidate, selection))
                .await;
        if result
            .as_ref()
            .is_err_and(|error| *error == RuntimeError::SourceDenied)
        {
            self.close_denied_sources(context).await?;
        }
        result
    }

    async fn finish_final_stage_inner(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        candidate: &CandidateReceipt,
        selection: &FinalOutputSelection,
    ) -> Result<PublishedImage, RuntimeError> {
        self.publisher.select_output(context, selection).await?;
        let transform_started = std::time::Instant::now();
        let raw = self
            .usage
            .read_usage_revision(
                &candidate.scope,
                UsageRevision(u64::from(candidate.receipt.usage_revision)),
            )
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        let image = dispatch::ExecutedImage {
            bytes: raw.raw_output.ok_or(RuntimeError::InvalidRecord)?,
            units: Vec::new(),
            evidence: crate::cost::UsageEvidence::Estimated,
        };
        let remaining = context
            .recovery
            .as_ref()
            .map_or(context.stage.lease_until_ms, |lease| lease.lease_until_ms)
            .checked_sub(self.clock.now_ms())
            .filter(|remaining| *remaining > 0)
            .ok_or(RuntimeError::Expired)?;
        let timeout =
            Duration::from_millis(u64::try_from(remaining).map_err(|_| RuntimeError::Expired)?);
        self.emit(
            crate::telemetry::EventKind::TransformStarted,
            &context.job,
            &context.stage,
            Some(context.dispatch),
        );
        let image = dispatch::normalize_output(context, intent, config, image, timeout).await?;
        self.telemetry.observe(
            crate::telemetry::ImageEvent::new(
                crate::telemetry::EventKind::TransformCompleted,
                self.event_context(&context.job, &context.stage, Some(context.dispatch)),
            )
            .elapsed(transform_started.elapsed()),
        );
        let checkpoint = self
            .publisher
            .store_output(
                context,
                &PreparedImageOutput {
                    master: image.bytes,
                    display: None,
                },
            )
            .await?;
        self.emit(
            crate::telemetry::EventKind::OutputStored,
            &context.job,
            &context.stage,
            Some(context.dispatch),
        );
        self.complete(
            context,
            CompletionOutcome::Succeeded {
                usage_revision: checkpoint.usage_revision,
                checkpoint,
            },
        )
        .await?;
        self.admission
            .outcome(
                &context.job,
                context.dispatch,
                DispatchMode::Local,
                Some(true),
            )
            .await?;
        let result = self.publish_output(context).await?;
        self.publication_ack(context, &result).await;
        self.settle_image_chain(&context.job, candidate).await?;
        self.completed(context, &result).await;
        Ok(result)
    }
}

fn dependency_digest(candidate: &CandidateReceipt, kind: StageKind) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"df-imagery/candidate-child-v1");
    digest.update(kind.label());
    digest.update(candidate.scope.dispatch.as_bytes());
    digest.update(candidate.descriptor.sha256);
    digest.update(candidate.basis.input_digest);
    digest.update(candidate.basis.config_digest);
    digest.finalize().into()
}
fn candidate_value(candidate: &CandidateReceipt) -> serde_json::Value {
    serde_json::json!({"stage":candidate.scope.stage.as_bytes(),"dispatch":candidate.scope.dispatch.as_bytes(),
        "kind":candidate.basis.kind.label(),"input_digest":candidate.basis.input_digest,
        "config_revision":candidate.basis.config_revision,"config_digest":candidate.basis.config_digest,
        "parent":candidate.basis.parent_dispatch.map(|parent|*parent.as_bytes()),"observation_version":candidate.observation_version,
        "usage_revision":candidate.receipt.usage_revision,"mime":candidate.descriptor.mime,
        "width_px":candidate.descriptor.width_px,"height_px":candidate.descriptor.height_px,
        "alpha":match candidate.descriptor.alpha {CheckpointAlpha::Opaque=>"opaque",CheckpointAlpha::Transparent=>"transparent"},
        "byte_len":candidate.descriptor.byte_len,"sha256":candidate.descriptor.sha256,
        "validator_revision":candidate.descriptor.validator_revision})
}
