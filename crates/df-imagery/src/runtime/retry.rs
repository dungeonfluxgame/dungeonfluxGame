//! A bounded iterative sequence keeps one quota probe across proven rejected submissions.
use super::{ports::*, worker::NativeImageRuntime};
use crate::cost::ImageCostProjection;
use crate::execution::*;
use crate::{ImageJob, ImageJournal, ServiceClock};

#[derive(Default)]
pub(super) struct RetryState {
    pub(super) quota: Option<super::quota::QuotaPermit>,
    pub(super) credentials: Option<super::dispatch::PinnedCredentials>,
    pub(super) next: Option<(ImageJob, StageView, i64)>,
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
    pub(super) async fn run_queued(
        &self,
        mut job: ImageJob,
        mut stage: StageView,
    ) -> Result<Option<crate::publication::PublishedImage>, RuntimeError> {
        let mut retry = RetryState::default();
        loop {
            let result = Box::pin(self.run_queued_attempt(job, stage, &mut retry)).await;
            let Some((next_job, next_stage, due)) = retry.next.take() else {
                return result;
            };
            if result?.is_some() {
                return Err(RuntimeError::InvalidRecord);
            }
            job = self.wait_for_retry(&next_job, due).await?;
            stage = next_stage;
        }
    }
    async fn wait_for_retry(&self, job: &ImageJob, due: i64) -> Result<ImageJob, RuntimeError> {
        loop {
            let current = self
                .journal
                .read_job(&job.owner, job.job)
                .await
                .map_err(RuntimeError::Journal)?
                .ok_or(RuntimeError::InvalidRecord)?;
            let now = self.clock.now_ms();
            if current.state == crate::JobState::Retired {
                return Err(RuntimeError::Cancelled);
            }
            if now >= current.absolute_deadline_ms {
                return Err(RuntimeError::Expired);
            }
            if now >= due {
                return Ok(current);
            }
            let remaining = u64::try_from(due.checked_sub(now).ok_or(RuntimeError::Expired)?)
                .map_err(|_| RuntimeError::InvalidRecord)?;
            tokio::time::sleep(
                std::time::Duration::from_millis(remaining).min(self.settings.poll_interval),
            )
            .await;
        }
    }
    pub(super) async fn retain_decline(
        &self,
        context: &PublicationContext,
        failure: &crate::backends::api::ApiFailure,
        raw: &RuntimeUsage,
        mode: DispatchMode,
    ) -> Result<crate::execution::DeclineReceipt, RuntimeError> {
        let dispatch = self
            .execution
            .read_dispatch(&context.job.owner, &context.dispatch)
            .await
            .map_err(RuntimeError::Execution)?;
        let basis = self
            .execution
            .read_stage_basis(&context.job.owner, &context.job.job, &context.stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let job = context.job.clone();
        let failure = failure.clone();
        let raw_owned = raw.clone();
        let observation = tokio::task::spawn_blocking(move || {
            DeclineObservation::qualify(&job, &dispatch, basis, &failure, &raw_owned)
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?
        .map_err(RuntimeError::Execution)?;
        let receipt = self
            .execution
            .record_decline_observation(&observation)
            .await
            .map_err(RuntimeError::Execution)?;
        self.complete(
            context,
            CompletionOutcome::Declined {
                observation_version: receipt.observation_version,
                usage_revision: receipt.receipt.usage_revision,
            },
        )
        .await?;
        // Book evidence is independent of whether finance still awaits economic proof.
        match self.project_usage(raw).await {
            Ok(()) | Err(RuntimeError::CostProjectionPending) => {}
            Err(error) => return Err(error),
        }
        self.admission
            .outcome(&context.job, context.dispatch, mode, None)
            .await?;
        Ok(receipt)
    }
    pub(super) async fn reconcile_decline(
        &self,
        job: &ImageJob,
        lease: &RecoveryLease,
    ) -> Result<(), RuntimeError> {
        let previous = &lease.stage;
        let scope = ExecutionScope {
            owner: job.owner.clone(),
            job: job.job,
            stage: previous.stage,
            dispatch: previous.current.dispatch,
        };
        let receipt = self
            .execution
            .read_decline_reference(&scope)
            .await
            .map_err(RuntimeError::Execution)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let usage = self
            .usage
            .read_usage_revision(
                &scope,
                crate::cost::UsageRevision(u64::from(receipt.receipt.usage_revision)),
            )
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        self.project_usage(&usage).await?;
        let mode = previous
            .current
            .finance
            .map_or(DispatchMode::Local, DispatchMode::Hosted);
        self.admission
            .outcome(job, scope.dispatch, mode, None)
            .await?;
        // Requeue grants no egress; the ordinary queue path acquires all fresh authorities.
        match self
            .execution
            .requeue_declined(&DeclineRetryCommand {
                claim: StageClaim {
                    scope: StageScope {
                        owner: job.owner.clone(),
                        job: job.job,
                        stage: previous.stage,
                    },
                    token: previous.claim,
                    generation: previous.claim_generation,
                },
                dispatch: scope.dispatch,
                now_ms: self.clock.now_ms(),
            })
            .await
        {
            Ok(_) => Ok(()),
            Err(
                ExecutionError::Expired
                | ExecutionError::Retired
                | ExecutionError::DemandLapsed
                | ExecutionError::Disabled
                | ExecutionError::ConfigFenced { .. }
                | ExecutionError::OperationConflict,
            ) => {
                let mut stage = self
                    .execution
                    .read_stage(&job.owner, &job.job, &previous.stage)
                    .await
                    .map_err(RuntimeError::Execution)?;
                stage.claim_generation = previous.claim_generation;
                self.close_final_decline(
                    &PublicationContext {
                        job: job.clone(),
                        stage,
                        dispatch: scope.dispatch,
                        claim: previous.claim,
                        now_ms: self.clock.now_ms(),
                        usage_revision: receipt.receipt.usage_revision,
                        recovery: Some(lease.clone()),
                    },
                    &receipt,
                )
                .await
            }
            Err(
                ExecutionError::CapacityFull
                | ExecutionError::OutputSealed
                | ExecutionError::Superseded
                | ExecutionError::LostClaim,
            ) => Ok(()),
            Err(error) => Err(RuntimeError::Execution(error)),
        }
    }
}
