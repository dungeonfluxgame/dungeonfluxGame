//! Standalone decision admission uses the same receipts, interests and authorization boundary.
use super::analysis_codec::{self, DecodedAnalysis};
use super::*;

impl<J, C, I, A, R, O> ImageService<J, C, I, A, R, O>
where
    J: ImageJournal,
    C: ServiceClock,
    I: ServiceIds,
    A: ImageAuthorizer,
    R: crate::publication::ImageResults,
    O: ImageObservations,
{
    pub fn canonical_analysis_fingerprint(
        &self,
        request: &ImageAnalysisRequest,
    ) -> Result<[u8; 32], ImageServiceError> {
        analysis_codec::fingerprint(&DecodedAnalysis::from_request(request))
            .map_err(map_encode_error)
    }

    pub async fn request_analysis(
        &self,
        principal: &A::Principal,
        request: &ImageAnalysisRequest,
    ) -> Result<RequestOutcome, ImageServiceError> {
        let started = std::time::Instant::now();
        let mut context = crate::telemetry::EventContext {
            operation: Some(request.identity.operation),
            purpose: Some(request.purpose.purpose),
            gate_revision: self.active_config_revision,
            run: match request.identity.owner {
                crate::ImageOwner::Run { run, .. } => Some(run),
                _ => None,
            },
            ..Default::default()
        };
        self.observe(crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::Requested,
            context.clone(),
        ));
        let result = self.request_analysis_inner(principal, request).await;
        let kind = match &result {
            Ok(RequestOutcome::Reused(view)) => {
                context.job = Some(view.job);
                context.request = Some(view.interest);
                crate::telemetry::EventKind::CacheHit
            }
            Ok(RequestOutcome::Recorded(view)) => {
                context.job = Some(view.job);
                context.request = Some(view.interest);
                crate::telemetry::EventKind::Recorded
            }
            Ok(RequestOutcome::Replayed(view)) => {
                context.job = Some(view.job);
                context.request = Some(view.interest);
                crate::telemetry::EventKind::Replayed
            }
            Ok(RequestOutcome::ReusedArtifact(_)) => crate::telemetry::EventKind::CacheHit,
            Ok(RequestOutcome::Refused(_)) => crate::telemetry::EventKind::Refused,
            Err(ImageServiceError::Unknown { .. }) => crate::telemetry::EventKind::Unknown,
            Err(_) => crate::telemetry::EventKind::Failed,
        };
        if let Ok(RequestOutcome::Recorded(view)) = &result
            && view.execution == ImageExecutionState::Queued
        {
            self.observe(crate::telemetry::ImageEvent::new(
                crate::telemetry::EventKind::Queued,
                context.clone(),
            ));
        }
        self.observe(crate::telemetry::ImageEvent::new(kind, context).elapsed(started.elapsed()));
        result
    }
    async fn request_analysis_inner(
        &self,
        principal: &A::Principal,
        request: &ImageAnalysisRequest,
    ) -> Result<RequestOutcome, ImageServiceError> {
        let decoded = DecodedAnalysis::from_request(request);
        let bytes = analysis_codec::encode_analysis(&decoded).map_err(map_encode_error)?;
        self.authorizer
            .authorize_analysis(principal, request)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        self.authorizer
            .authorize_owner(principal, &request.identity.owner, ServiceAction::Read)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        if analysis_codec::fingerprint(&decoded).map_err(map_encode_error)?
            != request.identity.fingerprint
        {
            return Err(ImageServiceError::InvalidRequest {
                cause: RequestCause::FingerprintMismatch,
            });
        }
        let handle = OperationHandle {
            owner: request.identity.owner.clone(),
            operation: request.identity.operation,
        };
        if let Some(view) = self
            .journal
            .lookup_operation(&handle.owner, handle.operation)
            .await
            .map_err(|error| map_lookup_error(error, &handle))?
        {
            return self
                .replay_analysis(
                    principal,
                    &decoded,
                    &request.identity.fingerprint,
                    &view,
                    &handle,
                )
                .await;
        }
        self.config
            .validate()
            .map_err(|_| ImageServiceError::Unavailable)?;
        analysis_codec::validate(&decoded, &self.config.request_limits)
            .map_err(map_encode_error)?;
        match analysis_policy(
            &self.config,
            decoded.purpose.purpose,
            decoded.execution,
            !allows_local_decision(&decoded.operation),
        ) {
            PolicyDecision::Refuse(refusal) => return Ok(RequestOutcome::Refused(refusal)),
            PolicyDecision::ReuseReady => return Err(ImageServiceError::Unavailable),
            PolicyDecision::Admit => {}
        }
        if !allows_local_decision(&decoded.operation)
            && !analysis_route_present(&self.config, &decoded.operation)
        {
            return Ok(RequestOutcome::Refused(crate::Refusal::BackendDisabled));
        }
        let config_bytes = encode_config(&self.config, PROTOCOL_MAX_SNAPSHOT_BYTES)
            .map_err(|_| ImageServiceError::Unavailable)?;
        let config = VersionedSnapshot::new(
            u32::from(CONFIG_CODEC_VERSION),
            config_bytes.bytes().to_vec(),
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::Unavailable)?;
        let intent = VersionedSnapshot::new(
            u32::from(analysis_codec::VERSION),
            bytes,
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::Unavailable)?;
        let now_ms = self.clock.now_ms();
        let command = AcceptCommand {
            identity: request.identity.clone(),
            sharing: sharing_identity(intent.bytes(), config.bytes())?,
            intent,
            config,
            interest: self
                .ids
                .next_interest_id()
                .map_err(|_| ImageServiceError::Unavailable)?,
            target: AcceptTarget::Create {
                job: self
                    .ids
                    .next_job_id()
                    .map_err(|_| ImageServiceError::Unavailable)?,
                absolute_deadline_ms: checked_deadline(now_ms, &self.config.scheduler)?,
            },
            now_ms,
        };
        match self
            .journal
            .accept_scheduled(&command, request.urgency)
            .await
        {
            Ok(AcceptOutcome::Created(view)) => Ok(RequestOutcome::Recorded(
                self.open_analysis_view(principal, ServiceAction::Read, &view, &handle)
                    .await?,
            )),
            Ok(AcceptOutcome::Replayed(view) | AcceptOutcome::Joined(view)) => {
                self.replay_analysis(
                    principal,
                    &decoded,
                    &request.identity.fingerprint,
                    &view,
                    &handle,
                )
                .await
            }
            Err(JournalError::OperationConflict) => {
                let view = self
                    .journal
                    .lookup_operation(&handle.owner, handle.operation)
                    .await
                    .map_err(|error| map_lookup_error(error, &handle))?
                    .ok_or(ImageServiceError::InvalidRecord)?;
                self.replay_analysis(
                    principal,
                    &decoded,
                    &request.identity.fingerprint,
                    &view,
                    &handle,
                )
                .await
            }
            Err(error) => Err(map_accept_error(error, &handle)),
        }
    }
    async fn replay_analysis(
        &self,
        principal: &A::Principal,
        request: &DecodedAnalysis,
        fingerprint: &[u8; 32],
        view: &OperationView,
        handle: &OperationHandle,
    ) -> Result<RequestOutcome, ImageServiceError> {
        // Authorization always precedes conflict comparison, including cross-kind receipt
        // collisions.
        let demand = self
            .open_view(principal, ServiceAction::Read, view, handle)
            .await?;
        if view.receipt.fingerprint != *fingerprint {
            return Err(ImageServiceError::Conflict);
        }
        let stored = analysis_codec::decode_analysis(view.job.intent.bytes())
            .map_err(|_| ImageServiceError::Conflict)?;
        if stored != *request {
            return Err(ImageServiceError::Conflict);
        }
        Ok(RequestOutcome::Replayed(demand))
    }
    pub(super) async fn open_analysis_view(
        &self,
        principal: &A::Principal,
        action: ServiceAction,
        view: &OperationView,
        handle: &OperationHandle,
    ) -> Result<ImageDemandView, ImageServiceError> {
        if !snapshot_version_matches(&view.job.intent)
            || !snapshot_version_matches(&view.job.config)
        {
            return Err(ImageServiceError::InvalidRecord);
        }
        let stored = analysis_codec::decode_analysis(view.job.intent.bytes())
            .map_err(|_| ImageServiceError::InvalidRecord)?;
        decode_config(view.job.config.bytes(), PROTOCOL_MAX_SNAPSHOT_BYTES)
            .map_err(|_| ImageServiceError::InvalidRecord)?;
        validate_linkage(handle, view, &stored.owner)?;
        self.authorizer
            .authorize_stored_analysis(
                principal,
                action,
                view,
                &StoredAnalysisAuthorization {
                    handle,
                    owner: &stored.owner,
                    purpose: stored.purpose,
                    operation: &stored.operation,
                    execution: stored.execution,
                    freshness: stored.freshness,
                    audience: &stored.audience,
                    expected_source_revision: stored.expected_source_revision,
                },
            )
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        self.project_analysis_result(handle, view, &stored).await
    }
    async fn project_analysis_result(
        &self,
        handle: &OperationHandle,
        view: &OperationView,
        stored: &DecodedAnalysis,
    ) -> Result<ImageDemandView, ImageServiceError> {
        let mut demand = project_demand(handle, view)?;
        if view.interest.state != crate::InterestState::Attached
            || view.job.state == crate::JobState::Retired
        {
            return Ok(demand);
        }
        if let Some(progress) = self
            .observations
            .read_progress(&handle.owner, view.job.job, self.clock.now_ms())
            .await
            .map_err(|error| map_lookup_error(error, handle))?
        {
            if progress.revision != view.job.revision {
                return Err(ImageServiceError::Unknown {
                    handle: handle.clone(),
                });
            }
            demand.execution = progress.state;
            demand.failure = progress.failure;
            demand.operational_notice = progress.operational_notice;
        }
        let observation = if final_result_visible(view) {
            self.observations
                .read_analysis(&handle.owner, view.job.job)
                .await
                .map_err(|error| map_lookup_error(error, handle))?
        } else {
            None
        };
        if final_result_visible(view)
            && let Some(observation) = observation
        {
            if observation.job_revision != view.job.revision {
                return Err(ImageServiceError::Unknown {
                    handle: handle.clone(),
                });
            }
            if observation.selected_at_ms <= 0 {
                return Err(ImageServiceError::InvalidRecord);
            }
            if !crate::is_expired(view.job.absolute_deadline_ms, observation.selected_at_ms) {
                validate_result_binding(&stored.operation, &observation.result)
                    .map_err(|_| ImageServiceError::InvalidRecord)?;
                demand.analysis_result = Some(observation.result);
                demand.execution = ImageExecutionState::Completed;
                demand.operational_notice = None;
            } else if demand.execution != ImageExecutionState::Unknown
                && !matches!(demand.execution, ImageExecutionState::Failed { .. })
            {
                demand.execution = ImageExecutionState::Expired;
            }
        }
        if let Some(usage) = self
            .observations
            .read_usage(&handle.owner, view.job.job)
            .await
            .map_err(|error| map_lookup_error(error, handle))?
        {
            if usage.job_revision != view.job.revision {
                return Err(ImageServiceError::Unknown {
                    handle: handle.clone(),
                });
            }
            demand.usage = Some(usage.summary);
        }
        apply_result_visibility(&mut demand, view);
        if demand.execution == ImageExecutionState::Ready
            || (demand.execution == ImageExecutionState::Completed
                && demand.analysis_result.is_none())
        {
            return Err(ImageServiceError::InvalidRecord);
        }
        Ok(demand)
    }
}
pub(crate) fn analysis_route_present(
    config: &ImageConfig,
    operation: &ImageAnalysisOperation,
) -> bool {
    match operation {
        ImageAnalysisOperation::PhotoInspection(spec) => {
            config.analysis.inspection.as_ref() == Some(&spec.inspection.route)
        }
        ImageAnalysisOperation::HeadLocations(_) => config.analysis.head_locations.is_some(),
        ImageAnalysisOperation::Segmentation(_) => config.analysis.segmentation.is_some(),
        ImageAnalysisOperation::StagingIdentity(_) => config.analysis.staging_identity.is_some(),
        ImageAnalysisOperation::StagingFacing(_) => config.analysis.staging_facing.is_some(),
    }
}
pub(crate) fn validate_result_binding(
    operation: &ImageAnalysisOperation,
    result: &ImageAnalysisResult,
) -> Result<(), ()> {
    let (pin, contract, same_kind) = match result {
        ImageAnalysisResult::PhotoInspection { source, assessment } => (
            source,
            assessment.contract_revision,
            matches!(operation, ImageAnalysisOperation::PhotoInspection(_)),
        ),
        ImageAnalysisResult::HeadLocations {
            source,
            contract_revision,
            ..
        } => (
            source,
            *contract_revision,
            matches!(operation, ImageAnalysisOperation::HeadLocations(_)),
        ),
        ImageAnalysisResult::Segmentation {
            source,
            contract_revision,
            ..
        } => (
            source,
            *contract_revision,
            matches!(operation, ImageAnalysisOperation::Segmentation(_)),
        ),
        ImageAnalysisResult::StagingIdentity {
            source,
            contract_revision,
            ..
        } => (
            source,
            *contract_revision,
            matches!(operation, ImageAnalysisOperation::StagingIdentity(_)),
        ),
        ImageAnalysisResult::StagingFacing {
            source,
            contract_revision,
            ..
        } => (
            source,
            *contract_revision,
            matches!(operation, ImageAnalysisOperation::StagingFacing(_)),
        ),
    };
    if !same_kind
        || pin != analysis_codec::source(operation)
        || contract != analysis_codec::revision(operation)
    {
        return Err(());
    }
    if let (
        ImageAnalysisOperation::Segmentation(spec),
        ImageAnalysisResult::Segmentation { masks, .. },
    ) = (operation, result)
        && (masks.len() != spec.prompts.len()
            || masks.iter().zip(&spec.prompts).any(|(masks, prompt)| {
                masks.prompt != *prompt
                    || masks.instances.len()
                        > usize::try_from(spec.maximum_masks).unwrap_or(usize::MAX)
                    || masks.validate().is_err()
            }))
    {
        return Err(());
    }
    Ok(())
}

