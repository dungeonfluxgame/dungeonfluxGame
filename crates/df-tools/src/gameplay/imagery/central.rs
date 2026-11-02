//! Consumer calls into the mounted durable image service; no supplier or local pixel execution.

use df_imagery::{
    AssetDigest, Audience, ExecutionMode, Freshness, ImageConfig, ImageDemandView,
    ImageExecutionState, ImageOperation, ImageOwner, ImageServiceApi, ImageServiceError,
    OperationIdentity, OutputProfile, PurposedUse, RasterOutputSpec, RequestOutcome, SourceRef,
    TextPrompt, TransformSpec, Urgency, WatchObservation,
};
use df_types::OperationId;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use super::super::imagery_identity::ImageryPrincipal;
use super::service_request::{PipelinePolicy, RequestScope};
use super::{ImageFailure, ImageKey, ImagePriority, ImageRequest, ReferenceImage};

pub(in crate::gameplay) type ReferenceFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ImageFailure>> + Send + 'a>>;

/// Implemented by the host's actual grant and catalog adapter. Raw image bytes are never authority.
pub(in crate::gameplay) trait ReferenceAuthority: Send + Sync {
    fn owner(&self) -> &ImageOwner;
    fn reviewed_input_limit(&self) -> u64;
    fn read_annotation<'a>(
        &'a self,
        key: &'a ImageKey,
        kind: super::super::image_references::PresentationAnnotation,
    ) -> ReferenceFuture<'a, Option<Vec<u8>>>;
    fn write_annotation<'a>(
        &'a self,
        key: &'a ImageKey,
        kind: super::super::image_references::PresentationAnnotation,
        bytes: Vec<u8>,
    ) -> ReferenceFuture<'a, ()>;
    fn canonical_reference<'a>(
        &'a self,
        reference: &'a ReferenceImage,
    ) -> ReferenceFuture<'a, ReferenceImage>;
    fn read_reference<'a>(&'a self, source: &'a SourceRef) -> ReferenceFuture<'a, ReferenceImage>;
    /// Reads an actual current-campaign artifact without impersonating its producer operation.
    fn read_ready_artifact<'a>(
        &'a self,
        _asset: &'a df_assets::AssetId,
    ) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async { Err(ImageFailure::Refused) })
    }
    /// Resolves the owner-scoped stable operation and published result after process restart.
    fn restore_reference<'a>(&'a self, key: &'a ImageKey) -> ReferenceFuture<'a, ReferenceImage>;
    /// Issued aliases are only index hints; production still validates the actual publication.
    fn read_issued_reference<'a>(
        &'a self,
        key: &'a ImageKey,
        expected: &'a SourceRef,
    ) -> ReferenceFuture<'a, ReferenceImage> {
        Box::pin(async move {
            let reference = self.restore_reference(key).await?;
            if reference.canonical_source() != Some(expected) {
                return Err(ImageFailure::Refused);
            }
            Ok(reference)
        })
    }
    /// Finds an existing owner-authorized published operation for a canonical source.
    fn published_alias<'a>(
        &'a self,
        source: &'a SourceRef,
    ) -> ReferenceFuture<'a, Option<ImageKey>>;
    fn authorize_handle<'a>(
        &'a self,
        handle: &'a df_imagery::OperationHandle,
    ) -> ReferenceFuture<'a, ImageryPrincipal>;
    fn committed_portrait<'a>(
        &'a self,
        asset: &'a df_model::checkpoint::AssetReference,
        proof: Arc<dyn super::CommittedPortraitProof>,
    ) -> ReferenceFuture<'a, ReferenceImage>;
    fn principal(&self) -> ReferenceFuture<'_, ImageryPrincipal>;
    /// Resolves fresh current authority and verifies every actual catalog source claim.
    fn authorize<'a>(
        &'a self,
        request: &'a df_imagery::ImageRequest,
    ) -> ReferenceFuture<'a, ImageryPrincipal>;
}

pub(in crate::gameplay) struct ConsumerContext {
    pub service: Arc<dyn ImageServiceApi<Principal = ImageryPrincipal>>,
    pub principal: ImageryPrincipal,
    pub owner: ImageOwner,
    pub config: ImageConfig,
    pub expected_source_revision: u64,
    pub now_ms: i64,
    pub poll_interval: Duration,
}

pub(in crate::gameplay) trait CreatorContexts: Send + Sync {
    fn for_local_creator(&self, credential: [u8; 32]) -> ReferenceFuture<'_, CentralImages>;
}

pub(in crate::gameplay) struct CentralSettings {
    pub control: Arc<Mutex<df_imagery::ConfigControl>>,
    pub rating: df_imagery::ImageContentRating,
    pub source_revision: u64,
    pub clock: Arc<dyn df_imagery::ServiceClock>,
    pub poll_interval: Duration,
    pub observer: Arc<dyn df_imagery::telemetry::ImageObserver>,
}

pub(in crate::gameplay) type ServiceFactory = dyn Fn(
        &ImageConfig,
        u64,
    ) -> Result<Arc<dyn ImageServiceApi<Principal = ImageryPrincipal>>, ImageFailure>
    + Send
    + Sync;

struct DemandConfiguration {
    config: ImageConfig,
    revision: u64,
    service: Arc<dyn ImageServiceApi<Principal = ImageryPrincipal>>,
}

pub(in crate::gameplay) struct CentralImages {
    services: Arc<ServiceFactory>,
    creators: Option<Arc<dyn CreatorContexts>>,
    references: Arc<dyn ReferenceAuthority>,
    settings: CentralSettings,
    /// Presentation aliases only; durable demand, pixels and cost remain in their owners.
    issued: Mutex<IssuedAliases>,
}

#[derive(Default)]
struct IssuedAliases {
    sources: HashMap<String, SourceRef>,
    order: VecDeque<String>,
}
impl IssuedAliases {
    fn insert(&mut self, key: String, source: SourceRef, maximum: usize) {
        if !self.sources.contains_key(&key) {
            if self.sources.len() >= maximum
                && let Some(oldest) = self.order.pop_front()
            {
                self.sources.remove(&oldest);
            }
            self.order.push_back(key.clone());
        }
        self.sources.insert(key, source);
    }
}

