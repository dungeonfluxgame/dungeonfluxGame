//! Bounded, scoped fixture records shared by the infrastructure ports.
use super::FixtureError;
use crate::execution::*;
use crate::publication::PublishedImage;
use crate::runtime::{RuntimeError, RuntimeIds, SourceReceipt};
use crate::{
    AssetDigest, AssetId, Audience, ImageInterest, ImageJob, ImageOwner, OperationReceipt,
    SourceRef,
};
use df_assets::{
    AttemptId, ImageAlpha, ImageAlphaRequirement, ImageIntent, ObjectVersion, WriteId,
};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicU64, Ordering},
};

pub(super) const NOW_MS: i64 = 1_700_000_000_000;
/// Fixtures retain exact receipts and bytes until owned shutdown, never unbounded test history.
pub(super) const MAXIMUM_RECORDS: usize = 256;
pub(super) const MAXIMUM_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct FixtureIds(Arc<AtomicU64>);
impl FixtureIds {
    pub(super) fn new() -> Self {
        Self(Arc::new(AtomicU64::new(1)))
    }
}
impl df_assets::IdSource for FixtureIds {
    fn next_id_bytes(&self) -> [u8; 16] {
        let mut bytes = [0; 16];
        bytes[0] = 0x7f;
        bytes[8..].copy_from_slice(&self.0.fetch_add(1, Ordering::Relaxed).to_be_bytes());
        bytes
    }
}
#[derive(Clone, Copy)]
pub(super) struct FixtureClock;
impl crate::ServiceClock for FixtureClock {
    fn now_ms(&self) -> i64 {
        NOW_MS
    }
}

