//! Consumer boundary over the accepted journal and canonical codec.
//!
//! The service records demand, replays exact receipts and withdraws interests. A configured
//! result reader adds durable publication to authorized views; native work is owned separately
//! by the runtime and never exposed as a consumer execution callback.
//!
//! Every observation reauthorizes against the stored audience and owner. Denial and
//! absence are indistinguishable, and comparison never precedes stored authorization,
//! so probing a forbidden receipt cannot reveal whether it exists or what it holds.

pub(crate) mod analysis;
pub(crate) mod analysis_codec;
pub(crate) mod analysis_types;
pub(crate) mod artifact_recipe;
mod artifacts;
pub mod cache;
mod consumer;
mod watch;
pub use analysis_types::*;
pub mod ports;
pub mod types;
pub use consumer::{ImageServiceApi, ImageServiceFuture};

use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::config::{ConfigError, ImageConfig, SchedulerBounds};
use crate::intent_codec::{
    CONFIG_CODEC_VERSION, CodecError, DecodedIntent, decode_config, decode_intent, encode_config,
    encode_intent,
};
use crate::journal::{
    AcceptCommand, AcceptOutcome, AcceptTarget, ImageJournal, JournalError, OperationView,
    SharingIdentity, VersionedSnapshot, WithdrawOutcome,
};
use crate::policy::{PolicyDecision, PolicyInput, decide};
use crate::request::{ImageOperation, ImageRequest, RequestLimits};

pub use self::ports::{
    AuthDenied, ImageAuthorizer, ImageObservations, NoImageObservations, ServiceAction,
    ServiceClock, ServiceIdError, ServiceIds, StoredImageAuthorization,
};
pub use self::types::{
    CancelOutcome, CancelReceipt, CreatorAssessment, CreatorCategory, CreatorRoute,
    ImageDemandView, ImageExecutionState, ImageInspectionObservation, ImageProgress,
    ImageServiceError, ImageUsageObservation, OperationHandle, RequestOutcome, RequestProblem,
    RevokeReport, WatchObservation,
};

/// Fixed protocol safety ceiling for bounded input parsing and stored comparison.
/// Exact replay must never depend on subsequently tightened runtime limits; the
/// current snapshot's admission limits apply to genuine misses only.
pub(crate) const PROTOCOL_LIMITS: RequestLimits = RequestLimits {
    max_prompt_chars: PROTOCOL_MAX_SNAPSHOT_BYTES,
    max_references: PROTOCOL_MAX_SNAPSHOT_BYTES,
    max_transform_steps: PROTOCOL_MAX_SNAPSHOT_BYTES,
    max_composite_layers: PROTOCOL_MAX_SNAPSHOT_BYTES,
};

/// Fixed bound every canonical snapshot must fit; receipts above it are refused.
pub(crate) const PROTOCOL_MAX_SNAPSHOT_BYTES: usize = 1_048_576;

/// Domain separator for the sharing digest: a planning aid, never a media key.
const SHARING_DOMAIN: &[u8] = b"df-imagery-sharing-v1";

/// The single consumer image boundary. Native composition separately owns its matching worker.
pub struct ImageService<J, C, I, A, R = NoImageResults, O = NoImageObservations> {
    journal: J,
    config: ImageConfig,
    clock: C,
    ids: I,
    authorizer: A,
    results: R,
    observations: O,
    observer: Option<std::sync::Arc<dyn crate::telemetry::ImageObserver>>,
    active_config_revision: Option<u64>,
    waiters: Arc<watch::WatchWaiters>,
}

impl<J, C, I, A> ImageService<J, C, I, A>
where
    J: ImageJournal,
    C: ServiceClock,
    I: ServiceIds,
    A: ImageAuthorizer,
{
    /// Binds one validated configuration snapshot. The snapshot stays fixed for the
    /// service lifetime; a config change builds a new service over the same journal.
    pub fn new(
        journal: J,
        config: ImageConfig,
        clock: C,
        ids: I,
        authorizer: A,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        let waiters = Arc::new(watch::WatchWaiters::new(&config.scheduler));
        Ok(Self {
            journal,
            config,
            clock,
            ids,
            authorizer,
            results: NoImageResults,
            observations: NoImageObservations,
            observer: None,
            active_config_revision: None,
            waiters,
        })
    }

    /// Adds durable owner-scoped result reads to the same authenticated service boundary.
    pub fn with_results<R: crate::publication::ImageResults>(
        self,
        results: R,
    ) -> ImageService<J, C, I, A, R> {
        ImageService {
            journal: self.journal,
            config: self.config,
            clock: self.clock,
            ids: self.ids,
            authorizer: self.authorizer,
            results,
            observations: self.observations,
            observer: self.observer,
            active_config_revision: self.active_config_revision,
            waiters: self.waiters,
        }
    }
}

/// Planning-only services have no publication authority and never report a result.
pub struct NoImageResults;

impl crate::publication::ImageResults for NoImageResults {
    async fn read_result(
        &self,
        _owner: &crate::ImageOwner,
        _job: &crate::ImageJobId,
    ) -> Result<Option<crate::publication::PublishedImage>, crate::publication::PublicationError>
    {
        Ok(None)
    }
}