impl CentralImages {
    pub(in crate::gameplay) fn new(
        services: Arc<ServiceFactory>,
        references: Arc<dyn ReferenceAuthority>,
        settings: CentralSettings,
    ) -> Result<Self, ImageFailure> {
        let effective = super::locked(&settings.control).read_effective();
        effective
            .config
            .validate()
            .map_err(|_| ImageFailure::InvalidRequest)?;
        if settings.poll_interval.is_zero() || effective.provenance.revision == 0 {
            return Err(ImageFailure::InvalidRequest);
        }
        Ok(Self {
            services,
            creators: None,
            references,
            settings,
            issued: Mutex::new(IssuedAliases::default()),
        })
    }

    pub(in crate::gameplay) fn with_creator_contexts(
        mut self,
        contexts: Arc<dyn CreatorContexts>,
    ) -> Self {
        self.creators = Some(contexts);
        self
    }

    pub(super) async fn for_local_creator(
        &self,
        credential: [u8; 32],
    ) -> Result<CentralImages, ImageFailure> {
        self.creators
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .for_local_creator(credential)
            .await
    }

    pub(super) fn model_label(&self) -> Result<String, ImageFailure> {
        let demand = self.capture()?;
        let purpose = demand.config.purpose(df_imagery::Purpose::PartyMural);
        let backend = purpose
            .backend_override
            .unwrap_or(demand.config.default_backend);
        let gate = demand.config.backend(backend);
        if backend == df_imagery::BackendId::CodexCli {
            return Ok("codex-image-profile".to_owned());
        }
        Ok(purpose
            .model_override
            .as_ref()
            .or(gate.edit_model.as_ref())
            .or(gate.model.as_ref())
            .map(|model| model.as_str().to_owned())
            .unwrap_or_else(|| match backend {
                df_imagery::BackendId::OpenAi => "gpt-image-2.5-flare".to_owned(),
                df_imagery::BackendId::Fal => "fal-ai/flux-pro/kontext/max/multi".to_owned(),
                df_imagery::BackendId::Bfl => "flux-3-image".to_owned(),
                df_imagery::BackendId::CodexCli => "codex-image-profile".to_owned(),
            }))
    }

    pub(super) fn flux_prompts(&self) -> bool {
        let config = super::locked(&self.settings.control)
            .read_effective()
            .config;
        config
            .purpose(df_imagery::Purpose::ScenePlate)
            .backend_override
            .unwrap_or(config.default_backend)
            == df_imagery::BackendId::Fal
    }

    fn capture(&self) -> Result<DemandConfiguration, ImageFailure> {
        let snapshot = super::super::image_configuration::snapshot(&self.settings.control)
            .map_err(|_| ImageFailure::Interrupted)?;
        let revision = snapshot.provenance.revision;
        let service = Arc::new(super::authorized_service::AuthorizedConsumerService {
            inner: (self.services)(&snapshot.config, revision)?,
            references: self.references.clone(),
        });
        Ok(DemandConfiguration {
            config: snapshot.config,
            revision,
            service,
        })
    }

    pub(super) fn key_for(&self, request: &ImageRequest) -> ImageKey {
        let snapshot = super::super::image_configuration::snapshot(&self.settings.control);
        match snapshot.and_then(|snapshot| {
            self.key_at(request, snapshot.provenance.revision, &snapshot.config)
                .map_err(|_| std::io::Error::other("image key configuration refused"))
        }) {
            Ok(key) => key,
            Err(_) => {
                self.settings.observer.observe(
                    df_imagery::telemetry::ImageEvent::new(
                        df_imagery::telemetry::EventKind::Failed,
                        df_imagery::telemetry::EventContext::default(),
                    )
                    .failed(df_imagery::telemetry::FailureKind::Validation),
                );
                // A presentation placeholder carries no authority; submission still refuses.
                alias_bytes(&Sha256::digest(b"df-image/unavailable-configuration-key/1")[..16])
            }
        }
    }

    fn key_at(
        &self,
        request: &ImageRequest,
        revision: u64,
        config: &ImageConfig,
    ) -> Result<ImageKey, ImageFailure> {
        let namespace = format!("central-image-service/1/{revision}");
        let rating = match self.settings.rating {
            df_imagery::ImageContentRating::Restrained => "restrained",
            df_imagery::ImageContentRating::Mature => "mature",
        };
        let raw = super::request_key(request, &namespace, "frozen-profile", rating);
        let mut digest = Sha256::new();
        digest.update(raw.as_str().as_bytes());
        if let Some(artwork) = request.prompt.artwork() {
            digest.update(b"df-gameplay/public-artwork-selection/1");
            digest.update(artwork.spec.binding_digest());
            digest.update(artwork.selection.run.as_bytes());
            digest.update(artwork.selection.binding);
        }
        digest.update(
            config
                .canonical_fingerprint()
                .map_err(|_| ImageFailure::InvalidRequest)?,
        );
        Ok(alias_bytes(&digest.finalize()[..16]))
    }

    pub(super) async fn register_reference(
        &self,
        reference: &ReferenceImage,
    ) -> Result<ImageKey, ImageFailure> {
        let demand = self.capture()?;
        let canonical = self.references.canonical_reference(reference).await?;
        if let Some(source) = canonical.canonical_source()
            && let Some(key) = self.references.published_alias(source).await?
        {
            return self.remember_alias(key, source.clone());
        }
        let source = canonical
            .canonical_source()
            .cloned()
            .ok_or(ImageFailure::InvalidRequest)?;
        // Header facts use the same JPEG orientation policy as central pixel execution.
        let dimensions = df_imagery::source_header(&canonical.bytes)
            .map_err(|_| ImageFailure::BadOutput)?
            .dimensions;
        let (alpha, encoding) = match canonical.format {
            df_providers::ImageFormat::Png => (
                df_imagery::AlphaHandling::PreserveAlpha,
                df_imagery::Encoding::Png,
            ),
            df_providers::ImageFormat::Jpeg => (
                df_imagery::AlphaHandling::Opaque,
                df_imagery::Encoding::Jpeg {
                    quality: demand.config.quality.jpeg_quality,
                },
            ),
            df_providers::ImageFormat::Webp => (
                df_imagery::AlphaHandling::PreserveAlpha,
                df_imagery::Encoding::WebP { quality: None },
            ),
        };
        let output = RasterOutputSpec::new(dimensions, alpha, encoding)
            .map_err(|_| ImageFailure::BadOutput)?;
        let mut request = df_imagery::ImageRequest {
            identity: OperationIdentity {
                owner: self.references.owner().clone(),
                operation: operation(source.asset.as_bytes())?,
                fingerprint: [0; 32],
            },
            purpose: PurposedUse {
                purpose: df_imagery::Purpose::ReferencePrep,
                role: df_imagery::VariantRole::Reference,
            },
            operation: ImageOperation::Transform(TransformSpec {
                source: df_imagery::TransformSource::Image(source),
                steps: Vec::new(),
            }),
            execution: ExecutionMode::Live,
            freshness: Freshness::ReuseCompatible,
            urgency: Urgency::Present,
            audience: owner_audience(self.references.owner()),
            expected_source_revision: self.settings.source_revision,
            output: OutputProfile::local(output),
        };
        self.bind_operation(&demand, "reviewed-reference-delivery", &mut request)?;
        let key = alias_bytes(&request.identity.fingerprint[..16]);
        request.identity.operation = operation_for_alias(&key)?;
        self.execute(&demand, &request)
            .await
            .map(|(_, alias)| alias)
    }

