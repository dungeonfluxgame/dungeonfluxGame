//! Trusted host references use the established catalog staging, validation and publication flow.

use bytes::Bytes;
use df_assets::{
    Admission, AdmitOutcome, Advance, AssetId, AssetKind, AttemptId, Audience, CatalogConfig,
    CatalogPorts, CatalogService, CatalogStore, Clock, DedupeScope, Dispatcher, FinanceAttempts,
    FinanceUnavailable, IdSource, KeyLayout, MediaRequestKey, MediaValidator, NotAdmitted,
    ObjectStore, Provenance, QuotaScope, RefetchAnswer, RejectReason, RetentionClass,
    StagingOutcome, StagingRefusal, StagingStream, StreamError, ValidatedMedia,
};
use df_imagery::{ImageAuthorizer, ImageOwner, SourceRef, runtime::ImageSources};
use df_persistence::media_catalog::PostgresMediaCatalog;
use sha2::{Digest, Sha256};
use std::sync::Arc;

use super::{
    image_references::{TrustedReferenceImport, format_of, source_failure},
    image_sources::CatalogImageSources,
    imagery::{ImageFailure, ReferenceImage, central::ReferenceFuture},
    imagery_identity::{ExactAssetAllowlist, ImageryAuthorizer},
};

/// Reviewed imports have no supplier attempt; these ports refuse every supplier operation.
/// CatalogService admission/stage/advance never consults them for reviewed local media.
pub(super) struct ImportSupplierBoundary;
impl FinanceAttempts for ImportSupplierBoundary {
    async fn attempt_state(
        &self,
        _: AttemptId,
    ) -> Result<df_assets::ProviderAttemptState, FinanceUnavailable> {
        Err(FinanceUnavailable)
    }
    async fn cancel_unclaimed_effect(
        &self,
        _: AttemptId,
    ) -> Result<df_assets::CancelOutcome, FinanceUnavailable> {
        Err(FinanceUnavailable)
    }
}
impl Dispatcher for ImportSupplierBoundary {
    async fn request_refetch(&self, _: AssetId, _: AttemptId) -> RefetchAnswer {
        RefetchAnswer::Unavailable
    }
}

/// Uses the same bounded image validation/re-encoding as existing uploads, with owned CPU work.
pub(super) struct ImportImageValidator {
    decode: Arc<std::sync::Mutex<Arc<tokio::sync::Semaphore>>>,
    deadline: std::time::Duration,
    maximum_bytes: u64,
    #[cfg(test)]
    probe: Option<Arc<DecodeProbe>>,
}
#[cfg(test)]
struct DecodeProbe {
    started: tokio::sync::Notify,
    released: std::sync::Mutex<bool>,
    release: std::sync::Condvar,
}
impl MediaValidator for ImportImageValidator {
    async fn validate(
        &self,
        kind: AssetKind,
        staged: &[u8],
    ) -> Result<ValidatedMedia, RejectReason> {
        if kind != AssetKind::Image || staged.len() as u64 > self.maximum_bytes {
            return Err(RejectReason::ValidationFailed);
        }
        let decode = self
            .decode
            .lock()
            .map_err(|_| RejectReason::ValidationFailed)?
            .clone();
        let permit = tokio::time::timeout(self.deadline, decode.acquire_owned())
            .await
            .map_err(|_| RejectReason::ValidationFailed)?
            .map_err(|_| RejectReason::ValidationFailed)?;
        let staged = staged.to_vec();
        let maximum_bytes = self.maximum_bytes;
        #[cfg(test)]
        let probe = self.probe.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            #[cfg(test)]
            if let Some(probe) = probe {
                let released = probe.released.lock().unwrap();
                probe.started.notify_one();
                drop(
                    probe
                        .release
                        .wait_while(released, |released| !*released)
                        .unwrap(),
                );
            }
            let format = format_of(&staged).map_err(|_| RejectReason::ValidationFailed)?;
            let (mime, extension) = match format {
                df_providers::ImageFormat::Png => ("image/png", "png"),
                df_providers::ImageFormat::Jpeg => ("image/jpeg", "jpg"),
                df_providers::ImageFormat::Webp => ("image/webp", "webp"),
            };
            let validated = df_assets::validate_image_with_limits(
                &staged,
                mime,
                extension,
                maximum_bytes,
                df_assets::MAX_IMAGE_PIXELS,
            )
            .map_err(|_| RejectReason::ValidationFailed)?;
            Ok(ValidatedMedia {
                // Trusted host bytes already carry the approved pixels. Validation checks
                // their complete raster; keeping the original avoids a second lossy JPEG encode.
                master: staged,
                display: None,
                mime: validated.mime.to_owned(),
                width_px: Some(validated.width_px),
                height_px: Some(validated.height_px),
            })
        })
        .await
        .map_err(|_| RejectReason::ValidationFailed)?
    }
}