impl<J, C, I, A, R, O> ImageService<J, C, I, A, R, O>
where
    J: ImageJournal,
    C: ServiceClock,
    I: ServiceIds,
    A: ImageAuthorizer,
    R: crate::publication::ImageResults,
    O: ImageObservations,
{
    /// Adds the durable execution projection used by authenticated read/watch.
    pub fn with_observations<P: ImageObservations>(
        self,
        observations: P,
    ) -> ImageService<J, C, I, A, R, P> {
        ImageService {
            journal: self.journal,
            config: self.config,
            clock: self.clock,
            ids: self.ids,
            authorizer: self.authorizer,
            results: self.results,
            observations,
            observer: self.observer,
            active_config_revision: self.active_config_revision,
            waiters: self.waiters,
        }
    }

    /// Sends content-free lifecycle records through the host's shared observer.
    pub fn with_observer(
        mut self,
        observer: std::sync::Arc<dyn crate::telemetry::ImageObserver>,
    ) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Binds the same applied configuration revision acknowledged by native dispatch owners.
    pub fn with_config_revision(mut self, revision: u64) -> Result<Self, ImageServiceError> {
        if revision == 0 {
            return Err(ImageServiceError::Unavailable);
        }
        self.active_config_revision = Some(revision);
        Ok(self)
    }

    fn observe(&self, event: crate::telemetry::ImageEvent) {
        if let Some(observer) = &self.observer {
            observer.observe(event)
        }
    }

    fn request_context(&self, request: &ImageRequest) -> crate::telemetry::EventContext {
        crate::telemetry::EventContext {
            operation: Some(request.identity.operation),
            purpose: Some(request.purpose.purpose),
            gate_revision: self.active_config_revision,
            run: match request.identity.owner {
                crate::ImageOwner::Run { run, .. } => Some(run),
                _ => None,
            },
            ..Default::default()
        }
    }

    /// Computes the canonical fingerprint for one request without mutating anything.
    /// It uses the same fixed protocol encoder as admission; the caller assigns
    /// the result to the request identity before submitting.
    pub fn canonical_fingerprint(
        &self,
        request: &ImageRequest,
    ) -> Result<[u8; 32], ImageServiceError> {
        request
            .validate(&PROTOCOL_LIMITS)
            .map_err(RequestProblem::from_request)
            .map_err(ImageServiceError::InvalidRequest)?;
        let encoded = encode_intent(request, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES)
            .map_err(map_encode_error)?;
        Ok(*encoded.fingerprint())
    }

    /// Records a new Live demand, replays an exact receipt, or refuses new work.
    /// Structural validation and both caller authorizations precede any journal
    /// access; the receipt lookup precedes every mutable config gate.
    pub async fn request(
        &self,
        principal: &A::Principal,
        request: &ImageRequest,
    ) -> Result<RequestOutcome, ImageServiceError> {
        use crate::telemetry::{EventKind, ImageEvent};
        let context = self.request_context(request);
        self.observe(ImageEvent::new(EventKind::Requested, context.clone()));
        let started = std::time::Instant::now();
        let result = self.request_inner(principal, request).await;
        let mut context = context;
        let kind = match &result {
            Ok(RequestOutcome::Recorded(view)) => {
                context.job = Some(view.job);
                context.request = Some(view.interest);
                EventKind::Recorded
            }
            Ok(RequestOutcome::Replayed(view)) => {
                context.job = Some(view.job);
                context.request = Some(view.interest);
                EventKind::Replayed
            }
            Ok(RequestOutcome::Reused(view)) => {
                context.job = Some(view.job);
                context.request = Some(view.interest);
                EventKind::CacheHit
            }
            Ok(RequestOutcome::ReusedArtifact(_)) => EventKind::CacheHit,
            Ok(RequestOutcome::Refused(_)) => EventKind::Refused,
            Err(ImageServiceError::Unknown { .. }) => EventKind::Unknown,
            Err(_) => EventKind::Failed,
        };
        if let Ok(RequestOutcome::Recorded(view)) = &result
            && view.execution == crate::ImageExecutionState::Queued
        {
            self.observe(ImageEvent::new(EventKind::Queued, context.clone()));
        }
        self.observe(ImageEvent::new(kind, context).elapsed(started.elapsed()));
        result
    }

    async fn request_inner(
        &self,
        principal: &A::Principal,
        request: &ImageRequest,
    ) -> Result<RequestOutcome, ImageServiceError> {
        request
            .validate(&PROTOCOL_LIMITS)
            .map_err(RequestProblem::from_request)
            .map_err(ImageServiceError::InvalidRequest)?;
        self.authorizer
            .authorize_request(principal, request)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        self.authorizer
            .authorize_owner(principal, &request.identity.owner, ServiceAction::Generate)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        let encoded = encode_intent(request, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES)
            .map_err(map_encode_error)?;
        if encoded.fingerprint() != &request.identity.fingerprint {
            return Err(ImageServiceError::InvalidRequest(
                RequestProblem::FingerprintMismatch,
            ));
        }
        let handle = OperationHandle {
            owner: request.identity.owner.clone(),
            operation: request.identity.operation,
        };
        match self
            .journal
            .lookup_operation(&handle.owner, handle.operation)
            .await
            .map_err(|error| map_lookup_error(error, &handle))?
        {
            Some(view) => {
                self.finish_replay(principal, request, encoded.fingerprint(), view, &handle)
                    .await
            }
            None => {
                self.record_new(principal, request, encoded.bytes(), &handle)
                    .await
            }
        }
    }

    /// Returns the projected safe view of one demand. Reads never use current
    /// generation enablement to hide authorized historical receipts.
    pub async fn read(
        &self,
        principal: &A::Principal,
        handle: &OperationHandle,
    ) -> Result<ImageDemandView, ImageServiceError> {
        self.authorizer
            .authorize_owner(principal, &handle.owner, ServiceAction::Read)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        let view = self
            .journal
            .lookup_operation(&handle.owner, handle.operation)
            .await
            .map_err(|error| map_lookup_error(error, handle))?
            .ok_or(ImageServiceError::NotAuthorizedOrNotFound)?;
        self.open_view(principal, ServiceAction::Read, &view, handle)
            .await
    }

    /// One authenticated coherent read expressed against a seen revision: Changed
    /// when the job moved past it, Unchanged otherwise. Each call reauthorizes;
    /// a transport may long-poll this, but this method itself never blocks or pushes.
    pub async fn watch(
        &self,
        principal: &A::Principal,
        handle: &OperationHandle,
        after_revision: u64,
    ) -> Result<WatchObservation, ImageServiceError> {
        self.watch_within(principal, handle, after_revision, std::time::Duration::ZERO)
            .await
    }

    /// Waits up to `wait` (capped at `watch::WATCH_MAX_WAIT`) for the job revision to pass
    /// `after_revision`. Each poll rereads and reauthorizes the demand. Over the waiter caps the
    /// call is refused at once as retryable `Unavailable`, before any read.
    pub async fn watch_within(
        &self,
        principal: &A::Principal,
        handle: &OperationHandle,
        after_revision: u64,
        wait: std::time::Duration,
    ) -> Result<WatchObservation, ImageServiceError> {
        let wait = watch::bounded_wait(wait);
        let _permit = if wait.is_zero() {
            None
        } else {
            Some(self.waiters.enter(&handle.owner)?)
        };
        let started = std::time::Instant::now();
        loop {
            let view = self.read(principal, handle).await?;
            if view.job_revision > after_revision {
                return Ok(WatchObservation::Changed(Box::new(view)));
            }
            let elapsed = started.elapsed();
            if elapsed >= wait {
                return Ok(WatchObservation::Unchanged {
                    revision: view.job_revision,
                });
            }
            let remaining = wait - elapsed;
            tokio::time::sleep(remaining.min(watch::WATCH_POLL_INTERVAL)).await;
        }
    }

    /// Withdraws the caller's own interest, resolved from the stored receipt rather
    /// than any caller-chosen target. Repeat cancellation is idempotent; retiring
    /// the last interest is existing journal behavior, not supplier cancellation.
    pub async fn cancel(
        &self,
        principal: &A::Principal,
        handle: &OperationHandle,
    ) -> Result<CancelOutcome, ImageServiceError> {
        let started = std::time::Instant::now();
        let result = self.cancel_inner(principal, handle).await;
        let mut context = crate::telemetry::EventContext {
            operation: Some(handle.operation),
            gate_revision: self.active_config_revision,
            ..Default::default()
        };
        if let Ok(CancelOutcome::Withdrawn(receipt) | CancelOutcome::AlreadyWithdrawn(receipt)) =
            &result
        {
            context.job = Some(receipt.view.job);
            context.request = Some(receipt.view.interest);
        }
        let kind = if result.is_ok() {
            crate::telemetry::EventKind::Cancelled
        } else {
            crate::telemetry::EventKind::Failed
        };
        self.observe(crate::telemetry::ImageEvent::new(kind, context).elapsed(started.elapsed()));
        result
    }

    /// Operator revoke for one owner scope: every attached interest is withdrawn in one
    /// transaction. Idempotent, so an unknown commit is retried as `Unavailable` rather than
    /// reported as a stored receipt the caller cannot name.
    pub async fn revoke(
        &self,
        principal: &A::Principal,
        owner: &crate::request::ImageOwner,
    ) -> Result<RevokeReport, ImageServiceError> {
        let started = std::time::Instant::now();
        let result = self.revoke_inner(principal, owner).await;
        let context = crate::telemetry::EventContext {
            gate_revision: self.active_config_revision,
            ..Default::default()
        };
        let kind = if result.is_ok() {
            crate::telemetry::EventKind::Revoked
        } else {
            crate::telemetry::EventKind::Failed
        };
        self.observe(crate::telemetry::ImageEvent::new(kind, context).elapsed(started.elapsed()));
        result
    }

    async fn revoke_inner(
        &self,
        principal: &A::Principal,
        owner: &crate::request::ImageOwner,
    ) -> Result<RevokeReport, ImageServiceError> {
        self.authorizer
            .authorize_revoke(principal, owner)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        let outcomes = self
            .journal
            .withdraw_owner_interests(owner, self.clock.now_ms())
            .await
            .map_err(|error| match error {
                JournalError::Unavailable | JournalError::Unknown => ImageServiceError::Unavailable,
                _ => ImageServiceError::InvalidRecord,
            })?;
        let mut report = RevokeReport {
            withdrawn: 0,
            already_withdrawn: 0,
            jobs_retired: 0,
        };
        for outcome in outcomes {
            match outcome {
                WithdrawOutcome::Withdrawn(withdrawal) => {
                    report.withdrawn += 1;
                    if withdrawal.attached_remaining == 0 {
                        report.jobs_retired += 1;
                    }
                }
                WithdrawOutcome::AlreadyWithdrawn(_) => report.already_withdrawn += 1,
            }
        }
        Ok(report)
    }

    async fn cancel_inner(
        &self,
        principal: &A::Principal,
        handle: &OperationHandle,
    ) -> Result<CancelOutcome, ImageServiceError> {
        self.authorizer
            .authorize_owner(principal, &handle.owner, ServiceAction::Cancel)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        let view = self
            .journal
            .lookup_operation(&handle.owner, handle.operation)
            .await
            .map_err(|error| map_lookup_error(error, handle))?
            .ok_or(ImageServiceError::NotAuthorizedOrNotFound)?;
        self.open_view(principal, ServiceAction::Cancel, &view, handle)
            .await?;
        let now_ms = self.clock.now_ms();
        let withdrawal = self
            .journal
            .withdraw_interest(&handle.owner, view.receipt.interest, now_ms)
            .await
            .map_err(|error| map_withdraw_error(error, handle))?;
        let (interest, attached_remaining, withdrawal_revision) = match &withdrawal {
            WithdrawOutcome::Withdrawn(outcome) => (
                &outcome.interest,
                outcome.attached_remaining,
                outcome.job_revision,
            ),
            WithdrawOutcome::AlreadyWithdrawn(outcome) => (
                &outcome.interest,
                outcome.attached_remaining,
                outcome.job_revision,
            ),
        };
        if interest.interest != view.receipt.interest
            || interest.job != view.receipt.job
            || interest.owner != handle.owner
            || interest.operation != handle.operation
        {
            return Err(ImageServiceError::InvalidRecord);
        }
        let refreshed = self
            .journal
            .lookup_operation(&handle.owner, handle.operation)
            .await
            .map_err(|error| match error {
                JournalError::Unavailable | JournalError::Unknown => ImageServiceError::Unknown {
                    handle: handle.clone(),
                },
                _ => ImageServiceError::InvalidRecord,
            })?
            .ok_or(ImageServiceError::InvalidRecord)?;
        if refreshed.receipt.interest != interest.interest || refreshed.receipt.job != interest.job
        {
            return Err(ImageServiceError::InvalidRecord);
        }
        let reread = self
            .open_view(principal, ServiceAction::Cancel, &refreshed, handle)
            .await
            .map_err(|error| match error {
                ImageServiceError::Unavailable => ImageServiceError::Unknown {
                    handle: handle.clone(),
                },
                other => other,
            })?;
        if reread.job_revision != withdrawal_revision {
            return Err(ImageServiceError::Unknown {
                handle: handle.clone(),
            });
        }
        let receipt = CancelReceipt {
            view: reread,
            attached_remaining,
        };
        match withdrawal {
            WithdrawOutcome::Withdrawn(_) => Ok(CancelOutcome::Withdrawn(receipt)),
            WithdrawOutcome::AlreadyWithdrawn(_) => Ok(CancelOutcome::AlreadyWithdrawn(receipt)),
        }
    }

    /// Validates the current snapshot and admission limits, applies policy, and on
    /// admission records one Planning demand. Lookup-only modes return their typed
    /// miss without minting ids or touching the journal.
    async fn record_new(
        &self,
        principal: &A::Principal,
        request: &ImageRequest,
        intent_bytes: &[u8],
        handle: &OperationHandle,
    ) -> Result<RequestOutcome, ImageServiceError> {
        self.config
            .validate()
            .map_err(|_| ImageServiceError::Unavailable)?;
        request
            .validate(&self.config.request_limits)
            .map_err(RequestProblem::from_request)
            .map_err(ImageServiceError::InvalidRequest)?;
        if let Some(pipeline) = request.operation.pipeline()
            && (pipeline.creator.as_ref().is_some_and(|creator| {
                self.config.analysis.classification.as_ref() != Some(&creator.route)
            }) || pipeline.inspection.as_ref().is_some_and(|inspection| {
                self.config.analysis.inspection.as_ref() != Some(&inspection.route)
            }) || pipeline.moderation.is_some_and(|moderation| {
                self.config
                    .purpose(request.purpose.purpose)
                    .moderation_override
                    != Some(moderation)
            }))
        {
            return Err(ImageServiceError::InvalidRequest(
                RequestProblem::InvalidPipeline,
            ));
        }
        let intent = VersionedSnapshot::new(
            u32::from(crate::intent_codec::intent_version(intent_bytes).map_err(map_encode_error)?),
            intent_bytes.to_vec(),
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::InvalidRecord)?;
        let config =
            encode_config(&self.config, PROTOCOL_MAX_SNAPSHOT_BYTES).map_err(map_encode_error)?;
        let config = VersionedSnapshot::new(
            u32::from(CONFIG_CODEC_VERSION),
            config.bytes().to_vec(),
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::InvalidRecord)?;
        let compatible = self
            .find_compatible(principal, request, &intent, &config)
            .await?;
        if !matches!(
            &compatible,
            Some(cache::CompatibleImage::Ready { .. } | cache::CompatibleImage::Artifact(_))
        ) {
            self.observe(crate::telemetry::ImageEvent::new(
                crate::telemetry::EventKind::CacheMiss,
                self.request_context(request),
            ));
        }
        let mut policy = policy_input(&self.config, request);
        policy.cache_hit = matches!(
            &compatible,
            Some(cache::CompatibleImage::Ready { .. } | cache::CompatibleImage::Artifact(_))
        );
        match decide(&self.config, &policy) {
            PolicyDecision::Refuse(refusal) => Ok(RequestOutcome::Refused(refusal)),
            PolicyDecision::ReuseReady => {
                let (view, demand) = match compatible {
                    Some(cache::CompatibleImage::Artifact(artifact)) => {
                        return Ok(RequestOutcome::ReusedArtifact(*artifact));
                    }
                    Some(cache::CompatibleImage::Ready { view, demand }) => (view, demand),
                    _ => return Err(ImageServiceError::InvalidRecord),
                };
                if request.execution != crate::ExecutionMode::Live
                    || self.config.mode == crate::GlobalMode::CacheOnly
                    || view.job.state == crate::JobState::Retired
                    || view.interest.state != crate::InterestState::Attached
                    || crate::is_expired(view.job.absolute_deadline_ms, self.clock.now_ms())
                {
                    return Ok(RequestOutcome::Reused(*demand));
                }
                let stored = decode_intent(
                    view.job.intent.bytes(),
                    &PROTOCOL_LIMITS,
                    PROTOCOL_MAX_SNAPSHOT_BYTES,
                )
                .map_err(|_| ImageServiceError::InvalidRecord)?;
                if stored.execution != crate::ExecutionMode::Live {
                    return Ok(RequestOutcome::Reused(*demand));
                }
                self.accept_planning(
                    principal,
                    request,
                    intent_bytes,
                    handle,
                    Some(AcceptTarget::JoinReady { job: view.job.job }),
                )
                .await
            }
            PolicyDecision::Admit => {
                let target = match compatible {
                    Some(cache::CompatibleImage::Pending(view)) => {
                        Some(AcceptTarget::Join { job: view.job.job })
                    }
                    _ => None,
                };
                self.accept_planning(principal, request, intent_bytes, handle, target)
                    .await
            }
        }
    }

    /// Retains bounded canonical snapshots, computes the checked deadline, mints ids
    /// once and accepts a Create. A concurrent replay resolves through the same
    /// stored authorization and exact comparison as a lookup hit.
    async fn accept_planning(
        &self,
        principal: &A::Principal,
        request: &ImageRequest,
        intent_bytes: &[u8],
        handle: &OperationHandle,
        target: Option<AcceptTarget>,
    ) -> Result<RequestOutcome, ImageServiceError> {
        // Refuse before any journal write: a receipt the requester cannot read back is never recorded.
        self.authorizer
            .authorize_new_visibility(principal, request)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        // The encode above already bounded these bytes; the snapshot guards the store.
        let intent = VersionedSnapshot::new(
            u32::from(crate::intent_codec::intent_version(intent_bytes).map_err(map_encode_error)?),
            intent_bytes.to_vec(),
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::Unavailable)?;
        let encoded_config = encode_config(&self.config, PROTOCOL_MAX_SNAPSHOT_BYTES)
            .map_err(|_| ImageServiceError::Unavailable)?;
        let stored_config = VersionedSnapshot::new(
            u32::from(CONFIG_CODEC_VERSION),
            encoded_config.bytes().to_vec(),
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::Unavailable)?;
        let now_ms = self.clock.now_ms();
        let sharing = sharing_identity(intent.bytes(), stored_config.bytes())?;
        let interest = self
            .ids
            .next_interest_id()
            .map_err(|_| ImageServiceError::Unavailable)?;
        let target = match target {
            Some(target) => target,
            None => AcceptTarget::Create {
                job: self
                    .ids
                    .next_job_id()
                    .map_err(|_| ImageServiceError::Unavailable)?,
                absolute_deadline_ms: checked_deadline(now_ms, &self.config.scheduler)?,
            },
        };
        let ready_join = matches!(target, AcceptTarget::JoinReady { .. });
        let command = AcceptCommand {
            identity: request.identity.clone(),
            intent,
            config: stored_config,
            sharing,
            interest,
            target,
            now_ms,
        };
        match self
            .journal
            .accept_scheduled(&command, request.urgency)
            .await
        {
            Ok(AcceptOutcome::Created(view)) => Ok(RequestOutcome::Recorded(
                self.open_view(principal, ServiceAction::Read, &view, handle)
                    .await?,
            )),
            Ok(AcceptOutcome::Joined(view)) => {
                let demand = self
                    .open_view(principal, ServiceAction::Read, &view, handle)
                    .await?;
                if ready_join && demand.result.is_none() {
                    return Err(ImageServiceError::Unknown {
                        handle: handle.clone(),
                    });
                }
                let mut context = self.request_context(request);
                context.job = Some(demand.job);
                context.request = Some(demand.interest);
                self.observe(crate::telemetry::ImageEvent::new(
                    if ready_join {
                        crate::telemetry::EventKind::CacheHit
                    } else {
                        crate::telemetry::EventKind::Coalesced
                    },
                    context,
                ));
                Ok(RequestOutcome::Recorded(demand))
            }
            Ok(AcceptOutcome::Replayed(view)) => {
                self.finish_replay(
                    principal,
                    request,
                    &request.identity.fingerprint,
                    view,
                    handle,
                )
                .await
            }
            Err(JournalError::OperationConflict) => {
                // A concurrent winner holds this operation under different intent.
                // Reread the stored receipt and resolve through the same authorized
                // replay: comparison never precedes stored authorization, there is
                // no second admission, and a missing or failed reread fails closed.
                let reread = self
                    .journal
                    .lookup_operation(&handle.owner, handle.operation)
                    .await
                    .map_err(|error| map_lookup_error(error, handle))?
                    .ok_or(ImageServiceError::InvalidRecord)?;
                self.finish_replay(
                    principal,
                    request,
                    &request.identity.fingerprint,
                    reread,
                    handle,
                )
                .await
            }
            Err(error) => Err(map_accept_error(error, handle)),
        }
    }

    /// Decodes the retained snapshots under bounded limits, authorizes the stored
    /// view, then demands full canonical equality before exposing the original view.
    /// Comparison never precedes authorization, and replays mint nothing.
    async fn finish_replay(
        &self,
        principal: &A::Principal,
        request: &ImageRequest,
        fingerprint: &[u8; 32],
        view: OperationView,
        handle: &OperationHandle,
    ) -> Result<RequestOutcome, ImageServiceError> {
        if view.receipt.owner != handle.owner || view.receipt.operation != handle.operation {
            return Err(ImageServiceError::InvalidRecord);
        }
        if !snapshot_version_matches(&view.job.intent)
            || !snapshot_version_matches(&view.job.config)
        {
            return Err(ImageServiceError::InvalidRecord);
        }
        let stored = decode_intent(
            view.job.intent.bytes(),
            &PROTOCOL_LIMITS,
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::InvalidRecord)?;
        decode_config(view.job.config.bytes(), PROTOCOL_MAX_SNAPSHOT_BYTES)
            .map_err(|_| ImageServiceError::InvalidRecord)?;
        validate_linkage(handle, &view, &stored.owner)?;
        let authorization = stored_authorization(handle, &stored);
        self.authorizer
            .authorize_stored(principal, ServiceAction::Read, &view, &authorization)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        if view.receipt.fingerprint != *fingerprint {
            return Err(ImageServiceError::Conflict);
        }
        if !intent_matches(&stored, request) {
            return Err(ImageServiceError::Conflict);
        }
        Ok(RequestOutcome::Replayed(
            self.project_result(handle, &view).await?,
        ))
    }

    /// Verifies tuple and revision, decodes retained snapshots under bounded limits,
    /// authorizes the stored view and projects it without leaking stored facts.
    async fn open_view(
        &self,
        principal: &A::Principal,
        action: ServiceAction,
        view: &OperationView,
        handle: &OperationHandle,
    ) -> Result<ImageDemandView, ImageServiceError> {
        if view.receipt.owner != handle.owner || view.receipt.operation != handle.operation {
            return Err(ImageServiceError::InvalidRecord);
        }
        if !snapshot_version_matches(&view.job.intent)
            || !snapshot_version_matches(&view.job.config)
        {
            return Err(ImageServiceError::InvalidRecord);
        }
        if analysis_codec::is_analysis_intent(view.job.intent.bytes()) {
            return self
                .open_analysis_view(principal, action, view, handle)
                .await;
        }
        let stored = decode_intent(
            view.job.intent.bytes(),
            &PROTOCOL_LIMITS,
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::InvalidRecord)?;
        decode_config(view.job.config.bytes(), PROTOCOL_MAX_SNAPSHOT_BYTES)
            .map_err(|_| ImageServiceError::InvalidRecord)?;
        validate_linkage(handle, view, &stored.owner)?;
        let authorization = stored_authorization(handle, &stored);
        self.authorizer
            .authorize_stored(principal, action, view, &authorization)
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        self.project_result(handle, view).await
    }

    async fn project_result(
        &self,
        handle: &OperationHandle,
        view: &OperationView,
    ) -> Result<ImageDemandView, ImageServiceError> {
        let mut demand = project_demand(handle, view)?;
        if view.interest.state == crate::InterestState::Attached
            && let Some(progress) = self
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
            demand.assessment = progress.assessment;
        }
        if final_result_visible(view) {
            let result = self
                .results
                .read_result(&handle.owner, &view.job.job)
                .await
                .map_err(|error| match error {
                    crate::publication::PublicationError::Unavailable => {
                        ImageServiceError::Unavailable
                    }
                    crate::publication::PublicationError::Unknown => ImageServiceError::Unknown {
                        handle: handle.clone(),
                    },
                    _ => ImageServiceError::InvalidRecord,
                })?;
            let result = qualify_read_result(view, result)?;
            if final_result_visible(view)
                && let Some(result) = result
            {
                if result.job != view.job.job || result.private_reconciliation {
                    return Err(ImageServiceError::InvalidRecord);
                }
                if result.job_revision > view.job.revision {
                    return Err(ImageServiceError::Unknown {
                        handle: handle.clone(),
                    });
                }
                if view.job.state != crate::JobState::RecipeFrozen || result.variants.is_empty() {
                    return Err(ImageServiceError::InvalidRecord);
                }
                demand.execution = ImageExecutionState::Ready;
                demand.operational_notice = None;
                demand.result = Some(result);
                let intent = decode_intent(
                    view.job.intent.bytes(),
                    &PROTOCOL_LIMITS,
                    PROTOCOL_MAX_SNAPSHOT_BYTES,
                )
                .map_err(|_| ImageServiceError::InvalidRecord)?;
                demand.staging = intent
                    .operation
                    .pipeline()
                    .and_then(|pipeline| pipeline.inspection.as_ref())
                    .map_or_else(Vec::new, |spec| spec.staging.clone());
                if let Some(inspection) = self
                    .observations
                    .read_inspection(&handle.owner, view.job.job)
                    .await
                    .map_err(|error| map_lookup_error(error, handle))?
                {
                    if inspection.job_revision != view.job.revision {
                        return Err(ImageServiceError::Unknown {
                            handle: handle.clone(),
                        });
                    }
                    demand.inspection = Some(inspection.assessment);
                }
            }
        }
        if view.interest.state == crate::InterestState::Attached
            && let Some(usage) = self
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
        self.project_owned_artifact(handle, view, &mut demand)
            .await?;
        apply_result_visibility(&mut demand, view);
        if demand.execution == ImageExecutionState::Ready
            && demand.result.is_none()
            && demand.artifact.is_none()
        {
            demand.execution = ImageExecutionState::Publishing;
        }
        Ok(demand)
    }
}