    pub(super) async fn pin_reference(
        &self,
        reference: &ReferenceImage,
    ) -> Result<SourceRef, ImageFailure> {
        self.references
            .canonical_reference(reference)
            .await?
            .canonical_source()
            .cloned()
            .ok_or(ImageFailure::InvalidRequest)
    }

    pub(super) async fn reference(&self, key: &ImageKey) -> Result<ReferenceImage, ImageFailure> {
        let source = super::locked(&self.issued)
            .sources
            .get(key.as_str())
            .cloned();
        if let Some(source) = source {
            return self.references.read_issued_reference(key, &source).await;
        }
        let reference = self.references.restore_reference(key).await?;
        self.remember_alias(
            key.clone(),
            reference
                .canonical_source()
                .cloned()
                .ok_or(ImageFailure::BadOutput)?,
        )?;
        Ok(reference)
    }

    /// Used only by the actual byte-serving facade after current alias authorization.
    pub(super) async fn served_reference(
        &self,
        key: &ImageKey,
    ) -> Result<ReferenceImage, ImageFailure> {
        let reference = self.reference(key).await?;
        if let Some(source) = reference.canonical_source() {
            super::super::image_observability::delivery_authorized(
                self.settings.observer.as_ref(),
                df_imagery::telemetry::EventContext {
                    operation: operation_for_alias(key).ok(),
                    asset: Some(source.asset),
                    ..Default::default()
                },
                super::super::image_host::delivery_id(),
            );
        }
        Ok(reference)
    }

    pub(super) fn is_on(&self) -> bool {
        self.capture().is_ok_and(|captured| {
            matches!(
                captured.config.mode,
                df_imagery::GlobalMode::Live | df_imagery::GlobalMode::CacheOnly
            )
        })
    }

    pub(super) fn observer(&self) -> Arc<dyn df_imagery::telemetry::ImageObserver> {
        self.settings.observer.clone()
    }

    pub(super) async fn displayed_observation(
        &self,
        identity: u64,
    ) -> Result<Option<super::super::image_observability::PublishedObservation>, ImageFailure> {
        let matching = {
            let aliases = super::locked(&self.issued);
            let aliases = &aliases.sources;
            let mut matching = aliases.keys().filter(|key| {
                df_observe::presentation::local_identity(&format!("/gameplay/imagery/{key}"))
                    == identity
            });
            let first = matching.next().cloned();
            if matching.next().is_some() {
                return Err(ImageFailure::InvalidRequest);
            }
            first
        };
        let Some(key) = matching.and_then(|key| ImageKey::parse(&key)) else {
            return Ok(None);
        };
        let reference = self.reference(&key).await?;
        if artifact_for_alias(&key).is_some() {
            // Shared artifact delivery creates no current-owner supplier usage.
            return Ok(None);
        }
        let handle = df_imagery::OperationHandle {
            owner: self.references.owner().clone(),
            operation: operation_for_alias(&key)?,
        };
        let principal = self.references.authorize_handle(&handle).await?;
        let demand = self.capture()?;
        let view = demand
            .service
            .read(&principal, &handle)
            .await
            .map_err(failure)?;
        let Some(result) = &view.result else {
            return Ok(None);
        };
        if reference.canonical_source().map(|pin| pin.asset) != Some(result.asset) {
            return Err(ImageFailure::BadOutput);
        }
        Ok(Some(
            super::super::image_observability::PublishedObservation {
                operation: handle.operation,
                job: view.job,
                asset: result.asset,
                config_revision: None,
            },
        ))
    }

    pub(super) async fn committed_portrait(
        &self,
        asset: &df_model::checkpoint::AssetReference,
        proof: Arc<dyn super::CommittedPortraitProof>,
    ) -> Result<ReferenceImage, ImageFailure> {
        self.references.committed_portrait(asset, proof).await
    }

    pub(super) fn reviewed_input_limit(&self) -> u64 {
        self.references.reviewed_input_limit()
    }

    pub(super) async fn read_annotation(
        &self,
        key: &ImageKey,
        kind: super::super::image_references::PresentationAnnotation,
    ) -> Result<Option<Vec<u8>>, ImageFailure> {
        self.references.read_annotation(key, kind).await
    }
    pub(super) async fn write_annotation(
        &self,
        key: &ImageKey,
        kind: super::super::image_references::PresentationAnnotation,
        bytes: Vec<u8>,
    ) -> Result<(), ImageFailure> {
        self.references.write_annotation(key, kind, bytes).await
    }

    pub(super) async fn consumer_context(&self) -> Result<ConsumerContext, ImageFailure> {
        let demand = self.capture()?;
        let principal = self.references.principal().await?;
        Ok(ConsumerContext {
            service: demand.service,
            principal,
            owner: self.references.owner().clone(),
            config: demand.config,
            expected_source_revision: self.settings.source_revision,
            now_ms: self.settings.clock.now_ms(),
            poll_interval: self.settings.poll_interval,
        })
    }