pub(crate) fn allows_local_decision(operation: &ImageAnalysisOperation) -> bool {
    matches!(
        operation,
        ImageAnalysisOperation::StagingIdentity(_) | ImageAnalysisOperation::StagingFacing(_)
    )
}

/// Dedicated analytical routes are independent of the generation backend's enablement.
/// The caller proves local determinism before using provider_needed=false at execution.
pub(crate) fn analysis_policy(
    config: &ImageConfig,
    purpose: crate::Purpose,
    execution: crate::ExecutionMode,
    provider_needed: bool,
) -> PolicyDecision {
    let policy = decide(
        config,
        &PolicyInput::new(
            purpose,
            config.default_backend,
            execution,
            false,
            false,
            false,
        ),
    );
    if policy != PolicyDecision::Admit {
        return policy;
    }
    if provider_needed && config.mode == crate::GlobalMode::LocalOnly {
        return PolicyDecision::Refuse(crate::Refusal::ProviderStageNotPermitted);
    }
    PolicyDecision::Admit
}
#[cfg(test)]
mod policy_tests {
    use super::*;
    fn config(mode: crate::GlobalMode) -> ImageConfig {
        let mut config = ImageConfig::defaults().unwrap();
        config.mode = mode;
        config.purposes[crate::Purpose::ScenePlate.index()].enabled = true;
        for backend in &mut config.backends {
            backend.enabled = false;
        }
        config
    }
    #[test]
    fn analysis_provider_policy_does_not_read_generation_backend_gates() {
        let config = config(crate::GlobalMode::Live);
        assert_eq!(
            analysis_policy(
                &config,
                crate::Purpose::ScenePlate,
                crate::ExecutionMode::Live,
                true
            ),
            PolicyDecision::Admit
        );
    }
    #[test]
    fn local_only_permits_proven_local_analysis_but_refuses_a_required_supplier() {
        let config = config(crate::GlobalMode::LocalOnly);
        assert_eq!(
            analysis_policy(
                &config,
                crate::Purpose::ScenePlate,
                crate::ExecutionMode::Live,
                false
            ),
            PolicyDecision::Admit
        );
        assert_eq!(
            analysis_policy(
                &config,
                crate::Purpose::ScenePlate,
                crate::ExecutionMode::Live,
                true
            ),
            PolicyDecision::Refuse(crate::Refusal::ProviderStageNotPermitted)
        );
    }
    #[test]
    fn global_off_and_lookup_only_modes_preserve_their_no_new_work_contract() {
        assert_eq!(
            analysis_policy(
                &config(crate::GlobalMode::Off),
                crate::Purpose::ScenePlate,
                crate::ExecutionMode::Live,
                false
            ),
            PolicyDecision::Refuse(crate::Refusal::ServiceOff)
        );
        assert_eq!(
            analysis_policy(
                &config(crate::GlobalMode::Live),
                crate::Purpose::ScenePlate,
                crate::ExecutionMode::PreparedOnly,
                false
            ),
            PolicyDecision::Refuse(crate::Refusal::PreparedMiss)
        );
    }
}