fn qualify_read_result(
    view: &OperationView,
    result: Option<crate::publication::PublishedImage>,
) -> Result<Option<crate::publication::PublishedImage>, ImageServiceError> {
    let Some(result) = result else {
        return Ok(None);
    };
    if result.job != view.job.job || result.published_at_ms <= 0 {
        return Err(ImageServiceError::InvalidRecord);
    }
    if result.private_reconciliation
        || crate::is_expired(view.job.absolute_deadline_ms, result.published_at_ms)
    {
        return Ok(None);
    }
    Ok(Some(result))
}
fn final_result_visible(view: &OperationView) -> bool {
    view.interest.state == crate::InterestState::Attached
        && view.job.state != crate::JobState::Retired
}

fn apply_result_visibility(demand: &mut ImageDemandView, view: &OperationView) {
    if final_result_visible(view) {
        return;
    }
    demand.result = None;
    demand.artifact = None;
    demand.analysis_result = None;
    demand.assessment = None;
    demand.inspection = None;
    demand.staging.clear();
    if matches!(
        demand.execution,
        ImageExecutionState::Ready
            | ImageExecutionState::Completed
            | ImageExecutionState::Publishing
    ) {
        demand.execution = if view.interest.state == crate::InterestState::Withdrawn
            || view.job.state == crate::JobState::Retired
        {
            ImageExecutionState::Withdrawn
        } else {
            ImageExecutionState::Expired
        };
    }
}
// The decoder owns supported versions; the envelope must agree with its payload.
fn snapshot_version_matches(snapshot: &VersionedSnapshot) -> bool {
    snapshot.bytes().first().copied().map(u32::from) == Some(snapshot.revision())
}

