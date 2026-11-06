//! Real owned PostgreSQL and native object-store acceptance through the one ImageService.
use df_assets::{
    AssetVariant, CatalogConfig, CatalogStore, Clock, Deadline, IdSource, KeyLayout,
    NativeFileStore, ObjectEnvironment, ObjectStore,
};
use df_imagery::cost::*;
use df_imagery::execution::*;
use df_imagery::runtime::*;
use df_imagery::*;
use df_persistence::imagery_execution::PostgresImageExecution;
use df_persistence::imagery_publication::{
    IMAGERY_PUBLICATION_MIGRATION_SQL, PostgresImagePublication,
};
use df_persistence::imagery_publication::{
    NativeImagePublisher, RuntimePublicationPorts, RuntimePublicationSettings,
};
use df_persistence::imagery_runtime::*;
use df_persistence::media_catalog::PostgresMediaCatalog;
use df_persistence::postgres_tls::PostgresTransport;
use df_types::{AccountId, OperationId};
use std::ops::{Deref, DerefMut};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_postgres::{Client, Config, NoTls};
const NOW: i64 = 1_700_000_000_000;
const FINANCE_MIGRATIONS: [(&str, &str); 6] = [
    (
        "0020",
        include_str!("../migrations/0020_finance_journal.sql"),
    ),
    (
        "0021",
        include_str!("../migrations/0021_finance_funding.sql"),
    ),
    (
        "0022",
        include_str!("../migrations/0022_finance_admission.sql"),
    ),
    (
        "0023",
        include_str!("../migrations/0023_finance_refunds_disputes.sql"),
    ),
    (
        "0024",
        include_str!("../migrations/0024_finance_boundary.sql"),
    ),
    (
        "0025",
        include_str!("../migrations/0025_finance_refunds_disputes_boundary.sql"),
    ),
];

const MEDIA_MIGRATIONS: [(&str, &str); 5] = [
    ("0010", include_str!("../migrations/0010_media_catalog.sql")),
    (
        "0011",
        include_str!("../migrations/0011_character_library.sql"),
    ),
    ("0012", include_str!("../migrations/0012_media_exports.sql")),
    (
        "0013",
        include_str!("../migrations/0013_recovery_journal.sql"),
    ),
    ("0014", include_str!("../migrations/0014_media_sweep.sql")),
];

const MIGRATION_0038: &str = include_str!("../migrations/0038_imagery_operations.sql");
const MIGRATION_0039: &str = include_str!("../migrations/0039_imagery_admission_bindings.sql");
const MIGRATION_0040: &str = include_str!("../migrations/0040_imagery_execution.sql");

