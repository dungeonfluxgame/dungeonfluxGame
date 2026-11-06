//! Actual CPU Canvas -> NativeFileStore -> catalog Ready fixture; no source-ready rows are fabricated.
use super::*;
use df_assets::{CatalogStore, IdSource};
use df_imagery::cost::*;
use df_imagery::execution::DispatchId;
use df_imagery::runtime::*;
use df_imagery::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

#[derive(Clone)]
struct Ids(Arc<AtomicU64>);
impl IdSource for Ids {
    fn next_id_bytes(&self) -> [u8; 16] {
        let next = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        let mut bytes = [0; 16];
        bytes[8..].copy_from_slice(&next.to_be_bytes());
        bytes
    }
}
#[derive(Clone)]
struct Clock;
impl df_assets::Clock for Clock {
    fn now_ms(&self) -> i64 {
        df_finance::ports::Clock::now_ms(&SystemClock)
    }
}
struct Authority(ImageOwner);
impl ImageAuthorizer for Authority {
    type Principal = ();
    fn authorize_request(&self, _: &(), request: &ImageRequest) -> Result<(), AuthDenied> {
        (request.identity.owner == self.0
            && matches!(request.operation, ImageOperation::Transform(_)))
        .then_some(())
        .ok_or(AuthDenied)
    }
    fn authorize_owner(
        &self,
        _: &(),
        owner: &ImageOwner,
        _: ServiceAction,
    ) -> Result<(), AuthDenied> {
        (*owner == self.0).then_some(()).ok_or(AuthDenied)
    }
    fn authorize_stored(
        &self,
        _: &(),
        _: ServiceAction,
        _: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        (*stored.owner == self.0).then_some(()).ok_or(AuthDenied)
    }
}
struct NoCalls;
impl ImageSources for NoCalls {
    async fn read_source(
        &self,
        _: &ImageOwner,
        _: &SourceRef,
    ) -> Result<AuthorizedImageSource, RuntimeError> {
        panic!("Canvas requested external source")
    }
}
impl ImageCredentials for NoCalls {
    fn credential(&self, _: BackendId, _: &AccountHandle) -> Result<String, RuntimeError> {
        panic!("Canvas requested supplier credential")
    }
}
impl ImageRuntimeCostPolicy for NoCalls {
    fn route(
        &self,
        _: &ImageJob,
        _: &ImageConfig,
        _: BackendId,
        _: &str,
    ) -> Result<CostRoute, RuntimeError> {
        panic!("Canvas requested supplier route")
    }
}
impl ImageCostProjection for NoCalls {
    fn start_pinned<'a>(
        &'a self,
        _: &'a str,
        _: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>> {
        Box::pin(async { panic!("Canvas started billed work") })
    }
    fn correct<'a>(
        &'a self,
        _: &'a str,
        _: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>> {
        Box::pin(async { panic!("Canvas projected billed usage") })
    }
    fn record_unsent<'a>(&'a self, _: &'a str) -> CostFuture<'a, Result<PinnedUnsent, CostError>> {
        Box::pin(async { panic!("Canvas released money") })
    }
    fn is_durable<'a>(
        &'a self,
        _: &'a str,
        _: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>> {
        Box::pin(async { panic!("Canvas read billed usage") })
    }
}
impl ImageRuntimeTelemetry for NoCalls {
    fn record(
        &self,
        _: &ImageJob,
        _: &StageView,
        _: Option<DispatchId>,
        _: RuntimePhase,
        _: i64,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
}
/// Creates a genuine source with the existing local publisher and returns its actual consumed-version metadata.
pub(crate) async fn create(world: &World, owner: &ImageOwner) -> SourceReceipt {
    let config = world.scratch.config();
    let transport = crate::postgres_tls::PostgresTransport::PlaintextLoopbackOnly;
    let now = df_finance::ports::Clock::now_ms(&SystemClock);
    let ids = Ids(Arc::new(AtomicU64::new(90_000)));
    let capacity = CapacityScope::new("funding-native-source").unwrap();
    let execution = crate::imagery_execution::PostgresImageExecution::open_existing(
        config.clone(),
        transport.clone(),
        2,
    )
    .await
    .unwrap();
    execution
        .set_capacity_bounds(
            capacity.as_str(),
            &CapacityBounds::new(2, 4, 4_194_304).unwrap(),
            now,
        )
        .await
        .unwrap();
    execution
        .set_execution_gate(capacity.as_str(), 1, true, now)
        .await
        .unwrap();
    let journal = crate::imagery_runtime::PostgresRuntimeJournal::open_existing(
        config.clone(),
        transport.clone(),
        2,
        Arc::new(ids.clone()),
        capacity.clone(),
        1,
        true,
    )
    .unwrap();
    let mut image_config = ImageConfig::defaults().unwrap();
    image_config.mode = GlobalMode::LocalOnly;
    let service = ImageService::new(
        journal.clone(),
        image_config.clone(),
        Clock,
        ids.clone(),
        Authority(owner.clone()),
    )
    .unwrap();
    let audience = match owner {
        ImageOwner::Draft { account } => Audience::OwnerPrivate(*account),
        ImageOwner::Run { campaign, .. } => Audience::CampaignShared(*campaign),
    };
    let mut request = ImageRequest {
        identity: OperationIdentity {
            owner: owner.clone(),
            operation: OperationId::from_bytes(&[200; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ImportedDerivative,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Transform(TransformSpec {
            source: TransformSource::Canvas(
                CanvasSpec::new(
                    2,
                    2,
                    Rgb {
                        red: 30,
                        green: 60,
                        blue: 90,
                    },
                )
                .unwrap(),
            ),
            steps: vec![],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience,
        expected_source_revision: 1,
        output: OutputProfile::local(
            RasterOutputSpec::new(
                Dimensions::new(2, 2).unwrap(),
                AlphaHandling::Opaque,
                Encoding::Png,
            )
            .unwrap(),
        ),
    };
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    assert!(matches!(
        service.request(&(), &request).await.unwrap(),
        RequestOutcome::Recorded(_)
    ));
    let objects = Arc::new(
        df_assets::NativeFileStore::new(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/tmp/funding-native-sources")
                .join(config.get_dbname().unwrap()),
            16_777_216,
        )
        .unwrap(),
    );
    let catalog = Arc::new(
        crate::media_catalog::PostgresMediaCatalog::open_existing(
            config.clone(),
            transport.clone(),
            2,
        )
        .await
        .unwrap(),
    );
    let publication = crate::imagery_publication::PostgresImagePublication::open_existing(
        config.clone(),
        transport.clone(),
        2,
    )
    .await
    .unwrap();
    let publisher = || {
        crate::imagery_publication::NativeImagePublisher::new(
            crate::imagery_publication::RuntimePublicationSettings {
                catalog: df_assets::CatalogConfig::default(),
                layout: df_assets::KeyLayout::new(
                    df_assets::ObjectEnvironment::new("fund-source").unwrap(),
                ),
                decode_concurrency: 1,
            },
            crate::imagery_publication::RuntimePublicationPorts {
                catalog: catalog.clone(),
                publication: publication.clone(),
                objects: objects.clone(),
                ids: Arc::new(ids.clone()),
                clock: Arc::new(Clock),
            },
        )
        .unwrap()
    };
    let runtime = NativeImageRuntime {
        journal,
        execution,
        sources: NoCalls,
        publisher: publisher(),
        admission: publisher(),
        clock: Clock,
        ids,
        credentials: NoCalls,
        costs: NoCalls,
        cost_policy: NoCalls,
        usage: crate::imagery_runtime::PostgresImageUsage::open_existing(config, transport, 2)
            .unwrap(),
        telemetry: NoCalls,
        settings: RuntimeSettings {
            config: image_config,
            active_config_revision: 1,
            capacity,
            poll_interval: Duration::from_millis(5),
            codex_home: None,
        },
    };
    let ready = Box::pin(runtime.run_next())
        .await
        .unwrap()
        .expect("actual native Canvas publication");
    let asset = catalog.asset(ready.asset).await.unwrap().unwrap();
    assert_eq!(asset.state, df_assets::AssetState::Ready);
    let variants = catalog.variants(ready.asset).await.unwrap();
    let master = variants
        .into_iter()
        .find(|variant| {
            variant.variant == df_assets::AssetVariant::Master
                && asset.winning_writes.contains(&variant.write)
        })
        .unwrap();
    SourceReceipt {
        pin: SourceRef {
            asset: ready.asset,
            digest: AssetDigest::from_bytes(master.manifest.unwrap().sha256),
            audience: request.audience,
        },
        attempt: ready.attempt,
        write: master.write,
        version: master.version.unwrap(),
        suppression_generation: asset.suppression_generation,
        retention: asset.retention,
    }
}
