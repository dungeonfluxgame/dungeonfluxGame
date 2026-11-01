//! External integration tests for `ImageService` over the real PostgreSQL journal.
//!
//! Every test composes the public service API with `PostgresImageJournal` on an
//! owned scratch database named by `DUNGEONFLUX_RUNTIME_PG_PORT` (host
//! `DUNGEONFLUX_RUNTIME_PG_HOST`, default 127.0.0.1; user
//! `DUNGEONFLUX_RUNTIME_PG_USER`, default postgres). Without the server each
//! test reports SKIPPED, unless `DUNGEONFLUX_REQUIRE_SCRATCH_PG` is set.
//! Forwarding wrappers below only order or fault real journal calls; all rows
//! come from PostgreSQL. No live providers, no manufactured readiness.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use df_assets::Audience;
use df_imagery::{
    AlphaHandling, AssetDigest, AssetId, Dimensions, EditSpec, Encoding, GenerateSpec,
    LossyQuality, OutputProfile, RasterOutputSpec, Rgb, TextPrompt, Urgency,
};
use df_imagery::{
    AuthDenied, CancelOutcome, ExecutionMode, GlobalMode, ImageAuthorizer, ImageConfig,
    ImageInterestId, ImageJobId, ImageJournal, ImageOwner, ImageRequest, ImageService,
    ImageServiceError, InterestState, JobState, JournalError, OperationHandle, Purpose,
    PurposedUse, RequestOutcome, RequestProblem, ServiceAction, ServiceClock, ServiceIdError,
    ServiceIds, StoredImageAuthorization, VariantRole, WatchObservation,
};
use df_imagery::{Freshness, ImageOperation};
use df_persistence::PostgresImageJournal;
use df_persistence::postgres_tls::PostgresTransport;
use df_types::{AccountId, CampaignId, OperationId, RunId};
use tokio_postgres::{Client, Config, NoTls};

const START_MS: i64 = 1_700_000_000_000;
const RECORDED_DEADLINE_MS: i64 = START_MS + 240_000;
const BOUND_SECS: u64 = 20;

/// Never these ports: the game's and the other owned test servers.
const FORBIDDEN_PORTS: [u16; 4] = [0, 55517, 55439, 63309];

fn pg_config(dbname: &str) -> Option<Config> {
    let Ok(port) = std::env::var("DUNGEONFLUX_RUNTIME_PG_PORT") else {
        assert!(
            std::env::var_os("DUNGEONFLUX_REQUIRE_SCRATCH_PG").is_none(),
            "DUNGEONFLUX_REQUIRE_SCRATCH_PG is set but DUNGEONFLUX_RUNTIME_PG_PORT is not"
        );
        eprintln!("SKIPPED: DUNGEONFLUX_RUNTIME_PG_PORT must name an owned scratch PostgreSQL");
        return None;
    };
    let port: u16 = port.parse().expect("scratch port");
    assert!(!FORBIDDEN_PORTS.contains(&port));
    let mut config = Config::new();
    config
        .host(std::env::var("DUNGEONFLUX_RUNTIME_PG_HOST").unwrap_or("127.0.0.1".into()))
        .port(port)
        .user(std::env::var("DUNGEONFLUX_RUNTIME_PG_USER").unwrap_or("postgres".into()))
        .dbname(dbname);
    Some(config)
}

async fn raw_client(config: &Config) -> (Client, tokio::task::JoinHandle<()>) {
    let (client, connection) = config.connect(NoTls).await.expect("scratch server");
    let driver = tokio::spawn(async move {
        let _ended = connection.await;
    });
    (client, driver)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime")
}

struct Fixture {
    admin: Config,
    database: String,
    journal: PostgresImageJournal,
}

impl Fixture {
    async fn open() -> Option<Self> {
        let admin = pg_config("postgres")?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::SeqCst);
        let database = format!("df_imagery_svc_{nanos}_{}_{serial}", std::process::id());
        raw_client(&admin)
            .await
            .0
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .expect("create test database");
        let config = pg_config(&database)?;
        let (setup, _driver) = raw_client(&config).await;
        setup
            .batch_execute(include_str!("../migrations/0038_imagery_operations.sql"))
            .await
            .expect("imagery migration");
        let journal =
            PostgresImageJournal::open(config, PostgresTransport::PlaintextLoopbackOnly, 4)
                .await
                .expect("open journal");
        Some(Self {
            admin,
            database,
            journal,
        })
    }

    fn config(&self) -> Config {
        pg_config(&self.database).expect("scratch config")
    }

    async fn reopen(&self) -> PostgresImageJournal {
        PostgresImageJournal::open(self.config(), PostgresTransport::PlaintextLoopbackOnly, 4)
            .await
            .expect("reopen journal")
    }

    async fn row_count(&self, table: &str) -> i64 {
        let (client, _driver) = raw_client(&self.config()).await;
        client
            .query_one(&format!("SELECT COUNT(*) FROM imagery.{table}"), &[])
            .await
            .expect("count rows")
            .get(0)
    }

    async fn finish(self) {
        drop(self.journal);
        raw_client(&self.admin)
            .await
            .0
            .batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                self.database
            ))
            .await
            .expect("drop test database");
    }
}

fn account_id(byte: u8) -> AccountId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    AccountId::from_bytes(&bytes).unwrap()
}

fn campaign_id(byte: u8) -> CampaignId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    CampaignId::from_bytes(&bytes).unwrap()
}

