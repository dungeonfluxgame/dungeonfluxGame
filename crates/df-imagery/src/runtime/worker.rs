//! One owned native queue consumer. Stopping waits for the current owned execution.

use super::{dispatch, ports::*};
use crate::cost::{ImageCostProjection, UsageEvidence, UsageReport, UsageRevision};
use crate::execution::*;
use crate::publication::{FrozenOutput, PublishedImage};
use crate::{
    AlphaHandling, FreezeCommand, FreezeOutcome, ImageConfig, ImageJob, ImageJournal,
    ImageOperation, JobState, PolicyDecision, PolicyInput, ServiceClock, decide,
};
use df_assets::{ImageAlphaRequirement, ImageIntent, MediaRecipeRequest, MediaRequestKey};
use sha2::Digest;
use std::time::Duration;
use tokio::sync::oneshot;

/// Applied configuration and durable capacity scope acknowledged by this worker.
pub struct RuntimeSettings {
    pub config: ImageConfig,
    pub active_config_revision: u64,
    pub capacity: CapacityScope,
    pub poll_interval: Duration,
}

/// Infrastructure-only composition; consumer requests continue through ImageService.
pub struct NativeImageRuntime<J, E, S, P, F, C, I, K, B, T, Q, U> {
    pub journal: J,
    pub execution: E,
    pub sources: S,
    pub publisher: P,
    pub admission: F,
    pub clock: C,
    pub ids: I,
    pub credentials: K,
    pub costs: B,
    pub cost_policy: Q,
    pub usage: U,
    pub telemetry: T,
    pub settings: RuntimeSettings,
}

#[derive(Default)]
pub(super) struct RuntimeCursors {
    queue: Option<QueueCursor>,
    pending_projection: Option<(DispatchId, UsageRevision)>,
    pub(super) prepared: Option<DispatchId>,
    pub(super) terminal: Option<TerminalCursor>,
}

