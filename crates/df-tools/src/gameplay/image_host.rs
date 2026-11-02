//! Mounted local gameplay image service, owned runtime, current proof custody and shutdown.

use df_assets::{CatalogConfig, Clock, IdSource, KeyLayout, NativeFileStore, ObjectEnvironment};
use df_imagery::{
    AccountingMode, ConfigControl, GlobalMode, ImageConfig, ImageContentRating, ImageOwner,
    ImageService, ImageServiceApi,
    execution::{CapacityBounds, CapacityScope, ImageExecutionJournal},
    runtime::{
        ImageCredentials, NativeImageRuntime, RuntimeError, RuntimeSettings, UsageProjectionLimits,
    },
    telemetry::SharedImageObserver,
};
use df_persistence::{
    imagery_execution::PostgresImageExecution,
    imagery_publication::{
        NativeImagePublisher, PostgresImagePublication, RuntimePublicationPorts,
        RuntimePublicationSettings,
    },
    imagery_runtime::{PostgresImageObservations, PostgresImageUsage, PostgresRuntimeJournal},
    media_catalog::PostgresMediaCatalog,
    postgres_tls::PostgresTransport,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex as AsyncMutex, oneshot},
    task::JoinHandle,
};

use super::{
    cost::{
        self,
        imagery::{GameplayImageCostPolicy, ImageryCostAdapter},
    },
    image_authorization::HostImageAuthorizer,
    image_imports::{CatalogReferenceImporter, ImportPorts, ImportSettings},
    image_references::{AnnotationStore, CatalogReferenceAuthority, VerifiedReferenceFacts},
    image_sources::{CatalogImageSources, CurrentImageGrant, CurrentImageGrants},
    imagery::{
        ImageFailure, Imagery,
        central::{
            CentralImages, CentralSettings, CreatorContexts, ReferenceFuture, ServiceFactory,
        },
    },
    imagery_identity::{ImageryPrincipal, LocalRunGrant, resolve_local_run_principal},
    resources::Hub,
};

#[derive(Clone, Copy)]
pub(super) struct NativeClock;
impl Clock for NativeClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
            .unwrap_or(i64::MAX)
    }
}
#[derive(Clone)]
pub(super) struct NativeIds(Arc<Entropy>);
struct Entropy {
    seed: [u8; 32],
    sequence: AtomicU64,
}
impl NativeIds {
    fn new() -> Result<Self, io::Error> {
        let mut seed = [0; 32];
        getrandom::getrandom(&mut seed)
            .map_err(|_| failure("image identity entropy unavailable"))?;
        if seed == [0; 32] {
            return Err(failure("image identity entropy refused"));
        }
        Ok(Self(Arc::new(Entropy {
            seed,
            sequence: AtomicU64::new(1),
        })))
    }
}
impl IdSource for NativeIds {
    fn next_id_bytes(&self) -> [u8; 16] {
        let Ok(sequence) =
            self.0
                .sequence
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
        else {
            return [0; 16];
        };
        let mut hash = Sha256::new();
        hash.update(self.0.seed);
        hash.update(sequence.to_be_bytes());
        let digest = hash.finalize();
        let mut id = [0; 16];
        id.copy_from_slice(&digest[..16]);
        id
    }
}

/// Endpoint/model policy comes from trusted config; secret values stay in deployment custody.
#[derive(Clone)]
struct HostCredentials {
    control: Arc<Mutex<ConfigControl>>,
    fixture: Option<std::net::SocketAddr>,
    analysis_fixture: Option<std::net::SocketAddr>,
    custody: Arc<BTreeMap<String, String>>,
    quota: Arc<df_imagery::runtime::QuotaRegistry>,
}
impl ImageCredentials for HostCredentials {
    fn quota_registry(&self) -> Option<Arc<df_imagery::runtime::QuotaRegistry>> {
        Some(self.quota.clone())
    }
    fn credential(
        &self,
        backend: df_imagery::BackendId,
        account: &df_imagery::AccountHandle,
    ) -> Result<String, RuntimeError> {
        let current = self
            .control
            .lock()
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .read_effective();
        let segmentation = current
            .config
            .analysis
            .segmentation
            .as_ref()
            .filter(|route| backend == df_imagery::BackendId::Fal && route.account == *account);
        let configured = segmentation.is_some()
            || (backend == df_imagery::BackendId::Bfl
                && current.config.creator_bfl_account.as_ref() == Some(account))
            || current.config.backend(backend).account == *account
            || current.config.purposes.iter().any(|purpose| {
                purpose.account_override.as_ref() == Some(account)
                    && purpose
                        .backend_override
                        .unwrap_or(current.config.default_backend)
                        == backend
            });
        if !configured {
            return Err(RuntimeError::InvalidConfiguration);
        }
        if let Some(address) = self.fixture {
            let endpoint = format!("http://{address}");
            let selected_endpoint = segmentation.map(|route| &route.endpoint).or(current
                .config
                .backend(backend)
                .endpoint
                .as_ref());
            let analysis_endpoint = self
                .analysis_fixture
                .map(|address| format!("http://{address}"));
            if backend != df_imagery::BackendId::CodexCli
                && selected_endpoint.is_some_and(|configured| {
                    configured.as_str() == endpoint
                        || (segmentation.is_some()
                            && analysis_endpoint.as_deref() == Some(configured.as_str()))
                })
            {
                return Ok("test-credential".to_owned());
            }
            return Err(RuntimeError::InvalidConfiguration);
        }
        if let Some(name) = self.custody.get(account.as_str()) {
            return secret(&[name]);
        }
        if segmentation.is_some() {
            return secret(&["FAL_KEY", "DF_FAL_KEY"]);
        }
        if backend == df_imagery::BackendId::CodexCli {
            // CLI custody remains the operator's authenticated local session.
            return Ok(String::new());
        }
        secret(backend_secret_names(backend, account.as_str()))
    }