struct Connection {
    client: Client,
    task: tokio::task::JoinHandle<()>,
}
impl Deref for Connection {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.client
    }
}
impl DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut Client {
        &mut self.client
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn connect(config: &Config) -> Connection {
    let (client, connection) = config.connect(NoTls).await.expect("owned PG55471");
    let task = tokio::spawn(async move {
        // A dropped database ends its own connection with an administrator error; only the test sees it.
        if let Err(error) = connection.await {
            eprintln!("PG connection closed: {error}");
        }
    });
    Connection { client, task }
}
struct Scratch {
    config: Config,
    admin: Config,
}
impl Scratch {
    async fn open() -> Self {
        assert_eq!(
            std::env::var("DUNGEONFLUX_REQUIRE_SCRATCH_PG").as_deref(),
            Ok("1")
        );
        let mut admin = Config::new();
        admin
            .host("127.0.0.1")
            .port(55471)
            .user("postgres")
            .dbname("postgres");
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database = format!(
            "df_runtime_{}_{}_{}",
            std::process::id(),
            unique,
            SERIAL.fetch_add(1, Ordering::SeqCst)
        );
        let migration_admin = connect(&admin).await;
        migration_admin
            .query_one(
                "SELECT pg_advisory_lock(hashtext('df_imagery_test_role_migrations'))",
                &[],
            )
            .await
            .unwrap();
        migration_admin
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        let mut config = admin.clone();
        config.dbname(&database);
        let client = connect(&config).await;
        let mut client = client;
        let transaction = client.transaction().await.unwrap();
        df_persistence::imagery_migrations::migrate_local_in(&transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        migration_admin
            .query_one(
                "SELECT pg_advisory_unlock(hashtext('df_imagery_test_role_migrations'))",
                &[],
            )
            .await
            .unwrap();
        Self { config, admin }
    }
    async fn finish(self) {
        let database = self.config.get_dbname().unwrap();
        connect(&self.admin)
            .await
            .batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)"))
            .await
            .unwrap();
    }
}

#[derive(Clone)]
struct TestIds(Arc<AtomicU64>);
impl IdSource for TestIds {
    fn next_id_bytes(&self) -> [u8; 16] {
        let number = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        let mut bytes = [0u8; 16];
        bytes[8..].copy_from_slice(&number.to_be_bytes());
        bytes
    }
}
#[derive(Clone)]
struct TestClock;
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        NOW
    }
}
struct TestAuthority(ImageOwner);
impl ImageAuthorizer for TestAuthority {
    type Principal = ();
    fn authorize_request(&self, _: &(), request: &ImageRequest) -> Result<(), AuthDenied> {
        if request.identity.owner == self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_owner(
        &self,
        _: &(),
        owner: &ImageOwner,
        _: ServiceAction,
    ) -> Result<(), AuthDenied> {
        if *owner == self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_stored(
        &self,
        _: &(),
        _: ServiceAction,
        view: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        if view.job.owner == self.0 && *stored.owner == self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
}
struct NoSources;
impl ImageSources for NoSources {
    async fn read_source(
        &self,
        _: &ImageOwner,
        _: &SourceRef,
    ) -> Result<AuthorizedImageSource, RuntimeError> {
        Err(RuntimeError::SourceDenied)
    }
}
struct NoCredentials;
impl ImageCredentials for NoCredentials {
    fn credential(&self, _: BackendId, _: &AccountHandle) -> Result<String, RuntimeError> {
        panic!("local transform requested supplier credentials")
    }
}
struct NoCostPolicy;
impl ImageRuntimeCostPolicy for NoCostPolicy {
    fn route(
        &self,
        _: &ImageJob,
        _: &ImageConfig,
        _: BackendId,
        _: &str,
    ) -> Result<CostRoute, RuntimeError> {
        panic!("local transform requested billed route")
    }
}
struct NoCost;
impl ImageCostProjection for NoCost {
    fn start_pinned<'a>(
        &'a self,
        _: &'a str,
        _: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>> {
        Box::pin(async { panic!("local transform opened billed call") })
    }
    fn correct<'a>(
        &'a self,
        _: &'a str,
        _: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>> {
        Box::pin(async { panic!("local transform reported billed usage") })
    }
    fn record_unsent<'a>(&'a self, _: &'a str) -> CostFuture<'a, Result<PinnedUnsent, CostError>> {
        Box::pin(async { panic!("local transform released finance") })
    }
    fn is_durable<'a>(
        &'a self,
        _: &'a str,
        _: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>> {
        Box::pin(async { panic!("local transform read billed usage") })
    }
}
#[derive(Default)]
struct RecordingDiagnostics(std::sync::Mutex<Vec<df_observe::DiagnosticRecord>>);
impl df_observe::DiagnosticSink for RecordingDiagnostics {
    fn emit(&self, record: df_observe::DiagnosticRecord) -> Result<(), df_observe::TelemetryError> {
        let refuse = record.event_name == "image.completed";
        self.0.lock().unwrap().push(record);
        if refuse {
            Err(df_observe::TelemetryError::SinkUnavailable)
        } else {
            Ok(())
        }
    }
}

struct TestTelemetry;
impl ImageRuntimeTelemetry for TestTelemetry {
    fn record(
        &self,
        _: &ImageJob,
        _: &StageView,
        _: Option<df_imagery::execution::DispatchId>,
        _: RuntimePhase,
        _: i64,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
}
fn owner() -> ImageOwner {
    ImageOwner::Draft {
        account: AccountId::from_bytes(&[11; 16]).unwrap(),
    }
}
fn request() -> ImageRequest {
    let delivered = RasterOutputSpec::new(
        Dimensions::new(2, 3).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Png,
    )
    .unwrap();
    ImageRequest {
        identity: OperationIdentity {
            owner: owner(),
            operation: OperationId::from_bytes(&[12; 16]).unwrap(),
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
                    3,
                    Rgb {
                        red: 32,
                        green: 64,
                        blue: 96,
                    },
                )
                .unwrap(),
            ),
            steps: vec![],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience: Audience::OwnerPrivate(AccountId::from_bytes(&[11; 16]).unwrap()),
        expected_source_revision: 1,
        output: OutputProfile::local(delivered),
    }
}

#[tokio::test]
async fn one_service_reaches_real_durable_ready_and_replays_after_restart_without_finance() {
    run_local_pipeline(false).await;
}

#[tokio::test]
async fn zero_backoff_disabled_oldest_does_not_starve_eligible_local_work() {
    run_local_pipeline(true).await;
}

async fn run_local_pipeline(older_supplier: bool) {
    let scratch = Scratch::open().await;
    let config = scratch.config.clone();
    let transport = PostgresTransport::PlaintextLoopbackOnly;
    let client = connect(&config).await;
    client.batch_execute("CREATE TABLE IF NOT EXISTS imagery.schema_migration(version integer PRIMARY KEY,applied_at timestamptz NOT NULL DEFAULT now()); INSERT INTO imagery.schema_migration(version) VALUES(38),(40) ON CONFLICT DO NOTHING").await.unwrap();
    let ids = TestIds(Arc::new(AtomicU64::new(100)));
    let execution = PostgresImageExecution::open(config.clone(), transport.clone(), 2)
        .await
        .unwrap();
    let scope = CapacityScope::new("runtime/local").unwrap();
    execution
        .set_capacity_bounds(
            scope.as_str(),
            &CapacityBounds::new(2, 4, 4_194_304).unwrap(),
            NOW,
        )
        .await
        .unwrap();
    execution
        .set_execution_gate(scope.as_str(), 1, true, NOW)
        .await
        .unwrap();
    let journal = PostgresRuntimeJournal::open_existing(
        config.clone(),
        transport.clone(),
        2,
        Arc::new(ids.clone()),
        scope.clone(),
        1,
        true,
    )
    .unwrap();
    let publication = PostgresImagePublication::open_existing(config.clone(), transport.clone(), 2)
        .await
        .unwrap();
    let diagnostic_sink = Arc::new(RecordingDiagnostics::default());
    let observer = Arc::new(df_imagery::telemetry::SharedImageObserver::new(
        diagnostic_sink.clone(),
    ));
    observer.set_level(df_imagery::telemetry::ImageLogLevel::Debug);
    let mut image_config = ImageConfig::defaults().unwrap();
    image_config.mode = if older_supplier {
        GlobalMode::Live
    } else {
        GlobalMode::LocalOnly
    };
    image_config.retry.backoff_secs = 0;
    let service = ImageService::new(
        journal.clone(),
        image_config.clone(),
        TestClock,
        ids.clone(),
        TestAuthority(owner()),
    )
    .unwrap()
    .with_results(publication.clone())
    .with_observations(
        PostgresImageObservations::open_existing(config.clone(), transport.clone(), 2).unwrap(),
    )
    .with_observer(observer.clone())
    .with_config_revision(1)
    .unwrap();
    let blocked = if older_supplier {
        let mut demand = request();
        demand.identity.operation = OperationId::from_bytes(&[13; 16]).unwrap();
        demand.operation = ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("fixture must remain unsent", 1).unwrap(),
            references: vec![],
        });
        demand.output = OutputProfile::provider(
            Dimensions::new(1024, 1024).unwrap(),
            demand.output.delivered(),
        );
        demand.identity.fingerprint = service.canonical_fingerprint(&demand).unwrap();
        match service.request(&(), &demand).await.unwrap() {
            RequestOutcome::Recorded(view) => Some(view),
            other => panic!("expected queued supplier demand: {other:?}"),
        }
    } else {
        None
    };
    let mut request = request();
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    let accepted = match service.request(&(), &request).await.unwrap() {
        RequestOutcome::Recorded(view) => view,
        other => panic!("{other:?}"),
    };
    assert!(accepted.result.is_none());
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_execution_stage", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        if older_supplier { 2 } else { 1 }
    );
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/runtime-test")
        .join(format!("{}", config.get_dbname().unwrap()));
    let objects = Arc::new(NativeFileStore::new(&root, 16_777_216).unwrap());
    let catalog = Arc::new(
        PostgresMediaCatalog::open_existing(config.clone(), transport.clone(), 2)
            .await
            .unwrap(),
    );
    let build_publisher = || {
        NativeImagePublisher::new(
            RuntimePublicationSettings {
                catalog: CatalogConfig::default(),
                layout: KeyLayout::new(ObjectEnvironment::new("runtime-test").unwrap()),
                decode_concurrency: 1,
            },
            RuntimePublicationPorts {
                catalog: catalog.clone(),
                publication: publication.clone(),
                objects: objects.clone(),
                ids: Arc::new(ids.clone()),
                clock: Arc::new(TestClock),
            },
        )
        .unwrap()
    };
    let mut active_config = image_config.clone();
    active_config.mode = GlobalMode::LocalOnly;
    let runtime = NativeImageRuntime {
        journal: journal.clone(),
        execution,
        sources: NoSources,
        publisher: build_publisher(),
        admission: build_publisher(),
        clock: TestClock,
        ids: ids.clone(),
        credentials: NoCredentials,
        costs: NoCost,
        cost_policy: NoCostPolicy,
        usage: PostgresImageUsage::open_existing(config.clone(), transport.clone(), 2).unwrap(),
        telemetry: observer.clone(),
        settings: RuntimeSettings {
            config: active_config,
            active_config_revision: 1,
            capacity: scope,
            poll_interval: Duration::from_millis(10),
            codex_home: None,
        },
    };
    let result = runtime
        .run_next()
        .await
        .unwrap()
        .expect("actual published image");
    assert_eq!(result.variants.len(), 1);
    assert_eq!(result.variants[0].variant, AssetVariant::Master);
    if let Some(blocked) = blocked {
        let row = client.query_one(
            "SELECT s.stage_state,s.queued_not_before_ms,(SELECT count(*) FROM imagery.image_dispatch_attempt d WHERE d.job_id=s.job_id) FROM imagery.image_execution_stage s WHERE s.job_id=$1",
            &[&&blocked.job.as_bytes()[..]],
        ).await.unwrap();
        assert_eq!(row.get::<_, String>(0), "planned");
        assert_eq!(row.get::<_, i64>(1), NOW + 10);
        assert_eq!(row.get::<_, i64>(2), 0);
    }
    let records = diagnostic_sink.0.lock().unwrap();
    let names = records
        .iter()
        .map(|record| record.event_name)
        .collect::<Vec<_>>();
    for name in [
        "image.requested",
        "image.queued",
        "image.claimed",
        "image.admitted",
        "image.transform_started",
        "image.transform_completed",
        "image.output_stored",
        "image.published",
        "image.completed",
    ] {
        assert!(
            names.contains(&name),
            "missing actual pipeline event {name}: {names:?}"
        );
    }
    let published = records
        .iter()
        .find(|record| record.event_name == "image.published")
        .unwrap();
    for key in [
        "image.job_id",
        "image.stage_id",
        "image.attempt_id",
        "image.asset_id",
        "image.config_revision",
        "image.purpose",
    ] {
        assert!(
            published
                .input
                .attributes
                .iter()
                .any(|(name, _)| name == key),
            "missing {key}"
        );
    }
    assert!(
        !published
            .input
            .attributes
            .iter()
            .any(|(name, _)| name == "image.backend")
    );
    drop(records);
    assert_eq!(
        observer.health().failures,
        1,
        "sink refusal must be visible without failing publication"
    );
    let read = service.read(&(), &accepted.handle).await.unwrap();
    assert_eq!(read.result, Some(result.clone()));
    assert!(matches!(
        service
            .watch(&(), &accepted.handle, accepted.job_revision)
            .await
            .unwrap(),
        WatchObservation::Changed(_)
    ));
    let asset = catalog.asset(result.asset).await.unwrap().unwrap();
    assert_eq!(asset.state, df_assets::AssetState::Ready);
    let winners = catalog.variants(result.asset).await.unwrap();
    let master = winners
        .iter()
        .find(|record| record.variant == AssetVariant::Master)
        .unwrap();
    let bytes = objects
        .get_version(
            &df_assets::StoredObject {
                key: master.key.clone(),
                version: master.version.clone().unwrap(),
                manifest: master.manifest.unwrap(),
            },
            Deadline::after(Duration::from_secs(5)),
        )
        .await
        .unwrap();
    assert!(bytes.starts_with(&[137, 80, 78, 71]));
    assert_eq!(bytes.len() as u64, result.variants[0].byte_len);
    let (control, work) = runtime.start();
    let owned = tokio::spawn(work);
    control.shutdown().await.unwrap();
    owned.await.unwrap().unwrap();
    drop(service);
    let restarted = ImageService::new(
        journal,
        image_config,
        TestClock,
        ids,
        TestAuthority(owner()),
    )
    .unwrap()
    .with_results(publication);
    assert_eq!(
        restarted.read(&(), &accepted.handle).await.unwrap().result,
        Some(result.clone())
    );
    assert!(matches!(
        restarted.request(&(), &request).await.unwrap(),
        RequestOutcome::Replayed(ImageDemandView {
            result: Some(_),
            ..
        })
    ));
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    drop(client);
    scratch.finish().await;
}