fn run_id(byte: u8) -> RunId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    RunId::from_bytes(&bytes).unwrap()
}

fn operation_id(byte: u8) -> OperationId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    OperationId::from_bytes(&bytes).unwrap()
}

fn draft_owner() -> ImageOwner {
    ImageOwner::Draft {
        account: account_id(0x11),
    }
}

fn other_draft_owner() -> ImageOwner {
    ImageOwner::Draft {
        account: account_id(0x99),
    }
}

fn run_owner() -> ImageOwner {
    ImageOwner::Run {
        campaign: campaign_id(0x33),
        run: run_id(0x44),
    }
}

fn provider_output() -> OutputProfile {
    let delivered = RasterOutputSpec::new(
        Dimensions::new(512, 512).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Jpeg {
            quality: LossyQuality::new(85).unwrap(),
        },
    )
    .unwrap();
    OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), delivered)
}

fn local_output() -> OutputProfile {
    let delivered = RasterOutputSpec::new(
        Dimensions::new(64, 64).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Png,
    )
    .unwrap();
    OutputProfile::local(delivered)
}

fn source_ref() -> df_imagery::SourceRef {
    df_imagery::SourceRef {
        asset: AssetId::from_bytes(&[0x07; 16]).unwrap(),
        digest: AssetDigest::from_bytes([0x09; 32]),
        audience: Audience::PublicStatic,
    }
}