    fn analysis_credential(
        &self,
        account: &df_imagery::AccountHandle,
    ) -> Result<String, RuntimeError> {
        let current = self
            .control
            .lock()
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .read_effective();
        let routes = [
            &current.config.analysis.classification,
            &current.config.analysis.inspection,
            &current.config.analysis.head_locations,
            &current.config.analysis.staging_identity,
            &current.config.analysis.staging_facing,
        ];
        let selected = routes
            .into_iter()
            .flatten()
            .filter(|route| route.account == *account)
            .collect::<Vec<_>>();
        let segmentation = current
            .config
            .analysis
            .segmentation
            .as_ref()
            .filter(|route| route.account == *account);
        if selected.is_empty() && segmentation.is_none() {
            return Err(RuntimeError::InvalidConfiguration);
        }
        if let Some(address) = self.fixture {
            let origin = format!("http://{address}");
            let prefix = format!("{origin}/");
            let analysis_origin = self
                .analysis_fixture
                .map(|address| format!("http://{address}"));
            let valid = |endpoint: &str| {
                endpoint == origin
                    || endpoint.starts_with(&prefix)
                    || analysis_origin.as_ref().is_some_and(|origin| {
                        endpoint == origin || endpoint.starts_with(&format!("{origin}/"))
                    })
            };
            if selected.iter().all(|route| valid(route.endpoint.as_str()))
                && segmentation.is_none_or(|route| valid(route.endpoint.as_str()))
            {
                return Ok("test-credential".to_owned());
            }
            return Err(RuntimeError::InvalidConfiguration);
        }
        if let Some(name) = self.custody.get(account.as_str()) {
            return secret(&[name]);
        }
        let classification = current
            .config
            .analysis
            .classification
            .as_ref()
            .is_some_and(|route| route.account == *account);
        let openai = [
            &current.config.analysis.inspection,
            &current.config.analysis.head_locations,
            &current.config.analysis.staging_identity,
            &current.config.analysis.staging_facing,
        ]
        .into_iter()
        .flatten()
        .any(|route| route.account == *account);
        match (classification, openai, segmentation.is_some()) {
            (true, false, false) => secret(&["DF_GAMEPLAY_CREATOR_JEV_API_KEY"]),
            (false, true, false) => secret(&["OPENAI_API_KEY"]),
            (false, false, true) => secret(&["FAL_KEY", "DF_FAL_KEY"]),
            _ => Err(RuntimeError::InvalidConfiguration),
        }
    }
}
fn backend_secret_names(backend: df_imagery::BackendId, account: &str) -> &'static [&'static str] {
    match (backend, account) {
        (df_imagery::BackendId::OpenAi, "creator-openai") => &["DF_GAMEPLAY_CREATOR_OPENAI_KEY"],
        (df_imagery::BackendId::Fal, "creator-fal") => {
            &["DF_GAMEPLAY_CREATOR_FAL_KEY", "FAL_KEY", "DF_FAL_KEY"]
        }
        (df_imagery::BackendId::Bfl, "creator-bfl") => {
            &["DF_GAMEPLAY_CREATOR_BFL_KEY", "BFL_API_KEY", "BFL_KEY"]
        }
        (df_imagery::BackendId::OpenAi, _) => &["OPENAI_API_KEY"],
        (df_imagery::BackendId::Fal, _) => &["FAL_KEY", "DF_FAL_KEY"],
        (df_imagery::BackendId::Bfl, _) => &["BFL_API_KEY", "BFL_KEY"],
        (df_imagery::BackendId::CodexCli, _) => &[],
    }
}
fn secret(names: &[&str]) -> Result<String, RuntimeError> {
    secret_from(names, |name| std::env::var(name))
}
fn secret_from(
    names: &[&str],
    mut read: impl FnMut(&str) -> Result<String, std::env::VarError>,
) -> Result<String, RuntimeError> {
    for name in names {
        match read(name) {
            Ok(value) if !value.is_empty() => return Ok(value),
            Ok(_) | Err(std::env::VarError::NotPresent) => {}
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(RuntimeError::InvalidConfiguration);
            }
        }
    }
    Err(RuntimeError::CredentialUnavailable)
}

