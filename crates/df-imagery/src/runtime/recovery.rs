//! Reconciliation uses retained dispatch ownership and receipts; it never submits a new send.

use super::{dispatch, ports::*, worker::NativeImageRuntime};
use crate::cost::ImageCostProjection;
use crate::execution::*;
use crate::publication::PublishedImage;
use crate::{ImageJournal, ImageOperation, ServiceClock};
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
    pub(super) async fn recover_next(&self) -> Result<Option<PublishedImage>, RuntimeError> {
        let now_ms = self.clock.now_ms();
        let entries = self
            .execution
            .scan_recovery(self.settings.capacity.as_str(), None, 1, now_ms)
            .await
            .map_err(RuntimeError::Execution)?;
        let Some(entry) = entries.first() else {
            return Ok(None);
        };
        let lease_ms = i64::try_from(self.settings.config.scheduler.stage_deadline_secs)
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .checked_mul(1000)
            .ok_or(RuntimeError::InvalidConfiguration)?;
        let token = ClaimToken::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let lease = match self
            .execution
            .claim_recovery(&ClaimRecoveryCommand {
                owner: entry.owner.clone(),
                job: entry.job,
                stage: entry.stage,
                dispatch: entry.current.dispatch,
                claim: token,
                lease_ms,
                now_ms,
            })
            .await
        {
            Ok(lease) => lease,
            Err(ExecutionError::LostClaim | ExecutionError::Superseded) => return Ok(None),
            Err(error) => return Err(RuntimeError::Execution(error)),
        };
        let recovery_started = std::time::Instant::now();
        let mut recovery_context = None;
        if let Some(job) = self
            .journal
            .read_job(&entry.owner, entry.job)
            .await
            .map_err(RuntimeError::Journal)?
        {
            let stage = self
                .execution
                .read_stage(&entry.owner, &entry.job, &entry.stage)
                .await
                .map_err(RuntimeError::Execution)?;
            let context = self.event_context(&job, &stage, Some(entry.current.dispatch));
            self.telemetry.observe(crate::telemetry::ImageEvent::new(
                crate::telemetry::EventKind::RecoveryStarted,
                context.clone(),
            ));
            recovery_context = Some(context);
        }
        let result = Box::pin(self.reconcile(&lease)).await;
        let terminal = self
            .execution
            .read_dispatch(&entry.owner, &entry.current.dispatch)
            .await
            .map_err(RuntimeError::Execution)?
            .settlement;
        let disposition = if result.is_ok()
            && matches!(
                terminal,
                DispatchSettlement::Settled
                    | DispatchSettlement::UnusableSettled
                    | DispatchSettlement::Released
            ) {
            RecoveryDisposition::Completed
        } else {
            RecoveryDisposition::RetryAt {
                next_at_ms: self
                    .clock
                    .now_ms()
                    .checked_add(lease_ms)
                    .ok_or(RuntimeError::InvalidConfiguration)?,
            }
        };
        let recovery_completed = matches!(disposition, RecoveryDisposition::Completed);
        self.execution
            .finish_recovery(&FinishRecoveryCommand {
                owner: entry.owner.clone(),
                job: entry.job,
                stage: entry.stage,
                dispatch: entry.current.dispatch,
                claim: lease.token,
                claim_generation: lease.generation,
                disposition,
                now_ms: self.clock.now_ms(),
            })
            .await
            .map_err(RuntimeError::Execution)?;
        if let Some(context) = recovery_context {
            let kind = match &result {
                Ok(_) if recovery_completed => crate::telemetry::EventKind::RecoveryCompleted,
                Ok(_) | Err(RuntimeError::CostProjectionPending) => {
                    crate::telemetry::EventKind::RecoveryDeferred
                }
                Err(RuntimeError::ProviderUnknown) => crate::telemetry::EventKind::Unknown,
                Err(_) => crate::telemetry::EventKind::Failed,
            };
            self.telemetry.observe(
                crate::telemetry::ImageEvent::new(kind, context)
                    .elapsed(recovery_started.elapsed()),
            );
        }
        match result {
            Ok(result) => Ok(result),
            Err(
                RuntimeError::ProviderUnknown
                | RuntimeError::SourceDenied
                | RuntimeError::Disabled
                | RuntimeError::Expired
                | RuntimeError::Cancelled
                | RuntimeError::CostProjectionPending
                | RuntimeError::Execution(
                    ExecutionError::LostClaim
                    | ExecutionError::Superseded
                    | ExecutionError::CapacityFull
                    | ExecutionError::OutputSealed
                    | ExecutionError::Stale { .. },
                ),
            ) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn reconcile(
        &self,
        lease: &RecoveryLease,
    ) -> Result<Option<PublishedImage>, RuntimeError> {
        let recovery = &lease.stage;
        let job = self
            .journal
            .read_job(&recovery.owner, recovery.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        // A retained technical decline never authorizes polling or repeating an old submission.
        if recovery.current.state == DispatchState::Declined {
            self.reconcile_decline(&job, lease).await?;
            return Ok(None);
        }
        if crate::service::analysis_codec::is_analysis_intent(job.intent.bytes()) {
            return Box::pin(self.reconcile_standalone(job, lease)).await;
        }
        let config = super::retained_config(&job)?;
        let intent = crate::intent_codec::decode_intent(
            job.intent.bytes(),
            &crate::service::PROTOCOL_LIMITS,
            crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let mut stage = self
            .execution
            .read_stage(&recovery.owner, &recovery.job, &recovery.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        stage.claim_generation = recovery.claim_generation;
        let mut context = PublicationContext {
            job,
            stage,
            dispatch: recovery.current.dispatch,
            claim: recovery.claim,
            now_ms: self.clock.now_ms(),
            usage_revision: recovery.current.usage_revision,
            recovery: Some(lease.clone()),
        };
        let mode = recovery
            .current
            .finance
            .map_or(DispatchMode::Local, DispatchMode::Hosted);
        if context.stage.kind == StageKind::Classification
            && recovery.current.state != DispatchState::Failed
        {
            let raw = self
                .usage
                .latest_usage(&context.job.owner, context.dispatch)
                .await?
                .filter(|usage| usage.raw_output.is_some())
                .ok_or(RuntimeError::ProviderUnknown)?;
            let maximum = usize::try_from(
                self.execution
                    .read_capacity_bounds(self.settings.capacity.as_str())
                    .await
                    .map_err(RuntimeError::Execution)?
                    .max_decision_payload_bytes,
            )
            .map_err(|_| RuntimeError::InvalidConfiguration)?;
            let pipeline = intent
                .operation
                .pipeline()
                .ok_or(RuntimeError::InvalidRecord)?;
            let creator = pipeline
                .creator
                .as_ref()
                .ok_or(RuntimeError::InvalidRecord)?;
            let assessment = super::analysis::restore_response(
                raw.raw_output
                    .as_deref()
                    .ok_or(RuntimeError::InvalidRecord)?,
                maximum,
            )
            .and_then(|(status, value)| {
                if status != 200 {
                    return Err(RuntimeError::ProviderRefused);
                }
                super::analysis::classify(&value, creator, pipeline.rating)
            });
            self.finish_classification(&context, mode, &raw, maximum, assessment)
                .await?;
            return Ok(None);
        }
        if context.stage.kind == StageKind::Extraction
            && matches!(
                recovery.current.state,
                DispatchState::Sent | DispatchState::Unknown
            )
        {
            self.reconcile_extraction(&context, &intent, &config, mode)
                .await?;
            return Ok(None);
        }
        if context.stage.kind == StageKind::Inspection
            && recovery.current.state != DispatchState::Failed
        {
            self.reconcile_inspection(&context, &intent, &config, mode, recovery.current.state)
                .await?;
            return Ok(None);
        }
        let (config, _classification) = self
            .classified_config(&context.job, &context.stage, &config)
            .await?;
        if super::requires_final_selection(&context.job)? {
            if matches!(
                context.stage.kind,
                StageKind::Generation
                    | StageKind::Edit
                    | StageKind::Correction
                    | StageKind::Extraction
            ) && recovery.current.state == DispatchState::Succeeded
            {
                let candidate = self
                    .execution
                    .read_candidate_checkpoint(
                        &context.job.owner,
                        &context.job.job,
                        &context.stage.stage,
                        &context.dispatch,
                    )
                    .await
                    .map_err(RuntimeError::Execution)?
                    .ok_or(RuntimeError::InvalidRecord)?;
                if let Some(usage) = self
                    .usage
                    .latest_usage(&context.job.owner, context.dispatch)
                    .await?
                {
                    self.project_usage(&usage).await?;
                }
                if self
                    .settle_image_candidate(&context.job, &candidate)
                    .await?
                {
                    return Ok(None);
                }
                self.advance_candidate(&context, &intent, &config, &candidate)
                    .await?;
                return Ok(None);
            }
            if context.stage.kind == StageKind::LocalTransform
                && matches!(
                    recovery.current.state,
                    DispatchState::Sent | DispatchState::Unknown
                )
            {
                let basis = self
                    .execution
                    .read_stage_basis(&context.job.owner, &context.job.job, &context.stage.stage)
                    .await
                    .map_err(RuntimeError::Execution)?;
                let parent = basis.parent_dispatch.ok_or(RuntimeError::InvalidRecord)?;
                let source = self
                    .execution
                    .read_dispatch(&context.job.owner, &parent)
                    .await
                    .map_err(RuntimeError::Execution)?;
                let candidate = self
                    .execution
                    .read_candidate_checkpoint(
                        &context.job.owner,
                        &context.job.job,
                        &source.stage,
                        &parent,
                    )
                    .await
                    .map_err(RuntimeError::Execution)?
                    .ok_or(RuntimeError::InvalidRecord)?;
                let selected = self
                    .compose_selection(&context.job, &candidate, None)
                    .await?;
                return self
                    .finish_final_stage(&context, &intent, &config, &candidate, &selected)
                    .await
                    .map(Some);
            }
        }
        if recovery.current.state == DispatchState::Succeeded {
            if let Some(usage) = self
                .usage
                .latest_usage(&context.job.owner, context.dispatch)
                .await?
            {
                self.project_usage(&usage).await?;
            }

            let result = self.publish_output(&context).await?;
            self.publication_ack(&context, &result).await;
            self.admission
                .outcome(
                    &context.job,
                    context.dispatch,
                    mode,
                    Some(!result.private_reconciliation),
                )
                .await?;
            self.completed(&context, &result).await;
            return Ok(Some(result));
        }
        if recovery.current.state == DispatchState::Failed {
            let usage = self
                .usage
                .latest_usage(&context.job.owner, context.dispatch)
                .await?
                .ok_or(RuntimeError::InvalidRecord)?;
            if usage.usable
                || usage.evidence == crate::cost::UsageEvidence::Unknown
                || usage.revision.0 != u64::from(recovery.current.usage_revision)
            {
                return Err(RuntimeError::ProviderUnknown);
            }
            self.finish_no_output(&context, &usage, mode).await?;
            return Ok(None);
        }
        if !matches!(
            recovery.current.state,
            DispatchState::Sent | DispatchState::Unknown
        ) {
            return Err(RuntimeError::ProviderUnknown);
        }
        let raw = self
            .usage
            .latest_usage(&context.job.owner, context.dispatch)
            .await?;
        if let Some(raw) = raw
            .as_ref()
            .filter(|raw| raw.evidence != crate::cost::UsageEvidence::Unknown && !raw.usable)
        {
            self.project_usage(raw).await?;
            self.admission
                .outcome(&context.job, context.dispatch, mode, None)
                .await?;
            return Err(RuntimeError::ProviderUnknown);
        }
        let timeout = Duration::from_millis(
            u64::try_from(
                lease
                    .lease_until_ms
                    .checked_sub(self.clock.now_ms())
                    .filter(|remaining| *remaining > 0)
                    .ok_or(RuntimeError::Expired)?,
            )
            .map_err(|_| RuntimeError::Expired)?,
        );
        let (image, raw) =
            if let Some(raw) = raw.filter(|usage| usage.usable && usage.raw_output.is_some()) {
                let image = dispatch::ExecutedImage {
                    bytes: raw.raw_output.clone().ok_or(RuntimeError::InvalidRecord)?,
                    units: raw.units.clone(),
                    evidence: raw.evidence,
                };
                (image, raw)
            } else {
                if matches!(intent.operation, ImageOperation::Transform(_)) {
                    return Err(RuntimeError::ProviderUnknown);
                }
                let inputs = dispatch::resolve_inputs(&intent, &self.sources).await?;
                let scope = ExecutionScope {
                    owner: context.job.owner.clone(),
                    job: context.job.job,
                    stage: context.stage.stage,
                    dispatch: context.dispatch,
                };
                let retained_sources = self
                    .usage
                    .read_sources(&scope)
                    .await?
                    .ok_or(RuntimeError::InvalidRecord)?;
                if inputs.receipts != retained_sources {
                    return Err(RuntimeError::SourceDenied);
                }
                let credentials = dispatch::pin_credentials(&config, &intent, &self.credentials)?;
                let image = dispatch::execute(
                    dispatch::DispatchInput {
                        context: &context,
                        intent: &intent,
                        config: &config,
                        timeout,
                        resume: true,
                        telemetry: &self.telemetry,
                        event_context: self.event_context(
                            &context.job,
                            &context.stage,
                            Some(context.dispatch),
                        ),
                    },
                    &self.execution,
                    inputs,
                    &credentials,
                )
                .await
                .map_err(|failure| failure.error)?;
                let route = self.cost_policy.route(
                    &context.job,
                    &config,
                    dispatch::backend(&config, &intent),
                    &dispatch::model(&config, &intent)?,
                )?;
                let raw = self
                    .retain_usage(
                        &context,
                        &route,
                        image.evidence,
                        &image.units,
                        Some(&image.bytes),
                        true,
                    )
                    .await?;
                (image, raw)
            };
        context.usage_revision =
            u32::try_from(raw.revision.0).map_err(|_| RuntimeError::InvalidRecord)?;
        if super::requires_final_selection(&context.job)? {
            self.finish_candidate(&context, &intent, &config, &raw, mode)
                .await?;
            return Ok(None);
        }
        let image = dispatch::normalize_output(&context, &intent, &config, image, timeout).await?;
        let checkpoint = self
            .publisher
            .store_output(
                &context,
                &PreparedImageOutput {
                    master: image.bytes,
                    display: None,
                },
            )
            .await?;
        self.complete(
            &context,
            CompletionOutcome::Succeeded {
                usage_revision: checkpoint.usage_revision,
                checkpoint,
            },
        )
        .await?;
        self.project_usage(&raw).await?;
        let result = self.publish_output(&context).await?;
        self.publication_ack(&context, &result).await;
        self.admission
            .outcome(
                &context.job,
                context.dispatch,
                mode,
                Some(!result.private_reconciliation),
            )
            .await?;
        self.completed(&context, &result).await;
        Ok(Some(result))
    }
}