fn generate_request(owner: ImageOwner, operation: OperationId) -> ImageRequest {
    ImageRequest {
        identity: df_imagery::OperationIdentity {
            owner,
            operation,
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a tavern at dusk", 3).unwrap(),
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::PublicStatic,
        expected_source_revision: 7,
        output: provider_output(),
    }
}

fn edit_request(owner: ImageOwner, operation: OperationId) -> ImageRequest {
    let mut request = generate_request(owner, operation);
    request.purpose = PurposedUse {
        purpose: Purpose::SceneEdit,
        role: VariantRole::Primary,
    };
    request.operation = ImageOperation::Edit(EditSpec {
        prompt: TextPrompt::new("add a lantern glow", 3).unwrap(),
        base: source_ref(),
        references: Vec::new(),
    });
    request
}

fn transform_request(owner: ImageOwner, operation: OperationId) -> ImageRequest {
    let mut request = generate_request(owner, operation);
    request.purpose = PurposedUse {
        purpose: Purpose::ReferencePrep,
        role: VariantRole::Reference,
    };
    request.operation = ImageOperation::Transform(df_imagery::TransformSpec {
        source: df_imagery::TransformSource::Canvas(
            df_imagery::CanvasSpec::new(
                64,
                64,
                Rgb {
                    red: 1,
                    green: 2,
                    blue: 3,
                },
            )
            .unwrap(),
        ),
        steps: vec![df_imagery::ImageTransform::Flip(
            df_imagery::FlipAxis::Horizontal,
        )],
    });
    request.output = local_output();
    request
}

#[derive(Clone, Copy)]
enum Kind {
    Generate,
    Edit,
    Transform,
}

fn request_for(kind: Kind, owner: ImageOwner, operation: OperationId) -> ImageRequest {
    match kind {
        Kind::Generate => generate_request(owner, operation),
        Kind::Edit => edit_request(owner, operation),
        Kind::Transform => transform_request(owner, operation),
    }
}

fn stamp<J, C, I, A>(service: &ImageService<J, C, I, A>, mut request: ImageRequest) -> ImageRequest
where
    J: ImageJournal,
    C: ServiceClock,
    I: ServiceIds,
    A: ImageAuthorizer,
{
    let fingerprint = service.canonical_fingerprint(&request).unwrap();
    request.identity.fingerprint = fingerprint;
    request
}

#[derive(Clone)]
struct TestPrincipal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoredPolicy {
    AllowAll,
    DenyAll,
    PublicOnly,
}

#[derive(Clone)]
struct PgAuth {
    request_owner: ImageOwner,
    coarse_owner: Option<ImageOwner>,
    stored_policy: StoredPolicy,
    calls: Arc<Mutex<Vec<String>>>,
}

impl PgAuth {
    fn permitting(owner: ImageOwner) -> Self {
        Self {
            request_owner: owner,
            coarse_owner: None,
            stored_policy: StoredPolicy::AllowAll,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn logged(&self, entry: &str) {
        self.calls.lock().unwrap().push(entry.to_owned());
    }
}

impl ImageAuthorizer for PgAuth {
    type Principal = TestPrincipal;

    fn authorize_request(
        &self,
        _principal: &Self::Principal,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied> {
        self.logged("request");
        if request.identity.owner == self.request_owner {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    fn authorize_owner(
        &self,
        _principal: &Self::Principal,
        owner: &ImageOwner,
        action: ServiceAction,
    ) -> Result<(), AuthDenied> {
        self.logged(match action {
            ServiceAction::Read => "owner-read",
            ServiceAction::Cancel => "owner-cancel",
            ServiceAction::Generate => "owner-generate",
        });
        let allowed = self.coarse_owner.as_ref().unwrap_or(&self.request_owner);
        if owner == allowed {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    /// Operator revoke in these tests belongs to the scope owner only; other principals are denied.
    fn authorize_revoke(
        &self,
        _principal: &Self::Principal,
        owner: &ImageOwner,
    ) -> Result<(), AuthDenied> {
        if *owner == self.request_owner {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    /// Audience visibility only: a stored policy that hides private audiences must not let the
    /// creator record demand they cannot read back. Other stored denials still happen on read.
    fn authorize_new_visibility(
        &self,
        _principal: &Self::Principal,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied> {
        match (self.stored_policy, &request.audience) {
            (StoredPolicy::PublicOnly, audience) if !matches!(audience, Audience::PublicStatic) => {
                Err(AuthDenied)
            }
            _ => Ok(()),
        }
    }

    fn authorize_stored(
        &self,
        _principal: &Self::Principal,
        action: ServiceAction,
        _view: &df_imagery::OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        self.logged(match action {
            ServiceAction::Read => "stored-read",
            ServiceAction::Cancel => "stored-cancel",
            ServiceAction::Generate => "stored-generate",
        });
        match self.stored_policy {
            StoredPolicy::AllowAll => Ok(()),
            StoredPolicy::DenyAll => Err(AuthDenied),
            StoredPolicy::PublicOnly => match stored.audience {
                Audience::PublicStatic => Ok(()),
                _ => Err(AuthDenied),
            },
        }
    }
}

#[derive(Clone)]
struct PgClock {
    now: Arc<AtomicI64>,
}

impl ServiceClock for PgClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct PgIds {
    jobs: Arc<AtomicU64>,
    interests: Arc<AtomicU64>,
}

fn shared_job_counter() -> Arc<AtomicU64> {
    static CELL: OnceLock<Arc<AtomicU64>> = OnceLock::new();
    CELL.get_or_init(|| Arc::new(AtomicU64::new(1))).clone()
}

fn shared_interest_counter() -> Arc<AtomicU64> {
    static CELL: OnceLock<Arc<AtomicU64>> = OnceLock::new();
    CELL.get_or_init(|| Arc::new(AtomicU64::new(1))).clone()
}

impl PgIds {
    fn fresh() -> Self {
        // One process-wide source: services sharing one journal/database must
        // mint distinct ids, otherwise the second Create reports IdentityConflict.
        Self {
            jobs: shared_job_counter(),
            interests: shared_interest_counter(),
        }
    }

    fn counter(counter: &AtomicU64) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[8..].copy_from_slice(&counter.fetch_add(1, Ordering::SeqCst).to_be_bytes());
        bytes
    }
}

impl ServiceIds for PgIds {
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
        ImageJobId::from_bytes(&Self::counter(&self.jobs))
            .map_err(|_| ServiceIdError::InvalidGeneratedId)
    }

    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
        ImageInterestId::from_bytes(&Self::counter(&self.interests))
            .map_err(|_| ServiceIdError::InvalidGeneratedId)
    }
}

type PgService<J> = ImageService<J, PgClock, PgIds, PgAuth>;

fn pg_service<J: ImageJournal>(
    journal: J,
    owner: ImageOwner,
    stored_policy: StoredPolicy,
) -> (PgService<J>, TestPrincipal, Arc<Mutex<Vec<String>>>) {
    let clock = PgClock {
        now: Arc::new(AtomicI64::new(START_MS)),
    };
    let auth = PgAuth {
        request_owner: owner,
        coarse_owner: None,
        stored_policy,
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let calls = auth.calls.clone();
    let service = ImageService::new(
        journal,
        ImageConfig::defaults().unwrap(),
        clock,
        PgIds::fresh(),
        auth,
    )
    .unwrap();
    (service, TestPrincipal, calls)
}

/// Forwards every call to PostgreSQL but releases the first gated `accept`
/// only after the test commits a competing receipt, forcing the
/// lookup-miss then `OperationConflict` reread path with real rows.
#[derive(Clone)]
struct GatedJournal<J> {
    inner: J,
    lookup_done: Arc<tokio::sync::Notify>,
    accept_gate: Arc<tokio::sync::Notify>,
    gate: Arc<AtomicBool>,
}

impl<J> ImageJournal for GatedJournal<J>
where
    J: ImageJournal,
{
    async fn accept(
        &self,
        command: &df_imagery::AcceptCommand,
    ) -> Result<df_imagery::AcceptOutcome, JournalError> {
        if self.gate.load(Ordering::SeqCst) {
            self.accept_gate.notified().await;
            self.gate.store(false, Ordering::SeqCst);
        }
        self.inner.accept(command).await
    }

    async fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> Result<Option<df_imagery::OperationView>, JournalError> {
        let view = self.inner.lookup_operation(owner, operation).await?;
        self.lookup_done.notify_one();
        Ok(view)
    }

    async fn read_job(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<df_imagery::ImageJob>, JournalError> {
        self.inner.read_job(owner, job).await
    }

    async fn freeze_recipe(
        &self,
        command: &df_imagery::FreezeCommand,
    ) -> Result<df_imagery::FreezeOutcome, JournalError> {
        self.inner.freeze_recipe(command).await
    }

    async fn withdraw_interest(
        &self,
        owner: &ImageOwner,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> Result<df_imagery::WithdrawOutcome, JournalError> {
        self.inner.withdraw_interest(owner, interest, now_ms).await
    }
}

/// Forwards every call to PostgreSQL but fails exactly one numbered lookup,
/// so the post-mutation reread path is exercised against a real withdrawal.
#[derive(Clone)]
struct FailLookupJournal<J> {
    inner: J,
    count: Arc<AtomicU64>,
    fail_on: u64,
    error: JournalError,
}

impl<J> ImageJournal for FailLookupJournal<J>
where
    J: ImageJournal,
{
    async fn accept(
        &self,
        command: &df_imagery::AcceptCommand,
    ) -> Result<df_imagery::AcceptOutcome, JournalError> {
        self.inner.accept(command).await
    }

    async fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> Result<Option<df_imagery::OperationView>, JournalError> {
        let seen = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        if seen == self.fail_on {
            return Err(self.error);
        }
        self.inner.lookup_operation(owner, operation).await
    }

    async fn read_job(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<df_imagery::ImageJob>, JournalError> {
        self.inner.read_job(owner, job).await
    }

    async fn freeze_recipe(
        &self,
        command: &df_imagery::FreezeCommand,
    ) -> Result<df_imagery::FreezeOutcome, JournalError> {
        self.inner.freeze_recipe(command).await
    }

    async fn withdraw_interest(
        &self,
        owner: &ImageOwner,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> Result<df_imagery::WithdrawOutcome, JournalError> {
        self.inner.withdraw_interest(owner, interest, now_ms).await
    }
}

/// Forwards every call to PostgreSQL but reports the first committed accept
/// as unknown, so commit uncertainty is exercised against real rows.
#[derive(Clone)]
struct CommitUnknownJournal {
    inner: PostgresImageJournal,
    unknown_sent: Arc<AtomicBool>,
}

impl ImageJournal for CommitUnknownJournal {
    async fn accept(
        &self,
        command: &df_imagery::AcceptCommand,
    ) -> Result<df_imagery::AcceptOutcome, JournalError> {
        let outcome = self.inner.accept(command).await?;
        if !self.unknown_sent.swap(true, Ordering::SeqCst) {
            return Err(JournalError::Unknown);
        }
        Ok(outcome)
    }

    async fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> Result<Option<df_imagery::OperationView>, JournalError> {
        self.inner.lookup_operation(owner, operation).await
    }

    async fn read_job(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<df_imagery::ImageJob>, JournalError> {
        self.inner.read_job(owner, job).await
    }

    async fn freeze_recipe(
        &self,
        command: &df_imagery::FreezeCommand,
    ) -> Result<df_imagery::FreezeOutcome, JournalError> {
        self.inner.freeze_recipe(command).await
    }

    async fn withdraw_interest(
        &self,
        owner: &ImageOwner,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> Result<df_imagery::WithdrawOutcome, JournalError> {
        self.inner.withdraw_interest(owner, interest, now_ms).await
    }
}

fn recorded(outcome: RequestOutcome) -> df_imagery::ImageDemandView {
    match outcome {
        RequestOutcome::Recorded(view) => view,
        RequestOutcome::Replayed(_) => panic!("expected Recorded"),
        RequestOutcome::Reused(_) | RequestOutcome::ReusedArtifact(_) => {
            panic!("expected Recorded")
        }
        RequestOutcome::Refused(_) => panic!("expected Recorded"),
    }
}

#[test]
fn generate_edit_and_transform_round_trip_replay_read_watch_and_cancel() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = draft_owner();
        let (service, principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        let cases = [
            (Kind::Generate, 0x21_u8),
            (Kind::Edit, 0x22_u8),
            (Kind::Transform, 0x23_u8),
        ];
        for (kind, byte) in cases {
            let operation = operation_id(byte);
            let request = stamp(&service, request_for(kind, owner.clone(), operation));
            let handle = OperationHandle {
                owner: owner.clone(),
                operation,
            };
            let view = recorded(
                tokio::time::timeout(
                    Duration::from_secs(BOUND_SECS),
                    service.request(&principal, &request),
                )
                .await
                .expect("bounded")
                .unwrap(),
            );
            assert_eq!(view.handle, handle);
            assert_eq!(view.job_state, JobState::Planning);
            assert_eq!(view.interest_state, InterestState::Attached);
            assert_eq!(view.job_revision, 1);
            assert_eq!(view.absolute_deadline_ms, RECORDED_DEADLINE_MS);

            let read = tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.read(&principal, &handle),
            )
            .await
            .expect("bounded")
            .unwrap();
            assert_eq!(read, view);

            match tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &request),
            )
            .await
            .expect("bounded")
            .unwrap()
            {
                RequestOutcome::Replayed(replayed) => assert_eq!(replayed, view),
                other => panic!("expected Replayed, got {other:?}"),
            }

            match tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.watch(&principal, &handle, 0),
            )
            .await
            .expect("bounded")
            .unwrap()
            {
                WatchObservation::Changed(changed) => assert_eq!(changed, Box::new(view)),
                WatchObservation::Unchanged { .. } => panic!("expected Changed"),
            }
            match tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.watch(&principal, &handle, 1),
            )
            .await
            .expect("bounded")
            .unwrap()
            {
                WatchObservation::Unchanged { revision } => assert_eq!(revision, 1),
                WatchObservation::Changed(_) => panic!("expected Unchanged"),
            }

            match tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.cancel(&principal, &handle),
            )
            .await
            .expect("bounded")
            .unwrap()
            {
                CancelOutcome::Withdrawn(receipt) => {
                    assert_eq!(receipt.view.handle, handle);
                    assert_eq!(receipt.view.interest_state, InterestState::Withdrawn);
                    assert_eq!(receipt.view.job_state, JobState::Retired);
                    assert_eq!(receipt.view.job_revision, 2);
                }
                CancelOutcome::AlreadyWithdrawn(_) => panic!("expected Withdrawn"),
            }

            let retired = tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.read(&principal, &handle),
            )
            .await
            .expect("bounded")
            .unwrap();
            assert_eq!(retired.job_state, JobState::Retired);
            assert_eq!(retired.interest_state, InterestState::Withdrawn);

            match tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.cancel(&principal, &handle),
            )
            .await
            .expect("bounded")
            .unwrap()
            {
                CancelOutcome::AlreadyWithdrawn(receipt) => {
                    assert_eq!(receipt.view.job_revision, 2);
                }
                CancelOutcome::Withdrawn(_) => panic!("expected idempotent cancel"),
            }

            match tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &request),
            )
            .await
            .expect("bounded")
            .unwrap()
            {
                RequestOutcome::Replayed(replayed) => {
                    assert_eq!(replayed.job_state, JobState::Retired);
                    assert_eq!(replayed.interest_state, InterestState::Withdrawn);
                }
                other => panic!("retired receipts stay readable, got {other:?}"),
            }
        }
        assert_eq!(fixture.row_count("image_operation").await, 3);
        fixture.finish().await;
    });
}

#[test]
fn concurrent_same_operation_same_intent_records_once() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = draft_owner();
        let operation = operation_id(0x31);
        let (probe, _, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        let request = stamp(&probe, generate_request(owner.clone(), operation));
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let spawn_request = |journal: PostgresImageJournal,
                             owner: ImageOwner,
                             barrier: Arc<tokio::sync::Barrier>,
                             request: ImageRequest| {
            tokio::spawn(async move {
                barrier.wait().await;
                let clock = PgClock {
                    now: Arc::new(AtomicI64::new(START_MS)),
                };
                let service = ImageService::new(
                    journal,
                    ImageConfig::defaults().unwrap(),
                    clock,
                    PgIds::fresh(),
                    PgAuth::permitting(owner),
                )
                .unwrap();
                service.request(&TestPrincipal, &request).await
            })
        };
        let first = spawn_request(
            fixture.journal.clone(),
            owner.clone(),
            barrier.clone(),
            request.clone(),
        );
        let second = spawn_request(
            fixture.journal.clone(),
            owner.clone(),
            barrier,
            request.clone(),
        );
        let timeout = Duration::from_secs(BOUND_SECS);
        let first_outcome = tokio::time::timeout(timeout, first)
            .await
            .expect("bounded")
            .expect("task")
            .unwrap();
        let second_outcome = tokio::time::timeout(timeout, second)
            .await
            .expect("bounded")
            .expect("task")
            .unwrap();
        let views = [first_outcome, second_outcome].map(|outcome| match outcome {
            RequestOutcome::Recorded(view) | RequestOutcome::Replayed(view) => view,
            RequestOutcome::Reused(view) => view,
            RequestOutcome::ReusedArtifact(_) => panic!("race never reuses an artifact"),
            RequestOutcome::Refused(_) => panic!("race never refuses"),
        });
        assert_eq!(views[0], views[1]);
        assert_eq!(views[0].job_revision, 1);
        assert_eq!(views[0].job_state, JobState::Planning);
        assert_eq!(fixture.row_count("image_job").await, 1);
        assert_eq!(fixture.row_count("image_operation").await, 1);
        assert_eq!(fixture.row_count("image_interest").await, 1);
        fixture.finish().await;
    });
}

#[test]
fn concurrent_different_intent_conflicts_for_authorized_reader() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = draft_owner();
        let operation = operation_id(0x32);
        let (probe, _, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        let winner_request = stamp(&probe, generate_request(owner.clone(), operation));
        let mut loser_unstamped = generate_request(owner.clone(), operation);
        loser_unstamped.operation = ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a different tavern entirely", 3).unwrap(),
            references: Vec::new(),
        });
        let gated = GatedJournal {
            inner: fixture.journal.clone(),
            lookup_done: Arc::new(tokio::sync::Notify::new()),
            accept_gate: Arc::new(tokio::sync::Notify::new()),
            gate: Arc::new(AtomicBool::new(true)),
        };
        let lookup_done = gated.lookup_done.clone();
        let accept_gate = gated.accept_gate.clone();
        let (loser_service, loser_principal, loser_calls) =
            pg_service(gated, owner.clone(), StoredPolicy::AllowAll);
        let loser_request = stamp(&loser_service, loser_unstamped);
        let loser_task = tokio::spawn(async move {
            loser_service
                .request(&loser_principal, &loser_request)
                .await
        });
        tokio::time::timeout(Duration::from_secs(BOUND_SECS), lookup_done.notified())
            .await
            .expect("loser lookup misses first");
        let (winner_service, winner_principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            winner_service.request(&winner_principal, &winner_request),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            RequestOutcome::Recorded(_) => {}
            other => panic!("expected winner Recorded, got {other:?}"),
        }
        accept_gate.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(BOUND_SECS), loser_task)
                .await
                .expect("bounded")
                .expect("task")
                .unwrap_err(),
            ImageServiceError::Conflict
        );
        assert!(
            loser_calls
                .lock()
                .unwrap()
                .contains(&"stored-read".to_owned())
        );
        assert_eq!(fixture.row_count("image_job").await, 1);
        assert_eq!(fixture.row_count("image_operation").await, 1);
        assert_eq!(fixture.row_count("image_interest").await, 1);
        fixture.finish().await;
    });
}

