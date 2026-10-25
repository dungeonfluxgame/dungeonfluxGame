//! The catalog service (planning/asset-storage-research.md 6.2): admission, the staging endpoint's
//! write, and the validate, publish and finalize path. It owns no storage of its own: rows live in
//! the [`CatalogStore`], bytes in the [`ObjectStore`], money in F, provider output in O1's
//! dispatcher. It never retries a store call and never calls a provider.
//!
//! Failure rules it keeps: bytes are durable before metadata names them; a worker that loses its
//! lease or sees its attempt superseded stops and deletes nothing (the orphan sweep removes what it
//! wrote); only a terminal transition writes the settlement outbox row.
//!
//! TODO(O2-08): the read-only `dev assets` view (states, transitions, quotas, sweeper backlog) and
//! the storage metrics belong in df-tools' dev CLI and O1's exporter; this service exposes
//! `SweepReport` and the catalog reads they need.
//! TODO(O2-09): `CatalogConfig` is filled from `content/storage.json` once that loader exists.

use crate::asset_lifecycle::{AssetState, FinalizeOutcome, RejectReason};
use crate::audience::Audience;
use crate::catalog_config::CatalogConfig;
use crate::catalog_ports::{
    Clock, Dispatcher, FinanceAttempts, MediaValidator, StagingStream, StreamError, ValidatedMedia,
};
use crate::catalog_record::{
    AssetKind, AssetRecord, DedupeScope, HeldStep, Provenance, RetentionClass, StagedObjectRecord,
    Step,
};
use crate::catalog_store::{
    Admission, AdmitOutcome, CatalogError, CatalogLookup, CatalogStore, ClaimOutcome,
    FinalizeRequest, FinalizeResult, LeaseOutcome, NotAdmitted, RejectGuard, RejectResult,
    StageOutcome, StepClaim, VariantIntent, VariantWritten, Winner,
};
use crate::catalog_sweep::RefetchBackoff;
use crate::media_ids::{AssetId, AttemptId, ClaimToken, IdSource, WriteId, mint};
use crate::media_request_key::MediaRequestKey;
use crate::object_store::{
    AssetVariant, Deadline, KeyLayout, ObjectKey, ObjectStore, ObjectStoreError, StoredObject,
};
use crate::upload_ticket::{
    UploadPurpose, UploadTicket, UploadTicketClaim, UploadTicketRejection, validate_upload_ticket,
};
use crate::{AssetManifest, QuotaScope};
use crate::{ImageRejection, MAX_UPLOAD_BYTES, validate_image};
use df_types::{AccountId, CampaignId};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

/// A catalog or store call failed before the operation could finish; nothing was half-applied that
/// the next pass cannot repair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceFailure {
    Catalog(CatalogError),
    Store(ObjectStoreError),
    /// Personal (non-campaign) storage keys arrive with the upload round.
    /// TODO(O2-05): character-owned asset keys (`KeyLayout::character_asset`) in the upload round.
    UnsupportedOwner,
}

