//! Consumer pixel tests enter the actual service and native runtime through fake infrastructure.

use super::super::imagery_identity::{AssetReferenceFacts, ImageryAuthorizer, ImageryPrincipal};
use super::{
    ImageFailure, ImageKey, Imagery, ReferenceImage,
    central::{
        CentralImages, CentralSettings, ReferenceAuthority, ReferenceFuture, operation_for_alias,
    },
};
use bytes::Bytes;
use df_assets::Audience;
use df_imagery::{ImageConfig, ImageOwner, ImageServiceApi, OperationHandle, SourceRef};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

type Core = df_imagery::test_support::TransformFixture<ImageryPrincipal>;
#[derive(Clone, Default)]
struct Facts(Arc<Mutex<HashSet<SourceRef>>>);
impl AssetReferenceFacts for Facts {
    fn reference_authorized(&self, source: &SourceRef) -> bool {
        self.0.lock().is_ok_and(|pins| pins.contains(source))
    }
}
struct Authority {
    core: Arc<Core>,
    principal: ImageryPrincipal,
    facts: Facts,
}
impl Authority {
    async fn read(&self, pin: &SourceRef) -> Result<ReferenceImage, ImageFailure> {
        let bytes = self
            .core
            .read_asset(pin.asset)
            .await
            .map_err(|_| ImageFailure::BadOutput)?;
        if <[u8; 32]>::from(Sha256::digest(&bytes)) != *pin.digest.as_bytes() {
            return Err(ImageFailure::BadOutput);
        }
        self.facts
            .0
            .lock()
            .map_err(|_| ImageFailure::Interrupted)?
            .insert(pin.clone());
        let format = super::super::image_references::format_of(&bytes)?;
        ReferenceImage::from_catalog(format, bytes.into(), pin.clone())
    }
}
impl ReferenceAuthority for Authority {
    fn owner(&self) -> &ImageOwner {
        self.core.owner()
    }
    fn reviewed_input_limit(&self) -> u64 {
        16 * 1024 * 1024
    }

    fn canonical_reference<'a>(
        &'a self,
        reference: &'a ReferenceImage,
    ) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async move {
            if let Some(source) = reference.canonical_source() {
                return self.read(source).await;
            }
            // Only this cfg(test) adapter may promote synthetic raw fixture input.
            let pin = self
                .core
                .import_source(reference.bytes.to_vec(), Audience::PublicStatic)
                .await
                .map_err(|_| ImageFailure::BadOutput)?;
            self.read(&pin).await
        })
    }
    fn read_reference<'a>(&'a self, source: &'a SourceRef) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(self.read(source))
    }
    fn restore_reference<'a>(&'a self, key: &'a ImageKey) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async move {
            let view = self
                .core
                .service()
                .read(
                    &self.principal,
                    &OperationHandle {
                        owner: self.core.owner().clone(),
                        operation: operation_for_alias(key)?,
                    },
                )
                .await
                .map_err(|_| ImageFailure::BadOutput)?;
            let result = view.result.ok_or(ImageFailure::Interrupted)?;
            let master = result
                .variants
                .iter()
                .find(|variant| variant.variant == df_assets::AssetVariant::Master)
                .ok_or(ImageFailure::BadOutput)?;
            let audience = match self.owner() {
                ImageOwner::Run { campaign, .. } => Audience::CampaignShared(*campaign),
                ImageOwner::Draft { account } => Audience::OwnerPrivate(*account),
            };
            self.read(&SourceRef {
                asset: result.asset,
                digest: df_imagery::AssetDigest::from_bytes(master.sha256),
                audience,
            })
            .await
        })
    }
    fn read_issued_reference<'a>(
        &'a self,
        _: &'a ImageKey,
        source: &'a SourceRef,
    ) -> ReferenceFuture<'a, ReferenceImage> {
        // Only synthetic cfg(test) pixel sources lack a service operation; the fake
        // infrastructure still reads their exact owned admitted catalog source.
        Box::pin(self.read(source))
    }

    fn published_alias<'a>(
        &'a self,
        _source: &'a SourceRef,
    ) -> ReferenceFuture<'a, Option<ImageKey>> {
        Box::pin(async { Ok(None) })
    }

    fn committed_portrait<'a>(
        &'a self,
        _: &'a df_model::checkpoint::AssetReference,
        _: Arc<dyn super::CommittedPortraitProof>,
    ) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async { Err(ImageFailure::Refused) })
    }

    fn read_annotation<'a>(
        &'a self,
        _: &'a ImageKey,
        _: super::super::image_references::PresentationAnnotation,
    ) -> ReferenceFuture<'a, Option<Vec<u8>>> {
        Box::pin(async { Ok(None) })
    }
    fn write_annotation<'a>(
        &'a self,
        _: &'a ImageKey,
        _: super::super::image_references::PresentationAnnotation,
        _: Vec<u8>,
    ) -> ReferenceFuture<'a, ()> {
        Box::pin(async { Err(ImageFailure::Off) })
    }

    fn principal(&self) -> ReferenceFuture<'_, ImageryPrincipal> {
        Box::pin(async move { Ok(self.principal.clone()) })
    }

    fn authorize_handle<'a>(
        &'a self,
        handle: &'a OperationHandle,
    ) -> ReferenceFuture<'a, ImageryPrincipal> {
        Box::pin(async move {
            if handle.owner != *self.owner() {
                return Err(ImageFailure::Refused);
            }
            Ok(self.principal.clone())
        })
    }

    fn authorize<'a>(
        &'a self,
        request: &'a df_imagery::ImageRequest,
    ) -> ReferenceFuture<'a, ImageryPrincipal> {
        Box::pin(async move {
            if request.identity.owner != *self.owner() {
                return Err(ImageFailure::Refused);
            }
            for pin in request.operation.sources() {
                self.read(pin).await?;
            }
            Ok(self.principal.clone())
        })
    }
}
struct Clock(Arc<Core>);
impl df_imagery::ServiceClock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.now_ms()
    }
}