#[test]
fn concurrent_different_intent_denied_before_conflict_disclosure() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = draft_owner();
        let operation = operation_id(0x33);
        let (probe, _, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        let winner_request = stamp(&probe, generate_request(owner.clone(), operation));
        let mut loser_unstamped = generate_request(owner.clone(), operation);
        loser_unstamped.operation = ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a different tavern entirely", 3).unwrap(),
            references: Vec::new(),
        });
        let gated = GatedJournal {
            inner: fixture.journal.clone(),
            lookup_done: Arc::new(tokio::sync::Notify::new()),
            accept_gate: Arc::new(tokio::sync::Notify::new()),
            gate: Arc::new(AtomicBool::new(true)),
        };
        let lookup_done = gated.lookup_done.clone();
        let accept_gate = gated.accept_gate.clone();
        let (loser_service, loser_principal, loser_calls) =
            pg_service(gated, owner.clone(), StoredPolicy::DenyAll);
        let loser_request = stamp(&loser_service, loser_unstamped);
        let loser_task = tokio::spawn(async move {
            loser_service
                .request(&loser_principal, &loser_request)
                .await
        });
        tokio::time::timeout(Duration::from_secs(BOUND_SECS), lookup_done.notified())
            .await
            .expect("loser lookup misses first");
        let (winner_service, winner_principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            winner_service.request(&winner_principal, &winner_request),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            RequestOutcome::Recorded(_) => {}
            other => panic!("expected winner Recorded, got {other:?}"),
        }
        accept_gate.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(BOUND_SECS), loser_task)
                .await
                .expect("bounded")
                .expect("task")
                .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        assert!(
            loser_calls
                .lock()
                .unwrap()
                .contains(&"stored-read".to_owned())
        );
        assert_eq!(fixture.row_count("image_job").await, 1);
        assert_eq!(fixture.row_count("image_interest").await, 1);
        fixture.finish().await;
    });
}