#[derive(Clone)]
pub(super) struct LocalHostSettings {
    pub(super) worker_database: tokio_postgres::Config,
    pub(super) fixture: Option<std::net::SocketAddr>,
    pub(super) analysis_fixture: Option<std::net::SocketAddr>,
    pub(super) adventure: Option<super::imagery::AdventureManifest>,
    pub(super) operator_database: tokio_postgres::Config,
    pub(super) transport: PostgresTransport,
    pub(super) connections: usize,
    pub(super) max_current_grants: usize,
    pub(super) object_root: PathBuf,
    pub(super) environment: ObjectEnvironment,
    pub(super) catalog: CatalogConfig,
    pub(super) reviewed_maximum_bytes: u64,
    pub(super) annotation_maximum_bytes: u64,
    pub(super) control: Arc<Mutex<ConfigControl>>,
    pub(super) run_grant: LocalRunGrant,
    pub(super) source_revision: u64,
    pub(super) rating: ImageContentRating,
    pub(super) poll_interval: Duration,
    pub(super) shutdown_timeout: Duration,
    pub(super) database_timeout: Duration,
    pub(super) usage_limits: UsageProjectionLimits,
}
pub(super) struct LocalHostPorts {
    pub(super) observer: Arc<SharedImageObserver>,
    pub(super) costs: cost::port::MeteringPort,
}
struct Shared {
    control: Arc<Mutex<ConfigControl>>,
    accounting: AccountingMode,
    session: [u8; 16],
    sources: Arc<CatalogImageSources<NativeFileStore>>,
    grants: CurrentImageGrants,
    catalog: Arc<PostgresMediaCatalog>,
    journal: PostgresRuntimeJournal<NativeIds>,
    results: PostgresImagePublication,
    services: Arc<ServiceFactory>,
    facts: VerifiedReferenceFacts,
    ids: NativeIds,
    publisher: Arc<NativeImagePublisher<NativeFileStore, NativeIds, NativeClock>>,
    costs: Arc<ImageryCostAdapter>,
    usage: PostgresImageUsage,
    observer: Arc<SharedImageObserver>,
    execution: PostgresImageExecution,
    operator: PostgresImageExecution,
    capacity: CapacityScope,
    catalog_settings: CatalogConfig,
    layout: KeyLayout,
    source_revision: u64,
    rating: ImageContentRating,
    poll_interval: Duration,
    applied: Arc<AtomicU64>,
    shutdown_timeout: Duration,
    import_decode: Arc<Mutex<Arc<tokio::sync::Semaphore>>>,
    import_gate: Arc<tokio::sync::RwLock<()>>,
    import_concurrency: std::sync::atomic::AtomicU32,
    reviewed_maximum_bytes: u64,
    annotations: Arc<AnnotationStore>,
    fixture: Option<std::net::SocketAddr>,
    analysis_fixture: Option<std::net::SocketAddr>,
    adventure: Option<super::imagery::AdventureManifest>,
    running: AtomicBool,
    custody: Arc<BTreeMap<String, String>>,
    quota: Arc<df_imagery::runtime::QuotaRegistry>,
    _quota_notice: super::imagery::QuotaNoticeRegistration,
}
impl Shared {
    fn client(&self, owner: ImageOwner) -> Result<CentralImages, io::Error> {
        let mut import_catalog = self.catalog_settings;
        import_catalog.maximum_bytes.image = import_catalog
            .maximum_bytes
            .image
            .max(self.reviewed_maximum_bytes);
        let importer = CatalogReferenceImporter::new(
            ImportSettings {
                owner: owner.clone(),
                catalog: import_catalog,
                layout: self.layout.clone(),
            },
            ImportPorts {
                catalog: self.catalog.clone(),
                sources: self.sources.clone(),
                clock: Arc::new(NativeClock),
                ids: Arc::new(self.ids.clone()),
                decode: self.import_decode.clone(),
                gate: self.import_gate.clone(),
            },
        );
        let references = CatalogReferenceAuthority {
            owner,
            sources: self.sources.clone(),
            journal: self.journal.clone(),
            results: self.results.clone(),
            facts: self.facts.clone(),
            importer: Arc::new(importer),
            reviewed_maximum_bytes: self.reviewed_maximum_bytes,
            annotations: self.annotations.clone(),
            adventure: self.adventure.clone(),
        };
        CentralImages::new(
            self.services.clone(),
            Arc::new(references),
            CentralSettings {
                control: self.control.clone(),
                rating: self.rating,
                source_revision: self.source_revision,
                clock: Arc::new(NativeClock),
                poll_interval: self.poll_interval,
                observer: self.observer.clone(),
            },
        )
        .map_err(|_| failure("central image consumer refused"))
    }
}

#[derive(Clone)]
struct LocalCreatorContexts(std::sync::Weak<Shared>);
impl CreatorContexts for LocalCreatorContexts {
    fn for_local_creator(&self, credential: [u8; 32]) -> ReferenceFuture<'_, CentralImages> {
        Box::pin(async move {
            let shared = self.0.upgrade().ok_or(ImageFailure::Off)?;
            let owner = {
                let mut client = shared.sources.authority.lock().await;
                let transaction = client
                    .transaction()
                    .await
                    .map_err(|_| ImageFailure::Interrupted)?;
                let row = transaction
                    .query_opt(
                        "SELECT principal_id FROM df_local_demo.grants \
                     WHERE credential=$1 AND session_id=$2 AND active \
                       AND effective_role=$3 AND expires_at>clock_timestamp() FOR SHARE",
                        &[
                            &credential.as_slice(),
                            &shared.session.as_slice(),
                            &b"player".as_slice(),
                        ],
                    )
                    .await
                    .map_err(|_| ImageFailure::Interrupted)?
                    .ok_or(ImageFailure::Refused)?;
                let member: Vec<u8> = row.try_get(0).map_err(|_| ImageFailure::BadOutput)?;
                let member: [u8; 16] = member.try_into().map_err(|_| ImageFailure::BadOutput)?;
                let mut proof = Sha256::new();
                proof.update(b"df-gameplay/local-image-account/1");
                proof.update(member);
                proof.update(shared.session);
                let fingerprint: [u8; 32] = proof.finalize().into();
                let mut operation = [0; 16];
                operation.copy_from_slice(&fingerprint[..16]);
                let command = df_persistence::imagery_identity::LocalGuestBootstrap {
                    member,
                    session: shared.session,
                    credential,
                    candidate_account: shared.ids.next_id_bytes(),
                    operation,
                    fingerprint,
                    generation: shared.ids.next_id_bytes(),
                };
                let (binding, _) = df_persistence::imagery_identity::bootstrap_local_guest(
                    &transaction,
                    &command,
                    Clock::now_ms(&NativeClock),
                )
                .await
                .map_err(|_| ImageFailure::Refused)?;
                let account = df_types::AccountId::from_bytes(&binding.account)
                    .map_err(|_| ImageFailure::BadOutput)?;
                let owner = ImageOwner::Draft { account };
                shared
                    .grants
                    .register(
                        &transaction,
                        &owner,
                        CurrentImageGrant::LocalGuest {
                            member,
                            session: shared.session,
                            credential,
                        },
                    )
                    .await
                    .map_err(|error| match error {
                        super::image_sources::GrantRegistrationError::Capacity { maximum } => {
                            debug_assert!(maximum > 0);
                            ImageFailure::AuthorityCapacity
                        }
                        super::image_sources::GrantRegistrationError::Authority(error) => {
                            super::image_references::source_failure(error)
                        }
                    })?;
                transaction
                    .commit()
                    .await
                    .map_err(|_| ImageFailure::Interrupted)?;
                owner
            };
            shared
                .client(owner)
                .map(|client| client.with_creator_contexts(Arc::new(self.clone())))
                .map_err(|_| ImageFailure::Interrupted)
        })
    }
}