    pub(super) async fn published_reference(
        &self,
        result: &df_imagery::publication::PublishedImage,
        audience: Audience,
    ) -> Result<ReferenceImage, ImageFailure> {
        let master = result
            .variants
            .iter()
            .find(|variant| variant.variant == df_assets::AssetVariant::Master)
            .ok_or(ImageFailure::BadOutput)?;
        self.references
            .read_reference(&SourceRef {
                asset: result.asset,
                digest: AssetDigest::from_bytes(master.sha256),
                audience,
            })
            .await
    }

    async fn published_view(
        &self,
        key: &ImageKey,
    ) -> Result<Option<ImageDemandView>, ImageFailure> {
        let reference = self.reference(key).await?;
        let pin = reference
            .canonical_source()
            .ok_or(ImageFailure::BadOutput)?;
        let Some(alias) = self.references.published_alias(pin).await? else {
            return Ok(None);
        };
        let handle = df_imagery::OperationHandle {
            owner: self.references.owner().clone(),
            operation: operation_for_alias(&alias)?,
        };
        let principal = self.references.authorize_handle(&handle).await?;
        let demand = self.capture()?;
        let view = demand
            .service
            .read(&principal, &handle)
            .await
            .map_err(failure)?;
        if view
            .result
            .as_ref()
            .is_none_or(|result| result.asset != pin.asset)
        {
            return Err(ImageFailure::Refused);
        }
        Ok(Some(view))
    }

    pub(super) async fn plate_staging(
        &self,
        key: &ImageKey,
    ) -> Result<Vec<super::StagedPerson>, ImageFailure> {
        let Some(view) = self.published_view(key).await? else {
            return Ok(Vec::new());
        };
        Ok(view
            .staging
            .into_iter()
            .map(|person| super::StagedPerson {
                locator_id: person.locator_id.text().to_owned(),
                reference_id: person.reference_id.text().to_owned(),
                name: person.public_name.text().to_owned(),
                hero: person.hero,
            })
            .collect())
    }

    pub(super) async fn photo_evidence(
        &self,
        key: &ImageKey,
    ) -> Result<Option<super::PhotoEvidence>, ImageFailure> {
        let Some(view) = self.published_view(key).await? else {
            return Ok(None);
        };
        let Some(inspection) = view.inspection else {
            return Ok(None);
        };
        if inspection.verdict != df_imagery::PhotoInspectionVerdict::Pass {
            return Ok(None);
        }
        let Some(binding) = inspection.source_binding else {
            return Ok(None);
        };
        let features = inspection
            .visible_features
            .into_iter()
            .map(
                |feature| super::super::journey::living_scene::VisibleFeature {
                    name: feature.name.text().to_owned(),
                    look: feature.look.text().to_owned(),
                    region: feature.region,
                    recorded_id: feature.recorded_id.map(|id| id.text().to_owned()),
                },
            )
            .collect::<Vec<_>>();
        if features.iter().any(|feature| !feature.valid()) {
            return Err(ImageFailure::BadOutput);
        }
        Ok(Some(super::PhotoEvidence {
            place: binding.place.text().to_owned(),
            source_hash: binding.source_hash.text().to_owned(),
            visible_features: features,
        }))
    }

    pub(super) async fn usage_for_reference(
        &self,
        reference: &ReferenceImage,
    ) -> Result<Option<df_imagery::runtime::ImageUsageSummary>, ImageFailure> {
        let source = reference
            .canonical_source()
            .ok_or(ImageFailure::InvalidRequest)?;
        let Some(key) = self.references.published_alias(source).await? else {
            return Ok(None);
        };
        let handle = df_imagery::OperationHandle {
            owner: self.references.owner().clone(),
            operation: operation_for_alias(&key)?,
        };
        let principal = self.references.authorize_handle(&handle).await?;
        let demand = self.capture()?;
        Ok(demand
            .service
            .read(&principal, &handle)
            .await
            .map_err(failure)?
            .usage)
    }

    pub(super) async fn generate(
        &self,
        subject: &str,
        request: &ImageRequest,
        priority: ImagePriority,
    ) -> Result<ImageKey, ImageFailure> {
        self.generate_for_purpose(
            subject,
            request,
            purpose_for_kind(request.kind),
            priority,
            Freshness::ReuseCompatible,
        )
        .await
    }