#[test]
fn wrong_owner_and_stored_audience_reveal_nothing() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = run_owner();
        let operation = operation_id(0x34);
        let (service, principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::PublicOnly,
        );
        let mut private_request = stamp(&service, generate_request(owner.clone(), operation));
        private_request.audience = Audience::OwnerPrivate(account_id(0x77));
        let private_request = stamp(&service, private_request);
        let handle = OperationHandle {
            owner: owner.clone(),
            operation,
        };
        // Refused before recording: a private demand the stored policy would hide is never written.
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &private_request),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &private_request),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.read(&principal, &handle),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.watch(&principal, &handle, 0),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.cancel(&principal, &handle),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );

        let (stranger, stranger_principal, _) = pg_service(
            fixture.journal.clone(),
            other_draft_owner(),
            StoredPolicy::AllowAll,
        );
        let foreign = OperationHandle {
            owner: other_draft_owner(),
            operation,
        };
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                stranger.read(&stranger_principal, &foreign),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                stranger.cancel(&stranger_principal, &foreign),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                stranger.read(&stranger_principal, &handle),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        let forged = stamp(
            &stranger,
            generate_request(other_draft_owner(), operation_id(0x35)),
        );
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &forged),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        assert_eq!(fixture.row_count("image_operation").await, 0);
        fixture.finish().await;
    });
}