/// Holds the exact facts used by every per-revision service, outside supplier and pixel authority.
struct Services {
    journal: PostgresRuntimeJournal<NativeIds>,
    accounting: AccountingMode,
    results: AuthorizedResults,
    observations: PostgresImageObservations,
    facts: VerifiedReferenceFacts,
    ids: NativeIds,
    observer: Arc<SharedImageObserver>,
    applied: Arc<AtomicU64>,
}
impl Services {
    fn capture(
        &self,
        config: &ImageConfig,
        revision: u64,
    ) -> Result<Arc<dyn ImageServiceApi<Principal = ImageryPrincipal>>, ImageFailure> {
        if config.accounting.mode != self.accounting
            || self.applied.load(Ordering::Acquire) != revision
        {
            return Err(ImageFailure::Interrupted);
        }
        let journal = self
            .journal
            .with_config_revision(revision, self.accounting == AccountingMode::RecordOnly)
            .map_err(|_| ImageFailure::Interrupted)?;
        let service = ImageService::new(
            journal,
            config.clone(),
            NativeClock,
            self.ids.clone(),
            HostImageAuthorizer::new(self.facts.clone(), config),
        )
        .map_err(|_| ImageFailure::InvalidRequest)?
        .with_results(self.results.clone())
        .with_observations(self.observations.clone())
        .with_observer(self.observer.clone())
        .with_config_revision(revision)
        .map_err(|_| ImageFailure::InvalidRequest)?;
        Ok(Arc::new(service))
    }
}