type ImportPipeline<S> = CatalogService<
    PostgresMediaCatalog,
    S,
    ImportSupplierBoundary,
    ImportSupplierBoundary,
    ImportImageValidator,
>;

pub(super) struct ImportSettings {
    pub(super) owner: ImageOwner,
    pub(super) catalog: CatalogConfig,
    pub(super) layout: KeyLayout,
}
pub(super) struct ImportPorts<S> {
    pub(super) catalog: Arc<PostgresMediaCatalog>,
    pub(super) sources: Arc<CatalogImageSources<S>>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) ids: Arc<dyn IdSource>,
    pub(super) decode: Arc<std::sync::Mutex<Arc<tokio::sync::Semaphore>>>,
    pub(super) gate: Arc<tokio::sync::RwLock<()>>,
}
pub(super) struct CatalogReferenceImporter<S> {
    settings: ImportSettings,
    ports: ImportPorts<S>,
    pipeline: ImportPipeline<S>,
}
impl<S: ObjectStore> CatalogReferenceImporter<S> {
    pub(super) fn new(settings: ImportSettings, ports: ImportPorts<S>) -> Self {
        let validator = ImportImageValidator {
            decode: ports.decode.clone(),
            deadline: ports.sources.deadline,
            maximum_bytes: settings.catalog.maximum_bytes.image,
            #[cfg(test)]
            probe: None,
        };
        let pipeline = CatalogService::new(
            CatalogPorts {
                catalog: ports.catalog.clone(),
                objects: ports.sources.objects.clone(),
                finance: Arc::new(ImportSupplierBoundary),
                dispatcher: Arc::new(ImportSupplierBoundary),
                validator: Arc::new(validator),
                clock: ports.clock.clone(),
                ids: ports.ids.clone(),
            },
            settings.layout.clone(),
            settings.catalog,
        );
        Self {
            settings,
            ports,
            pipeline,
        }
    }

