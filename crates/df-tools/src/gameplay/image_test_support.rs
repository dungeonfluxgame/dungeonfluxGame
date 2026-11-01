//! Disposable actual local image hosts for consumer tests; no completed view or provider is forged.
#![cfg(test)]
use super::{
    cost::port::MeteringPort,
    image_host::{LocalHostPorts, LocalHostSettings, LocalImageHost},
    imagery::{ImageFailure, ImageKey, Imagery, ReferenceImage, ReferenceStore},
    resources::Hub,
};
use df_imagery::{BackendId, ConfigControl, ImageContentRating, RuntimePatch};
use std::{
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_postgres::{Client, Config, NoTls};
struct Sink;
impl df_observe::DiagnosticSink for Sink {
    fn emit(
        &self,
        _record: df_observe::DiagnosticRecord,
    ) -> Result<(), df_observe::TelemetryError> {
        Ok(())
    }
}
async fn connection(
    config: &Config,
) -> (
    Client,
    tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
) {
    let (client, driver) = config.connect(NoTls).await.unwrap();
    (client, tokio::spawn(driver))
}
/// Existing ignored operator entrypoints, never selected by ordinary loopback tests.
pub(in crate::gameplay) enum LiveImageOptIn {
    Plan,
    FluxPlateSequence,
}
enum Transport {
    Captured(SocketAddr),
    Live {
        opt_in: LiveImageOptIn,
        rating: ImageContentRating,
    },
}
/// The real native host, canonical local grants, catalog, objects and caller-supplied cost book.
pub(in crate::gameplay) struct ImageHostFixture {
    host: Option<LocalImageHost>,
    settings: LocalHostSettings,
    costs: MeteringPort,
    observer: Arc<df_imagery::telemetry::SharedImageObserver>,
    hub: Hub,
    admin: Client,
    admin_driver: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
    database: Config,
    name: String,
    root: PathBuf,
    player: [u8; 32],
    display: [u8; 32],
}
impl ImageHostFixture {
    /// Missing PG skips only when REQUIRE_SCRATCH_PG is absent; every other setup failure fails.
    pub(in crate::gameplay) async fn open(
        address: SocketAddr,
        patch: RuntimePatch,
        costs: MeteringPort,
    ) -> Option<Self> {
        Self::open_with_hub(address, patch, costs, Hub::new()).await
    }
    /// Preserves the caller's actual shared resource hub and cost-book identity across restart.
    pub(in crate::gameplay) async fn open_with_hub(
        address: SocketAddr,
        patch: RuntimePatch,
        costs: MeteringPort,
        hub: Hub,
    ) -> Option<Self> {
        assert!(address.ip().is_loopback());
        Self::open_host(
            patch,
            costs,
            hub,
            Transport::Captured(address),
            Self::quiet_observer(),
        )
        .await
    }
    /// Captures the actual shared observation adapter across startup and every native restart.
    pub(in crate::gameplay) async fn open_with_observer(
        address: SocketAddr,
        patch: RuntimePatch,
        costs: MeteringPort,
        observer: Arc<df_imagery::telemetry::SharedImageObserver>,
    ) -> Option<Self> {
        assert!(address.ip().is_loopback());
        Self::open_host(
            patch,
            costs,
            Hub::new(),
            Transport::Captured(address),
            observer,
        )
        .await
    }
    /// Only the existing ignored operator tests opt in to production named environment custody.
    pub(in crate::gameplay) async fn open_live(
        patch: RuntimePatch,
        costs: MeteringPort,
        rating: ImageContentRating,
        opt_in: LiveImageOptIn,
    ) -> Option<Self> {
        Self::open_host(
            patch,
            costs,
            Hub::new(),
            Transport::Live { opt_in, rating },
            Self::quiet_observer(),
        )
        .await
    }
    async fn open_host(
        mut patch: RuntimePatch,
        costs: MeteringPort,
        hub: Hub,
        transport: Transport,
        observer: Arc<df_imagery::telemetry::SharedImageObserver>,
    ) -> Option<Self> {
        let (fixture, rating) = match transport {
            Transport::Captured(address) => (Some(address), ImageContentRating::Restrained),
            Transport::Live { opt_in, rating } => {
                require_live_configuration(&patch, opt_in);
                (None, rating)
            }
        };
        let Some(port) = std::env::var_os("DUNGEONFLUX_FINANCE_PG_PORT") else {
            assert!(
                std::env::var_os("DUNGEONFLUX_REQUIRE_SCRATCH_PG").is_none(),
                "required scratch PG missing"
            );
            return None;
        };
        assert_eq!(port, "55471");
        if let Some(user) = std::env::var_os("DUNGEONFLUX_FINANCE_PG_USER") {
            assert_eq!(user, "postgres");
        }
        let legacy = super::startup_configuration::ImagerySettings::On(
            super::startup_configuration::ImageryConfiguration {
                model: df_providers::ImageEditModel::Gpt25Flare,
                quality: df_providers::ImageQuality::Medium,
                profile: super::startup_configuration::ImageryProfile::Production,
                interval: Duration::from_secs(20),
                deadline: Duration::from_secs(170),
                cache_dir: PathBuf::new(),
            },
        );
        let defaults = super::image_configuration::legacy_profile_defaults(&legacy, false).unwrap();
        if let Some(address) = fixture {
            let endpoint = df_imagery::ApiEndpoint::new(&format!("http://{address}")).unwrap();
            for backend in [BackendId::OpenAi, BackendId::Fal, BackendId::Bfl] {
                let index = backend.index();
                if patch.backend_enabled[index].unwrap_or(defaults.backend(backend).enabled)
                    && patch.backend_endpoint[index].is_none()
                {
                    patch.backend_endpoint[index] = Some(Some(endpoint.clone()));
                }
            }
        }
        let control = ConfigControl::build_with_defaults(defaults, "", &[], &patch, 1).unwrap();
        let mut admin_config = Config::new();
        admin_config
            .host("127.0.0.1")
            .port(55471)
            .user("postgres")
            .dbname("postgres")
            .connect_timeout(Duration::from_secs(5));
        let (admin, admin_driver) = connection(&admin_config).await;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let name = format!(
            "df_gameplay_demo_images_{}_{}_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        admin
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .await
            .unwrap();
        let mut registry=std::fs::OpenOptions::new().create(true).append(true).open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/tmp/imagery-finish-20261029/funded-adapter/image-host-owned-databases.txt")).unwrap();
        writeln!(registry, "pid={} database={name}", std::process::id()).unwrap();
        let mut database = admin_config.clone();
        database.dbname(&name);
        let (mut client, driver) = connection(&database).await;
        let checkpoint = super::model::initial().unwrap();
        let mut fence = [0; 16];
        let mut player = [0; 32];
        let mut display = [0; 32];
        super::model::fill_os_entropy(&mut fence).unwrap();
        super::model::fill_os_entropy(&mut player).unwrap();
        super::model::fill_os_entropy(&mut display).unwrap();
        df_persistence::local_demo_scope::initialize(
            &mut client,
            &checkpoint,
            fence,
            player,
            display,
            super::live_codec_limits(),
        )
        .await
        .unwrap();
        let tx = client.transaction().await.unwrap();
        df_persistence::imagery_migrations::migrate_local_in(&tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        drop(client);
        driver.abort();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/imagery-finish-20261029/funded-adapter/image-host-fixtures")
            .join(&name);
        std::fs::create_dir_all(root.join("reviewed")).unwrap();
        let settings = LocalHostSettings {
            worker_database: database.clone(),
            operator_database: database.clone(),
            fixture,
            analysis_fixture: None,
            adventure: None,
            transport: df_persistence::postgres_tls::PostgresTransport::PlaintextLoopbackOnly,
            connections: 2,
            max_current_grants: 512,
            object_root: root.join("objects"),
            environment: df_assets::ObjectEnvironment::new("image-test").unwrap(),
            catalog: df_assets::CatalogConfig::default(),
            reviewed_maximum_bytes: 16 * 1024 * 1024,
            annotation_maximum_bytes: 1024 * 1024,
            control: Arc::new(Mutex::new(control)),
            run_grant: df_persistence::imagery_identity::LocalRunGrant {
                session: *checkpoint.basis().session.as_bytes(),
                run: *checkpoint.basis().run.as_bytes(),
                fence,
                bootstrap_credential: player,
            },
            source_revision: 1,
            rating,
            poll_interval: Duration::from_millis(2),
            shutdown_timeout: Duration::from_secs(5),
            database_timeout: Duration::from_secs(5),
            usage_limits: df_imagery::runtime::UsageProjectionLimits {
                max_attempts: 64,
                max_observations: 128,
                max_unit_lines: 32,
                max_word_bytes: 128,
            },
        };
        let host = LocalImageHost::open(
            settings.clone(),
            Self::ports(&costs, &observer),
            hub.clone(),
        )
        .await
        .unwrap();
        Some(Self {
            host: Some(host),
            settings,
            costs,
            observer,
            hub,
            admin,
            admin_driver,
            database,
            name,
            root,
            player,
            display,
        })
    }
    fn quiet_observer() -> Arc<df_imagery::telemetry::SharedImageObserver> {
        Arc::new(df_imagery::telemetry::SharedImageObserver::new(Arc::new(
            Sink,
        )))
    }
    fn ports(
        costs: &MeteringPort,
        observer: &Arc<df_imagery::telemetry::SharedImageObserver>,
    ) -> LocalHostPorts {
        LocalHostPorts {
            observer: observer.clone(),
            costs: costs.clone(),
        }
    }
    pub(in crate::gameplay) fn imagery(&self) -> &Imagery {
        self.host.as_ref().unwrap().imagery()
    }
    /// Resolves the actual persisted guest account from its current local player credential.
    pub(in crate::gameplay) async fn creator(
        &self,
        credential: [u8; 32],
    ) -> Result<Imagery, ImageFailure> {
        self.imagery().for_local_creator(credential).await
    }
    /// Database rows are bounded index hints; each returned view passes current public service auth.
    pub(in crate::gameplay) async fn views_for_purpose(
        &self,
        purpose: df_imagery::Purpose,
    ) -> Vec<df_imagery::ImageDemandView> {
        let context = self.imagery().consumer_context().await.unwrap();
        let owner = context.owner;
        let (client, driver) = connection(&self.database).await;
        let rows = client.query(
            "SELECT o.operation_id FROM imagery.image_operation o JOIN imagery.image_job j ON j.owner_scope=o.owner_scope AND j.job_id=o.job_id WHERE o.owner_scope=$1 AND get_byte(j.intent_bytes,1)=$2 ORDER BY o.created_at_ms,o.operation_id LIMIT 128",
            &[&df_imagery::owner_scope_bytes(&owner), &i32::try_from(purpose.index()).unwrap()],
        ).await.unwrap();
        drop(client);
        driver.await.unwrap().unwrap();
        let mut views = Vec::with_capacity(rows.len());
        for row in rows {
            let operation = df_types::OperationId::from_bytes(&row.get::<_, Vec<u8>>(0)).unwrap();
            let current = self.imagery().consumer_context().await.unwrap();
            assert_eq!(current.owner, owner);
            let handle = df_imagery::OperationHandle {
                owner: owner.clone(),
                operation,
            };
            views.push(
                current
                    .service
                    .read(&current.principal, &handle)
                    .await
                    .unwrap(),
            );
        }
        views
    }
    pub(in crate::gameplay) fn control(&self) -> &Arc<Mutex<ConfigControl>> {
        self.host.as_ref().unwrap().control()
    }
    pub(in crate::gameplay) fn metering_port(&self) -> &MeteringPort {
        &self.costs
    }
    pub(in crate::gameplay) fn database(&self) -> &Config {
        &self.database
    }
    pub(in crate::gameplay) fn player_credential(&self) -> [u8; 32] {
        self.player
    }
    pub(in crate::gameplay) fn display_credential(&self) -> [u8; 32] {
        self.display
    }
    pub(in crate::gameplay) fn root(&self) -> &Path {
        &self.root
    }
    pub(in crate::gameplay) fn reviewed_root(&self) -> PathBuf {
        self.root.join("reviewed")
    }
    /// Reloads a real reviewed photographic file and admits it through the actual catalog lifecycle.
    pub(in crate::gameplay) async fn import_reference(
        &self,
        stem: &str,
    ) -> Result<(ImageKey, ReferenceImage), ImageFailure> {
        let reference = ReferenceStore::new(self.reviewed_root())
            .load(stem)
            .map_err(|_| ImageFailure::InvalidRequest)?;
        let key = self
            .imagery()
            .register_reviewed("consumer-fixture-reviewed", reference)
            .await?;
        let reference = self.imagery().reference(&key).await?;
        Ok((key, reference))
    }
    /// Makes an owned authored input file, then uses the production reload and catalog importer.
    pub(in crate::gameplay) async fn import_bytes(
        &self,
        format: df_providers::ImageFormat,
        bytes: bytes::Bytes,
    ) -> Result<(ImageKey, ReferenceImage), ImageFailure> {
        use sha2::{Digest, Sha256};
        let extension = match format {
            df_providers::ImageFormat::Png => "png",
            df_providers::ImageFormat::Jpeg => "jpg",
            df_providers::ImageFormat::Webp => "webp",
        };
        let root = self.root.join("authored");
        std::fs::create_dir_all(&root).map_err(|_| ImageFailure::Interrupted)?;
        let file = format!("{:x}.{extension}", Sha256::digest(&bytes));
        std::fs::write(root.join(&file), &bytes).map_err(|_| ImageFailure::Interrupted)?;
        let reference = ReferenceImage::from_trusted(
            format,
            bytes,
            super::imagery::TrustedReferenceOrigin::AuthoredPainting {
                root,
                painting: format!("{}{}", super::home_theme::PAINTING_PREFIX, file),
            },
        );
        let key = self
            .imagery()
            .register_reviewed("consumer-fixture-authored", reference)
            .await?;
        let reference = self.imagery().reference(&key).await?;
        Ok((key, reference))
    }
    /// Restarts only this native host; the canonical run grant, database, objects and Book persist.
    pub(in crate::gameplay) async fn restart(&mut self) {
        self.host.take().unwrap().shutdown().await.unwrap();
        self.host = Some(
            LocalImageHost::open(
                self.settings.clone(),
                Self::ports(&self.costs, &self.observer),
                self.hub.clone(),
            )
            .await
            .unwrap(),
        );
    }
    /// Joins the actual failed worker before the test removes its injected database fault.
    pub(in crate::gameplay) async fn stop_after_worker_failure(&mut self) {
        assert!(self.host.take().unwrap().shutdown().await.is_err());
    }
    /// Reopens the same stored authority and Book after the failed worker has been joined.
    pub(in crate::gameplay) async fn restart_stopped(&mut self) {
        assert!(self.host.is_none());
        self.host = Some(
            LocalImageHost::open(
                self.settings.clone(),
                Self::ports(&self.costs, &self.observer),
                self.hub.clone(),
            )
            .await
            .unwrap(),
        );
    }
    /// Captures one explicit second loopback origin through the actual host startup gate.
    pub(in crate::gameplay) async fn restart_with_analysis_fixture(&mut self, address: SocketAddr) {
        assert!(self.settings.fixture.is_some());
        assert!(address.ip().is_loopback() && address.port() != 0);
        self.settings.analysis_fixture = Some(address);
        self.restart().await;
    }
    /// Exercises actual missing-key custody only in the explicit isolated negative-test child.
    pub(in crate::gameplay) async fn restart_without_fixture_credentials(&mut self) {
        assert_eq!(
            std::env::var("DF_TEST_MISSING_IMAGE_CREDENTIAL").as_deref(),
            Ok("on")
        );
        for name in [
            "OPENAI_API_KEY",
            "FAL_KEY",
            "DF_FAL_KEY",
            "BFL_API_KEY",
            "BFL_KEY",
            "DF_GAMEPLAY_CREATOR_JEV_API_KEY",
            "DF_GAMEPLAY_CREATOR_OPENAI_KEY",
            "DF_GAMEPLAY_CREATOR_FAL_KEY",
            "DF_GAMEPLAY_CREATOR_BFL_KEY",
            "DF_IMAGERY_CREDENTIAL_MAP",
            "DF_IMAGERY_CONFIG",
        ] {
            assert!(
                std::env::var_os(name).is_none(),
                "negative credential fixture requires isolated empty custody"
            );
        }
        let address = self
            .settings
            .fixture
            .expect("negative fixture requires captured loopback");
        assert!(address.ip().is_loopback() && address.port() != 0);
        let config = self.control().lock().unwrap().read_effective().config;
        assert!(!config.backend(BackendId::CodexCli).enabled);
        let endpoint = format!("http://{address}");
        for backend in [BackendId::OpenAi, BackendId::Fal, BackendId::Bfl] {
            let route = config.backend(backend);
            if route.enabled {
                assert_eq!(
                    route.endpoint.as_ref().map(|endpoint| endpoint.as_str()),
                    Some(endpoint.as_str())
                );
            }
        }
        assert!(config.analysis.classification.is_none());
        assert!(config.analysis.inspection.is_none());
        assert!(config.analysis.head_locations.is_none());
        assert!(config.analysis.segmentation.is_none());
        assert!(config.analysis.staging_identity.is_none());
        assert!(config.analysis.staging_facing.is_none());
        self.settings.fixture = None;
        self.settings.analysis_fixture = None;
        self.restart().await;
    }
    /// Call after dropping consumer contexts/references so no pool connection keeps its DB alive.
    pub(in crate::gameplay) async fn close(mut self) {
        self.host.take().unwrap().shutdown().await.unwrap();
        assert!(self.name.starts_with("df_gameplay_demo_images_"));
        self.admin
            .batch_execute(&format!("DROP DATABASE {}", self.name))
            .await
            .unwrap();
        drop(self.admin);
        self.admin_driver.abort();
        let root = self.root.canonicalize().unwrap();
        let permitted = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/imagery-finish-20261029/funded-adapter/image-host-fixtures")
            .canonicalize()
            .unwrap();
        assert!(root.starts_with(&permitted) && root != permitted);
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn require_live_configuration(patch: &RuntimePatch, opt_in: LiveImageOptIn) {
    let required: &[&str] = match opt_in {
        LiveImageOptIn::Plan => &["DF_L01_PLAN", "DF_L01_OUT"],
        LiveImageOptIn::FluxPlateSequence => &["DF_FAL_KEY"],
    };
    for name in required {
        assert!(
            std::env::var_os(name).is_some_and(|value| !value.is_empty()),
            "operator live input missing"
        );
    }
    assert!(
        std::env::var_os("DUNGEONFLUX_FINANCE_PG_PORT").is_some(),
        "live test requires real local bootstrap PG"
    );
    assert_eq!(patch.mode, Some(df_imagery::GlobalMode::Live));
    assert_eq!(
        patch.accounting_mode,
        Some(df_imagery::AccountingMode::RecordOnly)
    );
    assert_eq!(patch.retry_max_attempts, Some(1));
    assert!(patch.retry_deadline_secs.is_some_and(|seconds| seconds > 0));
    let mut enabled = false;
    for backend in [
        BackendId::OpenAi,
        BackendId::Fal,
        BackendId::Bfl,
        BackendId::CodexCli,
    ] {
        let index = backend.index();
        let selected = patch.backend_enabled[index].expect("live backend gates must be explicit");
        if !selected {
            continue;
        }
        enabled = true;
        assert_ne!(backend, BackendId::CodexCli);
        assert!(
            patch.backend_account[index].is_some(),
            "live account must be explicit"
        );
        assert!(
            matches!(patch.backend_endpoint[index], Some(Some(_))),
            "live endpoint must be explicit"
        );
        assert!(
            matches!(patch.backend_model[index], Some(Some(_)))
                || (matches!(patch.backend_generation_model[index], Some(Some(_)))
                    && matches!(patch.backend_edit_model[index], Some(Some(_)))),
            "live model must be explicit"
        );
    }
    assert!(enabled);
}