    pub(super) async fn generate_for_purpose(
        &self,
        subject: &str,
        request: &ImageRequest,
        purpose: df_imagery::Purpose,
        priority: ImagePriority,
        freshness: Freshness,
    ) -> Result<ImageKey, ImageFailure> {
        let demand = self.capture()?;
        let alias = if purpose == purpose_for_kind(request.kind)
            && freshness == Freshness::ReuseCompatible
        {
            Some(self.key_at(request, demand.revision, &demand.config)?)
        } else {
            None
        };
        let mut canonical = request.clone();
        canonical.references.clear();
        for reference in &request.references {
            canonical
                .references
                .push(self.references.canonical_reference(reference).await?);
        }
        let pins = canonical
            .references
            .iter()
            .map(|reference| {
                reference
                    .canonical_source()
                    .cloned()
                    .ok_or(ImageFailure::InvalidRequest)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let output = demand
            .config
            .output_profile_for(purpose)
            .ok_or(ImageFailure::InvalidRequest)?;
        let delivered = output.delivered();
        canonical.output = super::OutputSpec {
            width: delivered.dimensions().width_px(),
            height: delivered.dimensions().height_px(),
            transparent: delivered.alpha() == df_imagery::AlphaHandling::PreserveAlpha,
        };
        let audience = owner_audience(self.references.owner());
        let mut request = super::service_request::prepare(
            &canonical,
            RequestScope {
                owner: self.references.owner().clone(),
                operation: operation(subject.as_bytes())?,
                source_revision: self.settings.source_revision,
                audience,
                execution: ExecutionMode::Live,
                freshness,
                priority,
            },
            pins,
            output,
            PipelinePolicy {
                rating: self.settings.rating,
                moderation: demand.config.purpose(purpose).moderation_override,
                inspection: demand.config.analysis.inspection.clone(),
                creator: None,
            },
        )?;
        request.purpose.purpose = purpose;
        self.bind_operation(&demand, subject, &mut request)?;
        if let Some(alias) = alias {
            request.identity.operation = operation_for_alias(&alias)?;
        }
        self.execute(&demand, &request)
            .await
            .map(|(_, alias)| alias)
    }

    pub(super) async fn transform(
        &self,
        subject: &str,
        purpose: PurposedUse,
        spec: TransformSpec,
        output: RasterOutputSpec,
    ) -> Result<ReferenceImage, ImageFailure> {
        let demand = self.capture()?;
        let request = self.transform_request(&demand, subject, purpose, spec, output)?;
        self.execute(&demand, &request)
            .await
            .map(|(reference, _)| reference)
    }

    #[cfg(test)]
    pub(super) async fn read_transform(
        &self,
        subject: &str,
        purpose: PurposedUse,
        spec: TransformSpec,
        output: RasterOutputSpec,
    ) -> Result<Option<ReferenceImage>, ImageFailure> {
        let demand = self.capture()?;
        let request = self.transform_request(&demand, subject, purpose, spec, output)?;
        let principal = self.references.authorize(&request).await?;
        let handle = df_imagery::OperationHandle {
            owner: request.identity.owner.clone(),
            operation: request.identity.operation,
        };
        let view = match demand.service.read(&principal, &handle).await {
            Ok(view) => view,
            Err(ImageServiceError::NotAuthorizedOrNotFound) => return Ok(None),
            Err(error) => return Err(failure(error)),
        };
        self.result(&request, &view).await
    }

    fn transform_request(
        &self,
        demand: &DemandConfiguration,
        subject: &str,
        purpose: PurposedUse,
        spec: TransformSpec,
        output: RasterOutputSpec,
    ) -> Result<df_imagery::ImageRequest, ImageFailure> {
        let mut request = df_imagery::ImageRequest {
            identity: OperationIdentity {
                owner: self.references.owner().clone(),
                operation: operation(subject.as_bytes())?,
                fingerprint: [0; 32],
            },
            purpose,
            operation: ImageOperation::Transform(spec),
            execution: ExecutionMode::Live,
            freshness: Freshness::ReuseCompatible,
            urgency: Urgency::Present,
            audience: owner_audience(self.references.owner()),
            expected_source_revision: self.settings.source_revision,
            output: OutputProfile::local(output),
        };
        self.bind_operation(demand, subject, &mut request)?;
        Ok(request)
    }

    pub(super) async fn frame(
        &self,
        subject: &str,
        prompt: &str,
        references: &[ReferenceImage],
        output: super::OutputSpec,
        freshness: Freshness,
    ) -> Result<ReferenceImage, ImageFailure> {
        let demand = self.capture()?;
        let mut pins = Vec::with_capacity(references.len());
        for reference in references {
            pins.push(self.pin_reference(reference).await?);
        }
        let purpose = df_imagery::Purpose::AnimationFrame;
        let configured = demand
            .config
            .output_profile_for(purpose)
            .ok_or(ImageFailure::InvalidRequest)?;
        if configured.delivered().dimensions()
            != df_imagery::Dimensions::new(output.width, output.height)
                .map_err(|_| ImageFailure::InvalidRequest)?
        {
            return Err(ImageFailure::InvalidRequest);
        }
        let prompt = TextPrompt::new(prompt, 1)
            .and_then(|prompt| {
                prompt.with_pipeline(df_imagery::ImagePipeline {
                    revision: 1,
                    moderation: demand.config.purpose(purpose).moderation_override,
                    rating: self.settings.rating,
                    creator: None,
                    inspection: None,
                })
            })
            .map_err(|_| ImageFailure::InvalidRequest)?;
        let mut request = df_imagery::ImageRequest {
            identity: OperationIdentity {
                owner: self.references.owner().clone(),
                operation: operation(subject.as_bytes())?,
                fingerprint: [0; 32],
            },
            purpose: PurposedUse {
                purpose,
                role: df_imagery::VariantRole::Primary,
            },
            operation: ImageOperation::Generate(df_imagery::GenerateSpec {
                prompt,
                references: pins,
            }),
            execution: ExecutionMode::Live,
            freshness,
            urgency: Urgency::Present,
            audience: owner_audience(self.references.owner()),
            expected_source_revision: self.settings.source_revision,
            output: configured,
        };
        self.bind_operation(&demand, subject, &mut request)?;
        self.execute(&demand, &request)
            .await
            .map(|(reference, _)| reference)
    }

    fn bind_operation(
        &self,
        demand: &DemandConfiguration,
        subject: &str,
        request: &mut df_imagery::ImageRequest,
    ) -> Result<(), ImageFailure> {
        request.identity.fingerprint = demand
            .service
            .canonical_fingerprint(request)
            .map_err(failure)?;
        let mut identity = Sha256::new();
        identity.update(b"df-gameplay/image-operation/1");
        identity.update((subject.len() as u64).to_be_bytes());
        identity.update(subject.as_bytes());
        // Configuration is not part of the operation identity: an edit to it must not turn an
        // identical request into new paid work. Replays return the job's frozen config; fresh
        // regeneration is a separate, explicit request.
        identity.update(request.identity.fingerprint);
        request.identity.operation = operation(&identity.finalize())?;
        Ok(())
    }

    async fn execute(
        &self,
        demand: &DemandConfiguration,
        request: &df_imagery::ImageRequest,
    ) -> Result<(ReferenceImage, ImageKey), ImageFailure> {
        let principal = self.authorize_logged(request).await?;
        let mut view = match retry_unknown(|| demand.service.request(&principal, request))
            .await
            .map_err(failure)?
        {
            RequestOutcome::Recorded(view)
            | RequestOutcome::Replayed(view)
            | RequestOutcome::Reused(view) => view,
            RequestOutcome::ReusedArtifact(artifact) => {
                return self.ready_artifact(request, &artifact).await;
            }
            RequestOutcome::Refused(_) => return Err(ImageFailure::Off),
        };
        loop {
            if let Some(artifact) = &view.artifact {
                return self.ready_artifact(request, artifact).await;
            }
            if let Some(reference) = self.result(request, &view).await? {
                let alias = alias_for_operation(view.handle.operation);
                self.remember_alias(
                    alias.clone(),
                    reference
                        .canonical_source()
                        .cloned()
                        .ok_or(ImageFailure::BadOutput)?,
                )?;
                return Ok((reference, alias));
            }
            if let Some(cause) = view.failure {
                return Err(semantic_failure(cause));
            }
            if view.operational_notice.is_some() {
                // The synchronous caller withdraws only its own interest before reporting
                // the account guard. A raced send retains its existing financial liability.
                let principal = self.references.principal().await?;
                match demand
                    .service
                    .cancel(&principal, &view.handle)
                    .await
                    .map_err(failure)?
                {
                    df_imagery::CancelOutcome::Withdrawn(_)
                    | df_imagery::CancelOutcome::AlreadyWithdrawn(_) => {
                        return Err(ImageFailure::QuotaExhausted);
                    }
                }
            }
            match view.execution {
                ImageExecutionState::Unknown => return Err(ImageFailure::Interrupted),
                ImageExecutionState::Failed { cause } => return Err(semantic_failure(cause)),
                ImageExecutionState::Expired => return Err(ImageFailure::DeadlineExceeded),
                ImageExecutionState::Withdrawn => return Err(ImageFailure::Interrupted),
                _ => {}
            }
            if self.settings.clock.now_ms() >= view.absolute_deadline_ms {
                return Err(ImageFailure::DeadlineExceeded);
            }
            tokio::time::sleep(self.settings.poll_interval).await;
            let principal = self.authorize_logged(request).await?;
            match retry_unknown(|| {
                demand
                    .service
                    .watch(&principal, &view.handle, view.job_revision)
            })
            .await
            .map_err(failure)?
            {
                WatchObservation::Changed(changed) => view = *changed,
                WatchObservation::Unchanged { .. } => {}
            }
        }
    }

    /// Authorizes a request, naming the reason in the audit log when it is refused. A request that
    /// fails before the service sees it leaves no other trace, and the picture simply never comes.
    async fn authorize_logged(
        &self,
        request: &df_imagery::ImageRequest,
    ) -> Result<ImageryPrincipal, ImageFailure> {
        match self.references.authorize(request).await {
            Ok(principal) => Ok(principal),
            Err(failure) => {
                df_observe::record_audit_line(
                    "IMAGERY",
                    format!("IMAGERY authorize_failed failure={failure:?}"),
                );
                Err(failure)
            }
        }
    }

    async fn ready_artifact(
        &self,
        request: &df_imagery::ImageRequest,
        artifact: &df_imagery::publication::ReadyArtifact,
    ) -> Result<(ReferenceImage, ImageKey), ImageFailure> {
        if artifact.audience != request.audience {
            return Err(ImageFailure::Refused);
        }
        let reference = self.references.read_ready_artifact(&artifact.asset).await?;
        let master = artifact
            .variants
            .iter()
            .find(|variant| variant.variant == df_assets::AssetVariant::Master)
            .ok_or(ImageFailure::BadOutput)?;
        let expected = SourceRef {
            asset: artifact.asset,
            digest: AssetDigest::from_bytes(master.sha256),
            audience: artifact.audience.clone(),
        };
        if reference.canonical_source() != Some(&expected) {
            return Err(ImageFailure::Refused);
        }
        let alias = alias_for_artifact(artifact.asset);
        self.remember_alias(alias.clone(), expected)?;
        super::super::image_observability::delivery_authorized(
            self.settings.observer.as_ref(),
            df_imagery::telemetry::EventContext {
                asset: Some(artifact.asset),
                ..Default::default()
            },
            super::super::image_host::delivery_id(),
        );
        Ok((reference, alias))
    }

    async fn result(
        &self,
        request: &df_imagery::ImageRequest,
        view: &ImageDemandView,
    ) -> Result<Option<ReferenceImage>, ImageFailure> {
        let Some(result) = &view.result else {
            return Ok(None);
        };
        let master = result
            .variants
            .iter()
            .find(|variant| variant.variant == df_assets::AssetVariant::Master)
            .ok_or(ImageFailure::BadOutput)?;
        let source = SourceRef {
            asset: result.asset,
            digest: AssetDigest::from_bytes(master.sha256),
            audience: request.audience.clone(),
        };
        let reference = self.references.read_reference(&source).await?;
        let observation = super::super::image_observability::PublishedObservation {
            operation: view.handle.operation,
            job: view.job,
            asset: result.asset,
            config_revision: None,
        };
        super::super::image_observability::delivery_authorized(
            self.settings.observer.as_ref(),
            observation.context(),
            super::super::image_host::delivery_id(),
        );
        Ok(Some(reference))
    }

    #[cfg(test)]
    pub(super) fn issue_fixture(
        &self,
        key: ImageKey,
        source: SourceRef,
    ) -> Result<(), ImageFailure> {
        self.remember_alias(key, source).map(|_| ())
    }

    fn remember_alias(&self, key: ImageKey, source: SourceRef) -> Result<ImageKey, ImageFailure> {
        let mut issued = super::locked(&self.issued);
        issued.insert(key.as_str().to_owned(), source, super::MAXIMUM_ISSUED_KEYS);
        Ok(key)
    }
}

/// Content-addresses the complete public prompt and ordered reference bytes, without run fencing.
pub(in crate::gameplay) fn public_artwork_revision(request: &ImageRequest) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"df-gameplay/public-artwork-source/2");
    digest.update(request.kind.template_revision().to_be_bytes());
    digest.update((request.prompt.as_str().len() as u64).to_be_bytes());
    digest.update(request.prompt.as_str().as_bytes());
    digest.update((request.references.len() as u64).to_be_bytes());
    for reference in &request.references {
        digest.update(Sha256::digest(&reference.bytes));
    }
    digest.finalize().into()
}