    async fn reviewed(&self, reference: &ReferenceImage) -> Result<ReferenceImage, ImageFailure> {
        // This owned admission guard lets configuration fence and drain complete imports.
        let _admission = tokio::time::timeout(
            self.ports.sources.deadline,
            self.ports.gate.clone().read_owned(),
        )
        .await
        .map_err(|_| ImageFailure::DeadlineExceeded)?;
        let fresh = if let Some(reviewed) = reference.reviewed_source() {
            let reviewed = reviewed.clone();
            tokio::task::spawn_blocking(move || reviewed.reload())
                .await
                .map_err(|_| ImageFailure::Interrupted)?
                .map_err(|_| ImageFailure::Refused)?
        } else if let Some(origin) = reference.trusted_source() {
            origin.read_current(&self.settings.owner).await?
        } else {
            return Err(ImageFailure::InvalidRequest);
        };
        if fresh.bytes != reference.bytes || fresh.format != reference.format {
            return Err(ImageFailure::Refused);
        }
        let (scope, charge, campaign, account, audience, provenance, retention) =
            match self.settings.owner {
                ImageOwner::Run { campaign, .. } => (
                    DedupeScope::campaign(&campaign),
                    QuotaScope::Campaign(campaign),
                    Some(campaign),
                    None,
                    Audience::CampaignShared(campaign),
                    Provenance::Reviewed,
                    RetentionClass::CampaignMedia,
                ),
                ImageOwner::Draft { account } => {
                    if !matches!(
                        reference.trusted_source(),
                        Some(super::imagery::TrustedReferenceOrigin::ApprovedPortrait(_))
                    ) {
                        return Err(ImageFailure::Refused);
                    }
                    (
                        DedupeScope::account(&account),
                        QuotaScope::Account(account),
                        None,
                        Some(account),
                        Audience::OwnerPrivate(account),
                        Provenance::ClaimedLibrary,
                        RetentionClass::Personal,
                    )
                }
            };
        let mut connection = self.ports.sources.authority.lock().await;
        let transaction = connection
            .transaction()
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        let principal = self
            .ports
            .sources
            .grants
            .resolve(&transaction, &self.settings.owner)
            .await
            .map_err(source_failure)?;
        ImageryAuthorizer::new(ExactAssetAllowlist::default())
            .authorize_owner(
                &principal,
                &self.settings.owner,
                df_imagery::ServiceAction::Read,
            )
            .map_err(|_| ImageFailure::Refused)?;
        let mut key = Sha256::new();
        key.update(b"df-gameplay/reviewed-catalog-reference/1");
        key.update(scope.as_bytes());
        key.update(Sha256::digest(&fresh.bytes));
        let key = MediaRequestKey::from_bytes(key.finalize().into());
        let existing = self
            .ports
            .catalog
            .lookup(scope, &key)
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        let (asset, attempt) = match existing {
            df_assets::CatalogLookup::Ready { asset } => {
                let row = self
                    .ports
                    .catalog
                    .asset(asset)
                    .await
                    .map_err(|_| ImageFailure::Interrupted)?
                    .ok_or(ImageFailure::BadOutput)?;
                (asset, row.attempt)
            }
            df_assets::CatalogLookup::InFlight { attempt } => {
                let row=transaction.query_opt(
                    "SELECT asset_id FROM media.asset WHERE dedupe_scope=$1 AND request_key=$2 AND attempt=$3",
                    &[&scope.as_bytes().as_slice(),&key.as_bytes().as_slice(),&attempt.as_bytes().as_slice()],
                ).await.map_err(|_|ImageFailure::Interrupted)?.ok_or(ImageFailure::BadOutput)?;
                let bytes: Vec<u8> = row.try_get(0).map_err(|_| ImageFailure::BadOutput)?;
                (
                    AssetId::from_bytes(&bytes).map_err(|_| ImageFailure::BadOutput)?,
                    attempt,
                )
            }
            df_assets::CatalogLookup::Archived { .. } | df_assets::CatalogLookup::Suppressed => {
                return Err(ImageFailure::Refused);
            }
            df_assets::CatalogLookup::Absent => {
                let asset = AssetId::from_bytes(&self.ports.ids.next_id_bytes())
                    .map_err(|_| ImageFailure::Interrupted)?;
                let attempt = AttemptId::from_bytes(&self.ports.ids.next_id_bytes())
                    .map_err(|_| ImageFailure::Interrupted)?;
                let admission = Admission {
                    asset,
                    attempt,
                    kind: AssetKind::Image,
                    provenance,
                    charge_scope: charge,
                    origin_account: account,
                    campaign,
                    audience,
                    dedupe_scope: scope,
                    request_key: Some(key),
                    retention,
                    reserve_bytes: fresh.bytes.len() as u64,
                    byte_limit: self.settings.catalog.quota.byte_limit(charge),
                    in_flight_upload_limit: None,
                };
                match self
                    .ports
                    .catalog
                    .admit(&admission, self.ports.clock.now_ms())
                    .await
                    .map_err(|_| ImageFailure::Interrupted)?
                {
                    AdmitOutcome::Admitted => (asset, attempt),
                    // Resolve the same durable request key on a later call; never allocate around it.
                    AdmitOutcome::NotAdmitted(
                        NotAdmitted::Ready { .. } | NotAdmitted::InFlight { .. },
                    ) => {
                        return Err(ImageFailure::Interrupted);
                    }
                    AdmitOutcome::NotAdmitted(_) => return Err(ImageFailure::Refused),
                }
            }
        };
        let row = self
            .ports
            .catalog
            .asset(asset)
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .ok_or(ImageFailure::BadOutput)?;
        if row.state == df_assets::AssetState::Uploading {
            let mut stream = ImportStream(Some(fresh.bytes.to_vec()));
            match self
                .pipeline
                .stage_output(asset, attempt, &mut stream)
                .await
                .map_err(|_| ImageFailure::Interrupted)?
            {
                StagingOutcome::Staged { .. }
                | StagingOutcome::Refused(StagingRefusal::AlreadyStaged) => {}
                _ => return Err(ImageFailure::BadOutput),
            }
        }
        let row = self
            .ports
            .catalog
            .asset(asset)
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .ok_or(ImageFailure::BadOutput)?;
        if row.state != df_assets::AssetState::Ready
            && !matches!(
                self.pipeline
                    .advance(asset)
                    .await
                    .map_err(|_| ImageFailure::Interrupted)?,
                Advance::Ready
                    | Advance::NotWaiting {
                        state: df_assets::AssetState::Ready
                    }
            )
        {
            return Err(ImageFailure::Interrupted);
        }
        let row = self
            .ports
            .catalog
            .asset(asset)
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .ok_or(ImageFailure::BadOutput)?;
        let published = row.published.ok_or(ImageFailure::BadOutput)?;
        let pin = SourceRef {
            asset,
            digest: df_imagery::AssetDigest::from_bytes(published.sha256),
            audience: row.audience,
        };
        transaction
            .commit()
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        drop(connection);
        let source = self
            .ports
            .sources
            .read_source(&self.settings.owner, &pin)
            .await
            .map_err(source_failure)?;
        ReferenceImage::from_catalog(format_of(&source.bytes)?, Bytes::from(source.bytes), pin)
    }
}
impl<S: ObjectStore> TrustedReferenceImport for CatalogReferenceImporter<S> {
    fn import<'a>(&'a self, reference: &'a ReferenceImage) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(self.reviewed(reference))
    }
}
struct ImportStream(Option<Vec<u8>>);
impl StagingStream for ImportStream {
    async fn next_chunk(
        &mut self,
        deadline: df_assets::Deadline,
    ) -> Result<Option<Vec<u8>>, StreamError> {
        if deadline.has_passed() {
            return Err(StreamError::DeadlineExceeded);
        }
        Ok(self.0.take())
    }
}

