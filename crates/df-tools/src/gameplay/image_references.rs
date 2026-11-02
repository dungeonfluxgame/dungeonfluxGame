//! Host reference authority over current grants, the journal and actual catalog objects.

#[path = "image_annotations.rs"]
mod annotations;
pub(super) use annotations::{AnnotationStore, PresentationAnnotation};

use bytes::Bytes;
use df_assets::{AssetVariant, ObjectStore};
use df_imagery::{
    ImageAuthorizer, ImageJournal, ImageOwner, SourceRef,
    publication::ImageResults,
    runtime::{ImageSources, retained_audience, retained_sources},
};
use std::{cell::RefCell, collections::HashSet, future::Future, sync::Arc};

use super::{
    image_sources::CatalogImageSources,
    imagery::{
        ImageFailure, ImageKey, ReferenceImage,
        central::{
            ReferenceAuthority, ReferenceFuture, alias_for_artifact, alias_for_operation,
            artifact_for_alias, operation_for_alias,
        },
    },
    imagery_identity::{
        AssetReferenceFacts, ExactAssetAllowlist, ImageryAuthorizer, ImageryPrincipal,
    },
};

/// Fresh evidence belongs to one authorized call, never to the lifetime of the host.
#[derive(Clone, Default)]
pub(super) struct VerifiedReferenceFacts {
    _private: (),
}
#[derive(Default)]
struct VerificationFacts {
    sources: HashSet<SourceRef>,
    analyses: HashSet<(Vec<u8>, [u8; 16], SourceRef)>,
}
tokio::task_local! {
    static VERIFICATION: RefCell<VerificationFacts>;
}
/// Synchronous authorizers run in the same future as the fresh source checks.
pub(super) async fn verify_scope<F: Future>(future: F) -> F::Output {
    if VERIFICATION.try_with(|_| ()).is_ok() {
        future.await
    } else {
        VERIFICATION
            .scope(RefCell::new(VerificationFacts::default()), future)
            .await
    }
}
impl AssetReferenceFacts for VerifiedReferenceFacts {
    fn reference_authorized(&self, source: &SourceRef) -> bool {
        VERIFICATION
            .try_with(|facts| facts.borrow().sources.contains(source))
            .unwrap_or(false)
    }
    fn analysis_authorized(
        &self,
        handle: &df_imagery::OperationHandle,
        source: &SourceRef,
    ) -> bool {
        let key = (
            df_imagery::owner_scope_bytes(&handle.owner),
            *handle.operation.as_bytes(),
            source.clone(),
        );
        VERIFICATION
            .try_with(|facts| facts.borrow().analyses.contains(&key))
            .unwrap_or(false)
    }
}
impl VerifiedReferenceFacts {
    fn is_verifying(&self) -> bool {
        VERIFICATION.try_with(|_| ()).is_ok()
    }
    fn retain(&self, source: SourceRef) -> Result<(), ImageFailure> {
        VERIFICATION
            .try_with(|facts| facts.borrow_mut().sources.insert(source))
            .map(|_| ())
            .map_err(|_| ImageFailure::Refused)
    }
    fn retain_analysis(
        &self,
        handle: &df_imagery::OperationHandle,
        source: &SourceRef,
    ) -> Result<(), ImageFailure> {
        let key = (
            df_imagery::owner_scope_bytes(&handle.owner),
            *handle.operation.as_bytes(),
            source.clone(),
        );
        VERIFICATION
            .try_with(|facts| facts.borrow_mut().analyses.insert(key))
            .map(|_| ())
            .map_err(|_| ImageFailure::Refused)
    }
}

pub(super) trait TrustedReferenceImport: Send + Sync {
    fn import<'a>(&'a self, reference: &'a ReferenceImage) -> ReferenceFuture<'a, ReferenceImage>;
}

pub(super) struct CatalogReferenceAuthority<J, S> {
    pub(super) owner: ImageOwner,
    pub(super) sources: Arc<CatalogImageSources<S>>,
    pub(super) journal: J,
    pub(super) results: df_persistence::imagery_publication::PostgresImagePublication,
    pub(super) facts: VerifiedReferenceFacts,
    pub(super) importer: Arc<dyn TrustedReferenceImport>,
    pub(super) reviewed_maximum_bytes: u64,
    pub(super) annotations: Arc<AnnotationStore>,
    pub(super) adventure: Option<super::imagery::AdventureManifest>,
}