#[derive(Clone)]
pub(super) struct Store {
    pub(super) inner: Arc<Mutex<State>>,
    pub(super) ids: FixtureIds,
    pub(super) scope: CapacityScope,
}
pub(super) struct State {
    pub(super) owner: ImageOwner,
    pub(super) jobs: Vec<Job>,
    pub(super) assets: Vec<Asset>,
    pub(super) source_snapshots: Vec<(ExecutionScope, Vec<SourceReceipt>)>,
    pub(super) bounds: CapacityBounds,
    pub(super) gate: ExecutionGate,
    pub(super) byte_len: usize,
}
pub(super) struct Job {
    pub(super) job: ImageJob,
    pub(super) receipt: OperationReceipt,
    pub(super) interest: ImageInterest,
    pub(super) stage: StageView,
    pub(super) basis: StageBasis,
    pub(super) claim: Option<ClaimToken>,
    pub(super) dispatch: Option<DispatchView>,
    pub(super) checkpoint: Option<OutputCheckpoint>,
    pub(super) completion: Option<CompletionOutcome>,
    pub(super) staged: Option<Asset>,
    pub(super) result: Option<PublishedImage>,
    pub(super) sent: bool,
    pub(super) not_before_ms: i64,
    pub(super) deferred: bool,
    pub(super) queued_notice: Option<crate::execution::QueueDeferralNotice>,
    pub(super) urgency: crate::Urgency,
}
impl Job {
    pub(super) fn bump(&mut self, now_ms: i64) -> Result<(), ExecutionError> {
        self.job.revision = self
            .job
            .revision
            .checked_add(1)
            .ok_or(ExecutionError::InvalidRecord)?;
        self.job.updated_at_ms = now_ms;
        self.stage.job_revision = self.job.revision;
        Ok(())
    }
    pub(super) fn view(&self) -> crate::OperationView {
        crate::OperationView {
            job: self.job.clone(),
            receipt: self.receipt.clone(),
            interest: self.interest.clone(),
        }
    }
    pub(super) fn check_claim(
        &self,
        claim: ClaimToken,
        generation: u64,
    ) -> Result<(), ExecutionError> {
        if self.claim != Some(claim) || self.stage.claim_generation != generation {
            return Err(ExecutionError::LostClaim);
        }
        if self.stage.lease_until_ms <= NOW_MS {
            return Err(ExecutionError::Expired);
        }
        Ok(())
    }
}
#[derive(Clone)]
pub(super) struct Asset {
    pub(super) bytes: Vec<u8>,
    pub(super) receipt: SourceReceipt,
    pub(super) intent: ImageIntent,
    pub(super) alpha: ImageAlpha,
}
impl State {
    pub(super) fn job(
        &self,
        owner: &ImageOwner,
        job: crate::ImageJobId,
    ) -> Result<&Job, ExecutionError> {
        self.jobs
            .iter()
            .find(|entry| &entry.job.owner == owner && entry.job.job == job)
            .ok_or(ExecutionError::NotFound)
    }
    pub(super) fn job_mut(
        &mut self,
        owner: &ImageOwner,
        job: crate::ImageJobId,
    ) -> Result<&mut Job, ExecutionError> {
        self.jobs
            .iter_mut()
            .find(|entry| &entry.job.owner == owner && entry.job.job == job)
            .ok_or(ExecutionError::NotFound)
    }
    pub(super) fn dispatch(
        &self,
        owner: &ImageOwner,
        dispatch: DispatchId,
    ) -> Result<&Job, ExecutionError> {
        self.jobs
            .iter()
            .find(|entry| {
                &entry.job.owner == owner
                    && entry
                        .dispatch
                        .as_ref()
                        .is_some_and(|row| row.dispatch == dispatch)
            })
            .ok_or(ExecutionError::NotFound)
    }
}
impl Store {
    pub(super) fn new(owner: ImageOwner, ids: FixtureIds) -> Result<Self, RuntimeError> {
        let scope =
            CapacityScope::new("transform-fixture").map_err(|_| RuntimeError::InvalidRecord)?;
        let bounds = CapacityBounds::new(1, 256, 2 * 1024 * 1024)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(State {
                owner,
                jobs: Vec::new(),
                assets: Vec::new(),
                source_snapshots: Vec::new(),
                bounds,
                gate: ExecutionGate {
                    scope: scope.as_str().to_owned(),
                    applied_revision: 1,
                    enabled: true,
                },
                byte_len: 0,
            })),
            ids,
            scope,
        })
    }
    pub(super) fn lock(&self) -> Result<MutexGuard<'_, State>, RuntimeError> {
        self.inner
            .lock()
            .map_err(|_| RuntimeError::StorageUnavailable)
    }
    pub(super) fn import_source(
        &self,
        bytes: Vec<u8>,
        audience: Audience,
    ) -> Result<SourceRef, FixtureError> {
        let asset = self
            .asset(bytes, audience, None)
            .map_err(FixtureError::Runtime)?;
        let pin = asset.receipt.pin.clone();
        let mut state = self.lock().map_err(FixtureError::Runtime)?;
        if state.assets.len() >= MAXIMUM_RECORDS
            || state.byte_len.saturating_add(asset.bytes.len()) > MAXIMUM_BYTES
        {
            return Err(FixtureError::Capacity);
        }
        state.byte_len += asset.bytes.len();
        state.assets.push(asset);
        Ok(pin)
    }
    pub(super) fn read_asset(&self, asset: AssetId) -> Result<Vec<u8>, FixtureError> {
        self.lock()
            .map_err(FixtureError::Runtime)?
            .assets
            .iter()
            .find(|entry| entry.receipt.pin.asset == asset)
            .map(|entry| entry.bytes.clone())
            .ok_or(FixtureError::MissingAsset)
    }
    pub(super) fn asset(
        &self,
        bytes: Vec<u8>,
        audience: Audience,
        planned: Option<([u8; 16], [u8; 16])>,
    ) -> Result<Asset, RuntimeError> {
        if bytes.is_empty() || bytes.len() > MAXIMUM_BYTES {
            return Err(RuntimeError::InvalidRecord);
        }
        let format = image::guess_format(&bytes).map_err(|_| RuntimeError::InvalidRecord)?;
        let mime = match format {
            image::ImageFormat::Png => "image/png",
            image::ImageFormat::Jpeg => "image/jpeg",
            image::ImageFormat::WebP => "image/webp",
            _ => return Err(RuntimeError::InvalidRecord),
        };
        let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format)
            .into_dimensions()
            .map_err(|_| RuntimeError::InvalidRecord)?;
        if u64::from(width).saturating_mul(u64::from(height)) > crate::MAXIMUM_OUTPUT_PIXELS {
            return Err(RuntimeError::InvalidRecord);
        }
        let decoded = image::load_from_memory_with_format(&bytes, format)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let alpha = if decoded.to_rgba8().pixels().any(|pixel| pixel[3] != 255) {
            ImageAlpha::Transparent
        } else {
            ImageAlpha::Opaque
        };
        let intent = ImageIntent::new(mime, width, height, ImageAlphaRequirement::Preserve)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let (asset, attempt) =
            planned.unwrap_or_else(|| (self.ids.next_bytes(), self.ids.next_bytes()));
        let pin = SourceRef {
            asset: AssetId::from_bytes(&asset).map_err(|_| RuntimeError::InvalidRecord)?,
            digest: AssetDigest::from_bytes(Sha256::digest(&bytes).into()),
            audience,
        };
        let receipt = SourceReceipt {
            pin,
            attempt: AttemptId::from_bytes(&attempt).map_err(|_| RuntimeError::InvalidRecord)?,
            write: WriteId::from_bytes(&self.ids.next_bytes())
                .map_err(|_| RuntimeError::InvalidRecord)?,
            version: ObjectVersion::new(format!(
                "fixture-{}",
                crate::lower_hex(&self.ids.next_bytes())
            ))
            .ok_or(RuntimeError::InvalidRecord)?,
            suppression_generation: 0,
            retention: match &self.lock()?.owner {
                ImageOwner::Draft { .. } => df_assets::RetentionClass::Personal,
                ImageOwner::Run { .. } => df_assets::RetentionClass::CampaignMedia,
            },
        };
        Ok(Asset {
            bytes,
            receipt,
            intent,
            alpha,
        })
    }
}
