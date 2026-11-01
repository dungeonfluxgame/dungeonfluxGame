//! Terminal output disposition uses protected semantic evidence and a fenced actual graph tail.
use super::{
    PublicationDecisionEvidence, PublicationEvidence, ports::*, worker::NativeImageRuntime,
};
use crate::cost::{ImageCostProjection, UsageRevision};
use crate::execution::*;
use crate::publication::{NoDeliveryCause, NoDeliveryRequest};
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
    pub(super) async fn publication_evidence(
        &self,
        job: &ImageJob,
        actor: DispatchId,
    ) -> Result<PublicationEvidence, RuntimeError> {
        let mut cursor = Some(actor);
        let mut seen = BTreeSet::new();
        let mut evidence = PublicationEvidence::default();
        let mut decisions = Vec::new();
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
                    let decision = self
                        .execution
                        .read_decision_checkpoint(&job.owner, &job.job, &current.stage, &dispatch)
                        .await
                        .map_err(RuntimeError::Execution)?
                        .ok_or(RuntimeError::InvalidRecord)?;
                    decisions.push(decision);
                }
                StageKind::Generation
                | StageKind::Edit
                | StageKind::Correction
                | StageKind::Extraction => {
                    let candidate = self
                        .execution
                        .read_candidate_checkpoint(&job.owner, &job.job, &current.stage, &dispatch)
                        .await
                        .map_err(RuntimeError::Execution)?
                        .ok_or(RuntimeError::InvalidRecord)?;
                    evidence.candidates.push(candidate);
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
        evidence
            .candidates
            .sort_by_key(|candidate| candidate.scope.dispatch);
        for decision in decisions {
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
        for candidate in &evidence.candidates {
            if let Some(stage) = self
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
                if let Some(local) = self
                    .execution
                    .read_local_inspection(&job.owner, &job.job, &stage.stage)
                    .await
                    .map_err(RuntimeError::Execution)?
                {
                    evidence.local_inspections.push(local);
                }
            }
        }
        evidence
            .local_inspections
            .sort_by_key(|local| local.scope.stage);
        Ok(evidence)
    }

    pub(super) async fn finish_no_output(
        &self,
        context: &PublicationContext,
        usage: &RuntimeUsage,
        mode: DispatchMode,
    ) -> Result<(), RuntimeError> {
        if context.stage.kind.is_standalone() {
            let job = self
                .journal
                .read_job(&context.job.owner, context.job.job)
                .await
                .map_err(RuntimeError::Journal)?
                .ok_or(RuntimeError::InvalidRecord)?;
            let dispatch = self
                .execution
                .read_dispatch(&job.owner, &context.dispatch)
                .await
                .map_err(RuntimeError::Execution)?;
            let basis = self
                .execution
                .read_stage_basis(&job.owner, &job.job, &context.stage.stage)
                .await
                .map_err(RuntimeError::Execution)?;
            let retained_job = job.clone();
            let retained_raw = usage.clone();
            let qualified = tokio::task::spawn_blocking(move || {
                super::qualify_no_output(&retained_job, &dispatch, &basis, &retained_raw)
            })
            .await
            .map_err(|_| RuntimeError::WorkerFailed)??;
            self.admission
                .outcome_no_output(&DispatchNoOutputSettlement {
                    job: &job,
                    mode,
                    usage,
                    qualified: &qualified,
                })
                .await?;
            self.record_settlement(&job, context.dispatch, context.stage.stage)
                .await;
            return self.project_usage(usage).await;
        }
        let existing = self
            .publisher
            .read_no_delivery(&context.job.owner, &context.job.job)
            .await?;
        if existing.is_none() {
            let job = self
                .journal
                .read_job(&context.job.owner, context.job.job)
                .await
                .map_err(RuntimeError::Journal)?
                .ok_or(RuntimeError::InvalidRecord)?;
            let dispatch = self
                .execution
                .read_dispatch(&job.owner, &context.dispatch)
                .await
                .map_err(RuntimeError::Execution)?;
            let basis = self
                .execution
                .read_stage_basis(&job.owner, &job.job, &context.stage.stage)
                .await
                .map_err(RuntimeError::Execution)?;
            let retained_job = job.clone();
            let retained_usage = usage.clone();
            let retained_dispatch = dispatch.clone();
            let qualified = tokio::task::spawn_blocking(move || {
                super::qualify_no_output(&retained_job, &retained_dispatch, &basis, &retained_usage)
            })
            .await
            .map_err(|_| RuntimeError::WorkerFailed)??;
            let candidate = if matches!(
                context.stage.kind,
                StageKind::Correction | StageKind::Extraction
            ) {
                Some(self.source_candidate(&job, &context.stage).await?)
            } else {
                None
            };
            let actor = TerminalActor::Dispatch(ExecutionScope {
                owner: job.owner.clone(),
                job: job.job,
                stage: context.stage.stage,
                dispatch: context.dispatch,
            });
            let terminal = TerminalCommand {
                actor: actor.clone(),
                tail: actor,
                authority: terminal_authority(context),
                expected_job_revision: job.revision,
                candidate_dispatch: candidate.as_ref().map(|candidate| candidate.scope.dispatch),
                now_ms: self.clock.now_ms(),
            };
            self.publisher
                .no_delivery(&NoDeliveryRequest {
                    terminal: &terminal,
                    cause: NoDeliveryCause::NoOutput(&qualified),
                })
                .await?;
            self.emit(
                crate::telemetry::EventKind::Failed,
                &job,
                &context.stage,
                Some(context.dispatch),
            );
            if let Some(candidate) = candidate {
                self.settle_image_chain(&job, &candidate).await?;
            }
        }
        self.admission
            .outcome(&context.job, context.dispatch, mode, Some(false))
            .await?;
        self.record_settlement(&context.job, context.dispatch, context.stage.stage)
            .await;
        self.project_usage(usage).await
    }

    pub(super) async fn finish_required_preflight(
        &self,
        job: &ImageJob,
        claim: &StageClaim,
        candidate: &CandidateReceipt,
        source_digest: [u8; 32],
        reason: LocalInspectionReason,
    ) -> Result<(), RuntimeError> {
        let retained_job = job.clone();
        let retained_claim = claim.clone();
        let retained_candidate = candidate.clone();
        let now = self.clock.now_ms();
        let qualified = tokio::task::spawn_blocking(move || {
            super::qualify_required_preflight(
                &retained_job,
                &retained_claim,
                &retained_candidate,
                source_digest,
                reason,
                now,
            )
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)??;
        let actor = TerminalActor::UnsentStage(claim.clone());
        let terminal = TerminalCommand {
            actor: actor.clone(),
            tail: actor,
            authority: TerminalAuthority::Claim(claim.clone()),
            expected_job_revision: job.revision,
            candidate_dispatch: Some(candidate.scope.dispatch),
            now_ms: now,
        };
        self.publisher
            .no_delivery(&NoDeliveryRequest {
                terminal: &terminal,
                cause: NoDeliveryCause::RequiredPreflight(&qualified),
            })
            .await?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::Failed,
            crate::telemetry::EventContext {
                job: Some(job.job),
                stage: Some(claim.scope.stage),
                parent_attempt: Some(candidate.scope.dispatch),
                gate_revision: Some(self.settings.active_config_revision),
                ..Default::default()
            },
        )
        .failed(crate::telemetry::FailureKind::Validation);
        event.measurements.supplier_dispatched = Some(false);
        event.measurements.raw_checkpointed = Some(false);
        event.measurements.analysis_verdict = Some(crate::telemetry::AnalysisVerdict::Unjudged);
        self.telemetry.observe(event);
        self.settle_image_chain(job, candidate).await
    }

    pub(super) async fn finish_rejected_output(
        &self,
        context: &PublicationContext,
        candidate: Option<&CandidateReceipt>,
    ) -> Result<(), RuntimeError> {
        if self
            .publisher
            .read_no_delivery(&context.job.owner, &context.job.job)
            .await?
            .is_some()
        {
            if let Some(candidate) = candidate {
                self.settle_image_chain(&context.job, candidate).await?;
            }
            return Ok(());
        }
        let job = self
            .journal
            .read_job(&context.job.owner, context.job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let evidence = self.publication_evidence(&job, context.dispatch).await?;
        let retained_job = job.clone();
        let selected = candidate.cloned();
        let qualified = tokio::task::spawn_blocking(move || {
            super::qualify_no_delivery(&retained_job, &evidence, selected.as_ref())
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)??;
        let actor = TerminalActor::Dispatch(ExecutionScope {
            owner: job.owner.clone(),
            job: job.job,
            stage: context.stage.stage,
            dispatch: context.dispatch,
        });
        let terminal = TerminalCommand {
            actor: actor.clone(),
            tail: actor,
            authority: terminal_authority(context),
            expected_job_revision: job.revision,
            candidate_dispatch: candidate.map(|candidate| candidate.scope.dispatch),
            now_ms: self.clock.now_ms(),
        };
        self.publisher
            .no_delivery(&NoDeliveryRequest {
                terminal: &terminal,
                cause: NoDeliveryCause::Semantic(&qualified),
            })
            .await?;
        self.emit(
            crate::telemetry::EventKind::Failed,
            &job,
            &context.stage,
            Some(context.dispatch),
        );
        if let Some(candidate) = candidate {
            self.settle_image_chain(&job, candidate).await?;
        }
        Ok(())
    }
}

fn terminal_authority(context: &PublicationContext) -> TerminalAuthority {
    context.recovery.clone().map_or_else(
        || {
            TerminalAuthority::Claim(StageClaim {
                scope: StageScope {
                    owner: context.job.owner.clone(),
                    job: context.job.job,
                    stage: context.stage.stage,
                },
                token: context.claim,
                generation: context.stage.claim_generation,
            })
        },
        TerminalAuthority::Recovery,
    )
}