impl<J: ImageJournal, S: ObjectStore + 'static> CatalogReferenceAuthority<J, S> {
    async fn read(&self, pin: &SourceRef) -> Result<ReferenceImage, ImageFailure> {
        let source = self.verified_source(pin).await?;
        let format = format_of(&source.bytes)?;
        ReferenceImage::from_catalog(format, Bytes::from(source.bytes), pin.clone())
    }

    /// Re-checks the current grant and catalog for a retained source without building the image
    /// value again; polling a job only needs the authorization evidence.
    async fn verify(&self, pin: &SourceRef) -> Result<(), ImageFailure> {
        let source = self.verified_source(pin).await?;
        format_of(&source.bytes)?;
        Ok(())
    }

    async fn verified_source(
        &self,
        pin: &SourceRef,
    ) -> Result<df_imagery::runtime::AuthorizedImageSource, ImageFailure> {
        let source = self
            .sources
            .read_source(&self.owner, pin)
            .await
            .map_err(source_failure)?;
        // Byte delivery already checked the current grant/catalog. Only an active service
        // authorization call collects evidence for its following synchronous authorizer.
        if self.facts.is_verifying() {
            self.facts.retain(pin.clone())?;
        }
        Ok(source)
    }

    async fn read_dependency(
        &self,
        handle: &df_imagery::OperationHandle,
        analyzed: &SourceRef,
    ) -> Result<(), ImageFailure> {
        if handle.owner != self.owner {
            return Err(ImageFailure::Refused);
        }
        let observed = self
            .sources
            .read_analysis(&self.owner, handle)
            .await
            .map_err(source_failure)?;
        match observed.result {
            df_imagery::ImageAnalysisResult::Segmentation { source, .. } if &source == analyzed => {
                if self.facts.is_verifying() {
                    self.facts.retain_analysis(handle, analyzed)?;
                }
                Ok(())
            }
            df_imagery::ImageAnalysisResult::PhotoInspection { source, .. }
            | df_imagery::ImageAnalysisResult::HeadLocations { source, .. }
            | df_imagery::ImageAnalysisResult::StagingIdentity { source, .. }
            | df_imagery::ImageAnalysisResult::StagingFacing { source, .. }
                if &source == analyzed =>
            {
                Ok(())
            }
            _ => Err(ImageFailure::Refused),
        }
    }

    async fn current_principal(&self) -> Result<ImageryPrincipal, ImageFailure> {
        let mut client = self.sources.authority.lock().await;
        let transaction = client
            .transaction()
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        let principal = self
            .sources
            .grants
            .resolve(&transaction, &self.owner)
            .await
            .map_err(source_failure)?;
        ImageryAuthorizer::new(ExactAssetAllowlist::default())
            .authorize_owner(&principal, &self.owner, df_imagery::ServiceAction::Read)
            .map_err(|_| ImageFailure::Refused)?;
        transaction
            .commit()
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        Ok(principal)
    }
}