pub(super) struct LocalImageHost {
    imagery: Imagery,
    shared: Arc<Shared>,
    stop: Option<oneshot::Sender<()>>,
    supervisor: Option<JoinHandle<Result<(), io::Error>>>,
    authority_driver: AuthorityDriver,
    shutdown_timeout: Duration,
}
impl LocalImageHost {
    pub(super) async fn open(
        settings: LocalHostSettings,
        ports: LocalHostPorts,
        hub: Hub,
    ) -> Result<Self, io::Error> {
        if settings
            .fixture
            .is_some_and(|address| !address.ip().is_loopback() || address.port() == 0)
            || settings.analysis_fixture.is_some_and(|address| {
                settings.fixture.is_none() || !address.ip().is_loopback() || address.port() == 0
            })
            || settings.max_current_grants == 0
            || settings.connections == 0
            || settings.connections > tokio::sync::Semaphore::MAX_PERMITS
            || settings.shutdown_timeout.is_zero()
            || settings.database_timeout.is_zero()
            || settings.reviewed_maximum_bytes == 0
            || settings.annotation_maximum_bytes == 0
            || settings.poll_interval.is_zero()
            || settings.worker_database.get_dbname() != settings.operator_database.get_dbname()
            || settings.worker_database.get_hosts() != settings.operator_database.get_hosts()
            || settings.worker_database.get_ports() != settings.operator_database.get_ports()
        {
            return Err(failure("image host database or bounds invalid"));
        }
        let mut effective = super::image_configuration::snapshot(&settings.control)?;
        if effective.config.accounting.mode != AccountingMode::RecordOnly {
            return Err(failure("local run images require record-only accounting"));
        }
        let custody = Arc::new(credential_custody()?);
        let ids = NativeIds::new()?;
        let (client, driver) =
            df_persistence::postgres_tls::connect(&settings.worker_database, &settings.transport)
                .await
                .map_err(|_| failure("image authority connection unavailable"))?;
        let driver = AuthorityDriver(Some(tokio::spawn(driver)));
        let timeout_ms = u64::try_from(settings.database_timeout.as_millis())
            .map_err(|_| failure("image database timeout invalid"))?;
        let timeout = format!("{timeout_ms}ms");
        client.query_one(
            "SELECT set_config('statement_timeout',$1,false), set_config('lock_timeout',$1,false), set_config('idle_in_transaction_session_timeout',$1,false)",
            &[&timeout],
        ).await.map_err(|_| failure("image database timeout unavailable"))?;
        let authority = Arc::new(AsyncMutex::new(client));
        let grants = CurrentImageGrants::new(settings.max_current_grants)
            .map_err(|_| failure("image authority capacity invalid"))?;
        let source_authority = Arc::new(grants.clone());
        let session = settings.run_grant.session;
        let owner = {
            let mut client = authority.lock().await;
            let transaction = client
                .transaction()
                .await
                .map_err(|_| failure("image authority transaction unavailable"))?;
            let principal = resolve_local_run_principal(&transaction, &settings.run_grant)
                .await
                .map_err(|_| failure("current local image run refused"))?;
            let owner = principal
                .local_run_owner()
                .cloned()
                .ok_or_else(|| failure("local image owner missing"))?;
            grants
                .register(
                    &transaction,
                    &owner,
                    CurrentImageGrant::LocalRun(settings.run_grant),
                )
                .await
                .map_err(|_| failure("local image grant refused"))?;
            transaction
                .commit()
                .await
                .map_err(|_| failure("local image grant acknowledgement unknown"))?;
            owner
        };
        let catalog = Arc::new(
            PostgresMediaCatalog::open_existing(
                settings.worker_database.clone(),
                settings.transport.clone(),
                settings.connections,
            )
            .await
            .map_err(|_| failure("image catalog unavailable"))?,
        );
        let objects = Arc::new(
            NativeFileStore::new(
                &settings.object_root,
                effective
                    .config
                    .transforms
                    .max_output_bytes
                    .max(settings.catalog.maximum_bytes.image)
                    .max(settings.reviewed_maximum_bytes),
            )
            .map_err(|_| failure("private image object root unavailable"))?,
        );
        let layout = KeyLayout::new(settings.environment);
        let sources = Arc::new(CatalogImageSources {
            objects: objects.clone(),
            authority,
            grants: grants.clone(),
            layout: layout.clone(),
            maximum_bytes: settings
                .catalog
                .maximum_bytes
                .image
                .max(settings.reviewed_maximum_bytes),
            deadline: settings.catalog.store_deadline,
            analysis: std::sync::OnceLock::new(),
        });
        let capacity = CapacityScope::new("gameplay-images".to_owned())
            .map_err(|_| failure("image capacity scope invalid"))?;
        let journal = PostgresRuntimeJournal::open_existing(
            settings.worker_database.clone(),
            settings.transport.clone(),
            settings.connections,
            Arc::new(ids.clone()),
            capacity.clone(),
            effective.provenance.revision,
            true,
        )
        .map_err(|_| failure("image demand journal unavailable"))?;
        let execution = PostgresImageExecution::open_existing(
            settings.worker_database.clone(),
            settings.transport.clone(),
            settings.connections,
        )
        .await
        .map_err(|_| failure("image execution schema unavailable"))?
        .with_source_authority(source_authority.clone());
        let operator = PostgresImageExecution::open_existing(
            settings.operator_database.clone(),
            settings.transport.clone(),
            settings.connections,
        )
        .await
        .map_err(|_| failure("image operator schema unavailable"))?
        .with_source_authority(source_authority.clone());
        let adopted = adopt_startup_gate(&operator, &capacity, &effective).await?;
        {
            let mut control = settings
                .control
                .lock()
                .map_err(|_| failure("image startup config unavailable"))?;
            control
                .adopt_startup_revision(effective.provenance.revision, adopted)
                .map_err(|_| failure("image startup config changed during gate adoption"))?;
            effective = control.read_effective();
        }
        let results = PostgresImagePublication::open_existing(
            settings.worker_database.clone(),
            settings.transport.clone(),
            settings.connections,
        )
        .await
        .map_err(|_| failure("image publication schema unavailable"))?;
        let usage = PostgresImageUsage::open_existing(
            settings.worker_database.clone(),
            settings.transport.clone(),
            settings.connections,
        )
        .map_err(|_| failure("image usage schema unavailable"))?;
        let costs = Arc::new(ImageryCostAdapter::new(ports.costs)?);
        let observations = PostgresImageObservations::open_existing(
            settings.worker_database,
            settings.transport,
            settings.connections,
        )
        .map_err(|_| failure("image observation reader unavailable"))?
        .with_usage_projection(costs.clone(), settings.usage_limits)
        .map_err(|_| failure("image usage observation limits invalid"))?;
        let publisher = Arc::new(
            NativeImagePublisher::new(
                RuntimePublicationSettings {
                    catalog: settings.catalog,
                    layout: layout.clone(),
                    decode_concurrency: usize::try_from(
                        effective.config.transforms.max_concurrency,
                    )
                    .map_err(|_| failure("image decode limit invalid"))?,
                },
                RuntimePublicationPorts {
                    catalog: catalog.clone(),
                    publication: results.clone(),
                    objects,
                    ids: Arc::new(ids.clone()),
                    clock: Arc::new(NativeClock),
                },
            )
            .map_err(|_| failure("native image publisher unavailable"))?
            .with_source_authority(source_authority.clone()),
        );
        let applied = Arc::new(AtomicU64::new(0));
        let service_owner = Arc::new(Services {
            journal: journal.clone(),
            accounting: effective.config.accounting.mode,
            results: AuthorizedResults {
                inner: results.clone(),
                authority: source_authority.clone(),
            },
            observations: observations.clone(),
            facts: VerifiedReferenceFacts::default(),
            ids: ids.clone(),
            observer: ports.observer.clone(),
            applied: applied.clone(),
        });
        sources
            .analysis
            .set(Arc::new(
                super::image_analysis_read::HostAnalysisReader::new(
                    journal.clone(),
                    observations,
                    Arc::downgrade(&sources),
                ),
            ))
            .map_err(|_| failure("image analysis reader already mounted"))?;
        let maker = service_owner.clone();
        let services: Arc<ServiceFactory> =
            Arc::new(move |config, revision| maker.capture(config, revision));
        let quota = Arc::new(df_imagery::runtime::QuotaRegistry::new(Arc::new(
            NativeClock,
        )));
        let quota_notice = super::imagery::register_quota_notice(&quota);
        let shared = Arc::new(Shared {
            control: settings.control,
            accounting: effective.config.accounting.mode,
            session,
            sources,
            grants,
            catalog,
            journal,
            results,
            services,
            facts: service_owner.facts.clone(),
            ids,
            publisher,
            costs,
            usage,
            observer: ports.observer,
            execution,
            operator,
            capacity,
            catalog_settings: settings.catalog,
            layout,
            fixture: settings.fixture,
            analysis_fixture: settings.analysis_fixture,
            adventure: settings.adventure,
            running: AtomicBool::new(true),
            custody,
            quota,
            _quota_notice: quota_notice,
            source_revision: settings.source_revision,
            rating: settings.rating,
            poll_interval: settings.poll_interval,
            applied,
            shutdown_timeout: settings.shutdown_timeout,
            import_decode: Arc::new(Mutex::new(Arc::new(tokio::sync::Semaphore::new(
                usize::try_from(effective.config.transforms.max_concurrency)
                    .map_err(|_| failure("image import concurrency invalid"))?,
            )))),
            import_gate: Arc::new(tokio::sync::RwLock::new(())),
            import_concurrency: std::sync::atomic::AtomicU32::new(
                effective.config.transforms.max_concurrency,
            ),
            reviewed_maximum_bytes: settings.reviewed_maximum_bytes,
            annotations: Arc::new(AnnotationStore::new(
                settings.object_root.join("annotations"),
                settings.annotation_maximum_bytes,
            )?),
        });
        apply_gate(&shared, &effective.config, effective.provenance.revision).await?;
        let contexts = Arc::new(LocalCreatorContexts(Arc::downgrade(&shared)));
        let central = shared.client(owner)?.with_creator_contexts(contexts);
        let imagery = Imagery::from_central(central, hub);
        let (stop, stopped) = oneshot::channel();
        let running = shared.clone();
        let supervisor = tokio::spawn(async move {
            let result = supervise(running.clone(), stopped).await;
            running.running.store(false, Ordering::Release);
            if let Err(error) = &result {
                df_observe::record_audit_line(
                    "IMAGERY",
                    format!("IMAGERY runtime_stopped error={error:?}"),
                );
            }
            if result.is_err() {
                let revision = running.applied.swap(0, Ordering::AcqRel);
                df_imagery::telemetry::ImageObserver::observe(
                    running.observer.as_ref(),
                    df_imagery::telemetry::ImageEvent::new(
                        df_imagery::telemetry::EventKind::Failed,
                        df_imagery::telemetry::EventContext {
                            config_revision: Some(revision),
                            ..Default::default()
                        },
                    )
                    .failed(df_imagery::telemetry::FailureKind::Worker),
                );
            }
            result
        });
        Ok(Self {
            imagery,
            shared,
            stop: Some(stop),
            supervisor: Some(supervisor),
            authority_driver: driver,
            shutdown_timeout: settings.shutdown_timeout,
        })
    }