#[test]
fn same_operation_bytes_across_owners_stay_scoped() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let operation = operation_id(0x36);
        let (first, first_principal, _) = pg_service(
            fixture.journal.clone(),
            draft_owner(),
            StoredPolicy::AllowAll,
        );
        let (second, second_principal, _) = pg_service(
            fixture.journal.clone(),
            other_draft_owner(),
            StoredPolicy::AllowAll,
        );
        let first_request = stamp(&first, generate_request(draft_owner(), operation));
        let second_request = stamp(&second, generate_request(other_draft_owner(), operation));
        let first_view = recorded(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                first.request(&first_principal, &first_request),
            )
            .await
            .expect("bounded")
            .unwrap(),
        );
        let second_view = recorded(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                second.request(&second_principal, &second_request),
            )
            .await
            .expect("bounded")
            .unwrap(),
        );
        assert_ne!(first_view.job, second_view.job);
        assert_eq!(first_view.job_revision, 1);
        assert_eq!(second_view.job_revision, 1);
        let reread = tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            first.read(&first_principal, &first_view.handle),
        )
        .await
        .expect("bounded")
        .unwrap();
        assert_eq!(reread, first_view);
        assert_eq!(fixture.row_count("image_operation").await, 2);
        fixture.finish().await;
    });
}

#[test]
fn demand_revisions_and_cancellation_survive_a_fresh_service() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = draft_owner();
        let operation = operation_id(0x37);
        let (service, principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        let request = stamp(&service, generate_request(owner.clone(), operation));
        let handle = OperationHandle {
            owner: owner.clone(),
            operation,
        };
        recorded(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &request),
            )
            .await
            .expect("bounded")
            .unwrap(),
        );
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            service.cancel(&principal, &handle),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            CancelOutcome::Withdrawn(_) => {}
            CancelOutcome::AlreadyWithdrawn(_) => panic!("expected Withdrawn"),
        }

        let reopened = fixture.reopen().await;
        let (fresh, fresh_principal, _) =
            pg_service(reopened, owner.clone(), StoredPolicy::AllowAll);
        let reread = tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            fresh.read(&fresh_principal, &handle),
        )
        .await
        .expect("bounded")
        .unwrap();
        assert_eq!(reread.job_state, JobState::Retired);
        assert_eq!(reread.interest_state, InterestState::Withdrawn);
        assert_eq!(reread.job_revision, 2);
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            fresh.watch(&fresh_principal, &handle, 1),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            WatchObservation::Changed(changed) => assert_eq!(changed, Box::new(reread.clone())),
            WatchObservation::Unchanged { .. } => panic!("expected Changed"),
        }
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            fresh.request(&fresh_principal, &request),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            RequestOutcome::Replayed(replayed) => assert_eq!(replayed, reread),
            other => panic!("restart keeps receipts readable, got {other:?}"),
        }
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            fresh.cancel(&fresh_principal, &handle),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            CancelOutcome::AlreadyWithdrawn(receipt) => {
                assert_eq!(receipt.view.job_revision, 2);
            }
            CancelOutcome::Withdrawn(_) => panic!("expected idempotent cancel"),
        }
        fixture.finish().await;
    });
}

