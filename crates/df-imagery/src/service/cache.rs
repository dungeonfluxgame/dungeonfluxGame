//! Compatible reuse compares complete canonical content and consults current publication authority.
//! Lookup never creates an interest, extends a lifetime, or treats a digest as authorization.

use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::config::{GlobalMode, ImageConfig, SchedulerBounds};
use crate::intent_codec::{DecodedIntent, decode_config, decode_intent, encode_config};
use crate::journal::{ImageJobId, JournalError, OperationView, VersionedSnapshot};
use crate::publication::{FrozenOutput, ImageResults, PublicationError, PublishedImage};
use crate::request::{ExecutionMode, Freshness, ImageRequest};
use df_assets::AssetVariant;

use super::{
    ImageAuthorizer, ImageDemandView, ImageExecutionState, ImageJournal, ImageObservations,
    ImageService, ImageServiceError, OperationHandle, PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES,
    ServiceAction, ServiceClock, ServiceIds, checked_deadline, project_demand,
    snapshot_version_matches, stored_authorization, validate_linkage,
};

/// Maximum full job snapshots returned by one compatible lookup page.
pub const MAXIMUM_COMPATIBLE_CANDIDATES: usize = 64;

/// Version of normalized content identity; older persisted hints require full candidate comparison.
pub const COMPATIBLE_KEY_VERSION: u16 = 2;

/// An index hint plus its actual canonical encoding versions; never publication authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibleKey {
    pub digest: [u8; 32],
    pub schema_version: u16,
    pub config_revision: u32,
}

const CANONICAL_OPERATIONAL_COUNT: u32 = 1;
const CANONICAL_OPERATIONAL_SECONDS: u64 = 1;
const CANONICAL_SCHEDULER: SchedulerBounds = SchedulerBounds {
    foreground_limit: CANONICAL_OPERATIONAL_COUNT,
    presence_limit: CANONICAL_OPERATIONAL_COUNT,
    background_limit: CANONICAL_OPERATIONAL_COUNT,
    max_pending_jobs: CANONICAL_OPERATIONAL_COUNT,
    max_pending_bytes: 1,
    queue_age_secs: CANONICAL_OPERATIONAL_SECONDS,
    job_deadline_secs: CANONICAL_OPERATIONAL_SECONDS,
    stage_deadline_secs: CANONICAL_OPERATIONAL_SECONDS,
};

/// Stable descending keyset position; pagination never changes retained job facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibleCursor {
    pub created_at_ms: i64,
    pub job: ImageJobId,
}

impl CompatibleCursor {
    fn of(view: &OperationView) -> Self {
        Self {
            created_at_ms: view.job.created_at_ms,
            job: view.job.job,
        }
    }

    fn precedes(self, previous: Self) -> bool {
        (self.created_at_ms, self.job) < (previous.created_at_ms, previous.job)
    }
}

struct CanonicalContent {
    intent_revision: u32,
    intent: Vec<u8>,
    config_revision: u32,
    config: Vec<u8>,
}

impl CanonicalContent {
    fn decode(
        intent: &VersionedSnapshot,
        config: &VersionedSnapshot,
    ) -> Result<Self, JournalError> {
        if !snapshot_version_matches(intent) || !snapshot_version_matches(config) {
            return Err(JournalError::InvalidRecord);
        }
        let decoded = decode_intent(
            intent.bytes(),
            &PROTOCOL_LIMITS,
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| JournalError::InvalidRecord)?;
        Self::encode(decoded, intent.revision(), config)
    }

    fn encode(
        mut decoded: DecodedIntent,
        intent_revision: u32,
        config: &VersionedSnapshot,
    ) -> Result<Self, JournalError> {
        decoded.execution = ExecutionMode::Live;
        let encoded = crate::intent_codec::encode::encode_decoded_intent(
            &decoded,
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| JournalError::InvalidRecord)?;
        let encoded_config = normalized_config(config)?;

        Ok(Self {
            intent_revision,
            intent: encoded.bytes().to_vec(),
            config_revision: encoded_config.revision(),
            config: encoded_config.bytes().to_vec(),
        })
    }

    fn matches(&self, other: &Self) -> bool {
        self.intent_revision == other.intent_revision
            && self.intent == other.intent
            && self.config_revision == other.config_revision
            && self.config == other.config
    }