impl<J: ImageJournal, S: ObjectStore + 'static> ReferenceAuthority
    for CatalogReferenceAuthority<J, S>
{
    fn owner(&self) -> &ImageOwner {
        &self.owner
    }

    fn reviewed_input_limit(&self) -> u64 {
        self.reviewed_maximum_bytes
    }

    fn canonical_reference<'a>(
        &'a self,
        reference: &'a ReferenceImage,
    ) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async move {
            if let Some(pin) = reference.canonical_source() {
                return self.read(pin).await;
            }
            let reference = self.importer.import(reference).await?;
            let pin = reference
                .canonical_source()
                .ok_or(ImageFailure::InvalidRequest)?;
            self.read(pin).await
        })
    }

    fn read_reference<'a>(&'a self, source: &'a SourceRef) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(self.read(source))
    }

    fn read_ready_artifact<'a>(
        &'a self,
        asset: &'a df_assets::AssetId,
    ) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async move {
            self.current_principal().await?;
            let artifact = self
                .results
                .read_campaign_artifact_with_authority(
                    &self.owner,
                    asset,
                    df_assets::Clock::now_ms(&super::image_host::NativeClock),
                    &self.sources.grants,
                )
                .await
                .map_err(publication_read_failure)?
                .ok_or(ImageFailure::Refused)?;
            let master = artifact
                .variants
                .iter()
                .find(|variant| variant.variant == AssetVariant::Master)
                .ok_or(ImageFailure::BadOutput)?;
            self.read(&SourceRef {
                asset: artifact.asset,
                digest: df_imagery::AssetDigest::from_bytes(master.sha256),
                audience: artifact.audience,
            })
            .await
        })
    }

    fn restore_reference<'a>(&'a self, key: &'a ImageKey) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async move {
            self.current_principal().await?;
            if let Some(asset) = artifact_for_alias(key) {
                return self.read_ready_artifact(&asset).await;
            }
            let operation = match operation_for_alias(key) {
                Ok(operation) => operation,
                Err(_) => {
                    let adventure = self.adventure.clone().ok_or(ImageFailure::InvalidRequest)?;
                    let key = key.clone();
                    let reference = tokio::task::spawn_blocking(move || adventure.load(&key))
                        .await
                        .map_err(|_| ImageFailure::Interrupted)??;
                    return self.importer.import(&reference).await;
                }
            };
            let view = self
                .journal
                .lookup_operation(&self.owner, operation)
                .await
                .map_err(|_| ImageFailure::Interrupted)?
                .ok_or(ImageFailure::InvalidRequest)?;
            self.authorize_handle(&df_imagery::OperationHandle {
                owner: self.owner.clone(),
                operation,
            })
            .await?;
            let job = &view.job;
            for source in retained_sources(job).map_err(source_failure)? {
                self.read(&source).await?;
            }
            if self
                .results
                .read_result(&self.owner, &job.job)
                .await
                .map_err(|_| ImageFailure::Interrupted)?
                .is_none()
            {
                return Err(ImageFailure::Interrupted);
            }
            let result = self
                .results
                .read_reusable_result_with_authority(
                    &self.owner,
                    &job.job,
                    df_assets::Clock::now_ms(&super::image_host::NativeClock),
                    &self.sources.grants,
                )
                .await
                .map_err(publication_read_failure)?
                .ok_or(ImageFailure::Refused)?;
            let master = result
                .variants
                .iter()
                .find(|variant| variant.variant == AssetVariant::Master)
                .ok_or(ImageFailure::BadOutput)?;
            let pin = SourceRef {
                asset: result.asset,
                digest: df_imagery::AssetDigest::from_bytes(master.sha256),
                audience: retained_audience(job).map_err(source_failure)?,
            };
            self.read(&pin).await
        })
    }

    fn published_alias<'a>(
        &'a self,
        source: &'a SourceRef,
    ) -> ReferenceFuture<'a, Option<ImageKey>> {
        Box::pin(async move {
            self.read(source).await?;
            let mut client = self.sources.authority.lock().await;
            let transaction = client
                .transaction()
                .await
                .map_err(|_| ImageFailure::Interrupted)?;
            self.sources
                .grants
                .resolve(&transaction, &self.owner)
                .await
                .map_err(source_failure)?;
            let scope = df_imagery::owner_scope_bytes(&self.owner);
            let row=transaction.query_opt(
                "SELECT o.operation_id FROM imagery.image_operation o \
                 JOIN imagery.image_job_result r ON r.owner_scope=o.owner_scope AND r.job_id=o.job_id \
                 WHERE r.owner_scope=$1 AND r.asset_id=$2 AND NOT r.private_reconciliation \
                 ORDER BY o.operation_id LIMIT 1",
                &[&scope,&source.asset.as_bytes().as_slice()],
            ).await.map_err(|_|ImageFailure::Interrupted)?;
            let operation = row
                .map(|row| {
                    let bytes: Vec<u8> = row.try_get(0).map_err(|_| ImageFailure::BadOutput)?;
                    df_types::OperationId::from_bytes(&bytes).map_err(|_| ImageFailure::BadOutput)
                })
                .transpose()?;
            transaction
                .commit()
                .await
                .map_err(|_| ImageFailure::Interrupted)?;
            if let Some(operation) = operation {
                return Ok(Some(alias_for_operation(operation)));
            }
            let artifact = self
                .results
                .read_campaign_artifact_with_authority(
                    &self.owner,
                    &source.asset,
                    df_assets::Clock::now_ms(&super::image_host::NativeClock),
                    &self.sources.grants,
                )
                .await
                .map_err(publication_read_failure)?;
            match artifact {
                Some(artifact)
                    if artifact.audience == source.audience
                        && artifact.variants.iter().any(|variant| {
                            variant.variant == AssetVariant::Master
                                && variant.sha256 == *source.digest.as_bytes()
                        }) =>
                {
                    Ok(Some(alias_for_artifact(artifact.asset)))
                }
                Some(_) => Err(ImageFailure::Refused),
                None => Ok(None),
            }
        })
    }

    fn committed_portrait<'a>(
        &'a self,
        asset: &'a df_model::checkpoint::AssetReference,
        proof: Arc<dyn super::imagery::CommittedPortraitProof>,
    ) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async move {
            let origin = Arc::new(CommittedPortraitImport {
                owner: self.owner.clone(),
                sources: self.sources.clone(),
                asset: asset.clone(),
                proof,
            });
            let reference =
                super::imagery::ApprovedPortraitSource::read_current(origin.as_ref(), &self.owner)
                    .await?;
            Ok(ReferenceImage::from_trusted(
                reference.format,
                reference.bytes,
                super::imagery::TrustedReferenceOrigin::CommittedPortrait(origin),
            ))
        })
    }

    fn read_annotation<'a>(
        &'a self,
        key: &'a ImageKey,
        kind: PresentationAnnotation,
    ) -> ReferenceFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            let reference = self.restore_reference(key).await?;
            let source = self
                .sources
                .read_source(
                    &self.owner,
                    reference
                        .canonical_source()
                        .ok_or(ImageFailure::BadOutput)?,
                )
                .await
                .map_err(source_failure)?;
            let binding = annotations::source_binding(&source.receipt);
            let store = self.annotations.clone();
            let owner = self.owner.clone();
            let key = key.clone();
            tokio::task::spawn_blocking(move || store.read(&owner, &key, kind, binding))
                .await
                .map_err(|_| ImageFailure::Interrupted)?
                .map_err(|_| ImageFailure::BadOutput)
        })
    }
    fn write_annotation<'a>(
        &'a self,
        key: &'a ImageKey,
        kind: PresentationAnnotation,
        bytes: Vec<u8>,
    ) -> ReferenceFuture<'a, ()> {
        Box::pin(async move {
            let reference = self.restore_reference(key).await?;
            let source = self
                .sources
                .read_source(
                    &self.owner,
                    reference
                        .canonical_source()
                        .ok_or(ImageFailure::BadOutput)?,
                )
                .await
                .map_err(source_failure)?;
            let binding = annotations::source_binding(&source.receipt);
            let store = self.annotations.clone();
            let owner = self.owner.clone();
            let key = key.clone();
            tokio::task::spawn_blocking(move || store.write(&owner, &key, kind, binding, bytes))
                .await
                .map_err(|_| ImageFailure::Interrupted)?
                .map_err(|_| ImageFailure::Provider)
        })
    }

    fn principal(&self) -> ReferenceFuture<'_, ImageryPrincipal> {
        Box::pin(self.current_principal())
    }

    fn authorize_handle<'a>(
        &'a self,
        handle: &'a df_imagery::OperationHandle,
    ) -> ReferenceFuture<'a, ImageryPrincipal> {
        Box::pin(async move {
            if handle.owner != self.owner {
                return Err(ImageFailure::Refused);
            }
            self.current_principal().await?;
            let stored = self
                .journal
                .lookup_operation(&self.owner, handle.operation)
                .await
                .map_err(|_| ImageFailure::Interrupted)?
                .ok_or(ImageFailure::Refused)?;
            for pin in retained_sources(&stored.job).map_err(source_failure)? {
                self.verify(&pin).await?;
            }
            for (handle, source) in df_imagery::runtime::retained_analysis_dependencies(&stored.job)
                .map_err(source_failure)?
            {
                self.read_dependency(&handle, &source).await?;
            }
            self.current_principal().await
        })
    }

    fn authorize<'a>(
        &'a self,
        request: &'a df_imagery::ImageRequest,
    ) -> ReferenceFuture<'a, ImageryPrincipal> {
        Box::pin(verify_scope(async move {
            if request.identity.owner != self.owner {
                return Err(ImageFailure::Refused);
            }
            for source in request.operation.sources() {
                self.read(source).await?;
            }
            if let df_imagery::ImageOperation::Transform(df_imagery::TransformSpec {
                source:
                    df_imagery::TransformSource::SegmentationMask {
                        analyzed,
                        segmentation,
                        ..
                    },
                ..
            }) = &request.operation
            {
                self.read_dependency(segmentation, analyzed).await?;
            }
            let principal = self.current_principal().await?;
            ImageryAuthorizer::new(self.facts.clone())
                .authorize_request(&principal, request)
                .map_err(|_| ImageFailure::Refused)?;
            Ok(principal)
        }))
    }
}

