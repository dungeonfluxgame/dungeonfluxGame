//! A final proved submission rejection closes output capacity while unknown money remains held.
use super::{ports::*, worker::NativeImageRuntime};
use crate::cost::ImageCostProjection;
use crate::execution::*;
use crate::{ImageJournal, ServiceClock};
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
    pub(super) async fn close_final_decline(
        &self,
        context: &PublicationContext,
        receipt: &DeclineReceipt,
    ) -> Result<(), RuntimeError> {
        let result = Box::pin(self.close_final_decline_inner(context, receipt)).await;
        match result {
            Err(RuntimeError::Execution(
                ExecutionError::LostClaim
                | ExecutionError::Stale { .. }
                | ExecutionError::Superseded
                | ExecutionError::OutputSealed,
            ))
            | Err(RuntimeError::Publication(crate::publication::PublicationError::Execution(
                ExecutionError::LostClaim
                | ExecutionError::Stale { .. }
                | ExecutionError::Superseded
                | ExecutionError::OutputSealed,
            ))) => Ok(()),
            result => result,
        }
    }
    async fn close_final_decline_inner(
        &self,
        context: &PublicationContext,
        receipt: &DeclineReceipt,
    ) -> Result<(), RuntimeError> {
        let job = self
            .journal
            .read_job(&context.job.owner, context.job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        if self
            .publisher
            .read_no_delivery(&job.owner, &job.job)
            .await?
            .is_none()
        {
            let candidate = self
                .terminal_candidate(&StageScope {
                    owner: job.owner.clone(),
                    job: job.job,
                    stage: context.stage.stage,
                })
                .await?;
            let actor = TerminalActor::Dispatch(receipt.scope.clone());
            let authority = context.recovery.as_ref().map_or_else(
                || {
                    TerminalAuthority::Claim(StageClaim {
                        scope: StageScope {
                            owner: job.owner.clone(),
                            job: job.job,
                            stage: context.stage.stage,
                        },
                        token: context.claim,
                        generation: context.stage.claim_generation,
                    })
                },
                |lease| TerminalAuthority::Recovery(lease.clone()),
            );
            self.publisher
                .no_delivery(&crate::publication::NoDeliveryRequest {
                    terminal: &TerminalCommand {
                        actor: actor.clone(),
                        tail: actor,
                        authority,
                        expected_job_revision: job.revision,
                        candidate_dispatch: candidate.as_ref().map(|cp| cp.scope.dispatch),
                        now_ms: self.clock.now_ms(),
                    },
                    cause: crate::publication::NoDeliveryCause::SubmitDeclined(receipt),
                })
                .await?;
            let mut event = crate::telemetry::ImageEvent::new(
                crate::telemetry::EventKind::Unknown,
                self.event_context(&job, &context.stage, Some(context.dispatch)),
            );
            event.measurements.supplier_dispatched = Some(true);
            event.measurements.raw_checkpointed = Some(true);
            self.telemetry.observe(event);
        }
        if let Some(candidate) = self
            .terminal_candidate(&StageScope {
                owner: job.owner.clone(),
                job: job.job,
                stage: context.stage.stage,
            })
            .await?
        {
            self.settle_image_chain(&job, &candidate).await?;
        }
        // The rejected supplier attempt remains UnknownOutstanding; no false/zero outcome follows.
        Ok(())
    }
}