fn purpose_for_kind(kind: super::ImageKind) -> df_imagery::Purpose {
    match kind {
        super::ImageKind::CanonicalCharacter => df_imagery::Purpose::CanonicalPerson,
        super::ImageKind::CanonicalRoom => df_imagery::Purpose::CanonicalRoom,
        super::ImageKind::SceneRefresh | super::ImageKind::ScenePlate => {
            df_imagery::Purpose::ScenePlate
        }
        super::ImageKind::ScenePlateEdit => df_imagery::Purpose::SceneEdit,
        super::ImageKind::CastExpression => df_imagery::Purpose::Expression,
        super::ImageKind::ArtifactCloseUp => df_imagery::Purpose::ItemCloseUp,
    }
}

fn owner_audience(owner: &ImageOwner) -> Audience {
    match owner {
        ImageOwner::Run { campaign, .. } => Audience::CampaignShared(*campaign),
        ImageOwner::Draft { account } => Audience::OwnerPrivate(*account),
    }
}
/// How often a call that ended `Unknown` is asked again, and how long it waits between asks.
const UNKNOWN_RETRIES: u32 = 4;
const UNKNOWN_BACKOFF: std::time::Duration = std::time::Duration::from_millis(150);

/// `Unknown` means a read saw the job move between two reads, or a commit's outcome was not seen.
/// Requests and watches are idempotent by operation identity, so asking again returns the stored
/// state; the runtime moves jobs often, and one miss must not fail the image.
pub(in crate::gameplay) async fn retry_unknown<T, Call, Pending>(
    mut call: Call,
) -> Result<T, ImageServiceError>
where
    Call: FnMut() -> Pending,
    Pending: std::future::Future<Output = Result<T, ImageServiceError>>,
{
    let mut result = call().await;
    for _ in 0..UNKNOWN_RETRIES {
        if !matches!(result, Err(ImageServiceError::Unknown { .. })) {
            break;
        }
        tokio::time::sleep(UNKNOWN_BACKOFF).await;
        result = call().await;
    }
    result
}