/// The coarse recipe facts the current policy gate can read from a bare request:
/// Generate and Edit need a provider stage, Transform a local one. Later recipe
/// resolution refines this; the service never invents a ready hit (`cache_hit`
/// stays false until the result-binding lane lands).
fn policy_input(config: &ImageConfig, request: &ImageRequest) -> PolicyInput {
    let purpose = request.purpose.purpose;
    let backend = config
        .purpose(purpose)
        .backend_override
        .unwrap_or(config.default_backend);
    let (needs_provider, needs_local) = match &request.operation {
        ImageOperation::Generate(_) | ImageOperation::Edit(_) => (true, false),
        ImageOperation::Transform(_) => (false, true),
    };
    PolicyInput::new(
        purpose,
        backend,
        request.execution,
        needs_provider,
        needs_local,
        false,
    )
}

/// Full canonical intent equality: every decoded field must equal the request.
/// The hash alone never decides; execution and freshness participate.
fn intent_matches(stored: &DecodedIntent, request: &ImageRequest) -> bool {
    stored.owner == request.identity.owner
        && stored.purpose == request.purpose
        && stored.operation == request.operation
        && stored.execution == request.execution
        && stored.freshness == request.freshness
        && stored.audience == request.audience
        && stored.expected_source_revision == request.expected_source_revision
        && stored.output == request.output
}