fn publication_read_failure(error: df_imagery::publication::PublicationError) -> ImageFailure {
    use df_imagery::publication::PublicationError;
    match error {
        PublicationError::SourceDenied(_)
        | PublicationError::NotFound
        | PublicationError::BindingConflict => ImageFailure::Refused,
        PublicationError::InvalidRecord
        | PublicationError::InvalidRecipe
        | PublicationError::Proof(_) => ImageFailure::BadOutput,
        PublicationError::Unavailable
        | PublicationError::Unknown
        | PublicationError::StaleClaim
        | PublicationError::CheckpointMismatch
        | PublicationError::Catalog(_)
        | PublicationError::Execution(_) => ImageFailure::Interrupted,
    }
}

pub(super) fn source_failure(error: df_imagery::runtime::RuntimeError) -> ImageFailure {
    match error {
        df_imagery::runtime::RuntimeError::SourceDenied => ImageFailure::Refused,
        df_imagery::runtime::RuntimeError::InvalidRecord => ImageFailure::BadOutput,
        _ => ImageFailure::Interrupted,
    }
}

pub(super) fn format_of(bytes: &[u8]) -> Result<df_providers::ImageFormat, ImageFailure> {
    match image::guess_format(bytes).map_err(|_| ImageFailure::BadOutput)? {
        image::ImageFormat::Png => Ok(df_providers::ImageFormat::Png),
        image::ImageFormat::Jpeg => Ok(df_providers::ImageFormat::Jpeg),
        image::ImageFormat::WebP => Ok(df_providers::ImageFormat::Webp),
        _ => Err(ImageFailure::BadOutput),
    }
}

