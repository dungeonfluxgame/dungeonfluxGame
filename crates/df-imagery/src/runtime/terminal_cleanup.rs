//! Terminal cleanup consumes stored unsent proofs; elapsed time alone never releases finance.
use super::{
    ports::*,
    worker::{NativeImageRuntime, RuntimeCursors},
};
use crate::cost::ImageCostProjection;
use crate::execution::*;
use crate::publication::{NoDeliveryCause, NoDeliveryReceipt, NoDeliveryRequest};
use crate::{ImageJob, ImageJournal, ServiceClock};
use std::collections::BTreeSet;

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
    pub(super) async fn cleanup_unsent(
        &self,
        cursors: &mut RuntimeCursors,
    ) -> Result<(), RuntimeError> {
        let entries = self
            .execution
            .scan_unsent_prepared(
                self.settings.capacity.as_str(),
                cursors.prepared,
                1,
                self.clock.now_ms(),
            )
            .await
            .map_err(RuntimeError::Execution)?;
        cursors.prepared = entries.last().map(|entry| entry.scope.dispatch);
        if let Some(entry) = entries.first() {
            let result = self.cleanup_prepared(entry).await;
            Self::cleanup_result(result)?;
        }
        let entries = self
            .execution
            .scan_unsent_terminal(
                self.settings.capacity.as_str(),
                cursors.terminal,
                1,
                self.clock.now_ms(),
            )
            .await
            .map_err(RuntimeError::Execution)?;
        cursors.terminal = entries.last().map(|entry| TerminalCursor {
            job: entry.scope().job,
            stage: entry.scope().stage,
        });
        if let Some(entry) = entries.first() {
            let result = self.cleanup_unprepared(entry).await;
            Self::cleanup_result(result)?;
        }
        Ok(())
    }

    fn cleanup_result(result: Result<(), RuntimeError>) -> Result<(), RuntimeError> {
        match result {
            Err(RuntimeError::CostProjectionPending) => Ok(()),
            // This read-only refusal is not a new deferral transition. Actual source attempts
            // and committed closures carry their own correlated observation.
            Err(RuntimeError::SourceDenied) => Ok(()),
            Err(RuntimeError::Execution(
                ExecutionError::LostClaim
                | ExecutionError::Stale { .. }
                | ExecutionError::Superseded
                | ExecutionError::OutputSealed,
            )) => Ok(()),
            result => result,
        }
    }

    async fn cleanup_prepared(&self, entry: &UnsentPrepared) -> Result<(), RuntimeError> {
        if !entry.terminal_condition && !entry.sealed {
            return Ok(());
        }
        let job = self
            .journal
            .read_job(&entry.scope.owner, entry.scope.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        if entry.current.local_accounting
            && self
                .execution
                .read_stage(&job.owner, &job.job, &entry.scope.stage)
                .await
                .map_err(RuntimeError::Execution)?
                .kind
                == StageKind::LocalTransform
            && let Some(candidate) = self.terminal_candidate(&entry.claim.scope).await?
            && self.has_qualified_reconciliation(&job, &candidate).await?
        {
            return Ok(());
        }
        let receipt = if let Some(receipt) = self
            .publisher
            .read_no_delivery(&job.owner, &job.job)
            .await?
        {
            receipt
        } else {
            let actor = TerminalActor::Dispatch(entry.scope.clone());
            self.seal_demand_ended(
                &job,
                TerminalCommand {
                    actor: actor.clone(),
                    tail: actor,
                    authority: TerminalAuthority::Claim(entry.claim.clone()),
                    expected_job_revision: entry.job_revision,
                    candidate_dispatch: self
                        .terminal_candidate(&entry.claim.scope)
                        .await?
                        .map(|candidate| candidate.scope.dispatch),
                    now_ms: self.clock.now_ms(),
                },
            )
            .await?
        };
        if entry.current.finance.is_some() {
            self.admission
                .release_unused(&job, entry.scope.dispatch, &receipt)
                .await?;
            self.record_settlement(&job, entry.scope.dispatch, entry.scope.stage)
                .await;
        }
        let kind = self
            .execution
            .read_stage(&job.owner, &job.job, &entry.scope.stage)
            .await
            .map_err(RuntimeError::Execution)?
            .kind;
        if kind != StageKind::LocalTransform {
            let call = crate::cost::pinned_call_id(entry.scope.dispatch.as_bytes());
            let seal = self
                .costs
                .seal_unsent(&call)
                .await
                .map_err(RuntimeError::Cost)?;
            if seal.durability == crate::cost::UnsentDurability::Pending {
                return Err(RuntimeError::CostProjectionPending);
            }
            let projection =
                crate::cost::DurableUnsentReceipt::from_seal(seal, entry.scope.dispatch.as_bytes())
                    .map_err(RuntimeError::Cost)?;
            self.execution
                .acknowledge_unsent_projection(&AcknowledgeUnsentProjection {
                    scope: entry.scope.clone(),
                    delivery: receipt,
                    projection,
                    now_ms: self.clock.now_ms(),
                })
                .await
                .map_err(RuntimeError::Execution)?;
        }
        self.settle_terminal_candidate(&job, &entry.claim.scope)
            .await
    }

    async fn cleanup_unprepared(&self, entry: &UnsentTerminal) -> Result<(), RuntimeError> {
        let scope = entry.scope();
        let job = self
            .journal
            .read_job(&scope.owner, scope.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let candidate = self.terminal_candidate(scope).await?;
        let stage = self
            .execution
            .read_stage(&scope.owner, &scope.job, &scope.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        if matches!(
            stage.kind,
            StageKind::LocalTransform | StageKind::Inspection
        ) && let Some(candidate) = &candidate
            && self.has_qualified_reconciliation(&job, candidate).await?
        {
            return Ok(());
        }
        let (actor, authority) = match entry {
            UnsentTerminal::Unclaimed(proof) => (
                TerminalActor::UnclaimedStage(scope.clone()),
                TerminalAuthority::Unclaimed(proof.clone()),
            ),
            UnsentTerminal::Claimed(proof) => (
                TerminalActor::UnsentStage(proof.claim.clone()),
                TerminalAuthority::Claim(proof.claim.clone()),
            ),
            UnsentTerminal::Deferred(proof) => (
                TerminalActor::DeferredStage(scope.clone()),
                TerminalAuthority::Deferred(proof.clone()),
            ),
            UnsentTerminal::Released(proof) => (
                TerminalActor::ReleasedStage(scope.clone()),
                TerminalAuthority::Released(proof.clone()),
            ),
        };
        self.seal_demand_ended(
            &job,
            TerminalCommand {
                actor: actor.clone(),
                tail: actor,
                authority,
                expected_job_revision: entry.job_revision(),
                candidate_dispatch: candidate.as_ref().map(|candidate| candidate.scope.dispatch),
                now_ms: self.clock.now_ms(),
            },
        )
        .await?;
        if let Some(candidate) = candidate {
            self.settle_image_chain(&job, &candidate).await?;
        }
        Ok(())
    }

    async fn seal_demand_ended(
        &self,
        job: &ImageJob,
        terminal: TerminalCommand,
    ) -> Result<NoDeliveryReceipt, RuntimeError> {
        if let Some(receipt) = self
            .publisher
            .read_no_delivery(&job.owner, &job.job)
            .await?
        {
            return Ok(receipt);
        }
        let retained = job.clone();
        let now = self.clock.now_ms();
        let qualified =
            tokio::task::spawn_blocking(move || super::qualify_demand_ended(&retained, now))
                .await
                .map_err(|_| RuntimeError::WorkerFailed)??;
        let receipt = self
            .publisher
            .no_delivery(&NoDeliveryRequest {
                terminal: &terminal,
                cause: NoDeliveryCause::DemandEnded(&qualified),
            })
            .await?;
        let scope = terminal.tail.scope();
        let stage = self
            .execution
            .read_stage(&scope.owner, &scope.job, &scope.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::Cancelled,
            self.event_context(job, &stage, None),
        );
        event.measurements.supplier_dispatched = Some(false);
        event.measurements.raw_checkpointed = Some(false);
        self.telemetry.observe(event);
        Ok(receipt)
    }

    async fn settle_terminal_candidate(
        &self,
        job: &ImageJob,
        scope: &StageScope,
    ) -> Result<(), RuntimeError> {
        if let Some(candidate) = self.terminal_candidate(scope).await? {
            self.settle_image_chain(job, &candidate).await?;
        }
        Ok(())
    }

    pub(super) async fn terminal_candidate(
        &self,
        scope: &StageScope,
    ) -> Result<Option<CandidateReceipt>, RuntimeError> {
        let basis = self
            .execution
            .read_stage_basis(&scope.owner, &scope.job, &scope.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let mut cursor = basis.parent_dispatch;
        let mut seen = BTreeSet::new();
        while let Some(dispatch) = cursor {
            if !seen.insert(dispatch) || seen.len() > crate::publication::MAXIMUM_RECIPE_BYTES / 16
            {
                return Err(RuntimeError::InvalidRecord);
            }
            let current = self
                .execution
                .read_dispatch(&scope.owner, &dispatch)
                .await
                .map_err(RuntimeError::Execution)?;
            if current.job != scope.job {
                return Err(RuntimeError::InvalidRecord);
            }
            if let Some(candidate) = self
                .execution
                .read_candidate_checkpoint(&scope.owner, &scope.job, &current.stage, &dispatch)
                .await
                .map_err(RuntimeError::Execution)?
            {
                return Ok(Some(candidate));
            }
            cursor = self
                .execution
                .read_stage_basis(&scope.owner, &scope.job, &current.stage)
                .await
                .map_err(RuntimeError::Execution)?
                .parent_dispatch;
        }
        Ok(None)
    }
}