/// Builds the borrowed stored authorization view from decoded facts.
/// It carries no urgency and no raw snapshots.
fn stored_authorization<'a>(
    handle: &'a OperationHandle,
    stored: &'a DecodedIntent,
) -> crate::service::ports::StoredImageAuthorization<'a> {
    crate::service::ports::StoredImageAuthorization {
        handle,
        owner: &stored.owner,
        purpose: stored.purpose,
        operation: &stored.operation,
        execution: stored.execution,
        freshness: stored.freshness,
        audience: &stored.audience,
        expected_source_revision: stored.expected_source_revision,
        output: &stored.output,
    }
}

/// Validates full immutable linkage and the decoded owner before any projection.
/// A wrong owner or a broken link never exposes stored details.
fn validate_linkage(
    handle: &OperationHandle,
    view: &OperationView,
    decoded_owner: &crate::request::ImageOwner,
) -> Result<(), ImageServiceError> {
    if view.receipt.owner != handle.owner
        || view.receipt.operation != handle.operation
        || view.job.owner != handle.owner
        || view.interest.owner != handle.owner
        || view.interest.operation != handle.operation
    {
        return Err(ImageServiceError::InvalidRecord);
    }
    if view.receipt.job != view.job.job
        || view.receipt.interest != view.interest.interest
        || view.interest.job != view.job.job
        || view.receipt.job != view.interest.job
    {
        return Err(ImageServiceError::InvalidRecord);
    }
    if decoded_owner != &handle.owner
        || decoded_owner != &view.receipt.owner
        || decoded_owner != &view.job.owner
        || decoded_owner != &view.interest.owner
    {
        return Err(ImageServiceError::InvalidRecord);
    }
    Ok(())
}