impl From<CatalogError> for ServiceFailure {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

impl From<ObjectStoreError> for ServiceFailure {
    fn from(error: ObjectStoreError) -> Self {
        Self::Store(error)
    }
}

/// One generated picture the hub wants admitted (the admission transaction of F calls the same
/// catalog function; this is the form for admissions that carry no money).
#[derive(Clone, Debug)]
pub struct GenerationRequest {
    /// The attempt F minted at admission (D22).
    pub attempt: AttemptId,
    pub kind: AssetKind,
    pub charge_scope: QuotaScope,
    pub origin_account: Option<AccountId>,
    pub campaign: Option<CampaignId>,
    /// The meet of the requested audience and every reference input's audience (6.4).
    pub audience: Audience,
    pub dedupe_scope: DedupeScope,
    pub request_key: MediaRequestKey,
    pub retention: RetentionClass,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationAdmission {
    Admitted { asset: AssetId },
    NotAdmitted(NotAdmitted),
}

/// Why the staging endpoint took nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StagingRefusal {
    UnknownAsset,
    /// The attempt is not the asset's current attempt (FILE-02).
    NotCurrent,
    /// Validated bytes are never replaced by a second write.
    AlreadyStaged,
    NotAccepting {
        state: AssetState,
    },
    UploadTicket(UploadTicketRejection),
    ImageValidation(ImageRejection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StagingOutcome {
    Staged {
        bytes: u64,
    },
    /// The validated upload reached the ready state.
    Promoted {
        bytes: u64,
    },
    /// Staging completed but publication did not reach `Ready` in this pass.
    PromotionPending(Advance),
    Refused(StagingRefusal),
    /// The stream passed the kind's maximum: nothing was stored and the attempt is terminal
    /// `Invalid(over_maximum_bytes)`.
    TooLarge,
    /// The provider stream broke; nothing was recorded and the dispatcher may send again.
    StreamFailed,
    /// A chunk or the whole transfer missed its deadline; nothing was recorded.
    TimedOut,
}

/// What one pass over a staged row achieved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Advance {
    Ready,
    /// The attempt ended without a deliverable asset; the outbox row was written.
    Ended(FinalizeOutcome),
    /// Another worker holds the lease, or the row moved on while we claimed.
    Busy,
    /// The row is not waiting for validation or publication.
    NotWaiting {
        state: AssetState,
    },
    /// A newer claim replaced ours: this worker stopped and deleted nothing.
    LeaseLost,
    /// The attempt was superseded or the row ended: this worker stopped and deleted nothing.
    Superseded,
    /// A store call failed; the lease is left to expire and the sweeper tries again.
    Retry,
}

/// How often a lost winner is rewritten under a new write id before the pass gives up.
const MAXIMUM_WINNER_REWRITES: usize = 2;

pub struct CatalogService<C, S, F, D, V> {
    pub(crate) catalog: Arc<C>,
    pub(crate) objects: Arc<S>,
    pub(crate) finance: Arc<F>,
    pub(crate) dispatcher: Arc<D>,
    validator: Arc<V>,
    pub(crate) clock: Arc<dyn Clock>,
    ids: Arc<dyn IdSource>,
    pub(crate) layout: KeyLayout,
    pub(crate) config: CatalogConfig,
    pub(crate) refetch_backoff: Mutex<RefetchBackoff>,
}

/// The injected collaborators of a [`CatalogService`].
pub struct CatalogPorts<C, S, F, D, V> {
    pub catalog: Arc<C>,
    pub objects: Arc<S>,
    pub finance: Arc<F>,
    pub dispatcher: Arc<D>,
    pub validator: Arc<V>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdSource>,
}

pub(crate) fn audit(line: String) {
    df_observe::record_audit_line("ASSET", line);
}

pub(crate) fn manifest_of(bytes: &[u8]) -> AssetManifest {
    AssetManifest {
        byte_len: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    }
}

impl<C, S, F, D, V> CatalogService<C, S, F, D, V>
where
    C: CatalogStore,
    S: ObjectStore,
    F: FinanceAttempts,
    D: Dispatcher,
    V: MediaValidator,
{
    pub fn new(
        ports: CatalogPorts<C, S, F, D, V>,
        layout: KeyLayout,
        config: CatalogConfig,
    ) -> Self {
        let config = config.validated();
        Self {
            catalog: ports.catalog,
            objects: ports.objects,
            finance: ports.finance,
            dispatcher: ports.dispatcher,
            validator: ports.validator,
            clock: ports.clock,
            ids: ports.ids,
            layout,
            config,
            refetch_backoff: Mutex::new(RefetchBackoff::new(
                config.refetch_backoff_ms,
                config.refetch_backoff_cap_ms,
            )),
        }
    }

    pub(crate) fn deadline(&self) -> Deadline {
        Deadline::after(self.config.store_deadline)
    }

    pub(crate) fn fresh_claim(&self) -> ClaimToken {
        mint(self.ids.as_ref(), ClaimToken::from_bytes)
    }

    /// What the hub learns before it spends anything (6.3).
    pub async fn by_request_key(
        &self,
        scope: DedupeScope,
        key: &MediaRequestKey,
    ) -> Result<CatalogLookup, ServiceFailure> {
        Ok(self.catalog.lookup(scope, key).await?)
    }

    /// Admits one generated asset without money; F's admission transaction runs the same catalog
    /// function. A `Withheld` audience is refused here, before the catalog is touched (6.4).
    pub async fn admit_generation(
        &self,
        request: &GenerationRequest,
    ) -> Result<GenerationAdmission, ServiceFailure> {
        if request.audience == Audience::Withheld {
            audit(format!(
                "ASSET admission_refused reason=audience_empty attempt={}",
                request.attempt.to_hex()
            ));
            return Ok(GenerationAdmission::NotAdmitted(NotAdmitted::AudienceEmpty));
        }
        if request.campaign.is_none() {
            audit(format!(
                "ASSET admission_refused reason=owner_unsupported attempt={}",
                request.attempt.to_hex()
            ));
            return Ok(GenerationAdmission::NotAdmitted(
                NotAdmitted::OwnerUnsupported,
            ));
        }
        let asset = mint(self.ids.as_ref(), AssetId::from_bytes);
        let admission = Admission {
            asset,
            attempt: request.attempt,
            kind: request.kind,
            provenance: Provenance::Generated,
            charge_scope: request.charge_scope,
            origin_account: request.origin_account,
            campaign: request.campaign,
            audience: request.audience.clone(),
            dedupe_scope: request.dedupe_scope,
            request_key: Some(request.request_key),
            retention: request.retention,
            reserve_bytes: self.config.maximum_bytes.for_kind(request.kind),
            byte_limit: self.config.quota.byte_limit(request.charge_scope),
            in_flight_upload_limit: None,
        };
        match self.catalog.admit(&admission, self.clock.now_ms()).await? {
            AdmitOutcome::Admitted => {
                audit(format!(
                    "ASSET admitted asset={} attempt={} bytes={}",
                    asset.to_hex(),
                    request.attempt.to_hex(),
                    admission.reserve_bytes
                ));
                Ok(GenerationAdmission::Admitted { asset })
            }
            AdmitOutcome::NotAdmitted(reason) => Ok(GenerationAdmission::NotAdmitted(reason)),
        }
    }

    /// The staging endpoint's write (D18, D23): accepts output only while the attempt is current and
    /// the row still `Uploading`, reads at most one byte past the kind's maximum, stores the object
    /// under a fresh write id, and records it with a compare-and-set.
    pub async fn stage_output(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        stream: &mut impl StagingStream,
    ) -> Result<StagingOutcome, ServiceFailure> {
        let row = match self.catalog.asset(asset).await? {
            None => return Ok(self.refuse_staging(asset, StagingRefusal::UnknownAsset)),
            Some(row) if row.attempt != attempt => {
                return Ok(self.refuse_staging(asset, StagingRefusal::NotCurrent));
            }
            Some(row) => row,
        };
        if row.provenance == Provenance::Uploaded {
            return Ok(
                self.refuse_staging(asset, StagingRefusal::NotAccepting { state: row.state })
            );
        }
        if row.state != AssetState::Uploading {
            let refusal = match row.state {
                AssetState::Quarantined | AssetState::Validated => StagingRefusal::AlreadyStaged,
                state => StagingRefusal::NotAccepting { state },
            };
            return Ok(self.refuse_staging(asset, refusal));
        }
        let limit = self.config.maximum_bytes.for_kind(row.kind);
        let timeouts = (
            self.config.staging_chunk_timeout,
            self.config.staging_total_timeout,
        );
        let body = match read_bounded(stream, limit, timeouts).await {
            BoundedRead::Complete(body) => body,
            BoundedRead::OverLimit => return self.reject_oversize(asset, attempt).await,
            BoundedRead::Failed => return Ok(StagingOutcome::StreamFailed),
            BoundedRead::TimedOut => {
                audit(format!(
                    "ASSET staging_timed_out asset={} attempt={}",
                    asset.to_hex(),
                    attempt.to_hex()
                ));
                return Ok(StagingOutcome::TimedOut);
            }
        };
        self.store_staged(asset, attempt, body).await
    }

    /// Claims an authenticated portrait ticket before reading the request body, then validates
    /// and stages only the metadata-stripped image. Rejected bytes remain in quarantine.
    pub async fn stage_player_portrait(
        &self,
        ticket: &UploadTicket,
        account: AccountId,
        campaign: CampaignId,
        declared_mime: &str,
        extension: &str,
        stream: &mut impl StagingStream,
    ) -> Result<StagingOutcome, ServiceFailure> {
        match self
            .catalog
            .claim_upload_ticket(
                ticket,
                account,
                campaign,
                UploadPurpose::Portrait,
                self.clock.now_ms(),
            )
            .await?
        {
            UploadTicketClaim::Claimed => {}
            UploadTicketClaim::Refused(reason) => {
                audit(format!(
                    "ASSET upload_refused asset={} reason={}",
                    ticket.asset.to_hex(),
                    reason.label()
                ));
                return Ok(StagingOutcome::Refused(StagingRefusal::UploadTicket(
                    reason,
                )));
            }
        }
        let timeouts = (
            self.config.staging_chunk_timeout,
            self.config.staging_total_timeout,
        );
        let body = match read_bounded(stream, MAX_UPLOAD_BYTES as u64, timeouts).await {
            BoundedRead::Complete(body) => body,
            BoundedRead::OverLimit => {
                return self.reject_oversize(ticket.asset, ticket.attempt).await;
            }
            BoundedRead::Failed => {
                audit(format!(
                    "ASSET upload_refused asset={} reason=stream_failed",
                    ticket.asset.to_hex()
                ));
                return Ok(StagingOutcome::StreamFailed);
            }
            BoundedRead::TimedOut => {
                audit(format!(
                    "ASSET upload_refused asset={} reason=stream_timed_out",
                    ticket.asset.to_hex()
                ));
                return Ok(StagingOutcome::TimedOut);
            }
        };
        if let Err(reason) = validate_upload_ticket(
            ticket,
            account,
            campaign,
            UploadPurpose::Portrait,
            &ticket.object_key,
            &body,
            self.clock.now_ms(),
        ) {
            let quarantined = self
                .store_staged_at(ticket.asset, ticket.attempt, body, &ticket.object_key)
                .await?;
            if !matches!(quarantined, StagingOutcome::Staged { .. }) {
                return Ok(quarantined);
            }
            let _ = self
                .catalog
                .reject(
                    ticket.asset,
                    ticket.attempt,
                    RejectReason::ValidationFailed,
                    RejectGuard::WhileUnleased,
                    self.clock.now_ms(),
                )
                .await?;
            audit(format!(
                "ASSET upload_quarantined asset={} reason={}",
                ticket.asset.to_hex(),
                reason.label()
            ));
            return Ok(StagingOutcome::Refused(StagingRefusal::UploadTicket(
                reason,
            )));
        }
        let clean = match validate_image(&body, declared_mime, extension) {
            Ok(image) => image.bytes,
            Err(reason) => {
                let quarantined = self
                    .store_staged_at(ticket.asset, ticket.attempt, body, &ticket.object_key)
                    .await?;
                if !matches!(quarantined, StagingOutcome::Staged { .. }) {
                    return Ok(quarantined);
                }
                let _ = self
                    .catalog
                    .reject(
                        ticket.asset,
                        ticket.attempt,
                        RejectReason::ValidationFailed,
                        RejectGuard::WhileUnleased,
                        self.clock.now_ms(),
                    )
                    .await?;
                audit(format!(
                    "ASSET upload_quarantined asset={} reason={reason:?}",
                    ticket.asset.to_hex()
                ));
                return Ok(StagingOutcome::Refused(StagingRefusal::ImageValidation(
                    reason,
                )));
            }
        };
        match self
            .store_staged_at(ticket.asset, ticket.attempt, clean, &ticket.object_key)
            .await?
        {
            StagingOutcome::Staged { bytes } => match self.advance(ticket.asset).await? {
                Advance::Ready => Ok(StagingOutcome::Promoted { bytes }),
                pending => Ok(StagingOutcome::PromotionPending(pending)),
            },
            other => Ok(other),
        }
    }

    fn refuse_staging(&self, asset: AssetId, refusal: StagingRefusal) -> StagingOutcome {
        audit(format!(
            "ASSET staging_refused asset={} reason={refusal:?}",
            asset.to_hex()
        ));
        StagingOutcome::Refused(refusal)
    }

    /// Ends the attempt `Invalid(over_maximum_bytes)` only if no other writer staged a valid object
    /// meanwhile: the rejection is conditional on the row still being `Uploading`.
    async fn reject_oversize(
        &self,
        asset: AssetId,
        attempt: AttemptId,
    ) -> Result<StagingOutcome, ServiceFailure> {
        let result = self
            .catalog
            .reject(
                asset,
                attempt,
                RejectReason::OverMaximumBytes,
                RejectGuard::WhileUploading,
                self.clock.now_ms(),
            )
            .await?;
        match result {
            RejectResult::Rejected(_) => {
                audit(format!(
                    "ASSET invalid asset={} attempt={} reason=over_maximum_bytes",
                    asset.to_hex(),
                    attempt.to_hex()
                ));
                Ok(StagingOutcome::TooLarge)
            }
            RejectResult::GuardFailed { .. } => {
                Ok(self.refuse_staging(asset, StagingRefusal::AlreadyStaged))
            }
            RejectResult::AlreadyEnded { state } => {
                Ok(self.refuse_staging(asset, StagingRefusal::NotAccepting { state }))
            }
            RejectResult::NotCurrent => Ok(self.refuse_staging(asset, StagingRefusal::NotCurrent)),
        }
    }

    async fn store_staged(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        body: Vec<u8>,
    ) -> Result<StagingOutcome, ServiceFailure> {
        let write = mint(self.ids.as_ref(), WriteId::from_bytes);
        let key = self.layout.staged(asset, attempt, write);
        self.store_staged_at(asset, attempt, body, &key).await
    }

    async fn store_staged_at(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        body: Vec<u8>,
        key: &ObjectKey,
    ) -> Result<StagingOutcome, ServiceFailure> {
        let manifest = manifest_of(&body);
        let stored = self.put_confirmed(key, &body, manifest).await?;
        let record = StagedObjectRecord {
            key: stored.key.clone(),
            version: stored.version.clone(),
            manifest,
            staged_at_ms: self.clock.now_ms(),
        };
        let outcome = self
            .catalog
            .record_staged(asset, attempt, &record, self.clock.now_ms())
            .await?;
        let refusal = match outcome {
            StageOutcome::Staged => {
                audit(format!(
                    "ASSET staged asset={} attempt={} bytes={}",
                    asset.to_hex(),
                    attempt.to_hex(),
                    manifest.byte_len
                ));
                return Ok(StagingOutcome::Staged {
                    bytes: manifest.byte_len,
                });
            }
            StageOutcome::AlreadyStaged => StagingRefusal::AlreadyStaged,
            StageOutcome::NotCurrent => StagingRefusal::NotCurrent,
            StageOutcome::NotAccepting { state } => StagingRefusal::NotAccepting { state },
        };
        // Our object was never recorded, so nothing else can name it: remove it now.
        self.objects
            .delete_version(&stored.key, &stored.version, self.deadline())
            .await?;
        Ok(self.refuse_staging(asset, refusal))
    }

    /// `put_new`, and after a write whose acknowledgement was lost, the head check that decides
    /// whether it took effect before anything is retried.
    async fn put_confirmed(
        &self,
        key: &ObjectKey,
        body: &[u8],
        manifest: AssetManifest,
    ) -> Result<StoredObject, ObjectStoreError> {
        match self
            .objects
            .put_new(key, body, manifest, self.deadline())
            .await
        {
            Err(ObjectStoreError::Unknown) => {
                match self.objects.head(key, self.deadline()).await? {
                    Some(stored) if stored.manifest == manifest => Ok(stored),
                    _ => Err(ObjectStoreError::Unknown),
                }
            }
            other => other,
        }
    }

    /// Resumes one staged row: claim, read the staged object, validate, write the variants,
    /// finalize. Safe to call from the sweeper and from the staging path alike.
    pub async fn advance(&self, asset: AssetId) -> Result<Advance, ServiceFailure> {
        let Some(row) = self.catalog.asset(asset).await? else {
            return Ok(Advance::Superseded);
        };
        let step = match row.state {
            AssetState::Quarantined => Step::Validate,
            AssetState::Validated => Step::Publish,
            state => return Ok(Advance::NotWaiting { state }),
        };
        let Some(staged) = row.staged.clone() else {
            return Err(ServiceFailure::Catalog(CatalogError::InvalidRecord));
        };
        if row.campaign.is_none() {
            return Err(ServiceFailure::UnsupportedOwner);
        }
        let now_ms = self.clock.now_ms();
        let claim = StepClaim {
            asset,
            attempt: row.attempt,
            step,
            expected_state: row.state,
            owner: self.fresh_claim(),
            lease_until_ms: now_ms + self.config.step_lease_ms,
        };
        match self.catalog.claim_step(claim, now_ms).await? {
            ClaimOutcome::Claimed => {}
            ClaimOutcome::Held { .. } | ClaimOutcome::Stale => return Ok(Advance::Busy),
        }
        let held = HeldStep {
            asset,
            attempt: row.attempt,
            owner: claim.owner,
        };
        self.work_claimed_row(&row, held, &staged).await
    }

    async fn work_claimed_row(
        &self,
        row: &AssetRecord,
        held: HeldStep,
        staged: &StagedObjectRecord,
    ) -> Result<Advance, ServiceFailure> {
        let object = StoredObject {
            key: staged.key.clone(),
            version: staged.version.clone(),
            manifest: staged.manifest,
        };
        let bytes = match self.objects.get_version(&object, self.deadline()).await {
            Ok(bytes) => bytes,
            Err(ObjectStoreError::NotFound | ObjectStoreError::IntegrityMismatch) => {
                return self
                    .end_attempt(
                        held.asset,
                        held.attempt,
                        RejectReason::StagedLost,
                        RejectGuard::Current,
                    )
                    .await;
            }
            Err(error) => return Ok(self.retry_later(held, error)),
        };
        let validation = if row.provenance == Provenance::Uploaded {
            validate_uploaded_image(&bytes)
        } else {
            self.validator.validate(row.kind, &bytes).await
        };
        let media = match validation {
            Ok(media) => media,
            Err(reason) => {
                return self
                    .end_attempt(held.asset, held.attempt, reason, RejectGuard::Current)
                    .await;
            }
        };
        if row.state == AssetState::Quarantined {
            let moved = self
                .catalog
                .mark_validated(held, self.clock.now_ms())
                .await?;
            if let Some(stop) = stopped(moved) {
                return Ok(self.stop_worker(held, stop));
            }
            audit(format!(
                "ASSET validated asset={} attempt={}",
                held.asset.to_hex(),
                held.attempt.to_hex()
            ));
        }
        self.publish(row, held, media).await
    }

    fn retry_later(&self, held: HeldStep, error: ObjectStoreError) -> Advance {
        audit(format!(
            "ASSET retry_later asset={} attempt={} reason={}",
            held.asset.to_hex(),
            held.attempt.to_hex(),
            error.label()
        ));
        Advance::Retry
    }

    fn stop_worker(&self, held: HeldStep, stop: Advance) -> Advance {
        let label = if stop == Advance::LeaseLost {
            "lease_lost"
        } else {
            "superseded"
        };
        audit(format!(
            "ASSET {label} asset={} attempt={}",
            held.asset.to_hex(),
            held.attempt.to_hex()
        ));
        stop
    }

    /// The terminal rejection of the held attempt.
    pub(crate) async fn end_attempt(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        reason: RejectReason,
        guard: RejectGuard,
    ) -> Result<Advance, ServiceFailure> {
        let result = self
            .catalog
            .reject(asset, attempt, reason, guard, self.clock.now_ms())
            .await?;
        Ok(match result {
            RejectResult::Rejected(outcome) => {
                audit(format!(
                    "ASSET {} asset={} attempt={} reason={}",
                    outcome.label(),
                    asset.to_hex(),
                    attempt.to_hex(),
                    reason.label()
                ));
                Advance::Ended(outcome)
            }
            RejectResult::GuardFailed { .. } => Advance::Busy,
            RejectResult::AlreadyEnded { .. } | RejectResult::NotCurrent => Advance::Superseded,
        })
    }

    async fn publish(
        &self,
        row: &AssetRecord,
        held: HeldStep,
        media: ValidatedMedia,
    ) -> Result<Advance, ServiceFailure> {
        let Some(campaign) = row.campaign else {
            return Err(ServiceFailure::UnsupportedOwner);
        };
        let now_ms = self.clock.now_ms();
        let renewed = self
            .catalog
            .renew_step(held, now_ms + self.config.step_lease_ms, now_ms)
            .await?;
        if let Some(stop) = stopped(renewed) {
            return Ok(self.stop_worker(held, stop));
        }
        let mut variants = vec![(AssetVariant::Master, media.master)];
        variants.extend(
            media
                .display
                .map(|display| (AssetVariant::Display, display)),
        );
        let mut winners = Vec::with_capacity(variants.len());
        for (variant, bytes) in &variants {
            match self.write_variant(&campaign, held, *variant, bytes).await? {
                Written::Winner(winner) => winners.push(winner),
                Written::Stop(stop) => return Ok(stop),
            }
        }
        let request = FinalizeRequest {
            winners,
            mime: media.mime,
            width_px: media.width_px,
            height_px: media.height_px,
            byte_limit: self.config.quota.byte_limit(row.charge_scope),
        };
        self.finalize_with_rewrites(&campaign, held, request, &variants)
            .await
    }

    async fn finalize_with_rewrites(
        &self,
        campaign: &CampaignId,
        held: HeldStep,
        mut request: FinalizeRequest,
        variants: &[(AssetVariant, Vec<u8>)],
    ) -> Result<Advance, ServiceFailure> {
        for _ in 0..=MAXIMUM_WINNER_REWRITES {
            let result = self
                .catalog
                .finalize(held, &request, self.clock.now_ms())
                .await?;
            match result {
                FinalizeResult::Ready { committed_bytes } => {
                    audit(format!(
                        "ASSET ready asset={} attempt={} bytes={committed_bytes}",
                        held.asset.to_hex(),
                        held.attempt.to_hex()
                    ));
                    return Ok(Advance::Ready);
                }
                FinalizeResult::LeaseLost => {
                    return Ok(self.stop_worker(held, Advance::LeaseLost));
                }
                FinalizeResult::Superseded => {
                    return Ok(self.stop_worker(held, Advance::Superseded));
                }
                FinalizeResult::Rejected(outcome) => {
                    audit(format!(
                        "ASSET {} asset={} attempt={} reason=quota_exceeded",
                        outcome.label(),
                        held.asset.to_hex(),
                        held.attempt.to_hex()
                    ));
                    return Ok(Advance::Ended(outcome));
                }
                FinalizeResult::WinnerMissing { variant } => {
                    audit(format!(
                        "ASSET winner_missing asset={} attempt={} variant={}",
                        held.asset.to_hex(),
                        held.attempt.to_hex(),
                        variant.label()
                    ));
                    let Some((_, bytes)) = variants.iter().find(|(named, _)| *named == variant)
                    else {
                        return Err(ServiceFailure::Catalog(CatalogError::InvalidRecord));
                    };
                    match self.write_variant(campaign, held, variant, bytes).await? {
                        Written::Winner(winner) => replace_winner(&mut request.winners, winner),
                        Written::Stop(stop) => return Ok(stop),
                    }
                }
            }
        }
        Ok(Advance::Retry)
    }

    /// One variant under a fresh write id: the intent row first (conditional on the live lease),
    /// then the object, then the `written` mark.
    async fn write_variant(
        &self,
        campaign: &CampaignId,
        held: HeldStep,
        variant: AssetVariant,
        bytes: &[u8],
    ) -> Result<Written, ServiceFailure> {
        let write = mint(self.ids.as_ref(), WriteId::from_bytes);
        let key = self
            .layout
            .campaign_asset(campaign, held.asset, held.attempt, write, variant);
        let intent = VariantIntent {
            variant,
            write,
            key: key.clone(),
        };
        let intended = self
            .catalog
            .intend_variant(held, &intent, self.clock.now_ms())
            .await?;
        if let Some(stop) = stopped(intended) {
            return Ok(Written::Stop(self.stop_worker(held, stop)));
        }
        let manifest = manifest_of(bytes);
        let stored = match self.put_confirmed(&key, bytes, manifest).await {
            Ok(stored) => stored,
            Err(error) => return Ok(Written::Stop(self.retry_later(held, error))),
        };
        let written = VariantWritten {
            variant,
            write,
            version: stored.version,
            manifest,
        };
        let marked = self
            .catalog
            .mark_variant_written(held.asset, held.attempt, &written)
            .await?;
        if !marked {
            audit(format!(
                "ASSET variant_deleting asset={} attempt={} variant={}",
                held.asset.to_hex(),
                held.attempt.to_hex(),
                variant.label()
            ));
            return Ok(Written::Stop(Advance::Retry));
        }
        Ok(Written::Winner(Winner { variant, write }))
    }
}

enum Written {
    Winner(Winner),
    Stop(Advance),
}

fn replace_winner(winners: &mut [Winner], replacement: Winner) {
    for winner in winners {
        if winner.variant == replacement.variant {
            *winner = replacement;
        }
    }
}

/// The worker-stopping reading of a lease outcome; `None` means the worker continues.
fn stopped(outcome: LeaseOutcome) -> Option<Advance> {
    match outcome {
        LeaseOutcome::Done => None,
        LeaseOutcome::LeaseLost => Some(Advance::LeaseLost),
        LeaseOutcome::Superseded => Some(Advance::Superseded),
    }
}

fn validate_uploaded_image(bytes: &[u8]) -> Result<ValidatedMedia, RejectReason> {
    let (mime, extension) =
        match image::guess_format(bytes).map_err(|_| RejectReason::ValidationFailed)? {
            image::ImageFormat::Png => ("image/png", "png"),
            image::ImageFormat::Jpeg => ("image/jpeg", "jpg"),
            image::ImageFormat::WebP => ("image/webp", "webp"),
            _ => return Err(RejectReason::ValidationFailed),
        };
    let image =
        validate_image(bytes, mime, extension).map_err(|_| RejectReason::ValidationFailed)?;
    Ok(ValidatedMedia {
        master: image.bytes,
        display: None,
        mime: image.mime.to_owned(),
        width_px: Some(image.width_px),
        height_px: Some(image.height_px),
    })
}

enum BoundedRead {
    Complete(Vec<u8>),
    OverLimit,
    Failed,
    TimedOut,
}

/// Reads the stream up to `limit` bytes; the first byte past the limit aborts the read. Each chunk
/// gets `chunk_timeout`, and the whole transfer `total_timeout`, whichever ends first.
async fn read_bounded(
    stream: &mut impl StagingStream,
    limit: u64,
    (chunk_timeout, total_timeout): (std::time::Duration, std::time::Duration),
) -> BoundedRead {
    let total = Deadline::after(total_timeout);
    let mut body = Vec::new();
    loop {
        if total.has_passed() {
            return BoundedRead::TimedOut;
        }
        let chunk_deadline = Deadline::after(chunk_timeout).earlier_of(total);
        match stream.next_chunk(chunk_deadline).await {
            Ok(Some(chunk)) => {
                if body.len() as u64 + chunk.len() as u64 > limit {
                    return BoundedRead::OverLimit;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return BoundedRead::Complete(body),
            Err(StreamError::Failed) => return BoundedRead::Failed,
            Err(StreamError::DeadlineExceeded) => return BoundedRead::TimedOut,
        }
    }
}