fn operation(bytes: &[u8]) -> Result<OperationId, ImageFailure> {
    let digest = Sha256::digest(bytes);
    let mut id = [0; 16];
    id.copy_from_slice(&digest[..16]);
    OperationId::from_bytes(&id).map_err(|_| ImageFailure::InvalidRequest)
}
pub(super) fn semantic_failure(cause: df_imagery::execution::FailureCause) -> ImageFailure {
    match cause {
        df_imagery::execution::FailureCause::Refused => ImageFailure::Refused,
        df_imagery::execution::FailureCause::InvalidOutput => ImageFailure::BadOutput,
        df_imagery::execution::FailureCause::TransportLost => ImageFailure::Interrupted,
        df_imagery::execution::FailureCause::RateLimited => ImageFailure::RateLimited,
        df_imagery::execution::FailureCause::QuotaExhausted => ImageFailure::QuotaExhausted,
        df_imagery::execution::FailureCause::DeadlineExceeded => ImageFailure::DeadlineExceeded,
        df_imagery::execution::FailureCause::Provider => ImageFailure::Provider,
    }
}

fn failure(error: ImageServiceError) -> ImageFailure {
    df_observe::record_audit_line("IMAGERY", format!("IMAGERY service_error {error:?}"));
    match error {
        ImageServiceError::InvalidRequest(_) | ImageServiceError::Conflict => {
            ImageFailure::InvalidRequest
        }
        ImageServiceError::Unknown { .. } => ImageFailure::Interrupted,
        ImageServiceError::NotAuthorizedOrNotFound => ImageFailure::Refused,
        ImageServiceError::InvalidRecord => ImageFailure::BadOutput,
        ImageServiceError::Unavailable => ImageFailure::Provider,
    }
}

/// The alias's complete hash remains the fingerprint binding; a prefix collision is a conflict.
pub(in crate::gameplay) fn operation_for_alias(
    key: &ImageKey,
) -> Result<OperationId, ImageFailure> {
    let digest = super::key_digest(key.as_str()).ok_or(ImageFailure::InvalidRequest)?;
    if alias_bytes(&digest[..16]) != *key {
        return Err(ImageFailure::InvalidRequest);
    }
    OperationId::from_bytes(&digest[..16]).map_err(|_| ImageFailure::InvalidRequest)
}

/// The asset alias has a separate checksum domain and never claims an operation receipt.
pub(in crate::gameplay) fn alias_for_artifact(asset: df_assets::AssetId) -> ImageKey {
    let checksum = artifact_alias_checksum(asset.as_bytes());
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(asset.as_bytes());
    bytes[16..].copy_from_slice(&checksum[..16]);
    ImageKey(super::super::journey::hex(&bytes))
}

pub(in crate::gameplay) fn artifact_for_alias(key: &ImageKey) -> Option<df_assets::AssetId> {
    let bytes = super::key_digest(key.as_str())?;
    if bytes[16..] != artifact_alias_checksum(&bytes[..16])[..16] {
        return None;
    }
    df_assets::AssetId::from_bytes(&bytes[..16]).ok()
}

fn artifact_alias_checksum(asset: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"df-gameplay/image-artifact-alias/1");
    digest.update(asset);
    digest.finalize().into()
}

pub(in crate::gameplay) fn alias_for_operation(operation: OperationId) -> ImageKey {
    alias_bytes(operation.as_bytes())
}