/// Projects one coherent journal view after verifying receipt, job and interest
/// linkage. A wrong owner or a broken link never exposes stored details.
fn project_demand(
    handle: &OperationHandle,
    view: &OperationView,
) -> Result<ImageDemandView, ImageServiceError> {
    if view.receipt.owner != handle.owner
        || view.receipt.operation != handle.operation
        || view.job.owner != handle.owner
        || view.interest.owner != handle.owner
        || view.interest.operation != handle.operation
    {
        return Err(ImageServiceError::InvalidRecord);
    }
    if view.receipt.job != view.job.job
        || view.receipt.interest != view.interest.interest
        || view.interest.job != view.job.job
        || view.receipt.job != view.interest.job
    {
        return Err(ImageServiceError::InvalidRecord);
    }
    Ok(ImageDemandView {
        handle: handle.clone(),
        job: view.job.job,
        interest: view.receipt.interest,
        job_state: view.job.state,
        job_revision: view.job.revision,
        interest_state: view.interest.state,
        absolute_deadline_ms: view.job.absolute_deadline_ms,
        result: None,
        artifact: None,
        analysis_result: None,
        inspection: None,
        staging: Vec::new(),
        execution: if view.interest.state == crate::InterestState::Withdrawn
            || view.job.state == crate::JobState::Retired
        {
            ImageExecutionState::Withdrawn
        } else {
            ImageExecutionState::Planning
        },
        assessment: None,
        failure: None,
        operational_notice: None,
        usage: None,
    })
}

