//! Immutable supplier selections, protected settlement and explicit graph advancement.
use super::*;

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
    pub(super) async fn finish_analysis(
        &self,
        context: &PublicationContext,
        mode: DispatchMode,
        raw: &RuntimeUsage,
        finish: AnalysisFinish<'_>,
    ) -> Result<(), RuntimeError> {
        let AnalysisFinish {
            intent,
            result,
            dependencies,
            maximum,
        } = finish;
        let basis = self
            .execution
            .read_stage_basis(&context.job.owner, &context.job.job, &context.stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let frozen_revision = basis.config_revision;
        let parent_dispatch = basis.parent_dispatch;
        let usage_revision =
            u32::try_from(raw.revision.0).map_err(|_| RuntimeError::InvalidRecord)?;
        let observation = DecisionObservation::new(
            scope(context),
            decision_kind(&intent.operation),
            1,
            DecisionBasis {
                schema_revision: 1,
                validator_revision: 1,
                stage: basis,
            },
            encode_payload(result, dependencies, maximum)?,
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
        let mut checkpoint_context =
            self.event_context(&context.job, &context.stage, Some(context.dispatch));
        checkpoint_context.purpose = Some(intent.purpose.purpose);
        checkpoint_context.analysis_kind = Some(telemetry_kind(&intent.operation));
        checkpoint_context.config_revision = Some(frozen_revision);
        checkpoint_context.parent_attempt = parent_dispatch;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::DecisionCheckpointed,
            checkpoint_context,
        );
        event.measurements.raw_checkpointed = Some(true);
        event.measurements.supplier_dispatched = Some(true);
        event.measurements.analysis_verdict = Some(verdict(result));
        self.telemetry.observe(event);
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
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::AnalysisCompleted,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        event.context.purpose = Some(intent.purpose.purpose);
        event.context.analysis_kind = Some(telemetry_kind(&intent.operation));
        event.context.config_revision = Some(frozen_revision);
        event.context.parent_attempt = parent_dispatch;
        event.measurements.supplier_dispatched = Some(true);
        event.measurements.raw_checkpointed = Some(true);
        event.measurements.analysis_verdict = Some(verdict(result));
        self.telemetry.observe(event);
        if let (
            ImageAnalysisOperation::Segmentation(spec),
            ImageAnalysisResult::Segmentation { masks, .. },
        ) = (&intent.operation, result)
            && masks.len() < spec.prompts.len()
        {
            self.enqueue_analysis_prompt(context).await?;
        } else {
            self.select_analysis_output(context, result, maximum)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn enqueue_analysis_prompt(
        &self,
        context: &PublicationContext,
    ) -> Result<(), RuntimeError> {
        if self
            .execution
            .read_child_stage(
                &context.job.owner,
                &context.job.job,
                &context.dispatch,
                StageKind::Segmentation,
            )
            .await
            .map_err(RuntimeError::Execution)?
            .is_some()
        {
            return Ok(());
        }

        let job = self
            .journal
            .read_job(&context.job.owner, context.job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        if job.state == crate::JobState::Retired || self.clock.now_ms() >= job.absolute_deadline_ms
        {
            return Ok(());
        }
        let basis = self
            .execution
            .read_stage_basis(&job.owner, &job.job, &context.stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let frozen_revision = basis.config_revision;
        let queued = self
            .execution
            .enqueue(
                &EnqueueCommand::new(
                    StageScope {
                        owner: job.owner.clone(),
                        job: job.job,
                        stage: ExecutionStageId::from_bytes(&self.ids.next_bytes())
                            .map_err(|_| RuntimeError::InvalidRecord)?,
                    },
                    StageBasis {
                        parent_dispatch: Some(context.dispatch),
                        ..basis
                    },
                    None,
                    self.settings.capacity.clone(),
                    context.stage.local_accounting,
                    self.clock.now_ms(),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        if let EnqueueOutcome::Enqueued(child) = queued {
            let mut context_event = self.event_context(&job, &child, None);
            context_event.parent_attempt = Some(context.dispatch);
            context_event.config_revision = Some(frozen_revision);
            self.telemetry.observe(crate::telemetry::ImageEvent::new(
                crate::telemetry::EventKind::Queued,
                context_event,
            ));
        }
        Ok(())
    }
}