#[test]
fn post_mutation_lookup_failure_returns_unknown_then_recovers() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = draft_owner();
        let operation = operation_id(0x38);
        let (service, principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        let request = stamp(&service, generate_request(owner.clone(), operation));
        let handle = OperationHandle {
            owner: owner.clone(),
            operation,
        };
        recorded(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &request),
            )
            .await
            .expect("bounded")
            .unwrap(),
        );

        let failing = FailLookupJournal {
            inner: fixture.journal.clone(),
            count: Arc::new(AtomicU64::new(0)),
            fail_on: 2,
            error: JournalError::Unavailable,
        };
        let (fragile, fragile_principal, _) =
            pg_service(failing, owner.clone(), StoredPolicy::AllowAll);
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                fragile.cancel(&fragile_principal, &handle),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::Unknown {
                handle: handle.clone()
            }
        );

        let (steady, steady_principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            steady.cancel(&steady_principal, &handle),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            CancelOutcome::AlreadyWithdrawn(receipt) => {
                assert_eq!(receipt.view.interest_state, InterestState::Withdrawn);
                assert_eq!(receipt.view.job_state, JobState::Retired);
                assert_eq!(receipt.view.job_revision, 2);
            }
            CancelOutcome::Withdrawn(_) => panic!("withdrawal already committed"),
        }
        fixture.finish().await;
    });
}

#[test]
fn refusals_and_mismatches_fail_before_any_mutation() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = draft_owner();
        let (service, principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );

        let mut withheld = stamp(
            &service,
            generate_request(owner.clone(), operation_id(0x41)),
        );
        withheld.audience = Audience::Withheld;
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &withheld),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::InvalidRequest(RequestProblem::WithheldAudience)
        );

        let tampered = generate_request(owner.clone(), operation_id(0x42));
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &tampered),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::InvalidRequest(RequestProblem::FingerprintMismatch)
        );

        let forged = stamp(
            &service,
            generate_request(other_draft_owner(), operation_id(0x43)),
        );
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &forged),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );

        let off_config = ImageConfig {
            mode: GlobalMode::Off,
            ..ImageConfig::defaults().unwrap()
        };
        let off = ImageService::new(
            fixture.journal.clone(),
            off_config,
            PgClock {
                now: Arc::new(AtomicI64::new(START_MS)),
            },
            PgIds::fresh(),
            PgAuth::permitting(owner.clone()),
        )
        .unwrap();
        let off_request = stamp(&off, generate_request(owner.clone(), operation_id(0x44)));
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            off.request(&TestPrincipal, &off_request),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            RequestOutcome::Refused(_) => {}
            other => panic!("expected Off refusal, got {other:?}"),
        }

        let mut disabled = ImageConfig::defaults().unwrap();
        disabled.purposes[Purpose::ScenePlate.index()].enabled = false;
        let gated = ImageService::new(
            fixture.journal.clone(),
            disabled,
            PgClock {
                now: Arc::new(AtomicI64::new(START_MS)),
            },
            PgIds::fresh(),
            PgAuth::permitting(owner.clone()),
        )
        .unwrap();
        let disabled_request = stamp(&gated, generate_request(owner.clone(), operation_id(0x45)));
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            gated.request(&TestPrincipal, &disabled_request),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            RequestOutcome::Refused(_) => {}
            other => panic!("expected purpose refusal, got {other:?}"),
        }

        let mut prepared = stamp(
            &service,
            generate_request(owner.clone(), operation_id(0x46)),
        );
        prepared.execution = ExecutionMode::PreparedOnly;
        let prepared = stamp(&service, prepared);
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            service.request(&principal, &prepared),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            RequestOutcome::Refused(_) => {}
            other => panic!("expected prepared miss, got {other:?}"),
        }

        let mut replay = stamp(
            &service,
            generate_request(owner.clone(), operation_id(0x47)),
        );
        replay.execution = ExecutionMode::Replay;
        let replay = stamp(&service, replay);
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            service.request(&principal, &replay),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            RequestOutcome::Refused(_) => {}
            other => panic!("expected replay miss, got {other:?}"),
        }

        let cache_config = ImageConfig {
            mode: GlobalMode::CacheOnly,
            ..ImageConfig::defaults().unwrap()
        };
        let cached = ImageService::new(
            fixture.journal.clone(),
            cache_config,
            PgClock {
                now: Arc::new(AtomicI64::new(START_MS)),
            },
            PgIds::fresh(),
            PgAuth::permitting(owner.clone()),
        )
        .unwrap();
        let cache_request = stamp(&cached, generate_request(owner.clone(), operation_id(0x48)));
        match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            cached.request(&TestPrincipal, &cache_request),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            RequestOutcome::Refused(_) => {}
            other => panic!("expected cache-only miss, got {other:?}"),
        }

        assert_eq!(fixture.row_count("image_operation").await, 0);
        assert_eq!(fixture.row_count("image_job").await, 0);
        assert_eq!(fixture.row_count("image_interest").await, 0);
        fixture.finish().await;
    });
}