/// Domain-separated digest of canonical intent plus canonical config bytes for
/// future accepted sharing. It is a planning aid, never a media request key.
fn sharing_identity(intent: &[u8], config: &[u8]) -> Result<SharingIdentity, ImageServiceError> {
    let mut digest = Sha256::new();
    digest.update(SHARING_DOMAIN);
    digest.update(intent);
    digest.update(config);
    let bytes: [u8; 32] = digest.finalize().into();
    // SHA-256 always yields 32 bytes, so this length check cannot fail on this input.
    SharingIdentity::from_bytes(&bytes).map_err(|_| ImageServiceError::Unavailable)
}

/// Checked absolute deadline from injected time and the snapshot window.
/// Overflow is a service-side fact problem: nothing was stored.
fn checked_deadline(now_ms: i64, scheduler: &SchedulerBounds) -> Result<i64, ImageServiceError> {
    let window_ms = i64::try_from(scheduler.job_deadline_secs)
        .map_err(|_| ImageServiceError::Unavailable)?
        .checked_mul(1_000)
        .ok_or(ImageServiceError::Unavailable)?;
    now_ms
        .checked_add(window_ms)
        .ok_or(ImageServiceError::Unavailable)
}

/// New-request encoding failures are caller problems; the store was untouched.
fn map_encode_error(error: CodecError) -> ImageServiceError {
    match error {
        CodecError::Request(request) => {
            ImageServiceError::InvalidRequest(RequestProblem::from_request(request))
        }
        CodecError::TooLong => ImageServiceError::InvalidRequest(RequestProblem::SnapshotTooLarge),
        CodecError::BadVersion { .. }
        | CodecError::Truncated
        | CodecError::Trailing { .. }
        | CodecError::Malformed
        | CodecError::InvalidText
        | CodecError::InvalidId
        | CodecError::NonFinite { .. }
        | CodecError::Output(_)
        | CodecError::Config(_) => ImageServiceError::InvalidRequest(RequestProblem::Malformed),
    }
}