#[tokio::test]
async fn raw_usage_and_output_survive_before_projection_and_restart_under_restricted_role() {
    let scratch = Scratch::open().await;
    let config = scratch.config.clone();
    let transport = PostgresTransport::PlaintextLoopbackOnly;
    let client = connect(&config).await;
    client.batch_execute("CREATE TABLE IF NOT EXISTS imagery.schema_migration(version integer PRIMARY KEY,applied_at timestamptz NOT NULL DEFAULT now()); INSERT INTO imagery.schema_migration(version) VALUES(38),(40) ON CONFLICT DO NOTHING").await.unwrap();
    let ids = TestIds(Arc::new(AtomicU64::new(500)));
    let execution = PostgresImageExecution::open(config.clone(), transport.clone(), 1)
        .await
        .unwrap();
    let scope = CapacityScope::new("runtime/raw-receipts").unwrap();
    execution
        .set_capacity_bounds(
            scope.as_str(),
            &CapacityBounds::new(1, 2, 4_194_304).unwrap(),
            NOW,
        )
        .await
        .unwrap();
    execution
        .set_execution_gate(scope.as_str(), 1, true, NOW)
        .await
        .unwrap();
    let journal = PostgresRuntimeJournal::open_existing(
        config.clone(),
        transport.clone(),
        1,
        Arc::new(ids.clone()),
        scope.clone(),
        1,
        true,
    )
    .unwrap();
    let image_config = ImageConfig::defaults().unwrap();
    let service = ImageService::new(
        journal,
        image_config,
        TestClock,
        ids,
        TestAuthority(owner()),
    )
    .unwrap();
    let mut request = request();
    let output = request.output.delivered();
    request.output = OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), output);
    request.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("fixture response", 1).unwrap(),
        references: vec![],
    });
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    let view = match service.request(&(), &request).await.unwrap() {
        RequestOutcome::Recorded(view) => view,
        _ => panic!("new request"),
    };
    let queued = execution
        .scan_queue(scope.as_str(), 1, NOW)
        .await
        .unwrap()
        .remove(0);
    let claim = df_imagery::execution::ClaimToken::from_bytes(&[13; 16]).unwrap();
    let stage = execution
        .claim(&ClaimCommand::new(owner(), view.job, queued.stage, 1, claim, 10_000, NOW).unwrap())
        .await
        .unwrap();
    let dispatch = df_imagery::execution::DispatchId::from_bytes(&[14; 16]).unwrap();
    execution
        .prepare(
            &PrepareCommand::new(
                StageClaim {
                    scope: StageScope {
                        owner: owner(),
                        job: view.job,
                        stage: queued.stage,
                    },
                    token: claim,
                    generation: stage.claim_generation,
                },
                dispatch,
                DispatchMode::Local,
                NOW,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    execution
        .mark_sent(
            &MarkSentCommand::new(
                owner(),
                view.job,
                queued.stage,
                dispatch,
                claim,
                stage.claim_generation,
                NOW,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let mut restricted = config.clone();
    restricted.options("-c role=df_imagery_usage");
    let usage =
        PostgresImageUsage::open_existing(restricted.clone(), transport.clone(), 1).unwrap();
    let record = RuntimeUsage {
        owner: owner(),
        job: view.job,
        stage: queued.stage,
        dispatch,
        revision: UsageRevision(1),
        route: CostRoute {
            provider: "openai".to_owned(),
            model: "fixture-image-model".to_owned(),
            cause: "scene_plate".to_owned(),
            detail: "receipt-test".to_owned(),
            maxima: vec![CostUnits {
                category: "image_output".to_owned(),
                quantity: 1056,
            }],
        },
        evidence: UsageEvidence::Reported,
        units: vec![CostUnits {
            category: "image_output".to_owned(),
            quantity: 720,
        }],
        usable: true,
        at_ms: NOW as u64,
        raw_output: Some(vec![137, 80, 78, 71, 13, 10, 26, 10]),
    };
    usage.record_usage(&record).await.unwrap();
    usage.record_usage(&record).await.unwrap();
    let mut conflict = record.clone();
    conflict.units[0].quantity = 721;
    assert_eq!(
        usage.record_usage(&conflict).await,
        Err(RuntimeError::InvalidRecord)
    );
    assert_eq!(
        usage.pending_usage(scope.as_str(), 1).await.unwrap(),
        vec![record.clone()]
    );
    drop(usage);
    let restarted = PostgresImageUsage::open_existing(restricted.clone(), transport, 1).unwrap();
    assert_eq!(
        restarted.latest_usage(&owner(), dispatch).await.unwrap(),
        Some(record.clone())
    );
    restarted
        .acknowledge_projection(&owner(), dispatch, UsageRevision(1))
        .await
        .unwrap();
    assert!(
        restarted
            .pending_usage(scope.as_str(), 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        restarted.latest_usage(&owner(), dispatch).await.unwrap(),
        Some(record.clone())
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_dispatch_usage", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    // A provisional raw revision remains historical when a newer exact revision is durable.
    let mut provisional = record.clone();
    provisional.revision = UsageRevision(2);
    provisional.usable = false;
    restarted.record_usage(&provisional).await.unwrap();
    let mut final_report = record.clone();
    final_report.revision = UsageRevision(3);
    restarted.record_usage(&final_report).await.unwrap();
    assert_eq!(
        restarted.pending_usage(scope.as_str(), 16).await.unwrap(),
        vec![final_report.clone()]
    );
    restarted
        .acknowledge_projection(&owner(), dispatch, UsageRevision(3))
        .await
        .unwrap();
    let reopened = PostgresImageUsage::open_existing(
        restricted.clone(),
        PostgresTransport::PlaintextLoopbackOnly,
        1,
    )
    .unwrap();
    assert!(
        reopened
            .pending_usage(scope.as_str(), 16)
            .await
            .unwrap()
            .is_empty()
    );
    let scope = ExecutionScope {
        owner: owner(),
        job: view.job,
        stage: queued.stage,
        dispatch,
    };
    assert_eq!(
        reopened
            .read_usage_revision(&scope, UsageRevision(2))
            .await
            .unwrap(),
        Some(provisional)
    );
    assert_eq!(
        reopened
            .read_usage_revision(&scope, UsageRevision(3))
            .await
            .unwrap(),
        Some(final_report)
    );
    let projected=client.query("SELECT revision,projection_durable FROM imagery.image_dispatch_usage ORDER BY revision",&[]).await.unwrap();
    assert_eq!(
        projected
            .iter()
            .map(|row| (row.get::<_, i64>(0), row.get::<_, bool>(1)))
            .collect::<Vec<_>>(),
        vec![(1, true), (2, false), (3, true)]
    );
    drop(reopened);
    let unauthorized = connect(&restricted).await;
    assert!(
        unauthorized
            .query_one("SELECT * FROM finance.provider_attempt LIMIT 1", &[])
            .await
            .is_err()
    );
    let secret = record.raw_output.as_ref().unwrap();
    assert!(!format!("{record:?}").contains(&format!("{secret:?}")));
    drop(unauthorized);
    drop(restarted);
    drop(service);
    drop(client);
    scratch.finish().await;
}

#[derive(Clone)]
struct MovingClock(Arc<std::sync::atomic::AtomicI64>);
impl Clock for MovingClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
#[derive(Clone)]
struct FixtureCredentials(Arc<AtomicU64>, Arc<QuotaRegistry>);
impl ImageCredentials for FixtureCredentials {
    fn quota_registry(&self) -> Option<Arc<QuotaRegistry>> {
        Some(self.1.clone())
    }
    fn analysis_credential(&self, _: &AccountHandle) -> Result<String, RuntimeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok("local-analysis-fixture".to_owned())
    }

    fn credential(&self, backend: BackendId, _: &AccountHandle) -> Result<String, RuntimeError> {
        assert_eq!(backend, BackendId::OpenAi);
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok("local-fixture-credential".to_owned())
    }
}
struct FixtureCostPolicy;
impl ImageRuntimeCostPolicy for FixtureCostPolicy {
    fn stage_route(
        &self,
        job: &ImageJob,
        config: &ImageConfig,
        basis: &RuntimeCostBasis,
    ) -> Result<CostRoute, RuntimeError> {
        if let Some(route) = &basis.analysis {
            return Ok(CostRoute {
                provider: "typesafe".to_owned(),
                model: route.model.as_str().to_owned(),
                cause: "storyteller".to_owned(),
                detail: "classification-fixture".to_owned(),
                maxima: vec![
                    CostUnits {
                        category: "text_input".to_owned(),
                        quantity: basis.prompt_bytes.div_ceil(2),
                    },
                    CostUnits {
                        category: "text_output".to_owned(),
                        quantity: route.maximum_output_tokens,
                    },
                ],
            });
        }
        self.route(
            job,
            config,
            basis.backend.ok_or(RuntimeError::InvalidRecord)?,
            basis.model.as_str(),
        )
    }

    fn route(
        &self,
        _: &ImageJob,
        _: &ImageConfig,
        backend: BackendId,
        model: &str,
    ) -> Result<CostRoute, RuntimeError> {
        // The live Codex route only needs a route the ledger can name; its usage is observed, not priced here.
        let provider = match backend {
            BackendId::OpenAi => "openai",
            BackendId::CodexCli => "codex-cli",
            other => panic!("unexpected fixture backend {other:?}"),
        };
        Ok(CostRoute {
            provider: provider.to_owned(),
            model: model.to_owned(),
            cause: "scene_plate".to_owned(),
            detail: "native-http-fixture".to_owned(),
            maxima: vec![
                CostUnits {
                    category: "text_input".to_owned(),
                    quantity: 8,
                },
                CostUnits {
                    category: "image_input".to_owned(),
                    quantity: 0,
                },
                CostUnits {
                    category: "image_output".to_owned(),
                    quantity: 1056,
                },
            ],
        })
    }
}
#[derive(Clone, Default)]
struct FixtureCost {
    records: Arc<std::sync::Mutex<std::collections::BTreeMap<String, u64>>>,
    fail: Arc<std::sync::atomic::AtomicBool>,
}
impl ImageCostProjection for FixtureCost {
    fn start_pinned<'a>(
        &'a self,
        id: &'a str,
        _: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>> {
        Box::pin(async move {
            let mut records = self.records.lock().unwrap();
            if records.contains_key(id) {
                Ok(PinnedStart::ReplayedOpen)
            } else {
                records.insert(id.to_owned(), 0);
                Ok(PinnedStart::Opened)
            }
        })
    }
    fn correct<'a>(
        &'a self,
        id: &'a str,
        report: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>> {
        Box::pin(async move {
            if self.fail.swap(false, Ordering::SeqCst) {
                return Err(CostError::ProjectionFull);
            }
            self.records
                .lock()
                .unwrap()
                .insert(id.to_owned(), report.revision.0);
            Ok(PinnedCorrection::Applied)
        })
    }
    fn record_unsent<'a>(&'a self, _: &'a str) -> CostFuture<'a, Result<PinnedUnsent, CostError>> {
        Box::pin(async { panic!("a sent fixture call cannot release") })
    }
    fn is_durable<'a>(
        &'a self,
        id: &'a str,
        revision: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>> {
        Box::pin(async move {
            Ok(
                self.records.lock().unwrap().get(id).copied()
                    == revision.map(|revision| revision.0),
            )
        })
    }
}
struct LostUsageAck {
    inner: PostgresImageUsage,
    fail: std::sync::atomic::AtomicBool,
}
impl ImageUsageJournal for LostUsageAck {
    async fn record_sources(
        &self,
        scope: &ExecutionScope,
        sources: &[SourceReceipt],
    ) -> Result<(), RuntimeError> {
        self.inner.record_sources(scope, sources).await
    }
    async fn read_sources(
        &self,
        scope: &ExecutionScope,
    ) -> Result<Option<Vec<SourceReceipt>>, RuntimeError> {
        self.inner.read_sources(scope).await
    }

    async fn record_usage(&self, usage: &RuntimeUsage) -> Result<(), RuntimeError> {
        self.inner.record_usage(usage).await?;
        if self.fail.swap(false, Ordering::SeqCst) {
            Err(RuntimeError::Execution(ExecutionError::Unknown))
        } else {
            Ok(())
        }
    }
    async fn latest_usage(
        &self,
        owner: &ImageOwner,
        dispatch: df_imagery::execution::DispatchId,
    ) -> Result<Option<RuntimeUsage>, RuntimeError> {
        self.inner.latest_usage(owner, dispatch).await
    }
    async fn pending_usage(
        &self,
        scope: &str,
        limit: u32,
    ) -> Result<Vec<RuntimeUsage>, RuntimeError> {
        self.inner.pending_usage(scope, limit).await
    }
    async fn acknowledge_projection(
        &self,
        owner: &ImageOwner,
        dispatch: df_imagery::execution::DispatchId,
        revision: UsageRevision,
    ) -> Result<(), RuntimeError> {
        self.inner
            .acknowledge_projection(owner, dispatch, revision)
            .await
    }
}
struct LostOutputAck<P> {
    inner: P,
    fail: std::sync::atomic::AtomicBool,
}
impl<P: ImagePublisher> ImagePublisher for LostOutputAck<P> {
    async fn store_output(
        &self,
        context: &PublicationContext,
        output: &PreparedImageOutput,
    ) -> Result<OutputCheckpoint, RuntimeError> {
        let checkpoint = self.inner.store_output(context, output).await?;
        if self.fail.swap(false, Ordering::SeqCst) {
            Err(RuntimeError::StorageUnavailable)
        } else {
            Ok(checkpoint)
        }
    }
    async fn publish(
        &self,
        context: &PublicationContext,
    ) -> Result<df_imagery::publication::PublishedImage, RuntimeError> {
        self.inner.publish(context).await
    }
}

const PNG_FIXTURE: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 4, 0, 0, 0, 4, 0, 8, 2, 0,
    0, 0, 240, 127, 188, 212, 0, 0, 17, 146, 73, 68, 65, 84, 120, 218, 237, 215, 65, 13, 0, 0, 8,
    196, 176, 147, 134, 20, 164, 32, 29, 31, 208, 164, 10, 246, 91, 170, 7, 0, 0, 120, 34, 18, 0,
    0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6,
    0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0,
    48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0,
    0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 48,
    0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0,
    0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0,
    0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192,
    0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0,
    6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6,
    0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 6, 64,
    5, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0,
    0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192,
    0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0,
    6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0,
    0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192,
    0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0,
    6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0,
    0, 0, 192, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192,
    0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 192, 0, 168, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0,
    0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192,
    0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0,
    24, 0, 0, 0, 192, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0,
    0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 192, 0,
    0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24,
    0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0,
    0, 0, 24, 0, 0, 0, 48, 0, 18, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0,
    24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0,
    0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3,
    0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24,
    0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0,
    0, 3, 0, 0, 0, 24, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0,
    0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0,
    0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0,
    3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0,
    24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0,
    0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0,
    0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 24,
    0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0,
    0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0,
    0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0,
    0, 0, 0, 3, 0, 0, 0, 6, 64, 5, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0,
    3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0,
    0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0,
    0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3,
    0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0,
    96, 0, 0, 0, 0, 3, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0,
    0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 3, 0, 0,
    0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0,
    0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3,
    0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0,
    96, 0, 0, 0, 0, 3, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0,
    0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 3, 0, 0,
    0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0,
    0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0,
    3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0,
    0, 96, 0, 0, 0, 192, 0, 168, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0,
    96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0,
    0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12,
    0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0,
    96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0,
    0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12,
    0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 96,
    0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0,
    12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 48, 0, 18, 0, 0, 128, 1, 0, 0, 0, 12, 0,
    0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128,
    1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0,
    0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0,
    0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1,
    0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0,
    12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0,
    128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0,
    0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0,
    0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128,
    1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0,
    12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0,
    128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0,
    0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0,
    0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128,
    1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 6, 64, 5, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0,
    0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0,
    0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1,
    0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12,
    0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0,
    128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0,
    0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0,
    0, 0, 128, 1, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128,
    1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0,
    12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0,
    128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0,
    0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0,
    0, 0, 128, 1, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128,
    1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 128, 1, 0, 0,
    0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0,
    0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 192, 0, 168, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0,
    128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0,
    0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48,
    0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0,
    128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0,
    0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48,
    0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0,
    0, 6, 0, 0, 0, 48, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0,
    0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 48, 0,
    18, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0,
    0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0,
    0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192,
    0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0,
    6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6,
    0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0,
    0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192,
    0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0,
    0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0,
    0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192, 0,
    0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 6,
    64, 5, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0,
    0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 24, 0, 0, 0, 192, 0,
    0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24,
    0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0,
    0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192,
    0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0,
    24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0,
    192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0,
    0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0,
    0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0, 0, 192,
    0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0,
    24, 0, 0, 0, 192, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0, 24, 0, 0,
    0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 192, 0,
    168, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0,
    0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 3, 0, 0, 0, 24, 0,
    0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0,
    3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0,
    0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0,
    0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 128, 1, 0, 0, 0, 12, 0, 0, 0, 96, 0, 0, 0, 0,
    3, 0, 0, 0, 24, 0, 0, 0, 192, 0, 0, 0, 0, 6, 0, 0, 0, 48, 0, 0, 0, 96, 0, 0, 0, 0, 3, 0, 0, 0,
    220, 178, 176, 169, 14, 16, 111, 52, 206, 85, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
const PNG_FIXTURE_BASE64: &str = concat!(
    "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAARkklEQVR42u3XQQ0AAAjEsJOGFKQgHR/Q",
    "pAr2W6oHAAB4IhIAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAA",
    "AMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAGAAAAAAAwAAABgA",
    "AADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAY",
    "AAAAwAAAAAAGAAAAMAAAAIABAAAADAAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAA",
    "GAAAAMAAAAAABgAAADAAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAA",
    "ABgAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAAAYAAAAwAAA",
    "AAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAMAAAAYAAAAAADAAAAGAAAAMAA",
    "AAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAGQAUAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAA",
    "wAAAAAAGAAAAMAAAAIABAAAADAAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAA",
    "AMAAAAAABgAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAAAYA",
    "AAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAMAAAAYAAAAwAAAAAAG",
    "AAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAA",
    "BgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAA",
    "AAYAAAAwAAAAgAEAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAwAAA",
    "AIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAMAAqAAAAAYAAAAw",
    "AAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAMAAAAYAAAAwAAAAAAGAAAA",
    "MAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAA",
    "ADAAAACAAQAAAAwAAABgAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYA",
    "AAAwAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAMAASAACA",
    "AQAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAADAAAAAAAYAAAAwAAAA",
    "gAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAA",
    "AIABAAAADAAAAGAAAAAAAwAAABgAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAA",
    "AACAAQAAAAwAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAw",
    "AAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAMAAAAIABAAAA",
    "DAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAGAAAAMAAAAAABgAAADAAAACAAQAA",
    "AAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEA",
    "AAAMAAAAYAAAAAADAAAABkAFAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACA",
    "AQAAAAwAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAMAAAA",
    "YAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAGAAAAMAAAAIABAAAADAAA",
    "AGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwA",
    "AABgAAAAAAMAAAAYAAAAwAAAAAAGAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAM",
    "AAAAYAAAAAADAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAA",
    "AwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAACAAQAAAAwAAABgAAAA",
    "AAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAADAAKgAAAAGAAAAMAAAAIABAAAADAAAAGAA",
    "AAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABg",
    "AAAAAAMAAAAYAAAAwAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAA",
    "YAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAGAAAAAAAwAA",
    "ABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAADAAEgAAgAEAAAAMAAAAYAAAAAAD",
    "AAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAA",
    "AwAAABgAAADAAAAAAAYAAAAwAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAA",
    "AAMAAAAYAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAGAAA",
    "AMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAGAAAAAAAwAAABgA",
    "AADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAY",
    "AAAAwAAAAAAGAAAAMAAAAIABAAAADAAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAA",
    "GAAAAMAAAAAABgAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAA",
    "AAZABQAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAGAAAAMAA",
    "AAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAADAAAAGAAAAAAAwAAABgAAADA",
    "AAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAA",
    "wAAAAAAGAAAAMAAAAIABAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAA",
    "AMAAAAAABgAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAAAYA",
    "AAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAMAAAAYAAAAwAAAAAAG",
    "AAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAA",
    "BgAAADAAAACAAQAAAAwAAABgAAAAwACoAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAwAAA",
    "AAAGAAAAMAAAAIABAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAA",
    "AACAAQAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAADAAAAAAAYAAAAw",
    "AAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAA",
    "MAAAAIABAAAADAAAAGAAAAAAAwAAABgAAAAwABIAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYA",
    "AAAwAAAAgAEAAAAMAAAAYAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAG",
    "AAAAMAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAADAAAACA",
    "AQAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAABgAAADAAAAAAAYAAAAwAAAA",
    "gAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAA",
    "AIABAAAADAAAAGAAAAAAAwAAABgAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAA",
    "AACAAQAAAAwAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAM",
    "AAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAGQAUAADAAAACAAQAA",
    "AAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAABgAAADAAAAAAAYAAAAwAAAAgAEA",
    "AAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAAAwAAABgAAAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIAB",
    "AAAADAAAAGAAAAAAAwAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACA",
    "AQAAAAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAMAAAA",
    "YAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAAAMAAAAGAAAAMAAAAIABAAAADAAA",
    "AGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwA",
    "AABgAAAAAAMAAAAYAAAAwAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAAAYAAAAwAAAAgAEAAAAM",
    "AAAAYAAAAMAAqAAAAAYAAAAwAAAAgAEAAAAMAAAAYAAAAAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAA",
    "AAMAAAAYAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAAAAAAAwAAABgAAADAAAAAgAEAAAAMAAAAYAAA",
    "AAADAAAAGAAAAMAAAAAABgAAADAAAACAAQAAAAwAAABgAAAAwAAAAAAGAAAAMAAAAIABAAAADAAAAGAA",
    "AAAAAwAAABgAAADAAAAAAAYAAAAwAAAAYAAAAAADAAAA3LKwqQ4QbzTOVQAAAABJRU5ErkJggg==",
);
fn png_fixture() -> Vec<u8> {
    PNG_FIXTURE.to_vec()
}
fn fixture_server(
    bytes: &[u8],
) -> (
    std::net::SocketAddr,
    Arc<AtomicU64>,
    std::thread::JoinHandle<()>,
) {
    use std::io::{Read, Write};
    assert_eq!(bytes, PNG_FIXTURE);
    let body = serde_json::json!({"data":[{"b64_json":PNG_FIXTURE_BASE64}],
        "usage":{"input_tokens_details":{"text_tokens":7,"image_tokens":0},"output_tokens":720}})
    .to_string();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let sends = Arc::new(AtomicU64::new(0));
    let sent = sends.clone();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(stream) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&request[..end]).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
            assert!(request.len() < 1_048_576);
        }
        assert!(request.starts_with(b"POST /v1/images/generations "));
        sent.fetch_add(1, Ordering::SeqCst);
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    (address, sends, server)
}

#[tokio::test]
async fn private_api_receipt_recovers_each_durable_boundary_without_a_second_send() {
    for failure in [
        "raw-ack",
        "output-ack",
        "projection",
        "cancelled-raw-ack",
        "expired-raw-ack",
    ] {
        let scratch = Scratch::open().await;
        let config = scratch.config.clone();
        let transport = PostgresTransport::PlaintextLoopbackOnly;
        let client = connect(&config).await;
        client.batch_execute("CREATE TABLE IF NOT EXISTS imagery.schema_migration(version integer PRIMARY KEY,applied_at timestamptz NOT NULL DEFAULT now()); INSERT INTO imagery.schema_migration(version) VALUES(38),(40) ON CONFLICT DO NOTHING").await.unwrap();
        let ids = TestIds(Arc::new(AtomicU64::new(1000)));
        let clock = MovingClock(Arc::new(std::sync::atomic::AtomicI64::new(NOW)));
        let execution = PostgresImageExecution::open(config.clone(), transport.clone(), 2)
            .await
            .unwrap();
        let scope = CapacityScope::new("runtime/http-fixture").unwrap();
        execution
            .set_capacity_bounds(
                scope.as_str(),
                &CapacityBounds::new(2, 4, 4_194_304).unwrap(),
                NOW,
            )
            .await
            .unwrap();
        execution
            .set_execution_gate(scope.as_str(), 1, true, NOW)
            .await
            .unwrap();
        let journal = PostgresRuntimeJournal::open_existing(
            config.clone(),
            transport.clone(),
            2,
            Arc::new(ids.clone()),
            scope.clone(),
            1,
            true,
        )
        .unwrap();
        let publication =
            PostgresImagePublication::open_existing(config.clone(), transport.clone(), 2)
                .await
                .unwrap();
        let bytes = png_fixture();
        let (address, sends, server) = fixture_server(&bytes);
        let mut image_config = ImageConfig::defaults().unwrap();
        image_config.default_backend = BackendId::OpenAi;
        image_config.backends[BackendId::OpenAi.index()].enabled = true;
        image_config.backends[BackendId::OpenAi.index()].endpoint =
            Some(ApiEndpoint::new(&format!("http://{address}")).unwrap());
        image_config.quality.provider_quality = Some(GenerationQuality::Medium);
        image_config.scheduler.stage_deadline_secs = 10;
        let service = ImageService::new(
            journal.clone(),
            image_config.clone(),
            clock.clone(),
            ids.clone(),
            TestAuthority(owner()),
        )
        .unwrap()
        .with_results(publication.clone());
        let mut request = request();
        request.operation = ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("fixture response", 1).unwrap(),
            references: vec![],
        });
        request.purpose.purpose = Purpose::ScenePlate;
        request.output = OutputProfile::provider(
            Dimensions::new(1024, 1024).unwrap(),
            request.output.delivered(),
        );
        request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
        let accepted = match service.request(&(), &request).await.unwrap() {
            RequestOutcome::Recorded(view) => view,
            _ => panic!("new request"),
        };
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/runtime-test")
            .join(config.get_dbname().unwrap());
        let objects = Arc::new(NativeFileStore::new(&root, 16_777_216).unwrap());
        let catalog = Arc::new(
            PostgresMediaCatalog::open_existing(config.clone(), transport.clone(), 2)
                .await
                .unwrap(),
        );
        let build_publisher = || {
            NativeImagePublisher::new(
                RuntimePublicationSettings {
                    catalog: CatalogConfig::default(),
                    layout: KeyLayout::new(ObjectEnvironment::new("runtime-api").unwrap()),
                    decode_concurrency: 1,
                },
                RuntimePublicationPorts {
                    catalog: catalog.clone(),
                    publication: publication.clone(),
                    objects: objects.clone(),
                    ids: Arc::new(ids.clone()),
                    clock: Arc::new(clock.clone()),
                },
            )
            .unwrap()
        };
        let credentials = FixtureCredentials(
            Arc::new(AtomicU64::new(0)),
            Arc::new(QuotaRegistry::new(Arc::new(clock.clone()))),
        );
        let costs = FixtureCost::default();
        costs.fail.store(failure == "projection", Ordering::SeqCst);
        let usage =
            PostgresImageUsage::open_existing(config.clone(), transport.clone(), 2).unwrap();
        let runtime = NativeImageRuntime {
            journal: journal.clone(),
            execution: execution.clone(),
            sources: NoSources,
            publisher: LostOutputAck {
                inner: build_publisher(),
                fail: std::sync::atomic::AtomicBool::new(failure == "output-ack"),
            },
            admission: build_publisher(),
            clock: clock.clone(),
            ids: ids.clone(),
            credentials: credentials.clone(),
            costs: costs.clone(),
            cost_policy: FixtureCostPolicy,
            usage: LostUsageAck {
                inner: usage.clone(),
                fail: std::sync::atomic::AtomicBool::new(failure.ends_with("raw-ack")),
            },
            telemetry: TestTelemetry,
            settings: RuntimeSettings {
                config: image_config.clone(),
                active_config_revision: 1,
                capacity: scope.clone(),
                poll_interval: Duration::from_millis(10),
                codex_home: None,
            },
        };
        assert!(
            runtime.run_next().await.is_err(),
            "{failure} did not stop at its forced boundary"
        );
        tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap();
        assert!(
            service
                .read(&(), &accepted.handle)
                .await
                .unwrap()
                .result
                .is_none()
        );
        if failure.starts_with("cancelled") {
            service.cancel(&(), &accepted.handle).await.unwrap();
        }
        let dispatch = client
            .query_one(
                "SELECT dispatch_id FROM imagery.image_dispatch_attempt",
                &[],
            )
            .await
            .unwrap()
            .get::<_, Vec<u8>>(0);
        let dispatch = df_imagery::execution::DispatchId::from_bytes(&dispatch).unwrap();
        let retained = usage
            .latest_usage(&owner(), dispatch)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.raw_output.as_deref(), Some(bytes.as_slice()));
        drop(runtime);
        let recovery_at = if failure.starts_with("expired") {
            accepted.absolute_deadline_ms + 1
        } else {
            // Past the stage budget plus the pre-send lease margin, but before the job deadline.
            let lease_expired = NOW + 41_000;
            assert!(lease_expired < accepted.absolute_deadline_ms);
            lease_expired
        };
        clock.0.store(recovery_at, Ordering::SeqCst);
        let restarted = NativeImageRuntime {
            journal,
            execution: execution.clone(),
            sources: NoSources,
            publisher: build_publisher(),
            admission: build_publisher(),
            clock: clock.clone(),
            ids: ids.clone(),
            credentials: credentials.clone(),
            costs: costs.clone(),
            cost_policy: FixtureCostPolicy,
            usage: usage.clone(),
            telemetry: TestTelemetry,
            settings: RuntimeSettings {
                config: image_config,
                active_config_revision: 1,
                capacity: scope.clone(),
                poll_interval: Duration::from_millis(10),
                codex_home: None,
            },
        };
        let result = restarted
            .run_next()
            .await
            .unwrap_or_else(|error| panic!("case {failure}: recovery failed {error:?}"))
            .unwrap_or_else(|| panic!("case {failure}: no recovered durable output"));
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        assert_eq!(credentials.0.load(Ordering::SeqCst), 1);
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            1
        );
        assert!(
            usage
                .pending_usage(scope.as_str(), 1)
                .await
                .unwrap()
                .is_empty()
        );
        if failure.starts_with("cancelled") || failure.starts_with("expired") {
            assert!(result.private_reconciliation);
            assert!(
                service
                    .read(&(), &accepted.handle)
                    .await
                    .unwrap()
                    .result
                    .is_none()
            );
        } else {
            assert_eq!(
                service.read(&(), &accepted.handle).await.unwrap().result,
                Some(result)
            );
        }
        assert_eq!(costs.records.lock().unwrap().len(), 1);
        drop(restarted);
        drop(service);
        drop(client);
        scratch.finish().await;
    }
}