    pub(super) fn diagnostics(&self) -> HostDiagnostics {
        HostDiagnostics(Arc::downgrade(&self.shared))
    }
    pub(super) fn imagery(&self) -> &Imagery {
        &self.imagery
    }
    pub(super) fn control(&self) -> &Arc<Mutex<ConfigControl>> {
        &self.shared.control
    }
    pub(super) async fn shutdown(mut self) -> Result<(), io::Error> {
        if let Some(stop) = self.stop.take() {
            let _closed = stop.send(());
        }
        let result = if let Some(mut supervisor) = self.supervisor.take() {
            match tokio::time::timeout(self.shutdown_timeout, &mut supervisor).await {
                Ok(result) => result
                    .map_err(|_| failure("image supervisor join failed"))
                    .and_then(|result| result),
                Err(_) => {
                    supervisor.abort();
                    let _joined = supervisor.await;
                    Err(failure("image supervisor shutdown deadline"))
                }
            }
        } else {
            Ok(())
        };
        self.authority_driver.close().await;
        result
    }
}
impl Drop for LocalImageHost {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _closed = stop.send(());
        }
        if let Some(supervisor) = self.supervisor.take() {
            supervisor.abort();
        }
    }
}

async fn apply_gate(shared: &Shared, config: &ImageConfig, revision: u64) -> Result<(), io::Error> {
    if config.accounting.mode != shared.accounting {
        return Err(failure("image accounting differs from mounted authority"));
    }
    let active = u64::from(config.scheduler.foreground_limit)
        + u64::from(config.scheduler.presence_limit)
        + u64::from(config.scheduler.background_limit)
        + u64::from(config.transforms.max_concurrency);
    let bounds = CapacityBounds::new(
        i64::try_from(active).map_err(|_| failure("image active bound invalid"))?,
        i64::from(config.scheduler.max_pending_jobs),
        i64::try_from(config.scheduler.max_pending_bytes)
            .map_err(|_| failure("image pending bound invalid"))?,
    )
    .map_err(|_| failure("image capacity invalid"))?;
    shared
        .operator
        .set_capacity_bounds(
            shared.capacity.as_str(),
            &bounds,
            Clock::now_ms(&NativeClock),
        )
        .await
        .map_err(|_| failure("image capacity acknowledgement unavailable"))?;
    shared
        .operator
        .set_execution_gate(
            shared.capacity.as_str(),
            revision,
            config.mode != GlobalMode::Off,
            Clock::now_ms(&NativeClock),
        )
        .await
        .map_err(|_| failure("image gate acknowledgement unavailable"))?;
    let previous = shared.applied.swap(revision, Ordering::AcqRel);
    if previous != revision {
        super::image_observability::configuration_changed(shared.observer.as_ref(), revision);
    }
    Ok(())
}
async fn supervise(
    shared: Arc<Shared>,
    mut stopped: oneshot::Receiver<()>,
) -> Result<(), io::Error> {
    loop {
        let snapshot = super::image_configuration::snapshot(&shared.control)?;
        configure_resources(&shared, &snapshot.config).await?;
        apply_gate(&shared, &snapshot.config, snapshot.provenance.revision).await?;
        // Workers claim stages through the database, so several can run at once. One worker runs a
        // slow image call to the end before it looks at the next stage; the foreground limit says
        // how many overlap.
        let worker_count = usize::try_from(snapshot.config.scheduler.foreground_limit)
            .unwrap_or(1)
            .clamp(1, 4);
        let mut handles = Vec::with_capacity(worker_count);
        let mut workers = tokio::task::JoinSet::new();
        for _ in 0..worker_count {
            let runtime = NativeImageRuntime {
                journal: shared
                    .journal
                    .with_config_revision(
                        snapshot.provenance.revision,
                        shared.accounting == AccountingMode::RecordOnly,
                    )
                    .map_err(|_| failure("image journal config refused"))?,
                execution: shared.execution.clone(),
                sources: shared.sources.clone(),
                publisher: shared.publisher.clone(),
                admission: shared.publisher.clone(),
                clock: NativeClock,
                ids: shared.ids.clone(),
                credentials: HostCredentials {
                    control: shared.control.clone(),
                    fixture: shared.fixture,
                    analysis_fixture: shared.analysis_fixture,
                    custody: shared.custody.clone(),
                    quota: shared.quota.clone(),
                },
                costs: shared.costs.clone(),
                cost_policy: GameplayImageCostPolicy,
                usage: shared.usage.clone(),
                telemetry: shared.observer.clone(),
                settings: RuntimeSettings {
                    config: snapshot.config.clone(),
                    active_config_revision: snapshot.provenance.revision,
                    capacity: shared.capacity.clone(),
                    poll_interval: shared.poll_interval,
                },
            };
            let (handle, future) = runtime.start();
            handles.push(handle);
            workers.spawn(future);
        }
        let mut finished = false;
        let changed = loop {
            tokio::select! {
                _=&mut stopped=>break false,
                result=workers.join_next(), if !finished=>{
                    let result = result.ok_or_else(||failure("image runtime owner lost worker"))?
                        .map_err(|_|failure("image runtime join failed"))?;
                    match result {
                        Err(RuntimeError::Disabled)
                        | Err(RuntimeError::Execution(df_imagery::execution::ExecutionError::Disabled)) => {
                            finished = true;
                        },
                        Err(RuntimeError::Execution(df_imagery::execution::ExecutionError::ConfigFenced { .. })) => {
                            shared.applied.store(0, Ordering::Release);
                            finished = true;
                        },
                        other => {
                            df_observe::record_audit_line(
                                "IMAGERY",
                                format!(
                                    "IMAGERY runtime_worker_ended error={:?}",
                                    other.as_ref().err()
                                ),
                            );
                            return Err(failure("image runtime stopped without shutdown"));
                        }
                    }
                }
                _=tokio::time::sleep(shared.poll_interval)=>{
                    let latest=super::image_configuration::snapshot(&shared.control)?;
                    if latest.provenance.revision!=snapshot.provenance.revision {
                        break true;
                    }
                }
            }
        };
        // Freeze new admissions and egress before draining owned old-revision work.
        shared.applied.store(0, Ordering::Release);
        shared
            .operator
            .set_execution_gate(
                shared.capacity.as_str(),
                snapshot.provenance.revision,
                false,
                Clock::now_ms(&NativeClock),
            )
            .await
            .map_err(|_| failure("image reconfiguration fence unavailable"))?;
        if finished {
            if !changed {
                return Ok(());
            }
            continue;
        }
        let drained = tokio::time::timeout(shared.shutdown_timeout, async {
            let mut results = Vec::with_capacity(handles.len());
            for handle in handles {
                handle
                    .shutdown()
                    .await
                    .map_err(|_| failure("image runtime shutdown failed"))?;
            }
            while let Some(joined) = workers.join_next().await {
                results.push(joined.map_err(|_| failure("image runtime join failed"))?);
            }
            Ok::<_, io::Error>(results)
        })
        .await;
        let results = match drained {
            Ok(results) => results?,
            Err(_) => {
                workers.abort_all();
                while workers.join_next().await.is_some() {}
                return Err(failure("image runtime shutdown deadline"));
            }
        };
        for result in results {
            if let Err(error) = result
                && !matches!(
                    error,
                    RuntimeError::Disabled
                        | RuntimeError::Execution(
                            df_imagery::execution::ExecutionError::ConfigFenced { .. }
                                | df_imagery::execution::ExecutionError::Disabled
                        )
                )
            {
                return Err(failure("image runtime ended with failure"));
            }
        }
        if !changed {
            return Ok(());
        }
    }
}
fn failure(message: &'static str) -> io::Error {
    io::Error::other(message)
}

