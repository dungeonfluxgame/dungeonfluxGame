//! Current source authority alone decides whether the exact claimed output must close.
use super::{ports::*, worker::NativeImageRuntime};
use crate::cost::{ImageCostProjection, UsageEvidence, UsageRevision};
use crate::execution::*;
use crate::{ImageJob, ImageJournal, ServiceClock};
struct ClosureInput<'a> {
    job: &'a ImageJob,
    stage: &'a StageView,
    candidate: Option<&'a CandidateReceipt>,
    actor: TerminalActor,
    authority: TerminalAuthority,
    attempt: Option<DispatchId>,
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
    pub(super) async fn publish_output(
        &self,
        context: &PublicationContext,
    ) -> Result<crate::publication::PublishedImage, RuntimeError> {
        let result = self.publisher.publish(context).await;
        if result
            .as_ref()
            .is_err_and(|error| *error == RuntimeError::SourceDenied)
        {
            self.close_denied_sources(context).await?;
        }
        result
    }
    pub(super) async fn close_queued_denied_sources(
        &self,
        job: &ImageJob,
        stage: &StageView,
    ) -> Result<bool, RuntimeError> {
        let scope = StageScope {
            owner: job.owner.clone(),
            job: job.job,
            stage: stage.stage,
        };
        let Some(candidate) = self.terminal_candidate(&scope).await? else {
            return Ok(false);
        };
        if self
            .execution
            .read_current_dispatch(&scope.owner, &scope.job, &scope.stage)
            .await
            .map_err(RuntimeError::Execution)?
            .is_some()
        {
            return Ok(false);
        }
        let now = self.clock.now_ms();
        let lease = i64::try_from(self.settings.config.scheduler.stage_deadline_secs)
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .checked_mul(1000)
            .ok_or(RuntimeError::InvalidConfiguration)?
            .min(job.absolute_deadline_ms.saturating_sub(now));
        if lease <= 0 {
            return Err(RuntimeError::Expired);
        }
        let token = ClaimToken::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let stage = self
            .execution
            .claim(
                &ClaimCommand::new(
                    job.owner.clone(),
                    job.job,
                    stage.stage,
                    self.settings.active_config_revision,
                    token,
                    lease,
                    now,
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        let claim = StageClaim {
            scope,
            token,
            generation: stage.claim_generation,
        };
        self.commit_source_closure(ClosureInput {
            job,
            stage: &stage,
            candidate: Some(&candidate),
            actor: TerminalActor::UnsentStage(claim.clone()),
            authority: TerminalAuthority::Claim(claim),
            attempt: None,
        })
        .await
    }
    pub(super) async fn close_denied_sources(
        &self,
        context: &PublicationContext,
    ) -> Result<bool, RuntimeError> {
        let scope = StageScope {
            owner: context.job.owner.clone(),
            job: context.job.job,
            stage: context.stage.stage,
        };
        let current = self
            .execution
            .read_current_dispatch(&scope.owner, &scope.job, &scope.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        if current
            .as_ref()
            .is_some_and(|dispatch| dispatch.dispatch != context.dispatch)
        {
            return Err(RuntimeError::Execution(ExecutionError::Superseded));
        }
        let own = self
            .execution
            .read_candidate_checkpoint(&scope.owner, &scope.job, &scope.stage, &context.dispatch)
            .await
            .map_err(RuntimeError::Execution)?;
        let candidate = match own {
            Some(candidate) => Some(candidate),
            None => self.terminal_candidate(&scope).await?,
        };
        if candidate.is_none()
            && (current
                .as_ref()
                .is_none_or(|dispatch| dispatch.state != DispatchState::Succeeded)
                || self
                    .execution
                    .read_output_checkpoint(
                        &scope.owner,
                        &scope.job,
                        &scope.stage,
                        &context.dispatch,
                    )
                    .await
                    .map_err(RuntimeError::Execution)?
                    .is_none())
        {
            return Ok(false);
        }
        let claim = StageClaim {
            scope: scope.clone(),
            token: context.claim,
            generation: context.stage.claim_generation,
        };
        let actor = current.as_ref().map_or_else(
            || TerminalActor::UnsentStage(claim.clone()),
            |dispatch| {
                TerminalActor::Dispatch(ExecutionScope {
                    owner: scope.owner.clone(),
                    job: scope.job,
                    stage: scope.stage,
                    dispatch: dispatch.dispatch,
                })
            },
        );
        let authority = context.recovery.as_ref().map_or_else(
            || TerminalAuthority::Claim(claim),
            |lease| TerminalAuthority::Recovery(lease.clone()),
        );
        self.commit_source_closure(ClosureInput {
            job: &context.job,
            stage: &context.stage,
            candidate: candidate.as_ref(),
            actor,
            authority,
            attempt: current.map(|dispatch| dispatch.dispatch),
        })
        .await
    }
    async fn commit_source_closure(&self, input: ClosureInput<'_>) -> Result<bool, RuntimeError> {
        let job = self
            .journal
            .read_job(&input.job.owner, input.job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let existing = self
            .publisher
            .read_no_delivery(&job.owner, &job.job)
            .await?;
        let closed = if existing.is_some() {
            true
        } else {
            let command = TerminalCommand {
                actor: input.actor.clone(),
                tail: input.actor.clone(),
                authority: input.authority,
                expected_job_revision: job.revision,
                candidate_dispatch: input.candidate.map(|candidate| candidate.scope.dispatch),
                now_ms: self.clock.now_ms(),
            };
            match self
                .publisher
                .no_delivery_if_sources_denied(&command)
                .await?
            {
                crate::publication::SourceClosure::PublicationAllowed => false,
                crate::publication::SourceClosure::Closed(_) => {
                    self.telemetry.observe(
                        crate::telemetry::ImageEvent::new(
                            crate::telemetry::EventKind::Failed,
                            self.event_context(&job, input.stage, input.attempt),
                        )
                        .failed(crate::telemetry::FailureKind::Authorization),
                    );
                    true
                }
            }
        };
        if !closed {
            return Ok(false);
        }
        if let Some(candidate) = input.candidate {
            self.settle_image_chain(&job, candidate).await?;
        } else if let TerminalActor::Dispatch(scope) = input.actor {
            let disposition = self
                .publisher
                .image_economic_use(&scope)
                .await?
                .ok_or(RuntimeError::InvalidRecord)?;
            let usage = self
                .usage
                .read_usage_revision(&scope, UsageRevision(u64::from(disposition.usage_revision)))
                .await?
                .ok_or(RuntimeError::InvalidRecord)?;
            self.project_usage(&usage).await?;
            let dispatch = self
                .execution
                .read_dispatch(&scope.owner, &scope.dispatch)
                .await
                .map_err(RuntimeError::Execution)?;
            self.admission
                .outcome(
                    &job,
                    scope.dispatch,
                    dispatch
                        .finance
                        .map_or(DispatchMode::Local, DispatchMode::Hosted),
                    (usage.evidence != UsageEvidence::Unknown).then_some(disposition.usable),
                )
                .await?;
        }
        Ok(true)
    }
}