/// Stop control for a runtime future kept by the host task owner. Shutdown awaits owned cleanup.
pub struct RuntimeHandle {
    stop: Option<oneshot::Sender<()>>,
    finished: oneshot::Receiver<()>,
}
impl RuntimeHandle {
    pub async fn shutdown(mut self) -> Result<(), RuntimeError> {
        if let Some(stop) = self.stop.take() {
            // A closed receiver means the worker already ended; completion still reports below.
            let _closed = stop.send(());
        }
        self.finished.await.map_err(|_| RuntimeError::WorkerFailed)
    }
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
    /// Returns owned work, never a detached task. The host retains the future in its task owner.
    pub fn start(
        self,
    ) -> (
        RuntimeHandle,
        impl std::future::Future<Output = Result<(), RuntimeError>>,
    ) {
        let (stop, mut stopped) = oneshot::channel();
        let (finished, completion) = oneshot::channel();
        let handle = RuntimeHandle {
            stop: Some(stop),
            finished: completion,
        };
        let future = async move {
            let result = async {
                self.validate()?;
                let mut cursors = RuntimeCursors::default();
                loop {
                    tokio::select! {
                        _ = &mut stopped => return Ok(()),
                        _ = tokio::time::sleep(self.settings.poll_interval) => {}
                    }
                    match Box::pin(
                        self.run_next_cursor(&mut cursors, Some(self.settings.poll_interval)),
                    )
                    .await
                    {
                        Ok(_) => {}
                        // These outcomes already completed their durable attempt and emitted telemetry.
                        Err(
                            RuntimeError::ProviderRefused
                            | RuntimeError::ProviderUnknown
                            | RuntimeError::SupplierQuotaExhausted
                            | RuntimeError::SupplierRateLimited
                            | RuntimeError::SupplierFailure(_)
                            | RuntimeError::CostProjectionPending
                            // Another worker holds or has finished this stage.
                            | RuntimeError::Execution(
                                ExecutionError::LostClaim | ExecutionError::Superseded,
                            ),
                        ) => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            .await;
            // The host also retains the result future; this signal only acknowledges cleanup.
            let _closed = finished.send(());
            result
        };
        (handle, future)
    }

    pub(super) fn event_context(
        &self,
        job: &ImageJob,
        stage: &StageView,
        dispatch: Option<DispatchId>,
    ) -> crate::telemetry::EventContext {
        let mut context = crate::telemetry::EventContext::for_stage(job, stage, dispatch);
        context.gate_revision = Some(self.settings.active_config_revision);
        if crate::service::analysis_codec::is_analysis_intent(job.intent.bytes()) {
            if let Ok(intent) = crate::service::analysis_codec::decode_analysis(job.intent.bytes())
            {
                context.purpose = Some(intent.purpose.purpose);
                context.analysis_kind = Some(match intent.operation {
                    crate::ImageAnalysisOperation::PhotoInspection(_) => {
                        crate::telemetry::AnalysisKind::PhotoInspection
                    }
                    crate::ImageAnalysisOperation::HeadLocations(_) => {
                        crate::telemetry::AnalysisKind::HeadLocations
                    }
                    crate::ImageAnalysisOperation::Segmentation(_) => {
                        crate::telemetry::AnalysisKind::Segmentation
                    }
                    crate::ImageAnalysisOperation::StagingIdentity(_) => {
                        crate::telemetry::AnalysisKind::StagingIdentity
                    }
                    crate::ImageAnalysisOperation::StagingFacing(_) => {
                        crate::telemetry::AnalysisKind::StagingFacing
                    }
                });
            }
            return context;
        }
        if let Ok(intent) = crate::intent_codec::decode_intent(
            job.intent.bytes(),
            &crate::service::PROTOCOL_LIMITS,
            crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
        ) {
            context.purpose = Some(intent.purpose.purpose);
            context.analysis_kind = match stage.kind {
                StageKind::Classification => {
                    Some(crate::telemetry::AnalysisKind::CreatorClassification)
                }
                StageKind::Inspection => Some(crate::telemetry::AnalysisKind::Inspection),
                _ => None,
            };
            if !matches!(
                stage.kind,
                StageKind::LocalTransform | StageKind::Classification | StageKind::Inspection
            ) && let Ok(config) = super::retained_config(job)
            {
                context.backend = Some(dispatch::backend(&config, &intent));
            }
        }
        context
    }

    pub(super) fn emit(
        &self,
        kind: crate::telemetry::EventKind,
        job: &ImageJob,
        stage: &StageView,
        dispatch: Option<DispatchId>,
    ) {
        self.telemetry.observe(crate::telemetry::ImageEvent::new(
            kind,
            self.event_context(job, stage, dispatch),
        ));
    }

    pub(super) async fn publication_ack(
        &self,
        context: &PublicationContext,
        result: &PublishedImage,
    ) {
        let mut event_context = self.terminal_event_context(context).await;
        event_context.asset = Some(result.asset);
        self.telemetry.observe(crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::Published,
            event_context,
        ));
    }
    pub(super) async fn completed(&self, context: &PublicationContext, result: &PublishedImage) {
        let mut event_context = self.terminal_event_context(context).await;
        event_context.asset = Some(result.asset);
        self.telemetry.observe(crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::Completed,
            event_context,
        ));
    }
    async fn terminal_event_context(
        &self,
        context: &PublicationContext,
    ) -> crate::telemetry::EventContext {
        let mut event = self.event_context(&context.job, &context.stage, Some(context.dispatch));
        match self
            .execution
            .read_stage_basis(&context.job.owner, &context.job.job, &context.stage.stage)
            .await
        {
            Ok(basis) => {
                event.config_revision = Some(basis.config_revision);
                event.parent_attempt = basis.parent_dispatch;
            }
            Err(_) => self.telemetry.observe(
                crate::telemetry::ImageEvent::new(
                    crate::telemetry::EventKind::Failed,
                    event.clone(),
                )
                .failed(crate::telemetry::FailureKind::Journal),
            ),
        }
        event
    }

    fn validate(&self) -> Result<(), RuntimeError> {
        self.settings
            .config
            .validate()
            .map_err(|_| RuntimeError::InvalidConfiguration)?;
        if self.settings.active_config_revision == 0 || self.settings.poll_interval.is_zero() {
            return Err(RuntimeError::InvalidConfiguration);
        }
        Ok(())
    }

    /// Executes at most one durably queued stage, preserving the same dispatch on recovery.
    pub async fn run_next(&self) -> Result<Option<PublishedImage>, RuntimeError> {
        Box::pin(self.run_next_cursor(&mut RuntimeCursors::default(), None)).await
    }
    async fn run_next_cursor(
        &self,
        cursors: &mut RuntimeCursors,
        budget: Option<Duration>,
    ) -> Result<Option<PublishedImage>, RuntimeError> {
        self.validate()?;
        for usage in self
            .usage
            .pending_usage(self.settings.capacity.as_str(), 1)
            .await?
        {
            let key = (usage.dispatch, usage.revision);
            let observed = cursors.pending_projection != Some(key);
            match self.project_usage_observed(&usage, observed).await {
                Ok(()) => cursors.pending_projection = None,
                Err(RuntimeError::CostProjectionPending) => {
                    cursors.pending_projection = Some(key);
                    return Err(RuntimeError::CostProjectionPending);
                }
                Err(error) => {
                    cursors.pending_projection = None;
                    return Err(error);
                }
            }
        }
        if let Some(result) = Box::pin(self.recover_next()).await? {
            return Ok(Some(result));
        }
        Box::pin(self.cleanup_unsent(cursors)).await?;
        let cursor = &mut cursors.queue;
        let queue_started = std::time::Instant::now();
        let mut attempted = false;
        loop {
            if attempted && budget.is_some_and(|budget| queue_started.elapsed() >= budget) {
                return Ok(None);
            }
            let page = self
                .execution
                .scan_queue_after(
                    self.settings.capacity.as_str(),
                    *cursor,
                    1,
                    self.clock.now_ms(),
                )
                .await
                .map_err(RuntimeError::Execution)?;
            *cursor = page.next;
            attempted = true;
            let Some(queued) = page.stages.first() else {
                return Ok(None);
            };
            let job = self
                .journal
                .read_job(&queued.owner, queued.job)
                .await
                .map_err(RuntimeError::Journal)?
                .ok_or(RuntimeError::InvalidRecord)?;
            let stage = self
                .execution
                .read_stage(&queued.owner, &queued.job, &queued.stage)
                .await
                .map_err(RuntimeError::Execution)?;
            match Box::pin(self.run_queued(job.clone(), stage.clone())).await {
                Err(
                    error @ (RuntimeError::CredentialUnavailable
                    | RuntimeError::UnsupportedCapability),
                ) => {
                    let reason = if error == RuntimeError::CredentialUnavailable {
                        super::PreflightRejection::MissingCredential
                    } else {
                        super::PreflightRejection::UnsupportedProfile
                    };
                    match Box::pin(self.finish_preflight_rejection(&job, &stage, reason)).await {
                        Ok(()) => {}
                        Err(RuntimeError::Execution(
                            ExecutionError::Retired
                            | ExecutionError::Expired
                            | ExecutionError::DemandLapsed
                            | ExecutionError::LostClaim,
                        )) => {}
                        Err(error) => return Err(error),
                    }
                    if cursor.is_none() {
                        return Ok(None);
                    }
                }
                Err(
                    error @ (RuntimeError::Expired
                    | RuntimeError::Cancelled
                    | RuntimeError::Execution(
                        ExecutionError::Retired
                        | ExecutionError::Expired
                        | ExecutionError::DemandLapsed,
                    )),
                ) => {
                    let expired = matches!(
                        error,
                        RuntimeError::Expired | RuntimeError::Execution(ExecutionError::Expired)
                    );
                    let mut event = crate::telemetry::ImageEvent::new(
                        if expired {
                            crate::telemetry::EventKind::Failed
                        } else {
                            crate::telemetry::EventKind::Cancelled
                        },
                        self.event_context(&job, &stage, None),
                    );
                    event.failure = Some(if expired {
                        crate::telemetry::FailureKind::Expired
                    } else {
                        crate::telemetry::FailureKind::Cancelled
                    });
                    self.telemetry.observe(event);
                    if cursor.is_none() {
                        return Ok(None);
                    }
                }
                Err(
                    error @ (RuntimeError::SourceDenied
                    | RuntimeError::QuotaBlocked { .. }
                    | RuntimeError::Disabled
                    | RuntimeError::Execution(
                        ExecutionError::Disabled | ExecutionError::ConfigFenced { .. },
                    )),
                ) => {
                    if error == RuntimeError::SourceDenied {
                        match Box::pin(self.close_queued_denied_sources(&job, &stage)).await {
                            Ok(true) => {
                                if cursor.is_none() {
                                    return Ok(None);
                                }
                                continue;
                            }
                            Ok(false)
                            | Err(RuntimeError::Execution(
                                ExecutionError::LostClaim
                                | ExecutionError::CapacityFull
                                | ExecutionError::Retired
                                | ExecutionError::Expired
                                | ExecutionError::DemandLapsed
                                | ExecutionError::Disabled
                                | ExecutionError::ConfigFenced { .. },
                            )) => {}
                            Err(error) => return Err(error),
                        }
                    }
                    self.defer_unsent(&job, &stage, &error).await?;
                    if cursor.is_none() {
                        return Ok(None);
                    }
                }
                Err(error) => {
                    super::diagnostics::correlate_failure(
                        &self.telemetry,
                        &error,
                        self.event_context(&job, &stage, None),
                    );
                    return Err(error);
                }
                result => return result,
            }
        }
    }

    async fn defer_unsent(
        &self,
        job: &ImageJob,
        stage: &StageView,
        reason: &RuntimeError,
    ) -> Result<(), RuntimeError> {
        let now = self.clock.now_ms();
        let current = self
            .execution
            .read_current_dispatch(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let stage = self
            .execution
            .read_stage(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        if stage.state == StageState::Released {
            return Ok(());
        }
        let context = self.event_context(
            job,
            &stage,
            current.as_ref().map(|dispatch| dispatch.dispatch),
        );
        self.telemetry.observe(
            crate::telemetry::ImageEvent::new(crate::telemetry::EventKind::Failed, context).failed(
                if matches!(
                    reason,
                    RuntimeError::Disabled
                        | RuntimeError::Execution(
                            ExecutionError::Disabled | ExecutionError::ConfigFenced { .. }
                        )
                ) {
                    crate::telemetry::FailureKind::Disabled
                } else if matches!(reason, RuntimeError::QuotaBlocked { .. }) {
                    crate::telemetry::FailureKind::Capacity
                } else {
                    crate::telemetry::FailureKind::Authorization
                },
            ),
        );
        if current.is_some() || stage.lease_until_ms > now {
            return Ok(());
        }
        let backoff = self
            .settings
            .config
            .retry
            .backoff_secs
            .checked_mul(1000)
            .ok_or(RuntimeError::InvalidConfiguration)?
            .max(
                u64::try_from(self.settings.poll_interval.as_millis())
                    .map_err(|_| RuntimeError::InvalidConfiguration)?,
            );
        let due = now
            .checked_add(i64::try_from(backoff).map_err(|_| RuntimeError::InvalidConfiguration)?)
            .ok_or(RuntimeError::InvalidConfiguration)?;
        let due = match reason {
            RuntimeError::QuotaBlocked {
                retry_at_ms: Some(quota_due),
                ..
            } => due.max(*quota_due),
            _ => due,
        };
        match self
            .execution
            .defer_queued(
                &DeferQueuedCommand::new(
                    StageScope {
                        owner: job.owner.clone(),
                        job: job.job,
                        stage: stage.stage,
                    },
                    due,
                    match reason {
                        RuntimeError::QuotaBlocked { notice, .. } => *notice,
                        _ => None,
                    },
                    now,
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
        {
            Ok(()) => {}
            Err(
                ExecutionError::Retired
                | ExecutionError::Expired
                | ExecutionError::DemandLapsed
                | ExecutionError::LostClaim
                | ExecutionError::OutputSealed,
            ) => return Ok(()),
            Err(error) => return Err(RuntimeError::Execution(error)),
        }
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::SourceDeferred,
            self.event_context(job, &stage, None),
        );
        event.measurements.quota_deferral = match reason {
            RuntimeError::QuotaBlocked { notice, .. } => *notice,
            _ => None,
        };
        event.measurements.retry_after_ms =
            Some(u64::try_from(due - now).map_err(|_| RuntimeError::InvalidConfiguration)?);
        event.failure = Some(if matches!(reason, RuntimeError::QuotaBlocked { .. }) {
            crate::telemetry::FailureKind::Capacity
        } else if matches!(reason, RuntimeError::SourceDenied) {
            crate::telemetry::FailureKind::Authorization
        } else {
            crate::telemetry::FailureKind::Disabled
        });
        self.telemetry.observe(event);
        Ok(())
    }

    pub(super) async fn run_queued_attempt(
        &self,
        job: ImageJob,
        stage: StageView,
        retry: &mut super::retry::RetryState,
    ) -> Result<Option<PublishedImage>, RuntimeError> {
        if crate::service::analysis_codec::is_analysis_intent(job.intent.bytes()) {
            return Box::pin(self.run_standalone(job, stage)).await;
        }
        let current = self
            .execution
            .read_current_dispatch(&job.owner, &job.job, &stage.stage)
            .await
            .map_err(RuntimeError::Execution)?;
        let intent = crate::intent_codec::decode_intent(
            job.intent.bytes(),
            &crate::service::PROTOCOL_LIMITS,
            crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let config = crate::intent_codec::decode_config(
            job.config.bytes(),
            crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        if intent.owner != job.owner {
            return Err(RuntimeError::InvalidRecord);
        }
        if stage.kind == StageKind::Classification {
            if current.is_some() {
                return Err(RuntimeError::ProviderUnknown);
            }
            let policy = PolicyInput::new(
                intent.purpose.purpose,
                dispatch::backend(&self.settings.config, &intent),
                intent.execution,
                true,
                false,
                false,
            );
            if !matches!(
                decide(&self.settings.config, &policy),
                PolicyDecision::Admit
            ) {
                return Err(RuntimeError::Disabled);
            }
            Box::pin(self.run_classification(job, stage, &intent, &config)).await?;
            return Ok(None);
        }
        if matches!(stage.kind, StageKind::Inspection | StageKind::Extraction) {
            let policy = PolicyInput::new(
                intent.purpose.purpose,
                dispatch::backend(&self.settings.config, &intent),
                intent.execution,
                true,
                false,
                false,
            );
            if !matches!(
                decide(&self.settings.config, &policy),
                PolicyDecision::Admit
            ) {
                return Err(RuntimeError::Disabled);
            }
            if current.is_some() {
                return Err(RuntimeError::ProviderUnknown);
            }
            let (resolved_config, _) = self.classified_config(&job, &stage, &config).await?;
            if stage.kind == StageKind::Inspection {
                Box::pin(self.run_inspection_stage(job, stage, &intent, &resolved_config)).await?;
            } else {
                Box::pin(self.run_extraction_stage(job, stage, &intent, &resolved_config)).await?;
            }
            return Ok(None);
        }
        if stage.kind == StageKind::LocalTransform && super::requires_final_selection(&job)? {
            return Box::pin(self.run_final_stage(job, stage, &intent, &config))
                .await
                .map(Some);
        }
        let (resolved_config, classification) =
            self.classified_config(&job, &stage, &config).await?;
        let intent = self
            .execution_intent(&job, &stage, &intent, classification.as_ref())
            .await?;
        let is_local = matches!(intent.operation, ImageOperation::Transform(_));
        let policy = PolicyInput::new(
            intent.purpose.purpose,
            dispatch::backend(&resolved_config, &intent),
            intent.execution,
            !is_local,
            true,
            false,
        );
        let recovering = current.as_ref().is_some_and(|dispatch| {
            matches!(
                dispatch.state,
                DispatchState::Sent | DispatchState::Unknown | DispatchState::Succeeded
            )
        });
        if !recovering {
            if !matches!(
                decide(&self.settings.config, &policy),
                PolicyDecision::Admit
            ) {
                return Err(RuntimeError::Disabled);
            }
            dispatch::preflight(&resolved_config, &intent)?;
            if job.state == JobState::Retired {
                return Err(RuntimeError::Cancelled);
            }
        }
        let inputs = dispatch::resolve_inputs(&intent, &self.sources).await?;
        let credentials = match retry.credentials.take() {
            Some(credentials) => credentials,
            None => dispatch::pin_credentials(&resolved_config, &intent, &self.credentials)?,
        };
        let quota = if recovering || retry.quota.is_some() {
            None
        } else {
            dispatch::admit_image_quota(
                &resolved_config,
                &intent,
                self.settings.config.quota,
                &credentials,
            )?
        };
        let now_ms = self.clock.now_ms();
        let lease_ms = super::lease::claim_lease_ms(&config, job.absolute_deadline_ms, now_ms)?;
        let claim = ClaimToken::from_bytes(&self.ids.next_bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let stage = self
            .execution
            .claim(
                &ClaimCommand::new(
                    job.owner.clone(),
                    job.job,
                    stage.stage,
                    self.settings.active_config_revision,
                    claim,
                    lease_ms,
                    now_ms,
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(RuntimeError::Execution)?;
        self.emit(crate::telemetry::EventKind::Claimed, &job, &stage, None);
        let quota = match retry.quota.take() {
            Some(permit) => Some(permit),
            None => super::quota_gate::claim(quota)?,
        };
        let job = self
            .journal
            .read_job(&job.owner, job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        let job = if job.recipe.is_none() {
            self.freeze(
                job,
                &intent,
                &resolved_config,
                classification.as_ref(),
                inputs.analysis.as_ref(),
            )
            .await?
        } else {
            job
        };
        let route = if is_local {
            None
        } else {
            let mut basis = super::retained_cost_basis(&job, &resolved_config, stage.kind)?;
            basis.prompt_bytes = match &intent.operation {
                ImageOperation::Generate(spec) => spec.prompt.text().len() as u64,
                ImageOperation::Edit(spec) => spec.prompt.text().len() as u64,
                ImageOperation::Transform(_) => 0,
            };
            Some(
                self.cost_policy
                    .stage_route(&job, &resolved_config, &basis)?,
            )
        };
        // A successful retry claim proves the selected decline marker; it is not a resume.
        let retrying = current
            .as_ref()
            .is_some_and(|current| current.state == DispatchState::Declined);
        let current = current.filter(|current| current.state != DispatchState::Declined);
        let (dispatch, mode, resume) = if let Some(current) = current {
            match current.state {
                DispatchState::Sent | DispatchState::Unknown => (
                    current.dispatch,
                    current
                        .finance
                        .map_or(DispatchMode::Local, DispatchMode::Hosted),
                    true,
                ),
                DispatchState::Succeeded => {
                    let context = PublicationContext {
                        job,
                        stage,
                        dispatch: current.dispatch,
                        claim,
                        now_ms,
                        usage_revision: current.usage_revision,
                        recovery: None,
                    };
                    return self.publish_output(&context).await.map(Some);
                }
                // Prepared without a retained egress acknowledgement is recovered conservatively.
                DispatchState::Prepared => return Err(RuntimeError::ProviderUnknown),
                DispatchState::Failed | DispatchState::Released | DispatchState::Declined => {
                    return Ok(None);
                }
            }
        } else {
            let dispatch = DispatchId::from_bytes(&self.ids.next_bytes())
                .map_err(|_| RuntimeError::InvalidRecord)?;
            let admission = DispatchAdmission {
                job: &job,
                stage: &stage,
                dispatch,
                claim,
                config: &config,
                quote: route.as_ref(),
                sources: &inputs.receipts,
            };
            let admitted = if matches!(stage.kind, StageKind::Generation | StageKind::Edit)
                && crate::service::artifact_recipe::retained_campaign_artifact_recipe(
                    &job.intent,
                    &job.config,
                )
                .map_err(RuntimeError::Journal)?
                .is_some()
            {
                match self.admission.admit_with_artifact(&admission).await {
                    Ok(ArtifactAdmission::Admitted(mode)) => Ok(mode),
                    Ok(
                        ArtifactAdmission::ArtifactPending | ArtifactAdmission::ArtifactResolved,
                    ) => return Ok(None),
                    Err(error) => Err(error),
                }
            } else {
                self.admission.admit(&admission).await
            };
            let mode = match admitted {
                Ok(mode) => mode,
                Err(error) => {
                    if retrying
                        && matches!(
                            error,
                            RuntimeError::ProviderRefused
                                | RuntimeError::SourceDenied
                                | RuntimeError::Disabled
                                | RuntimeError::UnsupportedCapability
                                | RuntimeError::CredentialUnavailable
                        )
                    {
                        self.execution
                            .release(
                                &ReleaseCommand::new(
                                    job.owner.clone(),
                                    job.job,
                                    stage.stage,
                                    claim,
                                    stage.claim_generation,
                                    self.clock.now_ms(),
                                )
                                .map_err(|_| RuntimeError::InvalidRecord)?,
                            )
                            .await
                            .map_err(RuntimeError::Execution)?;
                    }
                    return Err(error);
                }
            };
            if stage.local_accounting != mode.is_local() {
                return Err(RuntimeError::InvalidRecord);
            }
            self.execution
                .prepare(
                    &PrepareCommand::new(
                        StageClaim {
                            scope: StageScope {
                                owner: job.owner.clone(),
                                job: job.job,
                                stage: stage.stage,
                            },
                            token: claim,
                            generation: stage.claim_generation,
                        },
                        dispatch,
                        mode,
                        self.clock.now_ms(),
                    )
                    .map_err(|_| RuntimeError::InvalidRecord)?,
                )
                .await
                .map_err(RuntimeError::Execution)?;
            self.emit(
                crate::telemetry::EventKind::Admitted,
                &job,
                &stage,
                Some(dispatch),
            );
            (dispatch, mode, false)
        };
        let mut context = PublicationContext {
            job,
            stage,
            dispatch,
            claim,
            now_ms: self.clock.now_ms(),
            usage_revision: 0,
            recovery: None,
        };
        self.usage
            .record_sources(
                &ExecutionScope {
                    owner: context.job.owner.clone(),
                    job: context.job.job,
                    stage: context.stage.stage,
                    dispatch,
                },
                &inputs.receipts,
            )
            .await?;
        let call_id = crate::cost::pinned_call_id(dispatch.as_bytes());
        if !is_local {
            self.costs
                .start_pinned(&call_id, route.as_ref().ok_or(RuntimeError::InvalidRecord)?)
                .await
                .map_err(RuntimeError::Cost)?;
        }
        if !resume {
            if self
                .admission
                .mark_sent(
                    &DispatchAdmission {
                        job: &context.job,
                        stage: &context.stage,
                        dispatch,
                        claim: context.claim,
                        config: &config,
                        quote: route.as_ref(),
                        sources: &inputs.receipts,
                    },
                    mode,
                )
                .await?
                == SendAuthorization::AlreadySent
            {
                if let Some(route) = &route {
                    let usage = self
                        .retain_usage(&context, route, UsageEvidence::Unknown, &[], None, false)
                        .await?;
                    self.project_usage(&usage).await?;
                }
                self.complete(&context, CompletionOutcome::Unknown).await?;
                self.admission
                    .outcome(&context.job, dispatch, mode, None)
                    .await?;
                return Err(RuntimeError::ProviderUnknown);
            }
            self.execution
                .mark_sent(
                    &MarkSentCommand::new(
                        context.job.owner.clone(),
                        context.job.job,
                        context.stage.stage,
                        dispatch,
                        claim,
                        context.stage.claim_generation,
                        self.clock.now_ms(),
                    )
                    .map_err(|_| RuntimeError::InvalidRecord)?,
                )
                .await
                .map_err(RuntimeError::Execution)?;
            self.emit(
                crate::telemetry::EventKind::Sent,
                &context.job,
                &context.stage,
                Some(dispatch),
            );
        }
        let timeout = super::lease::provider_call_timeout(
            &config,
            context.stage.lease_until_ms,
            self.clock.now_ms(),
        )?;
        let execution_started = std::time::Instant::now();
        if is_local {
            self.emit(
                crate::telemetry::EventKind::TransformStarted,
                &context.job,
                &context.stage,
                Some(dispatch),
            );
        }
        let executed = dispatch::execute(
            dispatch::DispatchInput {
                context: &context,
                intent: &intent,
                config: &resolved_config,
                timeout,
                resume,
                telemetry: &self.telemetry,
                event_context: self.event_context(&context.job, &context.stage, Some(dispatch)),
            },
            &self.execution,
            inputs,
            &credentials,
        )
        .await;
        match executed {
            Ok(image) => {
                super::quota_gate::observe(
                    quota,
                    super::quota::QuotaAnswer::Succeeded,
                    &self.telemetry,
                    self.event_context(&context.job, &context.stage, Some(dispatch)),
                );
                let mut event = crate::telemetry::ImageEvent::new(
                    if is_local {
                        crate::telemetry::EventKind::TransformCompleted
                    } else {
                        crate::telemetry::EventKind::SupplierCompleted
                    },
                    self.event_context(&context.job, &context.stage, Some(dispatch)),
                )
                .elapsed(execution_started.elapsed());
                event.measurements.output_bytes = u64::try_from(image.bytes.len()).ok();
                self.telemetry.observe(event);
                let raw = if is_local {
                    None
                } else {
                    Some(
                        self.retain_usage(
                            &context,
                            route.as_ref().ok_or(RuntimeError::InvalidRecord)?,
                            image.evidence,
                            &image.units,
                            Some(&image.bytes),
                            true,
                        )
                        .await?,
                    )
                };
                context.usage_revision = raw.as_ref().map_or(Ok(0), |usage| {
                    u32::try_from(usage.revision.0).map_err(|_| RuntimeError::InvalidRecord)
                })?;
                if !is_local && super::requires_final_selection(&context.job)? {
                    self.finish_candidate(
                        &context,
                        &intent,
                        &resolved_config,
                        raw.as_ref().ok_or(RuntimeError::InvalidRecord)?,
                        mode,
                    )
                    .await?;
                    return Ok(None);
                }
                if !is_local {
                    self.emit(
                        crate::telemetry::EventKind::NormalizationStarted,
                        &context.job,
                        &context.stage,
                        Some(dispatch),
                    );
                }
                let normalization_started = std::time::Instant::now();
                let image = if is_local {
                    image
                } else {
                    dispatch::normalize_output(&context, &intent, &resolved_config, image, timeout)
                        .await?
                };
                if !is_local {
                    self.telemetry.observe(
                        crate::telemetry::ImageEvent::new(
                            crate::telemetry::EventKind::NormalizationCompleted,
                            self.event_context(&context.job, &context.stage, Some(dispatch)),
                        )
                        .elapsed(normalization_started.elapsed()),
                    );
                }
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
                self.emit(
                    crate::telemetry::EventKind::OutputStored,
                    &context.job,
                    &context.stage,
                    Some(dispatch),
                );
                self.complete(
                    &context,
                    CompletionOutcome::Succeeded {
                        usage_revision: checkpoint.usage_revision,
                        checkpoint,
                    },
                )
                .await?;
                if let Some(raw) = raw {
                    self.project_usage(&raw).await?;
                }
                let result = self.publish_output(&context).await?;
                self.publication_ack(&context, &result).await;
                self.admission
                    .outcome(
                        &context.job,
                        dispatch,
                        mode,
                        Some(!result.private_reconciliation),
                    )
                    .await?;
                self.completed(&context, &result).await;
                Ok(Some(result))
            }
            Err(failure) => {
                let error = failure.error;
                let known_refusal = matches!(
                    error,
                    RuntimeError::ProviderRefused
                        | RuntimeError::UnsupportedCapability
                        | RuntimeError::SupplierFailure(
                            FailureCause::Refused
                                | FailureCause::InvalidOutput
                                | FailureCause::Provider
                                | FailureCause::QuotaExhausted
                        )
                );
                let raw = if is_local {
                    None
                } else {
                    let observed = failure
                        .usage
                        .or_else(|| failure.api.as_ref().map(dispatch::retain_api_failure));
                    Some(
                        self.retain_usage(
                            &context,
                            route.as_ref().ok_or(RuntimeError::InvalidRecord)?,
                            observed
                                .as_ref()
                                .map_or(UsageEvidence::Unknown, |usage| usage.evidence),
                            observed
                                .as_ref()
                                .map_or(&[], |usage| usage.units.as_slice()),
                            observed.as_ref().map(|usage| usage.raw.as_slice()),
                            false,
                        )
                        .await?,
                    )
                };
                if let (Some(failure), Some(raw)) = (&failure.api, &raw)
                    && failure.retryable_submit_decline().is_some()
                    && raw.evidence == UsageEvidence::Unknown
                {
                    let receipt = self.retain_decline(&context, failure, raw, mode).await?;
                    let queued = self
                        .execution
                        .requeue_declined(&DeclineRetryCommand {
                            claim: StageClaim {
                                scope: StageScope {
                                    owner: receipt.scope.owner.clone(),
                                    job: receipt.scope.job,
                                    stage: receipt.scope.stage,
                                },
                                token: context.claim,
                                generation: context.stage.claim_generation,
                            },
                            dispatch,
                            now_ms: self.clock.now_ms(),
                        })
                        .await;
                    match queued {
                        Ok(stage) => {
                            retry.quota = quota;
                            retry.credentials = Some(credentials);
                            retry.next =
                                Some((context.job.clone(), stage, receipt.retry_not_before_ms));
                            return Ok(None);
                        }
                        Err(
                            ExecutionError::Expired
                            | ExecutionError::Retired
                            | ExecutionError::DemandLapsed
                            | ExecutionError::Disabled
                            | ExecutionError::ConfigFenced { .. }
                            | ExecutionError::CapacityFull
                            | ExecutionError::OutputSealed,
                        ) => {}
                        Err(error) => return Err(RuntimeError::Execution(error)),
                    }
                    self.close_final_decline(&context, &receipt).await?;
                    super::quota_gate::observe(
                        quota,
                        super::quota::QuotaAnswer::Inconclusive,
                        &self.telemetry,
                        self.event_context(&context.job, &context.stage, Some(dispatch)),
                    );
                    self.emit(
                        crate::telemetry::EventKind::Unknown,
                        &context.job,
                        &context.stage,
                        Some(dispatch),
                    );
                    return Err(RuntimeError::SupplierRateLimited);
                }
                super::quota_gate::observe(
                    quota,
                    if error == RuntimeError::SupplierQuotaExhausted {
                        super::quota::QuotaAnswer::Exhausted
                    } else {
                        super::quota::QuotaAnswer::Inconclusive
                    },
                    &self.telemetry,
                    self.event_context(&context.job, &context.stage, Some(dispatch)),
                );
                let definitive = known_refusal
                    && (is_local
                        || raw
                            .as_ref()
                            .is_some_and(|usage| usage.evidence != UsageEvidence::Unknown));
                let cause = error.failure_cause().unwrap_or(FailureCause::Refused);
                let outcome = if definitive {
                    if let Some(usage) = &raw {
                        CompletionOutcome::Unusable {
                            cause,
                            usage_revision: u32::try_from(usage.revision.0)
                                .map_err(|_| RuntimeError::InvalidRecord)?,
                        }
                    } else {
                        CompletionOutcome::Failed { cause }
                    }
                } else {
                    error
                        .failure_cause()
                        .map_or(CompletionOutcome::Unknown, |cause| {
                            CompletionOutcome::UnknownWithCause { cause }
                        })
                };
                self.complete(&context, outcome).await?;
                if definitive && let Some(usage) = &raw {
                    self.finish_no_output(&context, usage, mode).await?;
                    return Err(error);
                }
                if let Some(raw) = &raw {
                    self.project_usage(raw).await?;
                }
                self.admission
                    .outcome(
                        &context.job,
                        dispatch,
                        mode,
                        if definitive { Some(false) } else { None },
                    )
                    .await?;
                self.emit(
                    if definitive {
                        crate::telemetry::EventKind::Failed
                    } else {
                        crate::telemetry::EventKind::Unknown
                    },
                    &context.job,
                    &context.stage,
                    Some(dispatch),
                );
                Err(error)
            }
        }
    }

    pub(super) async fn retain_usage(
        &self,
        context: &PublicationContext,
        route: &crate::cost::CostRoute,
        evidence: UsageEvidence,
        units: &[crate::cost::CostUnits],
        raw_output: Option<&[u8]>,
        usable: bool,
    ) -> Result<RuntimeUsage, RuntimeError> {
        let units = if evidence == UsageEvidence::Unknown {
            route.maxima.clone()
        } else {
            units.to_vec()
        };
        let previous = self
            .usage
            .latest_usage(&context.job.owner, context.dispatch)
            .await?;
        if let Some(previous) = &previous
            && previous.route == *route
            && previous.evidence == evidence
            && previous.units == units
            && previous.usable == usable
            && previous.raw_output.as_deref() == raw_output
        {
            return Ok(previous.clone());
        }
        let revision = previous
            .map_or(Some(1), |usage| usage.revision.0.checked_add(1))
            .ok_or(RuntimeError::InvalidRecord)?;
        let usage = RuntimeUsage {
            owner: context.job.owner.clone(),
            job: context.job.job,
            stage: context.stage.stage,
            dispatch: context.dispatch,
            revision: UsageRevision(revision),
            route: route.clone(),
            evidence,
            units,
            usable,
            at_ms: u64::try_from(self.clock.now_ms()).map_err(|_| RuntimeError::InvalidRecord)?,
            raw_output: raw_output.map(<[u8]>::to_vec),
        };
        self.usage.record_usage(&usage).await?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::UsageRecorded,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        event.measurements.usage_revision = Some(usage.revision);
        event.measurements.usage_evidence = Some(usage.evidence);
        event.measurements.raw_checkpointed = Some(usage.raw_output.is_some());
        self.telemetry.observe(event);
        Ok(usage)
    }

    pub(super) async fn project_usage(&self, usage: &RuntimeUsage) -> Result<(), RuntimeError> {
        self.project_usage_observed(usage, true).await
    }

    /// A projection that is not yet durable is finished by the cost log's own acknowledgement, so
    /// it must not abort a pipeline that still has to queue its next stage.
    pub(super) async fn project_usage_or_pending(
        &self,
        usage: &RuntimeUsage,
    ) -> Result<(), RuntimeError> {
        match self.project_usage(usage).await {
            Ok(()) | Err(RuntimeError::CostProjectionPending) => Ok(()),
            Err(error) => Err(error),
        }
    }
    async fn project_usage_observed(
        &self,
        usage: &RuntimeUsage,
        report_pending: bool,
    ) -> Result<(), RuntimeError> {
        let latest = self
            .usage
            .latest_usage(&usage.owner, usage.dispatch)
            .await?;
        let selected = latest
            .as_ref()
            .filter(|latest| latest.revision.0 > usage.revision.0)
            .unwrap_or(usage);
        if selected.owner != usage.owner
            || selected.job != usage.job
            || selected.stage != usage.stage
            || selected.route != usage.route
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let usage = selected;
        let result = self.project_usage_inner(usage).await;
        if let Err(error) = &result {
            if *error == RuntimeError::CostProjectionPending && !report_pending {
                return result;
            }
            let context = crate::telemetry::EventContext {
                job: Some(usage.job),
                stage: Some(usage.stage),
                attempt: Some(usage.dispatch),
                gate_revision: Some(self.settings.active_config_revision),
                ..Default::default()
            };
            let pending = *error == RuntimeError::CostProjectionPending;
            let mut event = crate::telemetry::ImageEvent::new(
                if pending {
                    crate::telemetry::EventKind::UsageProjectionPending
                } else {
                    crate::telemetry::EventKind::UsageProjectionFailed
                },
                context,
            );
            if !pending {
                event.failure = Some(crate::telemetry::FailureKind::UsageProjection);
            }
            event.measurements.usage_revision = Some(usage.revision);
            event.measurements.usage_evidence = Some(usage.evidence);
            self.telemetry.observe(event);
        }
        result
    }

    async fn project_usage_inner(&self, usage: &RuntimeUsage) -> Result<(), RuntimeError> {
        let call_id = crate::cost::pinned_call_id(usage.dispatch.as_bytes());
        self.costs
            .start_pinned(&call_id, &usage.route)
            .await
            .map_err(RuntimeError::Cost)?;
        let report = UsageReport {
            revision: usage.revision,
            evidence: usage.evidence,
            units: &usage.units,
            usable: usage.usable,
            at_ms: usage.at_ms,
        };
        match self.costs.correct(&call_id, &report).await {
            Ok(_) => {}
            Err(crate::cost::CostError::StaleRevision { stored, .. }) => {
                return self.acknowledge_newer_projection(usage, stored).await;
            }
            Err(error) => return Err(RuntimeError::Cost(error)),
        }
        if !self
            .costs
            .is_durable(&call_id, Some(usage.revision))
            .await
            .map_err(RuntimeError::Cost)?
        {
            return Err(RuntimeError::CostProjectionPending);
        }
        self.usage
            .acknowledge_projection(&usage.owner, usage.dispatch, usage.revision)
            .await?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::UsageProjected,
            crate::telemetry::EventContext {
                job: Some(usage.job),
                stage: Some(usage.stage),
                attempt: Some(usage.dispatch),
                gate_revision: Some(self.settings.active_config_revision),
                ..Default::default()
            },
        );
        event.measurements.usage_revision = Some(usage.revision);
        event.measurements.usage_evidence = Some(usage.evidence);
        self.telemetry.observe(event);
        Ok(())
    }

    async fn acknowledge_newer_projection(
        &self,
        older: &RuntimeUsage,
        revision: UsageRevision,
    ) -> Result<(), RuntimeError> {
        let scope = ExecutionScope {
            owner: older.owner.clone(),
            job: older.job,
            stage: older.stage,
            dispatch: older.dispatch,
        };
        let current = self
            .usage
            .read_usage_revision(&scope, revision)
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        if current.revision.0 <= older.revision.0 || current.route != older.route {
            return Err(RuntimeError::InvalidRecord);
        }
        let receipt = self
            .costs
            .read_receipt(&crate::cost::pinned_call_id(older.dispatch.as_bytes()))
            .await
            .map_err(RuntimeError::Cost)?
            .ok_or(RuntimeError::StorageUnavailable)?;
        if !receipt.durable {
            return Err(RuntimeError::CostProjectionPending);
        }
        let max_word_bytes = [
            &current.route.provider,
            &current.route.model,
            &current.route.cause,
            &current.route.detail,
        ]
        .into_iter()
        .map(|word| word.len())
        .chain(
            current
                .route
                .maxima
                .iter()
                .chain(&current.units)
                .map(|unit| unit.category.len()),
        )
        .max()
        .ok_or(RuntimeError::InvalidRecord)?;
        let limits = super::UsageProjectionLimits {
            max_attempts: 1,
            max_observations: 1,
            max_unit_lines: current.route.maxima.len().max(current.units.len()),
            max_word_bytes,
        };
        super::usage_projection::ActualCostReceipt::verify(receipt, &current, limits)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        self.usage
            .acknowledge_projection(&current.owner, current.dispatch, current.revision)
            .await
    }

    pub(super) async fn complete(
        &self,
        context: &PublicationContext,
        outcome: CompletionOutcome,
    ) -> Result<(), RuntimeError> {
        let stage_outcome = match &outcome {
            CompletionOutcome::Succeeded { .. } => crate::telemetry::StageOutcome::OutputStored,
            CompletionOutcome::Candidate { .. } => crate::telemetry::StageOutcome::Candidate,
            CompletionOutcome::Decision { .. } => crate::telemetry::StageOutcome::Decision,
            CompletionOutcome::Unusable { .. } => crate::telemetry::StageOutcome::Unusable,
            CompletionOutcome::Failed { .. } => crate::telemetry::StageOutcome::Failed,
            CompletionOutcome::Unknown
            | CompletionOutcome::UnknownWithCause { .. }
            | CompletionOutcome::Declined { .. } => crate::telemetry::StageOutcome::Unknown,
        };
        self.execution
            .complete(
                &CompleteCommand::new(
                    StageClaim {
                        scope: StageScope {
                            owner: context.job.owner.clone(),
                            job: context.job.job,
                            stage: context.stage.stage,
                        },
                        token: context.claim,
                        generation: context.stage.claim_generation,
                    },
                    context.dispatch,
                    outcome,
                    self.clock.now_ms(),
                )
                .map_err(|_| RuntimeError::InvalidRecord)?,
            )
            .await
            .map_err(|error| {
                if matches!(
                    error,
                    ExecutionError::Superseded
                        | ExecutionError::LostClaim
                        | ExecutionError::Stale { .. }
                        | ExecutionError::Retired
                        | ExecutionError::OutputSealed
                ) {
                    self.emit(
                        crate::telemetry::EventKind::StaleCompletion,
                        &context.job,
                        &context.stage,
                        Some(context.dispatch),
                    );
                }
                RuntimeError::Execution(error)
            })?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::StageCompleted,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        event.measurements.stage_outcome = Some(stage_outcome);
        event.measurements.supplier_dispatched =
            Some(context.stage.kind != StageKind::LocalTransform);
        self.telemetry.observe(event);
        Ok(())
    }

    async fn freeze(
        &self,
        job: ImageJob,
        intent: &crate::intent_codec::DecodedIntent,
        config: &ImageConfig,
        classification: Option<&DecisionObservation>,
        analysis: Option<&crate::ImageAnalysisObservation>,
    ) -> Result<ImageJob, RuntimeError> {
        let delivered = intent.output.delivered();
        let dimensions = delivered.dimensions();
        let output = ImageIntent::new(
            delivered.encoding().format().mime(),
            dimensions.width_px(),
            dimensions.height_px(),
            if matches!(delivered.alpha(), AlphaHandling::PreserveAlpha) {
                ImageAlphaRequirement::Preserve
            } else {
                ImageAlphaRequirement::Opaque
            },
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let mut plan = b"df-imagery/native-recipe-v1".to_vec();
        for bytes in [job.intent.bytes(), job.config.bytes()] {
            let length = u32::try_from(bytes.len()).map_err(|_| RuntimeError::InvalidRecord)?;
            plan.extend_from_slice(&length.to_be_bytes());
            plan.extend_from_slice(bytes);
        }
        if let Some(analysis) = analysis {
            let retained = crate::service::analysis_codec::encode_result(
                &analysis.result,
                crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
            )
            .map_err(|_| RuntimeError::InvalidRecord)?;
            plan.extend_from_slice(b"segmentation-mask-dependency-v1");
            plan.extend_from_slice(&analysis.job_revision.to_be_bytes());
            plan.extend_from_slice(&<[u8; 32]>::from(sha2::Sha256::digest(&retained)));
        }
        let resolved_model = dispatch::model(config, intent)?;
        for bytes in [
            dispatch::backend(config, intent).label().as_bytes(),
            resolved_model.as_bytes(),
            crate::raster::webp_lossy::WEBP_ENCODER_SOURCE.as_bytes(),
        ] {
            let length = u32::try_from(bytes.len()).map_err(|_| RuntimeError::InvalidRecord)?;
            plan.extend_from_slice(&length.to_be_bytes());
            plan.extend_from_slice(bytes);
        }
        for revision in [
            crate::backends::codex_cli::ADAPTER_REVISION,
            crate::raster::color::RASTER_COLOR_REV,
            crate::raster::webp_lossy::WEBP_ENCODER_REVISION,
        ] {
            plan.extend_from_slice(&revision.to_be_bytes());
        }
        let selected_text = match &intent.operation {
            ImageOperation::Generate(spec) => spec.prompt.text(),
            ImageOperation::Edit(spec) => spec.prompt.text(),
            ImageOperation::Transform(_) => "",
        };
        let selected = serde_json::json!({"text":selected_text,
            "account":dispatch::account(config,intent).as_str(),
            "moderation":match dispatch::moderation(config,intent) { crate::backends::api::ImageModeration::Auto=>"auto",crate::backends::api::ImageModeration::Low=>"low" },
            "rating":intent.operation.pipeline().map(|pipeline| match pipeline.rating {
                crate::ImageContentRating::Restrained=>"restrained", crate::ImageContentRating::Mature=>"mature" })}).to_string();
        plan.extend_from_slice(
            &(u32::try_from(selected.len()).map_err(|_| RuntimeError::InvalidRecord)?)
                .to_be_bytes(),
        );
        plan.extend_from_slice(selected.as_bytes());
        if let Some(classification) = classification {
            plan.extend_from_slice(b"classification-v1");
            plan.extend_from_slice(classification.scope.stage.as_bytes());
            plan.extend_from_slice(classification.scope.dispatch.as_bytes());
            plan.extend_from_slice(&classification.observation_version.to_be_bytes());
            plan.extend_from_slice(&classification.receipt.usage_revision.to_be_bytes());
            plan.extend_from_slice(
                &u32::try_from(classification.payload().len())
                    .map_err(|_| RuntimeError::InvalidRecord)?
                    .to_be_bytes(),
            );
            plan.extend_from_slice(classification.payload());
        }
        let recipe = FrozenOutput::new(output, None, plan)
            .map_err(RuntimeError::Publication)?
            .snapshot()
            .map_err(RuntimeError::Publication)?;
        let owner = dispatch::hex(&crate::owner_scope_bytes(&job.owner));
        let model = dispatch::model(config, intent)?;
        let key = MediaRequestKey::derive_recipe(&MediaRecipeRequest {
            operation_kind: "native",
            purpose: intent.purpose.purpose.label(),
            owner_scope: &owner,
            backend_profile: dispatch::backend(config, intent).label(),
            model: &model,
            quality: "frozen-config",
            rating: "auto",
            width_px: dimensions.width_px(),
            height_px: dimensions.height_px(),
            output_format: delivered.encoding().format().label(),
            alpha_policy: "frozen-output",
            template_revision: 1,
            route_revision: 1,
            acceptance_revision: 1,
            executor_revision: 1,
            source_binding: None,
            references: &[],
            prompt: "",
            pipeline_bytes: recipe.bytes(),
            regeneration_nonce: None,
            audience: &intent.audience,
        })
        .map_err(|_| RuntimeError::InvalidRecord)?;
        // Election semantics remove only the explicit run fence. The actual executable plan stays complete.
        let key = crate::service::artifact_recipe::retained_campaign_artifact_recipe(
            &job.intent,
            &job.config,
        )
        .map_err(RuntimeError::Journal)?
        .map_or(key, |artifact| artifact.key);
        if job.recipe.as_ref() == Some(&recipe) && job.resolved_key == Some(key) {
            return Ok(job);
        }
        let frozen = self
            .journal
            .freeze_recipe(&FreezeCommand {
                owner: job.owner.clone(),
                job: job.job,
                expected_revision: job.revision,
                key,
                recipe,
                now_ms: self.clock.now_ms(),
            })
            .await
            .map_err(RuntimeError::Journal)?;
        match frozen {
            FreezeOutcome::Frozen { job } | FreezeOutcome::AlreadyFrozen { job } => Ok(job),
        }
    }
}