pub(in crate::gameplay) struct TransformFixture {
    imagery: Option<Imagery>,
    core: Arc<Core>,
}
impl TransformFixture {
    pub(in crate::gameplay) async fn new() -> Self {
        Self::with_observer(Arc::new(FixtureObserver)).await
    }

    pub(in crate::gameplay) async fn with_observer(
        observer: Arc<dyn df_imagery::telemetry::ImageObserver>,
    ) -> Self {
        let principal = ImageryPrincipal::local_run_for_tests([0x31; 16], [0x32; 16], [0x33; 16]);
        let owner = principal
            .local_run_owner()
            .expect("local fixture owner")
            .clone();
        let facts = Facts::default();
        let core = Arc::new(
            Core::new_with_observer(
                owner,
                ImageryAuthorizer::new(facts.clone()),
                observer.clone(),
            )
            .await
            .expect("actual transform service fixture"),
        );
        let mut patch = df_imagery::RuntimePatch::empty();
        patch.mode = Some(df_imagery::GlobalMode::LocalOnly);
        patch.backend_enabled = [Some(false); df_imagery::BackendId::COUNT];
        let control =
            df_imagery::ConfigControl::build("", &[], &patch, 1).expect("local fixture config");
        assert_eq!(control.read_effective().config, core.config());
        let service = core.clone();
        let factory = Arc::new(
            move |config: &ImageConfig,
                  revision: u64|
                  -> Result<
                Arc<dyn ImageServiceApi<Principal = ImageryPrincipal>>,
                ImageFailure,
            > {
                if revision != 1 || *config != service.config() {
                    return Err(ImageFailure::InvalidRequest);
                }
                Ok(service.service())
            },
        );
        let authority = Arc::new(Authority {
            core: core.clone(),
            principal,
            facts,
        });
        let central = CentralImages::new(
            factory,
            authority,
            CentralSettings {
                control: Arc::new(Mutex::new(control)),
                rating: df_imagery::ImageContentRating::Restrained,
                source_revision: 1,
                clock: Arc::new(Clock(core.clone())),
                poll_interval: Duration::from_millis(1),
                observer,
            },
        )
        .expect("mounted central facade");
        Self {
            imagery: Some(Imagery::from_central(
                central,
                super::super::resources::Hub::new(),
            )),
            core,
        }
    }
    pub(in crate::gameplay) fn imagery(&self) -> &Imagery {
        self.imagery.as_ref().expect("open fixture")
    }
    pub(in crate::gameplay) async fn reference(
        &self,
        format: df_providers::ImageFormat,
        bytes: Bytes,
    ) -> ReferenceImage {
        assert_eq!(
            super::super::image_references::format_of(&bytes).expect("fixture format"),
            format
        );
        let pin = self
            .core
            .import_source(bytes.to_vec(), Audience::PublicStatic)
            .await
            .expect("fixture source");
        ReferenceImage::from_catalog(format, bytes, pin).expect("fixture digest")
    }
    pub(in crate::gameplay) async fn keyed_reference(
        &self,
        format: df_providers::ImageFormat,
        bytes: Bytes,
    ) -> (ImageKey, ReferenceImage) {
        let reference = self.reference(format, bytes).await;
        let key = ImageKey::parse(&reference.sha256).expect("fixture digest key");
        self.imagery()
            .inner
            .central
            .as_ref()
            .expect("central fixture")
            .issue_fixture(
                key.clone(),
                reference
                    .canonical_source()
                    .expect("fixture source")
                    .clone(),
            )
            .expect("registered fixture source");
        (key, reference)
    }

    pub(in crate::gameplay) async fn close(mut self) {
        self.imagery.take();
        match Arc::try_unwrap(self.core) {
            Ok(core) => core.shutdown().await.expect("owned transform shutdown"),
            Err(_) => panic!("transform fixture still has an active consumer"),
        }
    }
}

struct FixtureObserver;
impl df_imagery::telemetry::ImageObserver for FixtureObserver {
    fn observe(&self, _: df_imagery::telemetry::ImageEvent) {}
}