struct CommittedPortraitImport<S> {
    owner: ImageOwner,
    sources: Arc<CatalogImageSources<S>>,
    asset: df_model::checkpoint::AssetReference,
    proof: Arc<dyn super::imagery::CommittedPortraitProof>,
}
impl<S: ObjectStore> super::imagery::ApprovedPortraitSource for CommittedPortraitImport<S> {
    fn read_current<'a>(&'a self, owner: &'a ImageOwner) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async move {
            if owner != &self.owner {
                return Err(ImageFailure::Refused);
            }
            let bytes = self
                .sources
                .read_committed_portrait(owner, &self.asset, self.proof.as_ref())
                .await
                .map_err(source_failure)?;
            Ok(ReferenceImage::new(format_of(&bytes)?, Bytes::from(bytes)))
        })
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    fn source(byte: u8) -> SourceRef {
        SourceRef {
            asset: df_assets::AssetId::from_bytes(&[byte; 16]).unwrap(),
            digest: df_imagery::AssetDigest::from_bytes([byte; 32]),
            audience: df_assets::Audience::PublicStatic,
        }
    }
    #[tokio::test]
    async fn simultaneous_authorization_calls_never_share_verified_evidence() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let first = source(1);
        let second = source(2);
        let check = |own: SourceRef, foreign: SourceRef| {
            let barrier = barrier.clone();
            verify_scope(async move {
                let facts = VerifiedReferenceFacts::default();
                facts.retain(own.clone()).unwrap();
                barrier.wait().await;
                assert!(facts.reference_authorized(&own));
                assert!(!facts.reference_authorized(&foreign));
                assert!(
                    !tokio::task::spawn_blocking(
                        move || VerifiedReferenceFacts::default().reference_authorized(&own)
                    )
                    .await
                    .unwrap()
                );
            })
        };
        tokio::join!(
            check(first.clone(), second.clone()),
            check(second.clone(), first.clone())
        );
        assert!(!VerifiedReferenceFacts::default().reference_authorized(&first));
        assert!(VerifiedReferenceFacts::default().retain(first).is_err());
    }
    #[tokio::test]
    async fn a_later_call_requires_new_evidence_after_an_earlier_authorization() {
        let pin = source(3);
        let handle = df_imagery::OperationHandle {
            owner: ImageOwner::Draft {
                account: df_types::AccountId::from_bytes(&[4; 16]).unwrap(),
            },
            operation: df_types::OperationId::from_bytes(&[5; 16]).unwrap(),
        };
        verify_scope(async {
            VerifiedReferenceFacts::default()
                .retain(pin.clone())
                .unwrap();
            VerifiedReferenceFacts::default()
                .retain_analysis(&handle, &pin)
                .unwrap();
            assert!(VerifiedReferenceFacts::default().analysis_authorized(&handle, &pin));
        })
        .await;
        verify_scope(async {
            // No current catalog/analysis proof was supplied in this call.
            assert!(!VerifiedReferenceFacts::default().reference_authorized(&pin));
            assert!(!VerifiedReferenceFacts::default().analysis_authorized(&handle, &pin));
        })
        .await;
    }
}