struct AuthorityDriver(Option<JoinHandle<Result<(), tokio_postgres::Error>>>);
impl AuthorityDriver {
    async fn close(&mut self) {
        if let Some(driver) = self.0.take() {
            driver.abort();
            let _closed = driver.await;
        }
    }
}
impl Drop for AuthorityDriver {
    fn drop(&mut self) {
        if let Some(driver) = self.0.take() {
            driver.abort();
        }
    }
}

pub(super) fn delivery_id() -> Option<df_imagery::telemetry::DeliveryId> {
    let mut bytes = [0; 16];
    getrandom::getrandom(&mut bytes).ok()?;
    df_imagery::telemetry::DeliveryId::from_bytes(bytes)
}

#[derive(Clone)]
pub(super) struct HostDiagnostics(std::sync::Weak<Shared>);
impl HostDiagnostics {
    pub(super) fn grants(
        &self,
    ) -> Result<Option<super::image_sources::GrantDiagnostics>, RuntimeError> {
        self.0
            .upgrade()
            .map(|shared| shared.grants.diagnostics())
            .transpose()
    }
    pub(super) fn quota(
        &self,
    ) -> Result<Option<df_imagery::runtime::QuotaSummary>, df_imagery::runtime::QuotaRefusal> {
        self.0
            .upgrade()
            .map(|shared| shared.quota.summary())
            .transpose()
    }
    pub(super) fn health(&self) -> Option<(u64, bool, df_imagery::telemetry::ObserverHealth)> {
        self.0.upgrade().map(|shared| {
            (
                shared.applied.load(Ordering::Acquire),
                shared.running.load(Ordering::Acquire),
                shared.observer.health(),
            )
        })
    }
}

/// Deployment-only account -> secret environment-variable names. Values never enter ImageConfig.
fn credential_custody() -> Result<BTreeMap<String, String>, io::Error> {
    let text = match std::env::var("DF_IMAGERY_CREDENTIAL_MAP") {
        Ok(text) => text,
        Err(std::env::VarError::NotPresent) => return Ok(BTreeMap::new()),
        Err(_) => return Err(failure("image credential mapping invalid")),
    };
    if text.len() > 65_536 {
        return Err(failure("image credential mapping too large"));
    }
    let parsed = df_providers::parse_unique_json(
        text.as_bytes(),
        df_providers::TextSchemaLimits {
            maximum_schema_bytes: 65_536,
            maximum_depth: 2,
            maximum_nodes: 256,
        },
    )
    .map_err(|_| failure("image credential mapping invalid"))?;
    let pairs: BTreeMap<String, String> =
        serde_json::from_value(parsed).map_err(|_| failure("image credential mapping invalid"))?;
    if pairs.len() > 128 {
        return Err(failure("image credential mapping too large"));
    }
    for (account, name) in &pairs {
        df_imagery::AccountHandle::new(account)
            .map_err(|_| failure("image credential account invalid"))?;
        if name.is_empty()
            || name.len() > 256
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(failure("image credential environment name invalid"));
        }
    }
    Ok(pairs)
}

async fn adopt_startup_gate(
    operator: &PostgresImageExecution,
    capacity: &CapacityScope,
    snapshot: &df_imagery::ResolvedConfig,
) -> Result<u64, io::Error> {
    snapshot
        .config
        .validate()
        .map_err(|_| failure("image startup configuration invalid"))?;
    let mut expected = match operator.read_execution_gate(capacity.as_str()).await {
        Ok(gate) => gate.applied_revision,
        Err(df_imagery::execution::ExecutionError::ConfigFenced {
            current_revision: 0,
        }) => 0,
        Err(_) => return Err(failure("image startup gate read unavailable")),
    };
    // A provisional local edit may precede the first durable gate; advance monotonically
    // until its generation is covered. Losing CAS also retries, before any work is exposed.
    for _ in 0..3 {
        match operator
            .advance_execution_gate(
                capacity.as_str(),
                expected,
                snapshot.config.mode != GlobalMode::Off,
                Clock::now_ms(&NativeClock),
            )
            .await
        {
            Ok(gate) if gate.applied_revision >= snapshot.provenance.revision => {
                return Ok(gate.applied_revision);
            }
            Ok(gate) => expected = gate.applied_revision,
            Err(df_imagery::execution::ExecutionError::ConfigFenced { current_revision }) => {
                expected = current_revision
            }
            Err(_) => return Err(failure("image startup gate acknowledgement unavailable")),
        }
    }
    Err(failure("image startup gate adoption conflicted"))
}