    fn key(&self) -> Result<[u8; 32], JournalError> {
        let mut digest = Sha256::new();
        digest.update(b"df-imagery/compatible-content-v2");
        digest.update(self.intent_revision.to_be_bytes());
        let intent_len =
            u64::try_from(self.intent.len()).map_err(|_| JournalError::InvalidRecord)?;
        digest.update(intent_len.to_be_bytes());
        digest.update(&self.intent);
        digest.update(self.config_revision.to_be_bytes());
        let config_len =
            u64::try_from(self.config.len()).map_err(|_| JournalError::InvalidRecord)?;
        digest.update(config_len.to_be_bytes());
        digest.update(&self.config);
        Ok(digest.finalize().into())
    }
}

pub(super) fn normalized_config(
    config: &VersionedSnapshot,
) -> Result<VersionedSnapshot, JournalError> {
    if !snapshot_version_matches(config) {
        return Err(JournalError::InvalidRecord);
    }
    let mut decoded = decode_config(config.bytes(), PROTOCOL_MAX_SNAPSHOT_BYTES)
        .map_err(|_| JournalError::InvalidRecord)?;
    normalize_operational_config(&mut decoded);
    let encoded = encode_config(&decoded, PROTOCOL_MAX_SNAPSHOT_BYTES)
        .map_err(|_| JournalError::InvalidRecord)?;
    VersionedSnapshot::new(
        u32::from(encoded.version()),
        encoded.bytes().to_vec(),
        PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| JournalError::InvalidRecord)
}

// Fixed valid representatives keep operational defaults from changing the canonical content key.
fn normalize_operational_config(config: &mut ImageConfig) {
    config.mode = GlobalMode::Live;
    for backend in &mut config.backends {
        backend.enabled = true;
        backend.max_concurrent_jobs = CANONICAL_OPERATIONAL_COUNT;
    }
    for purpose in &mut config.purposes {
        purpose.enabled = true;
    }
    config.transforms.enabled = true;
    config.transforms.max_concurrency = CANONICAL_OPERATIONAL_COUNT;
    config.transforms.queue_capacity = CANONICAL_OPERATIONAL_COUNT;
    config.transforms.deadline_secs = CANONICAL_OPERATIONAL_SECONDS;
    config.scheduler = CANONICAL_SCHEDULER;
    config.retry.retry_deadline_secs = CANONICAL_OPERATIONAL_SECONDS;
    config.retry.backoff_secs = CANONICAL_OPERATIONAL_SECONDS;
    config.quota.enabled = true;
    config.quota.cooldown_ms = 1;
    config.quota.maximum_accounts = CANONICAL_OPERATIONAL_COUNT;
}

/// An indexed lookup aid; current authorization and full payload equality still decide reuse.
/// Request execution and the explicit operational controls are normalized; content policy remains.
pub fn compatible_key(
    intent: &VersionedSnapshot,
    config: &VersionedSnapshot,
) -> Result<[u8; 32], JournalError> {
    Ok(compatible_lookup_key(intent, config)?.digest)
}

/// Builds an index hint using the actual current canonical encoder, including for historical input.
pub fn compatible_lookup_key(
    intent: &VersionedSnapshot,
    config: &VersionedSnapshot,
) -> Result<CompatibleKey, JournalError> {
    let content = CanonicalContent::decode(intent, config)?;
    Ok(CompatibleKey {
        digest: content.key()?,
        schema_version: COMPATIBLE_KEY_VERSION,
        config_revision: content.config_revision,
    })
}

/// Compares full canonical content while ignoring only explicit operational controls.
/// Original receipt fingerprints and retained snapshots are never rewritten.
pub fn compatible_snapshots(
    left_intent: &VersionedSnapshot,
    left_config: &VersionedSnapshot,
    right_intent: &VersionedSnapshot,
    right_config: &VersionedSnapshot,
) -> Result<bool, JournalError> {
    Ok(CanonicalContent::decode(left_intent, left_config)?
        .matches(&CanonicalContent::decode(right_intent, right_config)?))
}

pub(super) enum CompatibleImage {
    Artifact(Box<crate::publication::ReadyArtifact>),
    Ready {
        view: Box<OperationView>,
        demand: Box<ImageDemandView>,
    },
    Pending(Box<OperationView>),
}