#[cfg(test)]
mod alias_read_tests {
    use image::ImageEncoder;

    #[tokio::test]
    async fn generated_read_bound_survives_a_smaller_output_setting_and_native_restart() {
        use df_imagery::{
            AlphaHandling, BlendMode, CanvasSpec, CompositeLayer, Dimensions, Encoding,
            ImageTransform, Purpose, PurposedUse, RasterOutputSpec, Rgb, TransformSource,
            TransformSpec, VariantRole,
        };
        let costs = super::super::cost::port::MeteringPort::new();
        costs
            .open(
                "source-bound-test",
                include_str!("../../../../content/pricing.json"),
            )
            .unwrap();
        let mut patch = df_imagery::RuntimePatch::empty();
        patch.mode = Some(df_imagery::GlobalMode::LocalOnly);
        patch.backend_enabled = [Some(false); df_imagery::BackendId::COUNT];
        patch.transforms_max_output_bytes = Some(32 * 1024 * 1024);
        patch.transforms_max_decoded_bytes = Some(512 * 1024 * 1024);
        let Some(mut fixture) = super::super::image_test_support::ImageHostFixture::open(
            "127.0.0.1:1".parse().unwrap(),
            patch,
            costs,
        )
        .await
        else {
            return;
        };
        let mut state = 0x7823_b195_u32;
        let mut pixels = vec![0; 2048 * 2048 * 3];
        for byte in &mut pixels {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(&pixels, 2048, 2048, image::ExtendedColorType::Rgb8)
            .unwrap();
        assert!(encoded.len() < 16 * 1024 * 1024);
        let (_, input) = fixture
            .import_bytes(df_providers::ImageFormat::Png, encoded.into())
            .await
            .unwrap();
        let pin = fixture.imagery().pin_reference(&input).await.unwrap();
        let result = fixture
            .imagery()
            .transform(
                "large-retained-source",
                PurposedUse {
                    purpose: Purpose::ImportedDerivative,
                    role: VariantRole::Primary,
                },
                TransformSpec {
                    source: TransformSource::Canvas(
                        CanvasSpec::new(
                            2048,
                            4096,
                            Rgb {
                                red: 0,
                                green: 0,
                                blue: 0,
                            },
                        )
                        .unwrap(),
                    ),
                    steps: vec![ImageTransform::Composite(vec![
                        CompositeLayer::new(pin.clone(), 0, 0, 1000, BlendMode::SourceOver)
                            .unwrap(),
                        CompositeLayer::new(pin, 0, 2048, 1000, BlendMode::SourceOver).unwrap(),
                    ])],
                },
                RasterOutputSpec::new(
                    Dimensions::new(2048, 4096).unwrap(),
                    AlphaHandling::Opaque,
                    Encoding::Png,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(result.bytes.len() > 16 * 1024 * 1024);
        let digest = result.sha256.clone();
        let key = fixture
            .imagery()
            .register_reviewed("large-retained-alias", result)
            .await
            .unwrap();
        {
            let mut control = fixture.control().lock().unwrap();
            let revision = control.revision();
            let mut patch = df_imagery::RuntimePatch::empty();
            patch.transforms_max_output_bytes = Some(6 * 1024 * 1024);
            control.apply(patch, revision).unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if fixture
                    .imagery()
                    .consumer_context()
                    .await
                    .is_ok_and(|context| {
                        context.config.transforms.max_output_bytes == 6 * 1024 * 1024
                    })
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            fixture.imagery().reference(&key).await.unwrap().sha256,
            digest
        );
        fixture.restart().await;
        assert_eq!(
            fixture.imagery().reference(&key).await.unwrap().sha256,
            digest
        );
        fixture.close().await;
    }

    #[tokio::test]
    async fn issued_and_restarted_alias_reads_reject_changed_consumed_source_receipts() {
        let costs = super::super::cost::port::MeteringPort::new();
        costs
            .open(
                "alias-read-test",
                include_str!("../../../../content/pricing.json"),
            )
            .unwrap();
        let mut patch = df_imagery::RuntimePatch::empty();
        patch.mode = Some(df_imagery::GlobalMode::LocalOnly);
        patch.backend_enabled = [Some(false); df_imagery::BackendId::COUNT];
        let Some(mut fixture) = super::super::image_test_support::ImageHostFixture::open(
            "127.0.0.1:1".parse().unwrap(),
            patch,
            costs,
        )
        .await
        else {
            return;
        };
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &[10, 20, 30, 255, 40, 50, 60, 255],
                2,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let (_, base) = fixture
            .import_bytes(df_providers::ImageFormat::Png, png.into())
            .await
            .unwrap();
        let pin = fixture.imagery().pin_reference(&base).await.unwrap();
        let derivative = fixture
            .imagery()
            .transform(
                "alias-receipt-derivative",
                df_imagery::PurposedUse {
                    purpose: df_imagery::Purpose::ImportedDerivative,
                    role: df_imagery::VariantRole::Primary,
                },
                df_imagery::TransformSpec {
                    source: df_imagery::TransformSource::Image(pin.clone()),
                    steps: vec![df_imagery::ImageTransform::Flip(
                        df_imagery::FlipAxis::Horizontal,
                    )],
                },
                df_imagery::RasterOutputSpec::new(
                    df_imagery::Dimensions::new(2, 1).unwrap(),
                    df_imagery::AlphaHandling::PreserveAlpha,
                    df_imagery::Encoding::Png,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let key = fixture
            .imagery()
            .register_reviewed("alias-receipt-published", derivative)
            .await
            .unwrap();
        assert!(matches!(
            fixture.imagery().image(key.as_str()).await,
            super::super::imagery::Lookup::Image { .. }
        ));
        let (client, driver) = fixture
            .database()
            .connect(tokio_postgres::NoTls)
            .await
            .unwrap();
        let driver = tokio::spawn(driver);
        assert_eq!(client.execute(
            "UPDATE media.asset SET suppression_generation=suppression_generation+1 WHERE asset_id=$1",
            &[&pin.asset.as_bytes().as_slice()],
        ).await.unwrap(), 1);
        // Output bytes still exist; only the exact consumed source receipt has changed.
        assert!(matches!(
            fixture.imagery().image(key.as_str()).await,
            super::super::imagery::Lookup::Unregistered
        ));
        drop(client);
        driver.abort();
        let _closed = driver.await;
        fixture.restart().await;
        assert!(matches!(
            fixture.imagery().image(key.as_str()).await,
            super::super::imagery::Lookup::Unregistered
        ));
        fixture.close().await;
    }
}