#[path = "imagery_runtime/classification.rs"]
mod classification;

#[path = "imagery_runtime/usage_projection.rs"]
mod usage_projection;

#[path = "imagery_runtime/analysis.rs"]
mod analysis_tests;

#[path = "imagery_runtime/compatible_versions.rs"]
mod compatible_versions;

/// The whole service path with the real Codex CLI: `ImageService` records the request, the
/// runtime claims the stage, `codex.exe` makes the image through the default backend, the worker
/// normalizes and publishes it, and the service reads the stored object back. Runs only with
/// `DF_LIVE_CODEX=1`, because it spends Codex quota.
#[tokio::test]
async fn live_codex_generation_publishes_through_the_service() {
    if std::env::var("DF_LIVE_CODEX").as_deref() != Ok("1") {
        eprintln!("skipped: set DF_LIVE_CODEX=1 to spend Codex quota on a real image");
        return;
    }
    let executable = std::env::var("DF_CODEX_EXE").unwrap_or_else(|_| {
        r"C:\Users\mreca\AppData\Local\Programs\OpenAI\Codex\bin\codex.exe".to_owned()
    });
    let scratch = Scratch::open().await;
    let config = scratch.config.clone();
    let transport = PostgresTransport::PlaintextLoopbackOnly;
    let client = connect(&config).await;
    client.batch_execute("CREATE TABLE IF NOT EXISTS imagery.schema_migration(version integer PRIMARY KEY,applied_at timestamptz NOT NULL DEFAULT now()); INSERT INTO imagery.schema_migration(version) VALUES(38),(40) ON CONFLICT DO NOTHING").await.unwrap();
    let ids = TestIds(Arc::new(AtomicU64::new(5000)));
    let clock = MovingClock(Arc::new(std::sync::atomic::AtomicI64::new(NOW)));
    let execution = PostgresImageExecution::open(config.clone(), transport.clone(), 2)
        .await
        .unwrap();
    let scope = CapacityScope::new("runtime/live-codex").unwrap();
    execution
        .set_capacity_bounds(
            scope.as_str(),
            &CapacityBounds::new(2, 4, 4_194_304).unwrap(),
            NOW,
        )
        .await
        .unwrap();
    execution
        .set_execution_gate(scope.as_str(), 1, true, NOW)
        .await
        .unwrap();
    let journal = PostgresRuntimeJournal::open_existing(
        config.clone(),
        transport.clone(),
        2,
        Arc::new(ids.clone()),
        scope.clone(),
        1,
        true,
    )
    .unwrap();
    let publication = PostgresImagePublication::open_existing(config.clone(), transport.clone(), 2)
        .await
        .unwrap();
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .unwrap()
        .to_path_buf();
    let work = repository
        .join("artifacts")
        .join("tmp")
        .join("live-codex-service")
        .join("work");
    std::fs::create_dir_all(&work).unwrap();
    // Production defaults: Codex is the default backend, with its own executable and limits.
    let mut image_config = ImageConfig::defaults().unwrap();
    image_config.codex.executable = df_imagery::CodexExecutable::new(&executable).unwrap();
    image_config.codex.workdir = df_imagery::JobWorkDir::new(&work.to_string_lossy()).unwrap();
    // Codex does not take a provider quality; the runtime refuses one for this backend.
    image_config.scheduler.job_deadline_secs = 900;
    image_config.scheduler.stage_deadline_secs = 600;
    let service = ImageService::new(
        journal.clone(),
        image_config.clone(),
        clock.clone(),
        ids.clone(),
        TestAuthority(owner()),
    )
    .unwrap()
    .with_results(publication.clone());
    let mut request = request();
    request.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new(
            "A plain solid red square on a white background, centered, with crisp edges.",
            1,
        )
        .unwrap(),
        references: vec![],
    });
    request.purpose.purpose = Purpose::ScenePlate;
    // Codex chooses its own size; the worker resizes to the delivered 1024 by 1024 PNG.
    let delivered = RasterOutputSpec::new(
        Dimensions::new(1024, 1024).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Png,
    )
    .unwrap();
    request.output = OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), delivered);
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    let accepted = match service.request(&(), &request).await.unwrap() {
        RequestOutcome::Recorded(view) => view,
        other => panic!("new live request was not recorded: {other:?}"),
    };
    let root = repository
        .join("artifacts")
        .join("tmp")
        .join("runtime-test")
        .join(config.get_dbname().unwrap());
    let objects = Arc::new(NativeFileStore::new(&root, 16_777_216).unwrap());
    let catalog = Arc::new(
        PostgresMediaCatalog::open_existing(config.clone(), transport.clone(), 2)
            .await
            .unwrap(),
    );
    let build_publisher = || {
        NativeImagePublisher::new(
            RuntimePublicationSettings {
                catalog: CatalogConfig::default(),
                layout: KeyLayout::new(ObjectEnvironment::new("runtime-live").unwrap()),
                decode_concurrency: 1,
            },
            RuntimePublicationPorts {
                catalog: catalog.clone(),
                publication: publication.clone(),
                objects: objects.clone(),
                ids: Arc::new(ids.clone()),
                clock: Arc::new(clock.clone()),
            },
        )
        .unwrap()
    };
    let credentials = FixtureCredentials(
        Arc::new(AtomicU64::new(0)),
        Arc::new(QuotaRegistry::new(Arc::new(clock.clone()))),
    );
    let usage = PostgresImageUsage::open_existing(config.clone(), transport.clone(), 2).unwrap();
    let runtime = NativeImageRuntime {
        journal: journal.clone(),
        execution: execution.clone(),
        sources: NoSources,
        publisher: build_publisher(),
        admission: build_publisher(),
        clock: clock.clone(),
        ids: ids.clone(),
        credentials,
        costs: FixtureCost::default(),
        cost_policy: FixtureCostPolicy,
        usage,
        telemetry: TestTelemetry,
        settings: RuntimeSettings {
            config: image_config,
            active_config_revision: 1,
            capacity: scope.clone(),
            poll_interval: Duration::from_millis(10),
            codex_home: None,
        },
    };
    let mut published = None;
    for _ in 0..200 {
        runtime.run_next().await.unwrap();
        if let Some(result) = service.read(&(), &accepted.handle).await.unwrap().result {
            published = Some(result);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if published.is_none() {
        for query in [
            "SELECT state::text FROM imagery.image_job",
            "SELECT stage_kind::text || ' ' || stage_state::text FROM imagery.image_execution_stage",
            "SELECT dispatch_state::text || ' ' || settlement::text FROM imagery.image_dispatch_attempt",
            "SELECT recovery_state::text FROM imagery.image_dispatch_recovery",
        ] {
            let rows = client.query(query, &[]).await.unwrap();
            eprintln!("STATE {query} => {:?}", rows.iter().map(|row| row.get::<_, String>(0)).collect::<Vec<_>>());
        }
    }
    let published = published.expect("the live Codex image was not published");
    let master = published
        .variants
        .iter()
        .find(|variant| variant.variant == AssetVariant::Master)
        .expect("master variant");
    assert!(master.byte_len > 0);
    // The stored object must be the bytes that were published: find it by size and digest.
    let found = find_object(&root, master.byte_len, &master.sha256)
        .expect("stored object with the published digest");
    let saved = repository
        .join("artifacts")
        .join("tmp")
        .join("live-codex-service")
        .join("result.png");
    std::fs::copy(&found, &saved).unwrap();
    eprintln!(
        "LIVE SERVICE OK: published {} bytes, stored at {}, copied to {}",
        master.byte_len,
        found.display(),
        saved.display()
    );
    scratch.finish().await;
}

fn find_object(
    directory: &std::path::Path,
    byte_len: u64,
    sha256: &[u8; 32],
) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(directory).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_object(&path, byte_len, sha256) {
                return Some(found);
            }
        } else if std::fs::metadata(&path).ok()?.len() == byte_len {
            let digest: [u8; 32] =
                <sha2::Sha256 as sha2::Digest>::digest(std::fs::read(&path).ok()?).into();
            if &digest == sha256 {
                return Some(path);
            }
        }
    }
    None
}