/// A bare receipt lookup reports transport and commit states only; durable state
/// refusals cannot arise here, so anything else is a corrupt record.
fn map_lookup_error(error: JournalError, handle: &OperationHandle) -> ImageServiceError {
    match error {
        JournalError::Unavailable => ImageServiceError::Unavailable,
        JournalError::Unknown => ImageServiceError::Unknown {
            handle: handle.clone(),
        },
        JournalError::InvalidRecord
        | JournalError::NotFound
        | JournalError::OperationConflict
        | JournalError::IdentityConflict
        | JournalError::JoinRefused(_)
        | JournalError::RecipeConflict
        | JournalError::Stale { .. }
        | JournalError::Retired
        | JournalError::Expired => ImageServiceError::InvalidRecord,
    }
}

/// The Create path expects replay or creation: operation conflicts resolve
/// through a scoped reread plus authorized replay above, so reaching here with
/// one broke a record invariant. A minted-id collision is retryable.
fn map_accept_error(error: JournalError, handle: &OperationHandle) -> ImageServiceError {
    match error {
        JournalError::OperationConflict
        | JournalError::InvalidRecord
        | JournalError::NotFound
        | JournalError::JoinRefused(_)
        | JournalError::RecipeConflict
        | JournalError::Stale { .. }
        | JournalError::Retired
        | JournalError::Expired => ImageServiceError::InvalidRecord,
        JournalError::IdentityConflict => ImageServiceError::Unavailable,
        JournalError::Unavailable => ImageServiceError::Unavailable,
        JournalError::Unknown => ImageServiceError::Unknown {
            handle: handle.clone(),
        },
    }
}

/// Withdrawal expects the receipt just read: losing it means the store broke its
/// never-delete invariant, and state refusals cannot arise on this path.
fn map_withdraw_error(error: JournalError, handle: &OperationHandle) -> ImageServiceError {
    match error {
        JournalError::Unavailable => ImageServiceError::Unavailable,
        JournalError::Unknown => ImageServiceError::Unknown {
            handle: handle.clone(),
        },
        JournalError::InvalidRecord
        | JournalError::NotFound
        | JournalError::OperationConflict
        | JournalError::IdentityConflict
        | JournalError::JoinRefused(_)
        | JournalError::RecipeConflict
        | JournalError::Stale { .. }
        | JournalError::Retired
        | JournalError::Expired => ImageServiceError::InvalidRecord,
    }
}