fn alias_bytes(operation: &[u8]) -> ImageKey {
    let suffix = Sha256::digest(operation);
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(operation);
    bytes[16..].copy_from_slice(&suffix[..16]);
    ImageKey(super::super::journey::hex(&bytes))
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use bytes::Bytes;
    use image::ImageEncoder;

    fn unknown() -> ImageServiceError {
        ImageServiceError::Unknown {
            handle: df_imagery::OperationHandle {
                owner: ImageOwner::Draft {
                    account: df_types::AccountId::from_bytes(&[3; 16]).unwrap(),
                },
                operation: OperationId::from_bytes(&[4; 16]).unwrap(),
            },
        }
    }

    /// Runs `retry_unknown` over a call that answers from `script`, counting its calls.
    async fn run_script(
        script: Vec<Result<u32, ImageServiceError>>,
    ) -> (Result<u32, ImageServiceError>, usize) {
        let calls = std::sync::Mutex::new(0usize);
        let script = std::sync::Mutex::new(script.into_iter());
        let result = retry_unknown(|| {
            *calls.lock().unwrap() += 1;
            let next = script.lock().unwrap().next().unwrap_or(Err(unknown()));
            async move { next }
        })
        .await;
        let calls = *calls.lock().unwrap();
        (result, calls)
    }

    #[tokio::test]
    async fn retry_unknown_asks_again_until_the_stored_state_is_read() {
        let (result, calls) = run_script(vec![Err(unknown()), Err(unknown()), Ok(7)]).await;
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls, 3, "two misses, then the stored state");
    }

    #[tokio::test]
    async fn retry_unknown_gives_up_after_its_bound_and_reports_unknown() {
        let (result, calls) = run_script(vec![]).await;
        assert!(matches!(result, Err(ImageServiceError::Unknown { .. })));
        assert_eq!(
            calls,
            1 + UNKNOWN_RETRIES as usize,
            "the first ask plus each retry, never a loop"
        );
    }

    #[tokio::test]
    async fn retry_unknown_never_repeats_an_answer_that_is_not_unknown() {
        for answer in [
            Err(ImageServiceError::Unavailable),
            Err(ImageServiceError::Conflict),
            Err(ImageServiceError::NotAuthorizedOrNotFound),
            Ok(1),
        ] {
            let (_, calls) = run_script(vec![answer]).await;
            assert_eq!(calls, 1, "only an Unknown read is asked again");
        }
    }

    #[test]
    fn artifact_alias_restores_only_its_asset_domain_and_never_an_operation_receipt() {
        let bytes = [7; 16];
        let asset = df_assets::AssetId::from_bytes(&bytes).unwrap();
        let operation = OperationId::from_bytes(&bytes).unwrap();
        let artifact_alias = alias_for_artifact(asset);
        let operation_alias = alias_for_operation(operation);
        assert_ne!(artifact_alias, operation_alias);
        assert_eq!(artifact_for_alias(&artifact_alias), Some(asset));
        assert!(operation_for_alias(&artifact_alias).is_err());
        assert_eq!(artifact_for_alias(&operation_alias), None);
        assert_eq!(operation_for_alias(&operation_alias).unwrap(), operation);
        let mut changed = super::super::key_digest(artifact_alias.as_str()).unwrap();
        changed[31] ^= 1;
        let changed = ImageKey::parse(&super::super::super::journey::hex(&changed)).unwrap();
        assert_eq!(artifact_for_alias(&changed), None);
    }

    #[test]
    fn presentation_alias_capacity_evicts_oldest_without_refusing_new_results() {
        let mut aliases = IssuedAliases::default();
        let source = |byte| SourceRef {
            asset: df_assets::AssetId::from_bytes(&[byte; 16]).unwrap(),
            digest: AssetDigest::from_bytes([byte; 32]),
            audience: Audience::PublicStatic,
        };
        aliases.insert("first".to_owned(), source(1), 2);
        aliases.insert("second".to_owned(), source(2), 2);
        aliases.insert("first".to_owned(), source(1), 2);
        aliases.insert("third".to_owned(), source(3), 2);
        assert!(!aliases.sources.contains_key("first"));
        assert_eq!(aliases.sources.get("second"), Some(&source(2)));
        assert_eq!(aliases.sources.get("third"), Some(&source(3)));
        aliases.insert("first".to_owned(), source(1), 2);
        assert_eq!(aliases.sources.get("first"), Some(&source(1)));
        assert_eq!(aliases.sources.len(), 2);
        assert_eq!(aliases.order.len(), 2);
    }

    #[tokio::test]
    async fn readonly_transform_miss_records_no_demand_then_reads_the_actual_published_derivative()
    {
        let fixture = super::super::test_support::TransformFixture::new().await;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &[20, 30, 40, 255, 50, 60, 70, 255],
                2,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let source = fixture
            .reference(df_providers::ImageFormat::Png, Bytes::from(png))
            .await;
        let pin = fixture.imagery().pin_reference(&source).await.unwrap();
        let purpose = PurposedUse {
            purpose: df_imagery::Purpose::ImportedDerivative,
            role: df_imagery::VariantRole::Primary,
        };
        let spec = TransformSpec {
            source: df_imagery::TransformSource::Image(pin),
            steps: Vec::new(),
        };
        let output = RasterOutputSpec::new(
            df_imagery::Dimensions::new(2, 1).unwrap(),
            df_imagery::AlphaHandling::PreserveAlpha,
            df_imagery::Encoding::Png,
        )
        .unwrap();
        assert!(
            fixture
                .imagery()
                .read_transform("readonly-derivative", purpose, spec.clone(), output)
                .await
                .unwrap()
                .is_none()
        );
        let central = fixture.imagery().inner.central.as_ref().unwrap();
        let demand = central.capture().unwrap();
        let request = central
            .transform_request(
                &demand,
                "readonly-derivative",
                purpose,
                spec.clone(),
                output,
            )
            .unwrap();
        let principal = central.references.principal().await.unwrap();
        let handle = df_imagery::OperationHandle {
            owner: request.identity.owner.clone(),
            operation: request.identity.operation,
        };
        assert!(matches!(
            demand.service.read(&principal, &handle).await,
            Err(ImageServiceError::NotAuthorizedOrNotFound)
        ));
        let published = fixture
            .imagery()
            .transform("readonly-derivative", purpose, spec.clone(), output)
            .await
            .unwrap();
        let reread = fixture
            .imagery()
            .read_transform("readonly-derivative", purpose, spec, output)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(published.canonical_source(), reread.canonical_source());
        assert_eq!(published.bytes, reread.bytes);
        drop(demand);
        fixture.close().await;
    }

    #[tokio::test]
    async fn reviewed_jpeg_orientation_is_applied_once_before_registering_its_delivered_dimensions()
    {
        let fixture = super::super::test_support::TransformFixture::new().await;
        let image = image::RgbImage::from_pixel(17, 23, image::Rgb([50, 100, 150]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode_image(&image)
            .unwrap();
        let exif: &[u8] =
            b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut oriented = vec![0xff, 0xd8, 0xff, 0xe1];
        oriented.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        oriented.extend_from_slice(exif);
        oriented.extend_from_slice(&jpeg[2..]);
        assert_eq!(
            df_imagery::source_header(&oriented).unwrap().dimensions,
            df_imagery::Dimensions::new(23, 17).unwrap()
        );
        let source = fixture
            .reference(df_providers::ImageFormat::Jpeg, Bytes::from(oriented))
            .await;
        let key = fixture
            .imagery()
            .register_reviewed("oriented-approved-reference", source)
            .await
            .unwrap();
        let result = fixture.imagery().reference(&key).await.unwrap();
        assert_eq!(
            df_imagery::source_header(&result.bytes).unwrap().dimensions,
            df_imagery::Dimensions::new(23, 17).unwrap()
        );
        fixture.close().await;
    }
}
