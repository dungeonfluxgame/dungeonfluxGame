//! Live unsent jobs close through protected preflight evidence and their real claim.
use super::{PreflightCause, QualifiedPreflightRejected, ports::*, worker::NativeImageRuntime};
use crate::cost::ImageCostProjection;
use crate::execution::*;
use crate::{ImageJob, ImageJournal, ServiceClock};

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
    pub(super) async fn finish_preflight_rejection(
        &self,
        job: &ImageJob,
        stage: &StageView,
        reason: PreflightCause,
    ) -> Result<(), RuntimeError> {
        if self
            .execution
            .read_current_dispatch(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?
            .is_some()
        {
            return Err(RuntimeError::ProviderUnknown);
        }
        let now = self.clock.now_ms();
        let lease_ms =
            super::lease::claim_lease_ms(&self.settings.config, job.absolute_deadline_ms, now)?;
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
                    lease_ms,
                    now,
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        let claim = StageClaim {
            scope: StageScope {
                owner: job.owner.clone(),
                job: job.job,
                stage: stage.stage,
            },
            token,
            generation: stage.claim_generation,
        };
        let job = self
            .journal
            .read_job(&job.owner, job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let basis = self
            .execution
            .read_stage_basis(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let candidate = self.terminal_candidate(&claim.scope).await?;
        let retained = job.clone();
        let proof = tokio::task::spawn_blocking(move || {
            QualifiedPreflightRejected::observed(&retained, claim, basis, reason, candidate)
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)??;
        let actor = TerminalActor::UnsentStage(proof.claim().clone());
        let terminal = TerminalCommand {
            actor: actor.clone(),
            tail: actor,
            authority: TerminalAuthority::Claim(proof.claim().clone()),
            expected_job_revision: job.revision,
            candidate_dispatch: proof.candidate().map(|candidate| candidate.scope.dispatch),
            now_ms: self.clock.now_ms(),
        };
        self.publisher
            .no_delivery(&crate::publication::NoDeliveryRequest {
                terminal: &terminal,
                cause: crate::publication::NoDeliveryCause::PreflightRejected(&proof),
            })
            .await?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::Refused,
            self.event_context(&job, &stage, None),
        )
        .failed(crate::telemetry::FailureKind::Validation);
        event.measurements.preflight_rejection = Some(reason);
        event.measurements.supplier_dispatched = Some(false);
        event.measurements.raw_checkpointed = Some(false);
        self.telemetry.observe(event);
        if let Some(candidate) = proof.candidate() {
            self.settle_image_chain(&job, candidate).await?;
        }
        Ok(())
    }
}