async fn configure_resources(shared: &Shared, config: &ImageConfig) -> Result<(), io::Error> {
    let concurrency = usize::try_from(config.transforms.max_concurrency)
        .map_err(|_| failure("image concurrency invalid"))?;
    if concurrency == 0 || concurrency > tokio::sync::Semaphore::MAX_PERMITS {
        return Err(failure("image concurrency invalid"));
    }
    let _imports = tokio::time::timeout(
        shared.shutdown_timeout,
        shared.import_gate.clone().write_owned(),
    )
    .await
    .map_err(|_| failure("image import drain deadline"))?;
    let old_decode = shared
        .import_decode
        .lock()
        .map_err(|_| failure("image import limit unavailable"))?
        .clone();
    // A cancelled async importer can leave an already-started blocking validator holding
    // its old semaphore permit. Drain every captured permit before replacing the semaphore.
    let _old_decodes = super::image_imports::drain_decoders(
        old_decode,
        shared.import_concurrency.load(Ordering::Acquire),
        shared.shutdown_timeout,
    )
    .await?;
    // Existing ObjectStore calls retain their captured write bound; old runtime is already joined.
    shared
        .sources
        .objects
        .set_max_object_bytes(
            config
                .transforms
                .max_output_bytes
                .max(shared.sources.maximum_bytes),
        )
        .map_err(|_| failure("image object bound refused"))?;
    shared
        .publisher
        .set_decode_concurrency(concurrency)
        .map_err(|_| failure("image publisher concurrency refused"))?;
    *shared
        .import_decode
        .lock()
        .map_err(|_| failure("image import limit unavailable"))? =
        Arc::new(tokio::sync::Semaphore::new(concurrency));
    shared
        .import_concurrency
        .store(config.transforms.max_concurrency, Ordering::Release);
    Ok(())
}

#[derive(Clone)]
struct AuthorizedResults {
    inner: PostgresImagePublication,
    authority: Arc<CurrentImageGrants>,
}
impl df_imagery::publication::ImageResults for AuthorizedResults {
    async fn find_ready_artifact(
        &self,
        request: &df_imagery::ImageRequest,
        config: &df_imagery::VersionedSnapshot,
        now_ms: i64,
    ) -> Result<
        Option<df_imagery::publication::ReadyArtifact>,
        df_imagery::publication::PublicationError,
    > {
        self.inner
            .find_ready_artifact_with_authority(request, config, now_ms, self.authority.as_ref())
            .await
    }

    async fn read_artifact(
        &self,
        owner: &ImageOwner,
        job: &df_imagery::ImageJobId,
        now_ms: i64,
    ) -> Result<
        Option<df_imagery::publication::ReadyArtifact>,
        df_imagery::publication::PublicationError,
    > {
        self.inner
            .read_artifact_with_authority(owner, job, now_ms, self.authority.as_ref())
            .await
    }

    async fn read_ready_artifact(
        &self,
        owner: &ImageOwner,
        asset: &df_assets::AssetId,
        now_ms: i64,
    ) -> Result<
        Option<df_imagery::publication::ReadyArtifact>,
        df_imagery::publication::PublicationError,
    > {
        self.inner
            .read_campaign_artifact_with_authority(owner, asset, now_ms, self.authority.as_ref())
            .await
    }

    async fn read_result(
        &self,
        owner: &ImageOwner,
        job: &df_imagery::ImageJobId,
    ) -> Result<
        Option<df_imagery::publication::PublishedImage>,
        df_imagery::publication::PublicationError,
    > {
        self.inner
            .read_reusable_result_with_authority(
                owner,
                job,
                Clock::now_ms(&NativeClock),
                self.authority.as_ref(),
            )
            .await
    }
    async fn read_reusable_result(
        &self,
        owner: &ImageOwner,
        job: &df_imagery::ImageJobId,
        now_ms: i64,
    ) -> Result<
        Option<df_imagery::publication::PublishedImage>,
        df_imagery::publication::PublicationError,
    > {
        self.inner
            .read_reusable_result_with_authority(owner, job, now_ms, self.authority.as_ref())
            .await
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    #[test]
    fn missing_credentials_are_preflight_unavailable_but_malformed_custody_is_not_retried() {
        assert!(matches!(
            secret_from(&["first", "second"], |_| Err(
                std::env::VarError::NotPresent
            )),
            Err(RuntimeError::CredentialUnavailable)
        ));
        assert!(matches!(
            secret_from(&["first"], |_| Ok(String::new())),
            Err(RuntimeError::CredentialUnavailable)
        ));
        let mut reads = 0;
        let result = secret_from(&["first", "second"], |_| {
            reads += 1;
            Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                "fixture-invalid-encoding",
            )))
        });
        assert!(matches!(result, Err(RuntimeError::InvalidConfiguration)));
        assert_eq!(reads, 1);
    }

    #[test]
    fn generic_routes_do_not_spend_creator_credentials_and_explicit_aliases_keep_legacy_fallback() {
        for (backend, alias, dedicated, general) in [
            (
                df_imagery::BackendId::Fal,
                "creator-fal",
                "DF_GAMEPLAY_CREATOR_FAL_KEY",
                "FAL_KEY",
            ),
            (
                df_imagery::BackendId::Bfl,
                "creator-bfl",
                "DF_GAMEPLAY_CREATOR_BFL_KEY",
                "BFL_API_KEY",
            ),
        ] {
            let mut available = BTreeMap::new();
            available.insert(dedicated, "fixture-dedicated");
            let resolve = |account: &str, values: &BTreeMap<&str, &str>| {
                secret_from(backend_secret_names(backend, account), |name| {
                    values
                        .get(name)
                        .map(|value| (*value).to_owned())
                        .ok_or(std::env::VarError::NotPresent)
                })
            };
            assert!(matches!(
                resolve("generic-account", &available),
                Err(RuntimeError::CredentialUnavailable)
            ));
            assert_eq!(resolve(alias, &available).unwrap(), "fixture-dedicated");
            available.insert(general, "fixture-general");
            assert_eq!(
                resolve("generic-account", &available).unwrap(),
                "fixture-general"
            );
            assert_eq!(resolve(alias, &available).unwrap(), "fixture-dedicated");
            available.remove(dedicated);
            assert_eq!(resolve(alias, &available).unwrap(), "fixture-general");
        }
    }
}
