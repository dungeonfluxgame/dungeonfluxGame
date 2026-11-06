//! Supplier recovery consumes retained bytes or a validated queued receipt, never a new submit.
use super::{ports::*, worker::NativeImageRuntime};
use crate::cost::{ImageCostProjection, UsageRevision};
use crate::execution::*;
use crate::intent_codec::DecodedIntent;
use crate::{ImageConfig, ImageJournal, ServiceClock};

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
    pub(super) async fn reconcile_extraction(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        mode: DispatchMode,
    ) -> Result<(), RuntimeError> {
        if let Some(raw) = self
            .usage
            .latest_usage(&context.job.owner, context.dispatch)
            .await?
            && raw.usable
            && raw.raw_output.is_some()
        {
            return self
                .finish_candidate(context, intent, config, &raw, mode)
                .await;
        }
        // Resume requires a validated retained supplier receipt; no receipt means no network retry.
        self.execution
            .read_supplier_checkpoint(
                &context.job.owner,
                &context.job.job,
                &context.stage.stage,
                &context.dispatch,
            )
            .await
            .map_err(RuntimeError::Execution)?
            .ok_or(RuntimeError::ProviderUnknown)?;
        let inputs = super::dispatch::resolve_inputs(intent, &self.sources).await?;
        let scope = ExecutionScope {
            owner: context.job.owner.clone(),
            job: context.job.job,
            stage: context.stage.stage,
            dispatch: context.dispatch,
        };
        let retained = self
            .usage
            .read_sources(&scope)
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        if inputs.receipts != retained {
            return Err(RuntimeError::SourceDenied);
        }
        let mut basis = super::retained_cost_basis(&context.job, config, StageKind::Extraction)?;
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
        let route = self.cost_policy.stage_route(&context.job, config, &basis)?;
        let deadline = context
            .recovery
            .as_ref()
            .map_or(context.stage.lease_until_ms, |lease| lease.lease_until_ms);
        let remaining = deadline
            .checked_sub(self.clock.now_ms())
            .filter(|remaining| *remaining > 0)
            .ok_or(RuntimeError::Expired)?;
        let timeout = std::time::Duration::from_millis(
            u64::try_from(remaining).map_err(|_| RuntimeError::Expired)?,
        );
        let executed = super::dispatch::execute_alpha(
            super::dispatch::DispatchInput {
                context,
                intent,
                config,
                codex_home: self.settings.codex_home.as_deref(),
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
            None,
            &self.credentials,
        )
        .await;
        self.finish_extraction(context, intent, config, &route, mode, executed)
            .await
    }

    pub(super) async fn reconcile_inspection(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        mode: DispatchMode,
        state: DispatchState,
    ) -> Result<(), RuntimeError> {
        let candidate = self.source_candidate(&context.job, &context.stage).await?;
        let raw = if state == DispatchState::Succeeded {
            let decision = self
                .execution
                .read_decision_checkpoint(
                    &context.job.owner,
                    &context.job.job,
                    &context.stage.stage,
                    &context.dispatch,
                )
                .await
                .map_err(RuntimeError::Execution)?
                .ok_or(RuntimeError::InvalidRecord)?;
            self.usage
                .read_usage_revision(
                    &decision.scope,
                    UsageRevision(u64::from(decision.receipt.usage_revision)),
                )
                .await?
        } else {
            self.usage
                .latest_usage(&context.job.owner, context.dispatch)
                .await?
        }
        .ok_or(RuntimeError::ProviderUnknown)?;
        let spec = intent
            .operation
            .pipeline()
            .and_then(|pipeline| pipeline.inspection.as_ref())
            .ok_or(RuntimeError::InvalidRecord)?;
        let retained = raw
            .raw_output
            .as_deref()
            .ok_or(RuntimeError::ProviderUnknown)?;
        let assessment = super::qualification::replay_inspection(retained, spec)?;
        let maximum = usize::try_from(
            self.execution
                .read_capacity_bounds(self.settings.capacity.as_str())
                .await
                .map_err(RuntimeError::Execution)?
                .max_decision_payload_bytes,
        )
        .map_err(|_| RuntimeError::InvalidConfiguration)?;
        if state == DispatchState::Succeeded {
            let decision = self
                .execution
                .read_decision_checkpoint(
                    &context.job.owner,
                    &context.job.job,
                    &context.stage.stage,
                    &context.dispatch,
                )
                .await
                .map_err(RuntimeError::Execution)?
                .ok_or(RuntimeError::InvalidRecord)?;
            self.settle_decision(
                context,
                mode,
                &raw,
                decision.clone(),
                Some(candidate.clone()),
            )
            .await?;
            self.project_usage(&raw).await?;
            if self
                .publisher
                .read_no_delivery(&context.job.owner, &context.job.job)
                .await?
                .is_some()
            {
                return self.settle_image_chain(&context.job, &candidate).await;
            }
            self.advance_inspection(context, intent, config, &candidate, &decision, &assessment)
                .await
        } else {
            self.finish_inspection(context, &candidate, mode, &raw, maximum, assessment)
                .await
        }
    }
}