impl<J, C, I, A, R, O> ImageService<J, C, I, A, R, O>
where
    J: ImageJournal,
    C: ServiceClock,
    I: ServiceIds,
    A: ImageAuthorizer,
    R: ImageResults,
    O: ImageObservations,
{
    pub(super) async fn find_compatible(
        &self,
        principal: &A::Principal,
        request: &ImageRequest,
        intent: &VersionedSnapshot,
        config: &VersionedSnapshot,
    ) -> Result<Option<CompatibleImage>, ImageServiceError> {
        if request.freshness != Freshness::ReuseCompatible
            || self.config.mode == GlobalMode::Off
            || !self.config.purpose(request.purpose.purpose).enabled
        {
            return Ok(None);
        }
        let lookup_deadline_ms = checked_deadline(self.clock.now_ms(), &self.config.scheduler)?;
        let budget = Duration::from_secs(self.config.scheduler.job_deadline_secs);
        tokio::time::timeout(
            budget,
            self.find_compatible_until_complete(
                principal,
                request,
                intent,
                config,
                lookup_deadline_ms,
            ),
        )
        .await
        .map_err(|_| ImageServiceError::Unavailable)?
    }

    fn require_lookup_time(&self, lookup_deadline_ms: i64) -> Result<(), ImageServiceError> {
        if crate::is_expired(lookup_deadline_ms, self.clock.now_ms()) {
            return Err(ImageServiceError::Unavailable);
        }
        Ok(())
    }

    async fn find_compatible_until_complete(
        &self,
        principal: &A::Principal,
        request: &ImageRequest,
        intent: &VersionedSnapshot,
        config: &VersionedSnapshot,
        lookup_deadline_ms: i64,
    ) -> Result<Option<CompatibleImage>, ImageServiceError> {
        self.require_lookup_time(lookup_deadline_ms)?;
        if let Some(artifact) = self.find_current_artifact(request, config).await? {
            self.require_lookup_time(lookup_deadline_ms)?;
            return Ok(Some(CompatibleImage::Artifact(Box::new(artifact))));
        }
        self.require_lookup_time(lookup_deadline_ms)?;
        let wanted = CanonicalContent::decode(intent, config)
            .map_err(|_| ImageServiceError::InvalidRecord)?;
        let mut after = None;
        loop {
            self.require_lookup_time(lookup_deadline_ms)?;
            let candidates = self
                .journal
                .compatible_candidates(&request.identity.owner, intent, config, after)
                .await
                .map_err(|_| ImageServiceError::Unavailable)?;
            self.require_lookup_time(lookup_deadline_ms)?;
            if candidates.len() > MAXIMUM_COMPATIBLE_CANDIDATES {
                return Err(ImageServiceError::InvalidRecord);
            }
            let has_more = candidates.len() == MAXIMUM_COMPATIBLE_CANDIDATES;
            for view in candidates {
                self.require_lookup_time(lookup_deadline_ms)?;
                let cursor = CompatibleCursor::of(&view);
                if after.is_some_and(|previous| !cursor.precedes(previous)) {
                    return Err(ImageServiceError::InvalidRecord);
                }
                after = Some(cursor);
                if let Some(found) = self
                    .authorize_compatible(principal, request, intent, config, &wanted, view)
                    .await?
                {
                    self.require_lookup_time(lookup_deadline_ms)?;
                    return Ok(Some(found));
                }
            }
            if !has_more {
                return Ok(None);
            }
            tokio::task::yield_now().await;
        }
    }

    async fn project_reused(
        &self,
        handle: &OperationHandle,
        view: &OperationView,
        result: PublishedImage,
    ) -> Result<ImageDemandView, ImageServiceError> {
        let mut demand = project_demand(handle, view)?;
        if let Some(progress) = self
            .observations
            .read_progress(&handle.owner, view.job.job, self.clock.now_ms())
            .await
            .map_err(|_| ImageServiceError::Unavailable)?
        {
            if progress.revision != view.job.revision {
                return Err(ImageServiceError::Unknown {
                    handle: handle.clone(),
                });
            }
            demand.assessment = progress.assessment;
            demand.failure = progress.failure;
        }
        if let Some(usage) = self
            .observations
            .read_usage(&handle.owner, view.job.job)
            .await
            .map_err(|_| ImageServiceError::Unavailable)?
        {
            if usage.job_revision != view.job.revision {
                return Err(ImageServiceError::Unknown {
                    handle: handle.clone(),
                });
            }
            demand.usage = Some(usage.summary);
        }
        let stored = decode_intent(
            view.job.intent.bytes(),
            &PROTOCOL_LIMITS,
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::InvalidRecord)?;
        demand.staging = stored
            .operation
            .pipeline()
            .and_then(|pipeline| pipeline.inspection.as_ref())
            .map_or_else(Vec::new, |spec| spec.staging.clone());
        if let Some(inspection) = self
            .observations
            .read_inspection(&handle.owner, view.job.job)
            .await
            .map_err(|error| super::map_lookup_error(error, handle))?
        {
            if inspection.job_revision != view.job.revision {
                return Err(ImageServiceError::Unknown {
                    handle: handle.clone(),
                });
            }
            demand.inspection = Some(inspection.assessment);
        }
        demand.execution = ImageExecutionState::Ready;
        demand.operational_notice = None;
        demand.result = Some(result);
        Ok(demand)
    }

    async fn authorize_compatible(
        &self,
        principal: &A::Principal,
        request: &ImageRequest,
        intent: &VersionedSnapshot,
        config: &VersionedSnapshot,
        wanted: &CanonicalContent,
        view: OperationView,
    ) -> Result<Option<CompatibleImage>, ImageServiceError> {
        let handle = OperationHandle {
            owner: request.identity.owner.clone(),
            operation: view.receipt.operation,
        };
        if super::analysis_codec::is_analysis_intent(view.job.intent.bytes())
            || crate::intent_codec::intent_version(view.job.intent.bytes()).is_err()
        {
            return Ok(None);
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
        validate_linkage(&handle, &view, &stored.owner)?;
        let authorization = stored_authorization(&handle, &stored);
        if self
            .authorizer
            .authorize_stored(principal, ServiceAction::Read, &view, &authorization)
            .is_err()
        {
            return Ok(None);
        }
        let stored_execution = stored.execution;
        let actual = CanonicalContent::encode(stored, view.job.intent.revision(), &view.job.config)
            .map_err(|_| ImageServiceError::InvalidRecord)?;
        if !wanted.matches(&actual) {
            return Ok(None);
        }
        let now_ms = self.clock.now_ms();
        if let Some(result) = self
            .results
            .read_reusable_result(&handle.owner, &view.job.job, now_ms)
            .await
            .map_err(map_publication_error)?
        {
            verify_result(&view, &result)?;
            let demand = self.project_reused(&handle, &view, result).await?;
            return Ok(Some(CompatibleImage::Ready {
                view: Box::new(view),
                demand: Box::new(demand),
            }));
        }
        if request.execution != ExecutionMode::Live
            || self.config.mode == GlobalMode::CacheOnly
            || stored_execution != ExecutionMode::Live
            || view.job.state == crate::JobState::Retired
            || view.interest.state != crate::InterestState::Attached
            || crate::is_expired(view.job.absolute_deadline_ms, now_ms)
            || view.job.intent != *intent
            || view.job.config != *config
        {
            return Ok(None);
        }
        let progress = self
            .observations
            .read_progress(&handle.owner, view.job.job, now_ms)
            .await
            .map_err(|_| ImageServiceError::Unavailable)?;
        let Some(progress) = progress else {
            return Ok(None);
        };
        if progress.revision != view.job.revision {
            return Err(ImageServiceError::Unknown { handle });
        }
        if matches!(
            progress.state,
            ImageExecutionState::Queued
                | ImageExecutionState::Running
                | ImageExecutionState::Unknown
                | ImageExecutionState::Publishing
        ) {
            return Ok(Some(CompatibleImage::Pending(Box::new(view))));
        }
        Ok(None)
    }
}

fn map_publication_error(error: PublicationError) -> ImageServiceError {
    match error {
        PublicationError::Unavailable | PublicationError::Unknown => ImageServiceError::Unavailable,
        _ => ImageServiceError::InvalidRecord,
    }
}

fn verify_result(view: &OperationView, result: &PublishedImage) -> Result<(), ImageServiceError> {
    let recipe = view
        .job
        .recipe
        .as_ref()
        .ok_or(ImageServiceError::InvalidRecord)?;
    let frozen = FrozenOutput::decode(recipe).map_err(|_| ImageServiceError::InvalidRecord)?;
    if result.job != view.job.job
        || result.private_reconciliation
        || result.job_revision > view.job.revision
        || result.recipe_digest != crate::publication::recipe_digest(recipe)
        || view.job.resolved_key.is_none()
        || view.job.state == crate::JobState::Planning
    {
        return Err(ImageServiceError::InvalidRecord);
    }
    let expected_count = if frozen.intent(AssetVariant::Display).is_some() {
        2
    } else {
        1
    };
    if result.variants.len() != expected_count {
        return Err(ImageServiceError::InvalidRecord);
    }
    for variant in [AssetVariant::Master, AssetVariant::Display] {
        let Some(intent) = frozen.intent(variant) else {
            continue;
        };
        let mut matches = result
            .variants
            .iter()
            .filter(|value| value.variant == variant);
        let Some(actual) = matches.next() else {
            return Err(ImageServiceError::InvalidRecord);
        };
        if matches.next().is_some() || !intent.accepts(actual.intent, actual.alpha) {
            return Err(ImageServiceError::InvalidRecord);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