/// Drains cancelled callers' remaining owned CPU work before a new limit is acknowledged.
pub(super) async fn drain_decoders(
    decode: Arc<tokio::sync::Semaphore>,
    concurrency: u32,
    timeout: std::time::Duration,
) -> Result<tokio::sync::OwnedSemaphorePermit, std::io::Error> {
    tokio::time::timeout(timeout, decode.acquire_many_owned(concurrency))
        .await
        .map_err(|_| std::io::Error::other("image blocking import drain deadline"))?
        .map_err(|_| std::io::Error::other("image import decoder closed"))
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use image::ImageEncoder;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ReleaseDecode(Arc<DecodeProbe>);
    impl Drop for ReleaseDecode {
        fn drop(&mut self) {
            *self.0.released.lock().unwrap() = true;
            self.0.release.notify_one();
        }
    }

    #[tokio::test]
    async fn cancelled_import_keeps_hot_limit_acknowledgement_pending_until_its_decoder_finishes() {
        let old = Arc::new(tokio::sync::Semaphore::new(1));
        let decode = Arc::new(std::sync::Mutex::new(old.clone()));
        let probe = Arc::new(DecodeProbe {
            started: tokio::sync::Notify::new(),
            released: std::sync::Mutex::new(false),
            release: std::sync::Condvar::new(),
        });
        let _release_on_failure = ReleaseDecode(probe.clone());
        let validator = ImportImageValidator {
            decode: decode.clone(),
            deadline: std::time::Duration::from_secs(5),
            maximum_bytes: 1024,
            probe: Some(probe.clone()),
        };
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[10, 20, 30, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let importing =
            tokio::spawn(async move { validator.validate(AssetKind::Image, &png).await });
        probe.started.notified().await;
        importing.abort();
        assert!(importing.await.unwrap_err().is_cancelled());
        assert_eq!(old.available_permits(), 0);
        let acknowledged = Arc::new(AtomicBool::new(false));
        let acknowledged_update = acknowledged.clone();
        let old_update = old.clone();
        let updating = async move {
            let _drained = drain_decoders(old_update, 1, std::time::Duration::from_secs(5))
                .await
                .unwrap();
            *decode.lock().unwrap() = Arc::new(tokio::sync::Semaphore::new(2));
            acknowledged_update.store(true, Ordering::Release);
        };
        tokio::pin!(updating);
        tokio::select! {
            biased;
            _ = &mut updating => panic!("hot limit acknowledged while old decoder still owned a permit"),
            _ = tokio::task::yield_now() => {}
        }
        assert!(!acknowledged.load(Ordering::Acquire));
        *probe.released.lock().unwrap() = true;
        probe.release.notify_one();
        updating.await;
        assert!(acknowledged.load(Ordering::Acquire));
        assert_eq!(old.available_permits(), 1);
    }
}