#[test]
fn committed_accept_unknown_returns_handle_then_replays_single_receipt() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = draft_owner();
        let operation = operation_id(0x39);
        let (probe, _, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        let request = stamp(&probe, generate_request(owner.clone(), operation));
        let handle = OperationHandle {
            owner: owner.clone(),
            operation,
        };
        let faulty = CommitUnknownJournal {
            inner: fixture.journal.clone(),
            unknown_sent: Arc::new(AtomicBool::new(false)),
        };
        let (service, principal, calls) = pg_service(faulty, owner.clone(), StoredPolicy::AllowAll);
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(BOUND_SECS),
                service.request(&principal, &request),
            )
            .await
            .expect("bounded")
            .unwrap_err(),
            ImageServiceError::Unknown {
                handle: handle.clone()
            }
        );
        let auth_calls = calls.lock().unwrap().clone();
        assert!(auth_calls.contains(&"request".to_owned()));
        assert!(auth_calls.contains(&"owner-generate".to_owned()));
        let (steady, steady_principal, steady_calls) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        let replayed = match tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            steady.request(&steady_principal, &request),
        )
        .await
        .expect("bounded")
        .unwrap()
        {
            RequestOutcome::Replayed(view) => view,
            other => panic!("expected Replayed after commit, got {other:?}"),
        };
        assert_eq!(replayed.handle, handle);
        assert_eq!(replayed.job_revision, 1);
        assert_eq!(replayed.job_state, JobState::Planning);
        assert_eq!(replayed.interest_state, InterestState::Attached);
        assert_eq!(replayed.absolute_deadline_ms, RECORDED_DEADLINE_MS);
        let reread = tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            steady.read(&steady_principal, &handle),
        )
        .await
        .expect("bounded")
        .unwrap();
        assert_eq!(reread, replayed);
        assert!(
            steady_calls
                .lock()
                .unwrap()
                .contains(&"stored-read".to_owned())
        );
        assert_eq!(fixture.row_count("image_operation").await, 1);
        assert_eq!(fixture.row_count("image_job").await, 1);
        assert_eq!(fixture.row_count("image_interest").await, 1);
        fixture.finish().await;
    });
}

#[test]
fn operator_revoke_withdraws_each_attached_interest_once_and_refuses_strangers() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = run_owner();
        let (service, principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        for byte in [0x40_u8, 0x41] {
            let request = stamp(
                &service,
                generate_request(owner.clone(), operation_id(byte)),
            );
            recorded(
                tokio::time::timeout(
                    Duration::from_secs(BOUND_SECS),
                    service.request(&principal, &request),
                )
                .await
                .expect("bounded")
                .unwrap(),
            );
        }
        let report = tokio::time::timeout(
            Duration::from_secs(BOUND_SECS),
            service.revoke(&principal, &owner),
        )
        .await
        .expect("bounded")
        .unwrap();
        assert_eq!(report.withdrawn, 2);
        assert_eq!(report.already_withdrawn, 0);
        assert_eq!(
            report.jobs_retired, 2,
            "each job loses its last interest and retires"
        );
        let again = service.revoke(&principal, &owner).await.unwrap();
        assert_eq!((again.withdrawn, again.jobs_retired), (0, 0));

        let (stranger, stranger_principal, _) = pg_service(
            fixture.journal.clone(),
            other_draft_owner(),
            StoredPolicy::AllowAll,
        );
        assert_eq!(
            stranger
                .revoke(&stranger_principal, &owner)
                .await
                .unwrap_err(),
            ImageServiceError::NotAuthorizedOrNotFound
        );
        fixture.finish().await;
    });
}

#[test]
fn bounded_watch_returns_unchanged_after_its_wait_and_zero_wait_never_sleeps() {
    runtime().block_on(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let owner = run_owner();
        let operation = operation_id(0x42);
        let (service, principal, _) = pg_service(
            fixture.journal.clone(),
            owner.clone(),
            StoredPolicy::AllowAll,
        );
        let request = stamp(&service, generate_request(owner.clone(), operation));
        let outcome = service.request(&principal, &request).await.unwrap();
        let RequestOutcome::Recorded(view) = outcome else {
            panic!("expected Recorded");
        };
        let handle = OperationHandle {
            owner: owner.clone(),
            operation,
        };
        let started = std::time::Instant::now();
        let zero = service
            .watch_within(&principal, &handle, view.job_revision, Duration::ZERO)
            .await
            .unwrap();
        assert!(matches!(zero, WatchObservation::Unchanged { .. }));
        assert!(started.elapsed() < Duration::from_millis(200));
        let waited = service
            .watch_within(
                &principal,
                &handle,
                view.job_revision,
                Duration::from_millis(600),
            )
            .await
            .unwrap();
        assert!(matches!(waited, WatchObservation::Unchanged { .. }));
        assert!(started.elapsed() >= Duration::from_millis(600));
        fixture.finish().await;
    });
}
