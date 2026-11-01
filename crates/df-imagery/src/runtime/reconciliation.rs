//! An already qualified paid candidate may finish locally under its real recovery authority.
use super::{
    PublicationDecisionEvidence, PublicationEvidence, ports::*, worker::NativeImageRuntime,
};
use crate::cost::{ImageCostProjection, UsageRevision};
use crate::execution::*;
use crate::intent_codec::DecodedIntent;
use crate::publication::{FinalOutputSelection, recipe_digest};
use crate::{ImageConfig, ImageJob, ImageJournal, ServiceClock};

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
    pub(super) async fn has_qualified_reconciliation(
        &self,
        job: &ImageJob,
        candidate: &CandidateReceipt,
    ) -> Result<bool, RuntimeError> {
        let current = self
            .execution
            .read_current_dispatch(&job.owner, &job.job, &candidate.scope.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        if current.as_ref().is_none_or(|current| {
            current.dispatch != candidate.scope.dispatch
                || current.state != DispatchState::Succeeded
        }) {
            return Ok(false);
        }
        let selection = self.compose_selection(job, candidate, None).await?;
        let evidence = self.selection_evidence(job, &selection, candidate).await?;
        let job = job.clone();
        let candidate = candidate.clone();
        let qualified = tokio::task::spawn_blocking(move || {
            super::qualify_publication(&job, &evidence, &candidate)
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?;
        match qualified {
            Ok(_) => Ok(true),
            Err(RuntimeError::InvalidRecord | RuntimeError::ProviderRefused) => Ok(false),
            Err(error) => Err(error),
        }
    }

    async fn selection_evidence(
        &self,
        job: &ImageJob,
        selection: &FinalOutputSelection,
        candidate: &CandidateReceipt,
    ) -> Result<PublicationEvidence, RuntimeError> {
        let mut evidence = self
            .publication_evidence(job, candidate.scope.dispatch)
            .await?;
        for reference in &selection.decisions {
            if evidence
                .decisions
                .iter()
                .any(|entry| entry.decision.scope == reference.scope)
            {
                continue;
            }
            let decision = self
                .execution
                .read_decision_checkpoint(
                    &job.owner,
                    &job.job,
                    &reference.scope.stage,
                    &reference.scope.dispatch,
                )
                .await
                .map_err(RuntimeError::Execution)?
                .ok_or(RuntimeError::InvalidRecord)?;
            let usage = self
                .usage
                .read_usage_revision(
                    &decision.scope,
                    UsageRevision(u64::from(decision.receipt.usage_revision)),
                )
                .await?
                .ok_or(RuntimeError::InvalidRecord)?;
            let source_candidate = if decision.kind == DecisionKind::Inspection {
                Some(
                    evidence
                        .candidates
                        .iter()
                        .find(|candidate| {
                            Some(candidate.scope.dispatch) == decision.basis.stage.parent_dispatch
                        })
                        .cloned()
                        .ok_or(RuntimeError::InvalidRecord)?,
                )
            } else {
                None
            };
            evidence.decisions.push(PublicationDecisionEvidence {
                decision,
                usage,
                source_candidate,
            });
        }
        Ok(evidence)
    }

    pub(super) async fn reconcile_selected_output(
        &self,
        context: &PublicationContext,
        intent: &DecodedIntent,
        config: &ImageConfig,
        candidate: &CandidateReceipt,
    ) -> Result<(), RuntimeError> {
        let lease = context
            .recovery
            .as_ref()
            .ok_or(RuntimeError::InvalidRecord)?;
        let selection = self
            .compose_selection(&context.job, candidate, None)
            .await?;
        if !self
            .has_qualified_reconciliation(&context.job, candidate)
            .await?
        {
            return Err(RuntimeError::Expired);
        }
        let prepared = self
            .execution
            .prepare_reconciliation(
                lease,
                &ReconciliationCommand {
                    candidate_dispatch: candidate.scope.dispatch,
                    child: ExecutionStageId::from_bytes(&self.ids.next_bytes())
                        .map_err(|_| RuntimeError::InvalidRecord)?,
                    dispatch: DispatchId::from_bytes(&self.ids.next_bytes())
                        .map_err(|_| RuntimeError::InvalidRecord)?,
                    claim: ClaimToken::from_bytes(&self.ids.next_bytes())
                        .map_err(|_| RuntimeError::InvalidRecord)?,
                    capacity: self.settings.capacity.clone(),
                    recipe_revision: selection.resolved_recipe.revision(),
                    recipe_digest: recipe_digest(&selection.resolved_recipe),
                    now_ms: self.clock.now_ms(),
                },
            )
            .await
            .map_err(RuntimeError::Execution)?;
        if matches!(
            prepared.current.state,
            DispatchState::Sent | DispatchState::Unknown | DispatchState::Succeeded
        ) {
            return Ok(());
        }
        if prepared.current.state != DispatchState::Prepared
            || prepared.current.finance.is_some()
            || !prepared.stage.local_accounting
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let job = self
            .journal
            .read_job(&context.job.owner, context.job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let final_context = PublicationContext {
            job,
            stage: prepared.stage,
            dispatch: prepared.current.dispatch,
            claim: prepared.claim.token,
            now_ms: self.clock.now_ms(),
            usage_revision: 0,
            recovery: None,
        };
        let sources = self
            .usage
            .read_sources(&candidate.scope)
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        self.usage
            .record_sources(
                &ExecutionScope {
                    owner: final_context.job.owner.clone(),
                    job: final_context.job.job,
                    stage: final_context.stage.stage,
                    dispatch: final_context.dispatch,
                },
                &sources,
            )
            .await?;
        let admission_config = super::retained_config(&final_context.job)?;
        let authority = self
            .admission
            .mark_sent(
                &DispatchAdmission {
                    job: &final_context.job,
                    stage: &final_context.stage,
                    dispatch: final_context.dispatch,
                    claim: final_context.claim,
                    config: &admission_config,
                    quote: None,
                    sources: &sources,
                },
                DispatchMode::Local,
            )
            .await?;
        if authority == SendAuthorization::New {
            self.finish_final_stage(&final_context, intent, config, candidate, &selection)
                .await?;
        }
        Ok(())
    }
}
