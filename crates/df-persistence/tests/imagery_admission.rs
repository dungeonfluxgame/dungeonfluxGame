//! Crate-boundary tests for the immutable campaign image admission binding:
//! the finance [`ImageStagePort`](df_finance::ImageStagePort) backed by real
//! catalog/journal rows in one PostgreSQL transaction.
//!
//! Every test uses an owned scratch database named by
//! `DUNGEONFLUX_ADMISSION_PG_PORT` (host `DUNGEONFLUX_ADMISSION_PG_HOST`,
//! default 127.0.0.1; user `DUNGEONFLUX_ADMISSION_PG_USER`, default
//! postgres) and drops only that database at the end. Without the variable
//! every test reports SKIPPED. The admission transaction runs as the
//! `df_imagery_admission` composition role from migration 0039 (inheriting the
//! ordinary `df_finance` permission plus the exact media/imagery grants); plain
//! `df_finance` must stay unable to admit. Finance rows always come from the
//! real `finance.lock_flow`/`finance.commit_event` definers through
//! [`FinanceService`](df_finance::service::FinanceService) (published price
//! version, opened session, confirmed purchase, allocation, admission); the
//! catalog and journal go through their real adapters. Only the fence and
//! the run/session authority are deterministic fakes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use df_assets::{
    Admission, AssetKind, AttemptId as CatalogAttemptId, Audience, CatalogStore, DedupeScope,
    MediaRequestKey, MemberSet, Provenance, QuotaScope, RetentionClass,
};
use df_commerce::admission::{AdmissionRefusal, WorkClass};
use df_commerce::digest::business_key;
use df_commerce::finance_ids::{
    AttemptId as FinanceAttemptId, CampaignKey, EffectId, OperationKey, PayerId, ReservationId,
    SessionKey, TenantKey,
};
use df_commerce::funding_flows::{Consent, SessionPolicy};
use df_commerce::ledger::CurrencyTag;
use df_commerce::price_schedule::{PriceVersion, TariffLine};
use df_finance::admit::{AdmitCommand, priced_route};
use df_finance::fakes::{FakeFence, FakeMedia, FakeMembers, SeededIds, SystemClock};
use df_finance::ports::{FenceRequest, FenceVerdict, MediaVerdict};
use df_finance::service::{FinanceConfig, FinanceError, FinanceService, SessionOpening};
use df_finance::{
    ImageAssetId, ImageJobId as FinanceJobId, ImageStageAdmission, ImageStageBinding, ImageStageId,
    ImageStagePort, OwnerScope, PublicationAttemptId, StageRefusal,
};
use df_imagery::{
    AcceptCommand, AcceptTarget, FreezeCommand, FreezeOutcome, ImageInterestId,
    ImageJobId as JournalJobId, ImageJournal, ImageOwner, OperationIdentity, SharingIdentity,
    VersionedSnapshot, owner_scope_bytes,
};
use df_persistence::PostgresImageJournal;
use df_persistence::media_catalog::PostgresMediaCatalog;
use df_persistence::postgres_tls::PostgresTransport;
use df_persistence::{
    ImageryAdmissionConfig, ImageryAdmissionPort, PortConfigError, RunSessionAuthority,
    RunSessionError,
};
use df_types::{AccountId, CampaignId, MemberId, OperationId, RunId};
use tokio_postgres::{Client, Config, IsolationLevel, NoTls};

const CURRENCY: CurrencyTag = CurrencyTag::USD;
// Imagery journal/port keep a fixed deterministic time; finance uses the wall
// clock so purchases land in the currently open period (FN003 guards it).
const NOW_MS: i64 = 1_700_000_000_000;
const DEADLINE_MS: i64 = NOW_MS + 50_000;
const SNAPSHOT_BOUND: usize = 1 << 20;

/// Never these ports: the game's and the other owned test servers.
const FORBIDDEN_PORTS: [u16; 4] = [0, 55517, 55439, 63309];

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime")
}

async fn connect(config: &Config) -> Client {
    let (client, connection) = config.connect(NoTls).await.expect("scratch server");
    tokio::spawn(async move {
        let _ended = connection.await;
    });
    client
}

/// Roles are cluster-wide: finance migrations serialize on an advisory lock.
static MIGRATING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Scratch {
    admin: Config,
    database: String,
}

impl Scratch {
    async fn open() -> Option<Self> {
        let Ok(port) = std::env::var("DUNGEONFLUX_ADMISSION_PG_PORT") else {
            eprintln!("SKIPPED: no scratch PostgreSQL (DUNGEONFLUX_ADMISSION_PG_PORT)");
            return None;
        };
        let port: u16 = port.parse().expect("scratch port");
        assert!(!FORBIDDEN_PORTS.contains(&port));
        let mut admin = Config::new();
        admin
            .host(std::env::var("DUNGEONFLUX_ADMISSION_PG_HOST").unwrap_or("127.0.0.1".into()))
            .port(port)
            .user(std::env::var("DUNGEONFLUX_ADMISSION_PG_USER").unwrap_or("postgres".into()))
            .dbname("postgres");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::SeqCst);
        let database = format!("df_admission_{nanos}_{}_{serial}", std::process::id());
        connect(&admin)
            .await
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .expect("create test database");
        let scratch = Self { admin, database };
        scratch.migrate().await;
        Some(scratch)
    }

    async fn migrate(&self) {
        let _held = MIGRATING.lock().await;
        let mut client = self.connect().await;
        let gate = connect(&self.admin).await;
        gate.execute("SELECT pg_advisory_lock(7000039)", &[])
            .await
            .expect("migration gate");
        let transaction = client.transaction().await.expect("migration transaction");
        df_persistence::imagery_migrations::migrate_local_in(&transaction)
            .await
            .expect("trusted local installer");
        transaction.commit().await.expect("migration commit");
        gate.execute("SELECT pg_advisory_unlock(7000039)", &[])
            .await
            .expect("migration gate release");
    }

    fn config(&self) -> Config {
        let mut config = self.admin.clone();
        config.dbname(&self.database);
        config
    }

    async fn connect(&self) -> Client {
        connect(&self.config()).await
    }

    async fn finish(self) {
        connect(&self.admin)
            .await
            .batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                self.database
            ))
            .await
            .expect("drop test database");
    }
}

/// The deterministic run/session authority fake: it pins one exact
/// (tenant, campaign, run, session) tuple and refuses anything else. It
/// never equates a run id with a session key; the mapping is explicit.
struct FakeAuthority {
    tenant: [u8; 16],
    campaign: [u8; 16],
    run: [u8; 16],
    session: [u8; 16],
}

impl RunSessionAuthority for FakeAuthority {
    async fn verify_canonical_run(
        &self,
        _transaction: &tokio_postgres::Transaction<'_>,
        owner: &ImageOwner,
        binding: &ImageStageBinding,
        _receipt: &df_commerce::snapshot::ReceiptRow,
    ) -> Result<(), RunSessionError> {
        let ImageOwner::Run { campaign, run } = owner else {
            return Err(RunSessionError::Refused);
        };
        if campaign.as_bytes() == &self.campaign
            && run.as_bytes() == &self.run
            && binding.tenant.as_bytes() == &self.tenant
            && binding.campaign.as_bytes() == &self.campaign
            && binding.session.as_bytes() == &self.session
        {
            Ok(())
        } else {
            Err(RunSessionError::Refused)
        }
    }
}

type Service = FinanceService<FakeFence, FakeMedia, FakeMembers>;
type Port = ImageryAdmissionPort<FakeAuthority>;

fn price_version() -> PriceVersion {
    let line = |category: &str| TariffLine {
        provider: "openai".into(),
        model: "gpt-6-luna".into(),
        category: category.into(),
        context_tier_max_tokens: 0,
        nano_usd: if category == "text_input" {
            100_000_000
        } else {
            500_000_000
        },
        per: 1_000_000,
    };
    PriceVersion {
        id: df_commerce::finance_ids::PriceVersionId::from_array([0x70; 16]),
        label: "pv-test1".into(),
        currency: CURRENCY,
        platform_rate_micro_dollars_per_hour: 2_000_000,
        markup_numerator: 3,
        markup_denominator: 2,
        tariff: vec![line("text_input"), line("text_output")],
        proposed_pilot: true,
    }
}

fn policy() -> SessionPolicy {
    SessionPolicy {
        hourly_ceiling_micro_dollars: 20_000_000,
        session_ceiling_micro_dollars: None,
        per_request_threshold_micro_dollars: 5_000_000,
    }
}

fn consent() -> Consent {
    Consent {
        approved_hourly_ceiling_micro_dollars: 20_000_000,
        approved_session_ceiling_micro_dollars: None,
        approval_threshold_micro_dollars: 500_000,
        terms_version: 1,
    }
}

fn payer(tag: u8) -> PayerId {
    PayerId::from_array([0xa0 + tag; 16])
}

struct World {
    scratch: Scratch,
    admin: Client,
    tenant: TenantKey,
    campaign: CampaignKey,
    campaign_id: CampaignId,
    run: RunId,
    session: SessionKey,
    version: PriceVersion,
    seed: AtomicU64,
}

impl World {
    async fn open() -> Option<Self> {
        let scratch = Scratch::open().await?;
        let admin = scratch.connect().await;
        let config = scratch.config();
        PostgresMediaCatalog::open(config.clone(), PostgresTransport::PlaintextLoopbackOnly, 2)
            .await
            .expect("open media catalog");
        PostgresImageJournal::open(config.clone(), PostgresTransport::PlaintextLoopbackOnly, 2)
            .await
            .expect("open imagery journal");
        let world = Self {
            scratch,
            admin,
            tenant: TenantKey::from_array([0x52; 16]),
            campaign: CampaignKey::from_array([0x53; 16]),
            campaign_id: CampaignId::from_bytes(&[0x53; 16]).expect("campaign id"),
            run: RunId::from_bytes(&[0x51; 16]).expect("run id"),
            session: SessionKey::from_array([0x51; 16]),
            version: price_version(),
            seed: AtomicU64::new(1000),
        };
        let mut service = world.service(vec![payer(1)]).await;
        service
            .publish_price_version(&world.version)
            .await
            .expect("publish the price version");
        service
            .open_session(SessionOpening {
                session: world.session,
                tenant: world.tenant,
                campaign: world.campaign,
                price_version: world.version.id,
                currency: CURRENCY,
                policy: policy(),
            })
            .await
            .expect("open the session");
        drop(service);
        Some(world)
    }

    async fn service(&self, members: Vec<PayerId>) -> Service {
        self.service_with(members, true).await
    }

    /// Race tests skip the lock timeout so a loser waits for the winner and
    /// reports the deterministic conflict instead of flaking on 55P03.
    async fn service_without_timeouts(&self, members: Vec<PayerId>) -> Service {
        self.service_with(members, false).await
    }

    async fn service_with(&self, members: Vec<PayerId>, timeouts: bool) -> Service {
        self.service_with_role(members, timeouts, "df_imagery_admission")
            .await
    }

    /// The ordinary finance service permission, without the composition grants:
    /// admission as this role must fail unavailable, proving `df_finance`
    /// gained nothing from migration 0039.
    async fn service_as_plain_finance(&self, members: Vec<PayerId>) -> Service {
        self.service_with_role(members, true, "df_finance").await
    }

    async fn service_with_role(
        &self,
        members: Vec<PayerId>,
        timeouts: bool,
        role: &str,
    ) -> Service {
        let client = self.scratch.connect().await;
        client
            .batch_execute(&format!("SET ROLE {role}"))
            .await
            .expect("set role");
        if timeouts {
            client
                .batch_execute("SET lock_timeout = '2s'; SET statement_timeout = '30s'")
                .await
                .expect("timeouts");
        }
        let seed = self.seed.fetch_add(1, Ordering::SeqCst);
        FinanceService::new(
            client,
            FakeFence::new(FenceVerdict::Current),
            FakeMedia::new(MediaVerdict::Admitted),
            FakeMembers::new(members),
            Box::new(SystemClock),
            Box::new(SeededIds::new(seed)),
            FinanceConfig::default(),
        )
    }

    async fn fund(&self, service: &mut Service, tag: u8, amount_micro_dollars: u64) {
        self.fund_session(
            service,
            tag,
            self.session,
            tag,
            format!("pay-{tag}"),
            amount_micro_dollars,
        )
        .await;
    }

    async fn fund_session(
        &self,
        service: &mut Service,
        tag: u8,
        session: SessionKey,
        op_tag: u8,
        payment: String,
        amount_micro_dollars: u64,
    ) {
        let payment: &'static str = Box::leak(payment.into_boxed_str());
        service
            .apply_purchase(df_finance::service::PurchaseCommand {
                payer: payer(tag),
                processor: "fake",
                payment_ref: payment,
                currency: CURRENCY,
                amount_micro_dollars,
            })
            .await
            .expect("confirmed purchase")
            .expect("purchase posts a lot");
        service
            .allocate(
                payer(tag),
                session,
                OperationKey::from_array([0x30 + op_tag; 16]),
                amount_micro_dollars,
                consent(),
            )
            .await
            .expect("allocation holds credit");
    }

    fn owner(&self) -> ImageOwner {
        ImageOwner::Run {
            campaign: self.campaign_id,
            run: self.run,
        }
    }

    fn authority(&self) -> FakeAuthority {
        FakeAuthority {
            tenant: *self.tenant.as_bytes(),
            campaign: *self.campaign.as_bytes(),
            run: *self.run.as_bytes(),
            session: *self.session.as_bytes(),
        }
    }

    fn fence(&self) -> FenceRequest {
        FenceRequest {
            tenant: self.tenant,
            campaign: self.campaign,
            recovery_epoch: 1,
            claim_seq: 1,
            placement_id: [0x54; 16],
        }
    }

    fn admit_command(&self, tag: u8, attempt: FinanceAttemptId, effect: EffectId) -> AdmitCommand {
        self.admit_command_for(tag, attempt, effect, self.fence(), self.session)
    }

    fn admit_command_for(
        &self,
        tag: u8,
        attempt: FinanceAttemptId,
        effect: EffectId,
        fence: FenceRequest,
        session: SessionKey,
    ) -> AdmitCommand {
        AdmitCommand {
            operation: OperationKey::from_array([0x40 + tag; 16]),
            attempt,
            effect,
            fence,
            session,
            route: priced_route(
                "openai",
                "gpt-6-luna",
                "narration",
                &["text_input", "text_output"],
            ),
            work_class: WorkClass::Essential,
            maximum_units: vec![("text_input".into(), 100), ("text_output".into(), 100)],
            estimate_units: vec![("text_input".into(), 50), ("text_output".into(), 50)],
            quote: None,
            request_digest: business_key("test_request", &[&[tag]]),
            media: None,
        }
    }

    /// Accepts a job under `operation` and freezes `recipe` under `key`.
    /// Returns the frozen job revision the port must expect.
    #[allow(clippy::too_many_arguments, reason = "a test fixture")]
    async fn frozen_job(
        &self,
        journal: &PostgresImageJournal,
        operation: OperationId,
        job: JournalJobId,
        interest: ImageInterestId,
        key: MediaRequestKey,
        recipe: &VersionedSnapshot,
        config: &VersionedSnapshot,
        deadline_ms: i64,
        now_ms: i64,
    ) -> u64 {
        self.frozen_job_for(
            journal,
            self.owner(),
            operation,
            job,
            interest,
            key,
            recipe,
            config,
            deadline_ms,
            now_ms,
        )
        .await
    }

    #[allow(clippy::too_many_arguments, reason = "a test fixture")]
    async fn frozen_job_for(
        &self,
        journal: &PostgresImageJournal,
        owner: ImageOwner,
        operation: OperationId,
        job: JournalJobId,
        interest: ImageInterestId,
        key: MediaRequestKey,
        recipe: &VersionedSnapshot,
        config: &VersionedSnapshot,
        deadline_ms: i64,
        now_ms: i64,
    ) -> u64 {
        let identity = OperationIdentity {
            owner: owner.clone(),
            operation,
            fingerprint: [7; 32],
        };
        let intent = VersionedSnapshot::new(1, vec![9; 16], SNAPSHOT_BOUND).expect("intent");
        let sharing = SharingIdentity::from_bytes(&[8; 32]).expect("sharing");
        let created = journal
            .accept(&AcceptCommand {
                identity,
                intent,
                config: config.clone(),
                sharing,
                interest,
                target: AcceptTarget::Create {
                    job,
                    absolute_deadline_ms: deadline_ms,
                },
                now_ms,
            })
            .await
            .expect("accept the job");
        let revision = created.view().job.revision;
        let frozen = journal
            .freeze_recipe(&FreezeCommand {
                owner,
                job,
                expected_revision: revision,
                key,
                recipe: recipe.clone(),
                now_ms,
            })
            .await
            .expect("freeze the recipe");
        let FreezeOutcome::Frozen { job } = frozen else {
            panic!("first freeze records the recipe");
        };
        job.revision
    }

    fn admission(
        &self,
        asset: df_assets::AssetId,
        attempt: CatalogAttemptId,
        key: MediaRequestKey,
    ) -> Admission {
        Admission {
            asset,
            attempt,
            kind: AssetKind::Image,
            provenance: Provenance::Generated,
            charge_scope: QuotaScope::Campaign(self.campaign_id),
            origin_account: None,
            campaign: Some(self.campaign_id),
            audience: Audience::CampaignShared(self.campaign_id),
            dedupe_scope: DedupeScope::campaign(&self.campaign_id),
            request_key: Some(key),
            retention: RetentionClass::CampaignMedia,
            reserve_bytes: 1 << 20,
            byte_limit: 1 << 30,
            in_flight_upload_limit: None,
        }
    }

    #[allow(clippy::too_many_arguments, reason = "a test fixture")]
    fn binding(
        &self,
        owner_scope: Vec<u8>,
        job: [u8; 16],
        stage: [u8; 16],
        asset: [u8; 16],
        publication_attempt: [u8; 16],
        operation: OperationKey,
        attempt: FinanceAttemptId,
        effect: EffectId,
    ) -> ImageStageBinding {
        ImageStageBinding {
            owner_scope: OwnerScope::new(owner_scope).expect("owner scope"),
            job: FinanceJobId::new(job).expect("job"),
            stage: ImageStageId::new(stage).expect("stage"),
            asset: ImageAssetId::new(asset).expect("asset"),
            publication_attempt: PublicationAttemptId::new(publication_attempt)
                .expect("publication attempt"),
            tenant: self.tenant,
            campaign: self.campaign,
            session: self.session,
            operation,
            attempt,
            effect,
        }
    }

    #[allow(clippy::too_many_arguments, reason = "a test fixture")]
    fn port(
        &self,
        admission: Admission,
        job: JournalJobId,
        stage: [u8; 16],
        key: MediaRequestKey,
        recipe: VersionedSnapshot,
        config: VersionedSnapshot,
        expected_job_revision: u64,
        deadline_ms: i64,
        now_ms: i64,
    ) -> Port {
        ImageryAdmissionPort::new(
            self.authority(),
            ImageryAdmissionConfig {
                admission,
                owner: self.owner(),
                job,
                stage,
                recipe_key: key,
                recipe,
                config,
                expected_job_revision,
                absolute_deadline_ms: deadline_ms,
                now_ms,
            },
        )
        .expect("valid port configuration")
    }

    async fn count(&self, table: &str) -> i64 {
        self.admin
            .query_one(&format!("SELECT COUNT(*) FROM {table}"), &[])
            .await
            .expect("count")
            .get(0)
    }

    async fn receipt_count(&self, campaign: CampaignKey, operation: OperationKey) -> i64 {
        self.admin
            .query_one(
                "SELECT COUNT(*) FROM finance.operation_receipt \
                 WHERE namespace = 'admission' AND principal = $1 AND operation_id = $2",
                &[
                    &campaign.as_bytes().as_slice(),
                    &operation.as_bytes().as_slice(),
                ],
            )
            .await
            .expect("receipt count")
            .get(0)
    }

    async fn finish(self) {
        self.scratch.finish().await;
    }

    async fn journal(&self) -> PostgresImageJournal {
        PostgresImageJournal::open(
            self.scratch.config(),
            PostgresTransport::PlaintextLoopbackOnly,
            2,
        )
        .await
        .expect("reopen imagery journal")
    }

    async fn catalog(&self) -> PostgresMediaCatalog {
        PostgresMediaCatalog::open(
            self.scratch.config(),
            PostgresTransport::PlaintextLoopbackOnly,
            2,
        )
        .await
        .expect("reopen media catalog")
    }
}

struct FirstAdmission {
    operation: OperationKey,
    attempt: FinanceAttemptId,
    effect: EffectId,
    job: JournalJobId,
    interest: ImageInterestId,
    stage: [u8; 16],
    asset: [u8; 16],
    publication_attempt: [u8; 16],
    key: MediaRequestKey,
    recipe: VersionedSnapshot,
    config: VersionedSnapshot,
    revision: u64,
}

impl FirstAdmission {
    fn ids(tag: u8) -> Self {
        Self {
            operation: OperationKey::from_array([0x40 + tag; 16]),
            attempt: FinanceAttemptId::from_array([0x60 + tag; 16]),
            effect: EffectId::from_array([0x80 + tag; 16]),
            job: JournalJobId::from_bytes(&[0x10 + tag; 16]).expect("job id"),
            interest: ImageInterestId::from_bytes(&[0x20 + tag; 16]).expect("interest id"),
            stage: [0x30 + tag; 16],
            asset: [0x90 + tag; 16],
            publication_attempt: [0xa0 + tag; 16],
            key: MediaRequestKey::from_bytes([0xb0 + tag; 32]),
            recipe: VersionedSnapshot::new(2, vec![0xc0 + tag; 64], SNAPSHOT_BOUND)
                .expect("recipe"),
            config: VersionedSnapshot::new(3, vec![0xd0 + tag; 64], SNAPSHOT_BOUND)
                .expect("config"),
            revision: 0,
        }
    }

    fn operation_id(&self, tag: u8) -> OperationId {
        OperationId::from_bytes(&[0x40 + tag; 16]).expect("operation id")
    }
}

/// Funds payer 1, freezes one job, and commits the first admission through
/// the real finance service. Returns the admitted shape for replays.
async fn admit_first(world: &World, tag: u8) -> FirstAdmission {
    admit_first_with_audience(world, tag, Audience::CampaignShared(world.campaign_id)).await
}

/// The same first admission carrying an explicit audience, for the
/// audience-compatibility regressions.
async fn admit_first_with_audience(world: &World, tag: u8, audience: Audience) -> FirstAdmission {
    let mut shaped = FirstAdmission::ids(tag);
    let mut service = world.service(vec![payer(1)]).await;
    world.fund(&mut service, 1, 50_000_000).await;
    let journal = world.journal().await;
    let operation = shaped.operation_id(tag);
    let revision = world
        .frozen_job(
            &journal,
            operation,
            shaped.job,
            shaped.interest,
            shaped.key,
            &shaped.recipe,
            &shaped.config,
            DEADLINE_MS,
            NOW_MS,
        )
        .await;
    shaped.revision = revision;
    let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
    let attempt =
        CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
    let mut admission = world.admission(asset, attempt, shaped.key);
    admission.audience = audience;
    let binding = world.binding(
        owner_scope_bytes(&world.owner()),
        *shaped.job.as_bytes(),
        shaped.stage,
        shaped.asset,
        shaped.publication_attempt,
        shaped.operation,
        shaped.attempt,
        shaped.effect,
    );
    let port = world.port(
        admission.clone(),
        shaped.job,
        shaped.stage,
        shaped.key,
        shaped.recipe.clone(),
        shaped.config.clone(),
        shaped.revision,
        DEADLINE_MS,
        NOW_MS,
    );
    let command = ImageStageAdmission {
        admit: world.admit_command(tag, shaped.attempt, shaped.effect),
        binding,
    };
    let outcome = service
        .admit_image_stage(&command, &port)
        .await
        .expect("first admission commits");
    assert!(
        matches!(
            outcome,
            df_finance::admit::AdmissionOutcome::Admitted { .. }
        ),
        "first admission is new, got {outcome:?}"
    );
    drop(service);
    shaped
}

#[test]
fn a_successful_new_admission_creates_one_of_each_row() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        admit_first(&world, 1).await;
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        assert_eq!(world.count("finance.provider_attempt").await, 1);
        assert_eq!(world.count("media.asset").await, 1);
        assert_eq!(world.count("media.quota_reservation").await, 1);
        assert_eq!(
            world
                .receipt_count(world.campaign, OperationKey::from_array([0x41; 16]))
                .await,
            1
        );
        world.finish().await;
    });
}

#[test]
fn an_exact_replay_returns_existing_and_changes_nothing() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        let mut service = world.service(vec![payer(1)]).await;
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(1, shaped.attempt, shaped.effect),
            binding,
        };
        let outcome = service
            .admit_image_stage(&command, &port)
            .await
            .expect("replay resolves");
        assert!(
            matches!(
                outcome,
                df_finance::admit::AdmissionOutcome::AlreadyAdmitted { .. }
            ),
            "same operation replays, got {outcome:?}"
        );
        drop(service);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        assert_eq!(world.count("finance.provider_attempt").await, 1);
        assert_eq!(world.count("media.asset").await, 1);
        world.finish().await;
    });
}

#[test]
fn a_quota_full_refusal_rolls_finance_plan_quota_and_binding_back() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let tag = 2;
        let mut shaped = FirstAdmission::ids(tag);
        let mut service = world.service(vec![payer(1)]).await;
        world.fund(&mut service, 1, 50_000_000).await;
        let journal = world.journal().await;
        let revision = world
            .frozen_job(
                &journal,
                shaped.operation_id(tag),
                shaped.job,
                shaped.interest,
                shaped.key,
                &shaped.recipe,
                &shaped.config,
                DEADLINE_MS,
                NOW_MS,
            )
            .await;
        shaped.revision = revision;
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let mut admission = world.admission(asset, attempt, shaped.key);
        admission.byte_limit = 1;
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(tag, shaped.attempt, shaped.effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("quota refusal rolls back");
        assert!(
            matches!(
                error,
                FinanceError::ImageStageRefusal(StageRefusal::QuotaFull)
            ),
            "quota refusal, got {error:?}"
        );
        drop(service);
        assert_eq!(
            world.receipt_count(world.campaign, shaped.operation).await,
            0
        );
        assert_eq!(world.count("finance.provider_attempt").await, 0);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            0
        );
        assert_eq!(world.count("media.asset").await, 0);
        assert_eq!(world.count("media.quota_reservation").await, 0);
        world.finish().await;
    });
}

#[test]
fn a_second_live_key_reports_in_flight_without_foreign_ids() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        // Same request key, a different asset and publication attempt: the
        // first row is still live, so the catalog reports in-flight.
        let tag = 3;
        let other_asset = [0x90 + tag; 16];
        let other_attempt = [0xa0 + tag; 16];
        let mut service = world.service(vec![payer(1)]).await;
        let asset = df_assets::AssetId::from_bytes(&other_asset).expect("asset id");
        let attempt = CatalogAttemptId::from_bytes(&other_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let operation = OperationKey::from_array([0x40 + tag; 16]);
        let finance_attempt = FinanceAttemptId::from_array([0x60 + tag; 16]);
        let effect = EffectId::from_array([0x80 + tag; 16]);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            other_asset,
            other_attempt,
            operation,
            finance_attempt,
            effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(tag, finance_attempt, effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("live key refuses");
        assert!(
            matches!(
                error,
                FinanceError::ImageStageRefusal(StageRefusal::InFlight)
            ),
            "in-flight refusal, got {error:?}"
        );
        drop(service);
        assert_eq!(world.receipt_count(world.campaign, operation).await, 0);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        world.finish().await;
    });
}

#[test]
fn a_planning_job_refuses_without_admitting() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let tag = 4;
        let shaped = FirstAdmission::ids(tag);
        let mut service = world.service(vec![payer(1)]).await;
        world.fund(&mut service, 1, 50_000_000).await;
        // Accepted but never frozen: the job is still planning.
        let journal = world.journal().await;
        let owner = world.owner();
        journal
            .accept(&AcceptCommand {
                identity: OperationIdentity {
                    owner,
                    operation: shaped.operation_id(tag),
                    fingerprint: [7; 32],
                },
                intent: VersionedSnapshot::new(1, vec![9; 16], SNAPSHOT_BOUND).expect("intent"),
                config: shaped.config.clone(),
                sharing: SharingIdentity::from_bytes(&[8; 32]).expect("sharing"),
                interest: shaped.interest,
                target: AcceptTarget::Create {
                    job: shaped.job,
                    absolute_deadline_ms: DEADLINE_MS,
                },
                now_ms: NOW_MS,
            })
            .await
            .expect("accept the job");
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            1,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(tag, shaped.attempt, shaped.effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("planning job refuses");
        assert!(
            matches!(
                error,
                FinanceError::ImageStageRefusal(StageRefusal::Other { ref reason })
                if reason == "job_not_frozen"
            ),
            "not-frozen refusal, got {error:?}"
        );
        drop(service);
        assert_eq!(
            world.receipt_count(world.campaign, shaped.operation).await,
            0
        );
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        world.finish().await;
    });
}

#[test]
fn a_retired_job_replay_still_returns_existing() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        // Withdrawing the last interest retires the job; the historical
        // receipt replay must still succeed.
        let journal = world.journal().await;
        journal
            .withdraw_interest(&world.owner(), shaped.interest, NOW_MS + 1)
            .await
            .expect("withdraw the last interest");
        let mut service = world.service(vec![payer(1)]).await;
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(1, shaped.attempt, shaped.effect),
            binding,
        };
        let outcome = service
            .admit_image_stage(&command, &port)
            .await
            .expect("retired replay resolves");
        assert!(
            matches!(
                outcome,
                df_finance::admit::AdmissionOutcome::AlreadyAdmitted { .. }
            ),
            "retired replay is historical, got {outcome:?}"
        );
        drop(service);
        world.finish().await;
    });
}

#[test]
fn an_expired_job_replay_still_returns_existing() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        // Past the job deadline the stored linkage still replays: replay is
        // historical receipt retrieval, not fresh authorization. Finance runs
        // on the wall clock, already past this fixed journal deadline.
        let mut service = world.service(vec![payer(1)]).await;
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(1, shaped.attempt, shaped.effect),
            binding,
        };
        let outcome = service
            .admit_image_stage(&command, &port)
            .await
            .expect("expired replay resolves");
        assert!(
            matches!(
                outcome,
                df_finance::admit::AdmissionOutcome::AlreadyAdmitted { .. }
            ),
            "expired replay is historical, got {outcome:?}"
        );
        drop(service);
        world.finish().await;
    });
}

#[test]
fn a_catalog_supersede_keeps_the_allocation_and_replay_valid() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        let catalog = world.catalog().await;
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let from =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let to = CatalogAttemptId::from_bytes(&[0xe0; 16]).expect("next attempt");
        let superseded = catalog
            .supersede(asset, from, to, NOW_MS + 2)
            .await
            .expect("supersede the attempt");
        assert_eq!(superseded, df_assets::SupersedeResult::Superseded);
        let record = catalog
            .asset(asset)
            .await
            .expect("read the asset")
            .expect("asset row survives");
        assert_eq!(record.attempt, to);
        // The current attempt moved on, with its normal outbox evidence, but
        // the immutable allocation and the receipt replay stay valid.
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        let mut service = world.service(vec![payer(1)]).await;
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(1, shaped.attempt, shaped.effect),
            binding,
        };
        let outcome = service
            .admit_image_stage(&command, &port)
            .await
            .expect("post-supersede replay resolves");
        assert!(
            matches!(
                outcome,
                df_finance::admit::AdmissionOutcome::AlreadyAdmitted { .. }
            ),
            "replay reads history, got {outcome:?}"
        );
        drop(service);
        world.finish().await;
    });
}

#[test]
fn a_mismatched_replay_binding_conflicts_without_repair() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        let mut service = world.service(vec![payer(1)]).await;
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        // Same finance operation but a different asset: the stored receipt
        // pins the exact first mapping, so the finance digest refuses the
        // replay before the port runs, and nothing is repaired.
        let wrong_asset = [0x9f; 16];
        assert_ne!(
            wrong_asset, shaped.asset,
            "replay binding must name another asset"
        );
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            wrong_asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(1, shaped.attempt, shaped.effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("changed binding conflicts");
        assert!(
            matches!(error, FinanceError::Admission(AdmissionRefusal::Conflict)),
            "replay mismatch, got {error:?}"
        );
        drop(service);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        world.finish().await;
    });
}

#[test]
fn a_replay_without_stored_linkage_reports_missing() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        // No finance admission and no binding rows at all: a direct replay
        // read reports the missing link instead of inventing one.
        let tag = 5;
        let shaped = FirstAdmission::ids(tag);
        let port = world.port(
            world.admission(
                df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id"),
                CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt"),
                shaped.key,
            ),
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            2,
            DEADLINE_MS,
            NOW_MS,
        );
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let receipt = df_commerce::snapshot::ReceiptRow {
            fingerprint: [0xf0; 32],
            reservation: ReservationId::from_array([0xf1; 16]),
            attempt: shaped.attempt,
            retail_maximum_micro_dollars: 0,
            supplier_maximum_micro_dollars: 0,
            price_version: world.version.id,
        };
        let mut raw = world.scratch.connect().await;
        let tx = raw
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await
            .expect("open read transaction");
        let error = port
            .verify_image_link(&tx, &binding, &receipt)
            .await
            .expect_err("missing linkage");
        assert!(
            matches!(error, FinanceError::ImageStageMissingLink),
            "missing link, got {error:?}"
        );
        tx.rollback().await.expect("roll back the read");
        world.finish().await;
    });
}

struct OtherScope {
    campaign: CampaignKey,
    campaign_id: CampaignId,
    run: RunId,
    session: SessionKey,
    fence: FenceRequest,
}

/// Opens a second campaign/session pot on the same price version, with its
/// own funding for payer 1. Same operation bytes stay usable there.
async fn open_other_scope(world: &World, service: &mut Service) -> OtherScope {
    let scope = OtherScope {
        campaign: CampaignKey::from_array([0x58; 16]),
        campaign_id: CampaignId::from_bytes(&[0x58; 16]).expect("other campaign"),
        run: RunId::from_bytes(&[0x57; 16]).expect("other run"),
        session: SessionKey::from_array([0x59; 16]),
        fence: FenceRequest {
            tenant: world.tenant,
            campaign: CampaignKey::from_array([0x58; 16]),
            recovery_epoch: 1,
            claim_seq: 1,
            placement_id: [0x54; 16],
        },
    };
    service
        .open_session(SessionOpening {
            session: scope.session,
            tenant: world.tenant,
            campaign: scope.campaign,
            price_version: world.version.id,
            currency: CURRENCY,
            policy: policy(),
        })
        .await
        .expect("open the other session");
    world
        .fund_session(
            service,
            1,
            scope.session,
            9,
            "pay-other-1".to_owned(),
            50_000_000,
        )
        .await;
    scope
}

fn other_owner(scope: &OtherScope) -> ImageOwner {
    ImageOwner::Run {
        campaign: scope.campaign_id,
        run: scope.run,
    }
}

#[test]
fn same_operation_bytes_in_different_campaigns_both_admit() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        admit_first(&world, 1).await;
        let mut service = world.service(vec![payer(1)]).await;
        let scope = open_other_scope(&world, &mut service).await;
        // Same operation bytes under the other campaign: a different scoped
        // operation with its own job, allocation and binding.
        let tag = 6;
        let other = FirstAdmission::ids(tag);
        let journal = world.journal().await;
        let owner = other_owner(&scope);
        let revision = world
            .frozen_job_for(
                &journal,
                owner.clone(),
                other.operation_id(tag),
                other.job,
                other.interest,
                other.key,
                &other.recipe,
                &other.config,
                DEADLINE_MS,
                NOW_MS,
            )
            .await;
        let asset = df_assets::AssetId::from_bytes(&other.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&other.publication_attempt).expect("catalog attempt");
        let admission = Admission {
            asset,
            attempt,
            kind: AssetKind::Image,
            provenance: Provenance::Generated,
            charge_scope: QuotaScope::Campaign(scope.campaign_id),
            origin_account: None,
            campaign: Some(scope.campaign_id),
            audience: Audience::CampaignShared(scope.campaign_id),
            dedupe_scope: DedupeScope::campaign(&scope.campaign_id),
            request_key: Some(other.key),
            retention: RetentionClass::CampaignMedia,
            reserve_bytes: 1 << 20,
            byte_limit: 1 << 30,
            in_flight_upload_limit: None,
        };
        let binding = ImageStageBinding {
            owner_scope: OwnerScope::new(owner_scope_bytes(&owner)).expect("owner scope"),
            job: FinanceJobId::new(*other.job.as_bytes()).expect("job"),
            stage: ImageStageId::new(other.stage).expect("stage"),
            asset: ImageAssetId::new(other.asset).expect("asset"),
            publication_attempt: PublicationAttemptId::new(other.publication_attempt)
                .expect("publication attempt"),
            tenant: world.tenant,
            campaign: scope.campaign,
            session: scope.session,
            operation: other.operation,
            attempt: other.attempt,
            effect: other.effect,
        };
        let port = ImageryAdmissionPort::new(
            FakeAuthority {
                tenant: *world.tenant.as_bytes(),
                campaign: *scope.campaign.as_bytes(),
                run: *scope.run.as_bytes(),
                session: *scope.session.as_bytes(),
            },
            ImageryAdmissionConfig {
                admission: admission.clone(),
                owner,
                job: other.job,
                stage: other.stage,
                recipe_key: other.key,
                recipe: other.recipe.clone(),
                config: other.config.clone(),
                expected_job_revision: revision,
                absolute_deadline_ms: DEADLINE_MS,
                now_ms: NOW_MS,
            },
        )
        .expect("other port configuration");
        let command = ImageStageAdmission {
            admit: world.admit_command_for(
                tag,
                other.attempt,
                other.effect,
                scope.fence,
                scope.session,
            ),
            binding,
        };
        let outcome = service
            .admit_image_stage(&command, &port)
            .await
            .expect("other campaign admits");
        assert!(
            matches!(
                outcome,
                df_finance::admit::AdmissionOutcome::Admitted { .. }
            ),
            "scoped operations are independent, got {outcome:?}"
        );
        drop(service);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 2);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            2
        );
        world.finish().await;
    });
}

#[test]
fn a_preexisting_allocation_under_a_new_receipt_conflicts() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        // A new finance operation reusing the same publication attempt is a
        // retry without a retry contract: conflict, never re-admission.
        let tag = 7;
        let mut service = world.service(vec![payer(1)]).await;
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let operation = OperationKey::from_array([0x40 + tag; 16]);
        let finance_attempt = FinanceAttemptId::from_array([0x60 + tag; 16]);
        let effect = EffectId::from_array([0x80 + tag; 16]);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            operation,
            finance_attempt,
            effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(tag, finance_attempt, effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("reused attempt conflicts");
        assert!(
            matches!(error, FinanceError::ImageStageConflict),
            "retry-not-enabled, got {error:?}"
        );
        drop(service);
        assert_eq!(world.receipt_count(world.campaign, operation).await, 0);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        world.finish().await;
    });
}

#[test]
fn concurrent_duplicate_initial_admissions_yield_one_allocation() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let world = Arc::new(world);
        let tag = 8;
        let shaped = FirstAdmission::ids(tag);
        let mut setup = world.service(vec![payer(1)]).await;
        world.fund(&mut setup, 1, 50_000_000).await;
        drop(setup);
        let journal = world.journal().await;
        let revision = world
            .frozen_job(
                &journal,
                shaped.operation_id(tag),
                shaped.job,
                shaped.interest,
                shaped.key,
                &shaped.recipe,
                &shaped.config,
                DEADLINE_MS,
                NOW_MS,
            )
            .await;
        drop(journal);
        let shaped = Arc::new((shaped, revision));
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let run_one = |world: Arc<World>,
                       shaped: Arc<(FirstAdmission, u64)>,
                       barrier: Arc<tokio::sync::Barrier>,
                       side: u8| async move {
            let (shaped, revision) = shaped.as_ref();
            let attempt = FinanceAttemptId::from_array([0x60 + tag + side; 16]);
            let effect = EffectId::from_array([0x80 + tag + side; 16]);
            let operation = OperationKey::from_array([0x40 + tag + side; 16]);
            let mut service = world.service_without_timeouts(vec![payer(1)]).await;
            let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
            let catalog_attempt =
                CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("attempt");
            let admission = world.admission(asset, catalog_attempt, shaped.key);
            let binding = world.binding(
                owner_scope_bytes(&world.owner()),
                *shaped.job.as_bytes(),
                shaped.stage,
                shaped.asset,
                shaped.publication_attempt,
                operation,
                attempt,
                effect,
            );
            let port = world.port(
                admission,
                shaped.job,
                shaped.stage,
                shaped.key,
                shaped.recipe.clone(),
                shaped.config.clone(),
                *revision,
                DEADLINE_MS,
                NOW_MS,
            );
            let command = ImageStageAdmission {
                admit: world.admit_command_for(
                    tag + side,
                    attempt,
                    effect,
                    world.fence(),
                    world.session,
                ),
                binding,
            };
            barrier.wait().await;
            (operation, service.admit_image_stage(&command, &port).await)
        };
        let first = tokio::spawn(run_one(world.clone(), shaped.clone(), barrier.clone(), 0));
        let second = tokio::spawn(run_one(world.clone(), shaped.clone(), barrier.clone(), 1));
        let (first, second) = tokio::time::timeout(Duration::from_secs(60), async {
            (
                first.await.expect("first task"),
                second.await.expect("second task"),
            )
        })
        .await
        .expect("no deadlock");
        let mut accepted = 0;
        let mut conflicted = 0;
        for (operation, result) in [first, second] {
            match result {
                Ok(df_finance::admit::AdmissionOutcome::Admitted { .. }) => accepted += 1,
                Err(FinanceError::ImageStageConflict) => {
                    conflicted += 1;
                    assert_eq!(world.receipt_count(world.campaign, operation).await, 0);
                }
                unexpected => panic!("one allocation wins, got {unexpected:?}"),
            }
        }
        assert_eq!((accepted, conflicted), (1, 1));
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        assert_eq!(world.count("finance.provider_attempt").await, 1);
        let world = Arc::try_unwrap(world).ok().expect("sole owner");
        world.finish().await;
    });
}

#[test]
fn raw_sql_cannot_rewrite_or_delete_the_bindings() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        admit_first(&world, 1).await;
        let update = world
            .admin
            .execute(
                "UPDATE imagery.image_publication_allocation SET created_at_ms = 0",
                &[],
            )
            .await
            .expect_err("allocation update rejected");
        assert_eq!(
            update.as_db_error().map(|db| db.code().code()),
            Some("P0001")
        );
        let delete = world
            .admin
            .execute("DELETE FROM imagery.image_finance_admission_binding", &[])
            .await
            .expect_err("binding delete rejected");
        assert_eq!(
            delete.as_db_error().map(|db| db.code().code()),
            Some("P0001")
        );
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        world.finish().await;
    });
}

#[test]
fn a_job_of_another_owner_conflicts_without_leaking_it() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        // The job freezes under the first campaign; the admission pays under
        // the second. The scoped job lookup misses and conflicts.
        let tag = 9;
        let shaped = FirstAdmission::ids(tag);
        let journal = world.journal().await;
        let revision = world
            .frozen_job(
                &journal,
                shaped.operation_id(tag),
                shaped.job,
                shaped.interest,
                shaped.key,
                &shaped.recipe,
                &shaped.config,
                DEADLINE_MS,
                NOW_MS,
            )
            .await;
        drop(journal);
        let mut service = world.service(vec![payer(1)]).await;
        let scope = open_other_scope(&world, &mut service).await;
        let owner = other_owner(&scope);
        let asset = df_assets::AssetId::from_bytes(&[0x99; 16]).expect("asset id");
        let attempt = CatalogAttemptId::from_bytes(&[0xa9; 16]).expect("catalog attempt");
        let admission = Admission {
            asset,
            attempt,
            kind: AssetKind::Image,
            provenance: Provenance::Generated,
            charge_scope: QuotaScope::Campaign(scope.campaign_id),
            origin_account: None,
            campaign: Some(scope.campaign_id),
            audience: Audience::CampaignShared(scope.campaign_id),
            dedupe_scope: DedupeScope::campaign(&scope.campaign_id),
            request_key: Some(shaped.key),
            retention: RetentionClass::CampaignMedia,
            reserve_bytes: 1 << 20,
            byte_limit: 1 << 30,
            in_flight_upload_limit: None,
        };
        let operation = OperationKey::from_array([0x49; 16]);
        let finance_attempt = FinanceAttemptId::from_array([0x69; 16]);
        let effect = EffectId::from_array([0x89; 16]);
        let binding = ImageStageBinding {
            owner_scope: OwnerScope::new(owner_scope_bytes(&owner)).expect("owner scope"),
            job: FinanceJobId::new(*shaped.job.as_bytes()).expect("job"),
            stage: ImageStageId::new(shaped.stage).expect("stage"),
            asset: ImageAssetId::new([0x99; 16]).expect("asset"),
            publication_attempt: PublicationAttemptId::new([0xa9; 16])
                .expect("publication attempt"),
            tenant: world.tenant,
            campaign: scope.campaign,
            session: scope.session,
            operation,
            attempt: finance_attempt,
            effect,
        };
        let port = ImageryAdmissionPort::new(
            FakeAuthority {
                tenant: *world.tenant.as_bytes(),
                campaign: *scope.campaign.as_bytes(),
                run: *scope.run.as_bytes(),
                session: *scope.session.as_bytes(),
            },
            ImageryAdmissionConfig {
                admission: admission.clone(),
                owner,
                job: shaped.job,
                stage: shaped.stage,
                recipe_key: shaped.key,
                recipe: shaped.recipe.clone(),
                config: shaped.config.clone(),
                expected_job_revision: revision,
                absolute_deadline_ms: DEADLINE_MS,
                now_ms: NOW_MS,
            },
        )
        .expect("cross-owner port configuration");
        let command = ImageStageAdmission {
            admit: world.admit_command_for(9, finance_attempt, effect, scope.fence, scope.session),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("foreign job conflicts");
        assert!(
            matches!(error, FinanceError::ImageStageConflict),
            "scoped miss, got {error:?}"
        );
        drop(service);
        assert_eq!(world.receipt_count(scope.campaign, operation).await, 0);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        world.finish().await;
    });
}

#[test]
fn a_changed_job_revision_refuses_as_stale() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let tag = 2;
        let shaped = FirstAdmission::ids(tag);
        let mut service = world.service(vec![payer(1)]).await;
        world.fund(&mut service, 1, 50_000_000).await;
        let journal = world.journal().await;
        let revision = world
            .frozen_job(
                &journal,
                shaped.operation_id(tag),
                shaped.job,
                shaped.interest,
                shaped.key,
                &shaped.recipe,
                &shaped.config,
                DEADLINE_MS,
                NOW_MS,
            )
            .await;
        // A second operation joins the job, advancing its revision past the
        // freeze the port was built from.
        let join_interest = ImageInterestId::from_bytes(&[0x2a; 16]).expect("join interest");
        journal
            .accept(&AcceptCommand {
                identity: OperationIdentity {
                    owner: world.owner(),
                    operation: OperationId::from_bytes(&[0x4a; 16]).expect("join operation"),
                    fingerprint: [7; 32],
                },
                intent: VersionedSnapshot::new(1, vec![9; 16], SNAPSHOT_BOUND).expect("intent"),
                config: shaped.config.clone(),
                sharing: SharingIdentity::from_bytes(&[8; 32]).expect("sharing"),
                interest: join_interest,
                target: AcceptTarget::Join { job: shaped.job },
                now_ms: NOW_MS,
            })
            .await
            .expect("join the job");
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(tag, shaped.attempt, shaped.effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("stale revision refuses");
        assert!(
            matches!(
                error,
                FinanceError::ImageStageRefusal(StageRefusal::Other { ref reason })
                if reason == "job_stale"
            ),
            "stale refusal, got {error:?}"
        );
        drop(service);
        assert_eq!(
            world.receipt_count(world.campaign, shaped.operation).await,
            0
        );
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        world.finish().await;
    });
}

#[test]
fn a_withdrawal_racing_an_admission_never_deadlocks() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let world = Arc::new(world);
        // Two fresh setups: each races one withdrawal against one admission
        // behind a barrier, so both serializations are exercised.
        for round in 0..2u8 {
            let tag = 3 + round;
            let shaped = FirstAdmission::ids(tag);
            let mut setup = world.service(vec![payer(1)]).await;
            world
                .fund_session(
                    &mut setup,
                    1,
                    world.session,
                    10 + round,
                    format!("pay-loop-{round}"),
                    50_000_000,
                )
                .await;
            drop(setup);
            let journal = world.journal().await;
            let revision = world
                .frozen_job(
                    &journal,
                    shaped.operation_id(tag),
                    shaped.job,
                    shaped.interest,
                    shaped.key,
                    &shaped.recipe,
                    &shaped.config,
                    DEADLINE_MS,
                    NOW_MS,
                )
                .await;
            drop(journal);
            let shaped = Arc::new((shaped, revision));
            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let admit = {
                let world = world.clone();
                let shaped = shaped.clone();
                let barrier = barrier.clone();
                tokio::spawn(async move {
                    let (shaped, revision) = shaped.as_ref();
                    let mut service = world.service_without_timeouts(vec![payer(1)]).await;
                    let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
                    let attempt = CatalogAttemptId::from_bytes(&shaped.publication_attempt)
                        .expect("catalog attempt");
                    let admission = world.admission(asset, attempt, shaped.key);
                    let binding = world.binding(
                        owner_scope_bytes(&world.owner()),
                        *shaped.job.as_bytes(),
                        shaped.stage,
                        shaped.asset,
                        shaped.publication_attempt,
                        shaped.operation,
                        shaped.attempt,
                        shaped.effect,
                    );
                    let port = world.port(
                        admission,
                        shaped.job,
                        shaped.stage,
                        shaped.key,
                        shaped.recipe.clone(),
                        shaped.config.clone(),
                        *revision,
                        DEADLINE_MS,
                        NOW_MS,
                    );
                    let command = ImageStageAdmission {
                        admit: world.admit_command(tag, shaped.attempt, shaped.effect),
                        binding,
                    };
                    barrier.wait().await;
                    service.admit_image_stage(&command, &port).await
                })
            };
            let withdraw = {
                let world = world.clone();
                let shaped = shaped.clone();
                let barrier = barrier.clone();
                tokio::spawn(async move {
                    let (shaped, _) = shaped.as_ref();
                    let journal = world.journal().await;
                    barrier.wait().await;
                    journal
                        .withdraw_interest(&world.owner(), shaped.interest, NOW_MS + 3)
                        .await
                        .expect("withdrawal commits");
                })
            };
            let (admitted, ()) = tokio::time::timeout(Duration::from_secs(60), async {
                (
                    admit.await.expect("admission task"),
                    withdraw.await.expect("withdraw task"),
                )
            })
            .await
            .expect("no deadlock");
            // Both serializations are valid: admission first (then retired),
            // or withdrawal first (then refused as retired).
            match admitted {
                Ok(df_finance::admit::AdmissionOutcome::Admitted { .. }) => {
                    assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
                }
                Err(FinanceError::ImageStageRefusal(StageRefusal::Other { reason }))
                    if reason == "job_retired" =>
                {
                    assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
                    assert_eq!(
                        world
                            .receipt_count(world.campaign, shaped.0.operation)
                            .await,
                        0
                    );
                }
                unexpected => panic!("valid serialization, got {unexpected:?}"),
            }
        }
        let world = Arc::try_unwrap(world).ok().expect("sole owner");
        world.finish().await;
    });
}

#[test]
fn an_upload_limit_maps_to_its_fixed_code() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let tag = 5;
        let shaped = FirstAdmission::ids(tag);
        let mut service = world.service(vec![payer(1)]).await;
        world.fund(&mut service, 1, 50_000_000).await;
        let journal = world.journal().await;
        let revision = world
            .frozen_job(
                &journal,
                shaped.operation_id(tag),
                shaped.job,
                shaped.interest,
                shaped.key,
                &shaped.recipe,
                &shaped.config,
                DEADLINE_MS,
                NOW_MS,
            )
            .await;
        drop(journal);
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let mut admission = world.admission(asset, attempt, shaped.key);
        admission.in_flight_upload_limit = Some(0);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(tag, shaped.attempt, shaped.effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("upload limit refuses");
        assert!(
            matches!(
                error,
                FinanceError::ImageStageRefusal(StageRefusal::Other { ref reason })
                if reason == "upload_limit"
            ),
            "fixed code, got {error:?}"
        );
        drop(service);
        assert_eq!(
            world.receipt_count(world.campaign, shaped.operation).await,
            0
        );
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        world.finish().await;
    });
}

fn valid_config() -> (FakeAuthority, ImageryAdmissionConfig) {
    let campaign = CampaignId::from_bytes(&[0x53; 16]).expect("campaign");
    let run = RunId::from_bytes(&[0x51; 16]).expect("run");
    let admission = Admission {
        asset: df_assets::AssetId::from_bytes(&[0x91; 16]).expect("asset"),
        attempt: CatalogAttemptId::from_bytes(&[0xa1; 16]).expect("attempt"),
        kind: AssetKind::Image,
        provenance: Provenance::Generated,
        charge_scope: QuotaScope::Campaign(campaign),
        origin_account: None,
        campaign: Some(campaign),
        audience: Audience::CampaignShared(campaign),
        dedupe_scope: DedupeScope::campaign(&campaign),
        request_key: Some(MediaRequestKey::from_bytes([0xb1; 32])),
        retention: RetentionClass::CampaignMedia,
        reserve_bytes: 1 << 20,
        byte_limit: 1 << 30,
        in_flight_upload_limit: None,
    };
    let authority = FakeAuthority {
        tenant: [0x52; 16],
        campaign: [0x53; 16],
        run: [0x51; 16],
        session: [0x51; 16],
    };
    let config = ImageryAdmissionConfig {
        admission,
        owner: ImageOwner::Run { campaign, run },
        job: JournalJobId::from_bytes(&[0x11; 16]).expect("job"),
        stage: [0x31; 16],
        recipe_key: MediaRequestKey::from_bytes([0xb1; 32]),
        recipe: VersionedSnapshot::new(2, vec![1; 64], SNAPSHOT_BOUND).expect("recipe"),
        config: VersionedSnapshot::new(3, vec![2; 64], SNAPSHOT_BOUND).expect("config"),
        expected_job_revision: 2,
        absolute_deadline_ms: NOW_MS + 1,
        now_ms: NOW_MS,
    };
    (authority, config)
}

#[test]
fn a_valid_port_configuration_is_accepted() {
    let (authority, config) = valid_config();
    assert!(ImageryAdmissionPort::new(authority, config).is_ok());
}

#[test]
fn invalid_port_configurations_are_refused_before_any_transaction() {
    let (authority, config) = valid_config();
    let mut config = config;
    config.admission.campaign = None;
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("non-campaign refused"),
        PortConfigError::NonCampaignAdmission
    );
    let (authority, mut config) = valid_config();
    config.admission.kind = df_assets::AssetKind::Audio;
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("non-image refused"),
        PortConfigError::NonGeneratedAdmission
    );
    let (authority, mut config) = valid_config();
    config.owner = ImageOwner::Draft {
        account: df_types::AccountId::from_bytes(&[0x61; 16]).expect("account"),
    };
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("draft refused"),
        PortConfigError::DraftOwnerUnsupported
    );
    let (authority, mut config) = valid_config();
    config.owner = ImageOwner::Run {
        campaign: CampaignId::from_bytes(&[0x59; 16]).expect("other campaign"),
        run: RunId::from_bytes(&[0x51; 16]).expect("run"),
    };
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("owner mismatch refused"),
        PortConfigError::OwnerCampaignMismatch
    );
    let (authority, mut config) = valid_config();
    config.stage = [0; 16];
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("zero stage refused"),
        PortConfigError::ZeroStage
    );
    let (authority, mut config) = valid_config();
    config.admission.request_key = Some(MediaRequestKey::from_bytes([0xee; 32]));
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("key mismatch refused"),
        PortConfigError::RequestKeyMismatch
    );
    let (authority, mut config) = valid_config();
    config.admission.dedupe_scope =
        DedupeScope::campaign(&CampaignId::from_bytes(&[0x59; 16]).expect("other campaign"));
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("scope mismatch refused"),
        PortConfigError::DedupeScopeMismatch
    );
    let (authority, mut config) = valid_config();
    config.expected_job_revision = 0;
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("zero revision refused"),
        PortConfigError::BadJobRevision
    );
    let (authority, mut config) = valid_config();
    config.now_ms = 0;
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("bad time refused"),
        PortConfigError::BadTime
    );
    let (authority, mut config) = valid_config();
    config.recipe = VersionedSnapshot::new(2, vec![1; SNAPSHOT_BOUND + 1], SNAPSHOT_BOUND + 1)
        .expect("oversized recipe");
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("oversized snapshot refused"),
        PortConfigError::SnapshotTooLarge
    );
}

#[test]
fn a_new_admission_with_a_forged_binding_owner_scope_conflicts() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let tag = 11;
        let shaped = FirstAdmission::ids(tag);
        let mut service = world.service(vec![payer(1)]).await;
        world.fund(&mut service, 1, 50_000_000).await;
        let journal = world.journal().await;
        let revision = world
            .frozen_job(
                &journal,
                shaped.operation_id(tag),
                shaped.job,
                shaped.interest,
                shaped.key,
                &shaped.recipe,
                &shaped.config,
                DEADLINE_MS,
                NOW_MS,
            )
            .await;
        drop(journal);
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let mut forged_scope = owner_scope_bytes(&world.owner());
        let last = forged_scope.len() - 1;
        forged_scope[last] ^= 0x01;
        let binding = world.binding(
            forged_scope,
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(tag, shaped.attempt, shaped.effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("forged owner scope conflicts");
        assert!(
            matches!(error, FinanceError::ImageStageConflict),
            "forged new binding, got {error:?}"
        );
        drop(service);
        assert_eq!(
            world.receipt_count(world.campaign, shaped.operation).await,
            0
        );
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            0
        );
        assert_eq!(world.count("media.asset").await, 0);
        world.finish().await;
    });
}

#[test]
fn a_replay_with_a_forged_binding_owner_scope_conflicts_without_repair() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        let mut service = world.service(vec![payer(1)]).await;
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let mut forged_scope = owner_scope_bytes(&world.owner());
        let last = forged_scope.len() - 1;
        forged_scope[last] ^= 0x01;
        assert_ne!(
            forged_scope,
            owner_scope_bytes(&world.owner()),
            "forged replay must name another owner scope"
        );
        let binding = world.binding(
            forged_scope,
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(1, shaped.attempt, shaped.effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("forged replay conflicts");
        assert!(
            matches!(error, FinanceError::Admission(AdmissionRefusal::Conflict)),
            "forged replay, got {error:?}"
        );
        drop(service);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        assert_eq!(world.count("media.asset").await, 1);
        world.finish().await;
    });
}

#[test]
fn a_request_key_mismatch_is_refused_before_any_write() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let tag = 12;
        let shaped = FirstAdmission::ids(tag);
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let mut admission = world.admission(asset, attempt, shaped.key);
        admission.request_key = Some(MediaRequestKey::from_bytes([0xee; 32]));
        let error = ImageryAdmissionPort::new(
            world.authority(),
            ImageryAdmissionConfig {
                admission,
                owner: world.owner(),
                job: shaped.job,
                stage: shaped.stage,
                recipe_key: shaped.key,
                recipe: shaped.recipe.clone(),
                config: shaped.config.clone(),
                expected_job_revision: 2,
                absolute_deadline_ms: DEADLINE_MS,
                now_ms: NOW_MS,
            },
        )
        .err()
        .expect("key mismatch refused");
        assert_eq!(error, PortConfigError::RequestKeyMismatch);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            0
        );
        world.finish().await;
    });
}

#[test]
fn a_dedupe_scope_mismatch_is_refused_before_any_write() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let tag = 13;
        let shaped = FirstAdmission::ids(tag);
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let mut admission = world.admission(asset, attempt, shaped.key);
        admission.dedupe_scope =
            DedupeScope::campaign(&CampaignId::from_bytes(&[0x59; 16]).expect("other campaign"));
        let error = ImageryAdmissionPort::new(
            world.authority(),
            ImageryAdmissionConfig {
                admission,
                owner: world.owner(),
                job: shaped.job,
                stage: shaped.stage,
                recipe_key: shaped.key,
                recipe: shaped.recipe.clone(),
                config: shaped.config.clone(),
                expected_job_revision: 2,
                absolute_deadline_ms: DEADLINE_MS,
                now_ms: NOW_MS,
            },
        )
        .err()
        .expect("scope mismatch refused");
        assert_eq!(error, PortConfigError::DedupeScopeMismatch);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            0
        );
        world.finish().await;
    });
}

#[test]
fn an_expired_reconstructed_replay_still_returns_existing() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        let mut service = world.service(vec![payer(1)]).await;
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            shaped.revision,
            DEADLINE_MS,
            DEADLINE_MS + 1_000,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(1, shaped.attempt, shaped.effect),
            binding,
        };
        let outcome = service
            .admit_image_stage(&command, &port)
            .await
            .expect("expired reconstruction replays");
        assert!(
            matches!(
                outcome,
                df_finance::admit::AdmissionOutcome::AlreadyAdmitted { .. }
            ),
            "expired replay is historical, got {outcome:?}"
        );
        drop(service);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        world.finish().await;
    });
}

fn other_campaign_id() -> CampaignId {
    CampaignId::from_bytes(&[0x59; 16]).expect("other campaign")
}

fn member(byte: u8) -> MemberId {
    MemberId::from_bytes(&[byte; 16]).expect("member")
}

fn account(byte: u8) -> AccountId {
    AccountId::from_bytes(&[byte; 16]).expect("account")
}

#[test]
fn a_cross_campaign_shared_audience_is_refused_before_any_transaction() {
    let (authority, mut config) = valid_config();
    config.admission.audience = Audience::CampaignShared(other_campaign_id());
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("cross-campaign shared refused"),
        PortConfigError::AudienceCampaignMismatch
    );
}

#[test]
fn a_cross_campaign_host_audience_is_refused_before_any_transaction() {
    let (authority, mut config) = valid_config();
    config.admission.audience = Audience::Host(other_campaign_id());
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("cross-campaign host refused"),
        PortConfigError::AudienceCampaignMismatch
    );
}

#[test]
fn a_cross_campaign_members_audience_is_refused_before_any_transaction() {
    let members = MemberSet::new(vec![member(1), member(2)]).expect("member set");
    let (authority, mut config) = valid_config();
    config.admission.audience = Audience::Members(other_campaign_id(), members);
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("cross-campaign members refused"),
        PortConfigError::AudienceCampaignMismatch
    );
}

#[test]
fn a_withheld_audience_is_refused_before_any_transaction() {
    let (authority, mut config) = valid_config();
    config.admission.audience = Audience::Withheld;
    assert_eq!(
        ImageryAdmissionPort::new(authority, config)
            .err()
            .expect("withheld refused"),
        PortConfigError::AudienceWithheld
    );
}

#[test]
fn same_campaign_host_and_members_audiences_are_accepted_at_construction() {
    let campaign = CampaignId::from_bytes(&[0x53; 16]).expect("campaign");
    for audience in [
        Audience::Host(campaign),
        Audience::Members(
            campaign,
            MemberSet::new(vec![member(1)]).expect("member set"),
        ),
    ] {
        let (authority, mut config) = valid_config();
        config.admission.audience = audience;
        assert!(ImageryAdmissionPort::new(authority, config).is_ok());
    }
}

#[test]
fn an_owner_private_audience_is_accepted_at_construction() {
    let (authority, mut config) = valid_config();
    config.admission.audience = Audience::OwnerPrivate(account(0x61));
    assert!(ImageryAdmissionPort::new(authority, config).is_ok());
}

#[test]
fn a_public_static_audience_is_accepted_at_construction() {
    let (authority, mut config) = valid_config();
    config.admission.audience = Audience::PublicStatic;
    assert!(ImageryAdmissionPort::new(authority, config).is_ok());
}

#[test]
fn cross_campaign_audiences_never_reach_the_database() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        // None of these names the port campaign (0x53): construction refuses
        // each one, so no transaction ever opens and nothing can be written.
        let members = MemberSet::new(vec![member(1), member(2)]).expect("member set");
        for audience in [
            Audience::CampaignShared(other_campaign_id()),
            Audience::Host(other_campaign_id()),
            Audience::Members(other_campaign_id(), members),
        ] {
            let (authority, mut config) = valid_config();
            config.admission.audience = audience;
            assert_eq!(
                ImageryAdmissionPort::new(authority, config)
                    .err()
                    .expect("cross-campaign refused"),
                PortConfigError::AudienceCampaignMismatch
            );
        }
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            0
        );
        assert_eq!(world.count("media.asset").await, 0);
        assert_eq!(world.count("media.quota_reservation").await, 0);
        world.finish().await;
    });
}

#[test]
fn a_withheld_audience_never_reaches_the_database() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (authority, mut config) = valid_config();
        config.admission.audience = Audience::Withheld;
        assert_eq!(
            ImageryAdmissionPort::new(authority, config)
                .err()
                .expect("withheld refused"),
            PortConfigError::AudienceWithheld
        );
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            0
        );
        assert_eq!(world.count("media.asset").await, 0);
        assert_eq!(world.count("media.quota_reservation").await, 0);
        world.finish().await;
    });
}

#[test]
fn an_owner_private_audience_admits_end_to_end() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        admit_first_with_audience(&world, 1, Audience::OwnerPrivate(account(0x61))).await;
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        assert_eq!(world.count("media.asset").await, 1);
        world.finish().await;
    });
}

#[test]
fn a_same_campaign_host_audience_admits_end_to_end() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        admit_first_with_audience(&world, 1, Audience::Host(world.campaign_id)).await;
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        assert_eq!(world.count("media.asset").await, 1);
        world.finish().await;
    });
}

#[test]
fn a_same_campaign_members_audience_admits_end_to_end() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let members = MemberSet::new(vec![member(1)]).expect("member set");
        admit_first_with_audience(&world, 1, Audience::Members(world.campaign_id, members)).await;
        assert_eq!(world.count("imagery.image_publication_allocation").await, 1);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            1
        );
        assert_eq!(world.count("media.asset").await, 1);
        world.finish().await;
    });
}

#[test]
fn admission_as_plain_finance_fails_unavailable_without_widening_finance() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        // The same funded setup and frozen job as a first admission, but the
        // whole finance transaction runs as ordinary df_finance: the definers
        // still work, while the port's media/imagery reads stay denied.
        let tag = 2;
        let shaped = FirstAdmission::ids(tag);
        let mut service = world.service_as_plain_finance(vec![payer(1)]).await;
        world.fund(&mut service, 1, 50_000_000).await;
        let journal = world.journal().await;
        let revision = world
            .frozen_job(
                &journal,
                shaped.operation_id(tag),
                shaped.job,
                shaped.interest,
                shaped.key,
                &shaped.recipe,
                &shaped.config,
                DEADLINE_MS,
                NOW_MS,
            )
            .await;
        drop(journal);
        let asset = df_assets::AssetId::from_bytes(&shaped.asset).expect("asset id");
        let attempt =
            CatalogAttemptId::from_bytes(&shaped.publication_attempt).expect("catalog attempt");
        let admission = world.admission(asset, attempt, shaped.key);
        let binding = world.binding(
            owner_scope_bytes(&world.owner()),
            *shaped.job.as_bytes(),
            shaped.stage,
            shaped.asset,
            shaped.publication_attempt,
            shaped.operation,
            shaped.attempt,
            shaped.effect,
        );
        let port = world.port(
            admission,
            shaped.job,
            shaped.stage,
            shaped.key,
            shaped.recipe.clone(),
            shaped.config.clone(),
            revision,
            DEADLINE_MS,
            NOW_MS,
        );
        let command = ImageStageAdmission {
            admit: world.admit_command(tag, shaped.attempt, shaped.effect),
            binding,
        };
        let error = service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("plain finance cannot admit");
        assert!(
            matches!(
                error,
                FinanceError::ImageStageUnavailable { ref sqlstate, .. }
                if sqlstate.as_deref() == Some("42501")
            ),
            "permission-denied, got {error:?}"
        );
        drop(service);
        assert_eq!(
            world.receipt_count(world.campaign, shaped.operation).await,
            0
        );
        assert_eq!(world.count("finance.provider_attempt").await, 0);
        assert_eq!(world.count("imagery.image_publication_allocation").await, 0);
        assert_eq!(
            world.count("imagery.image_finance_admission_binding").await,
            0
        );
        assert_eq!(world.count("media.asset").await, 0);
        assert_eq!(world.count("media.quota_reservation").await, 0);
        world.finish().await;
    });
}

#[test]
fn plain_finance_cannot_touch_composition_tables() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let raw = world.scratch.connect().await;
        raw.batch_execute("SET ROLE df_finance")
            .await
            .expect("set role");
        let asset = raw
            .query("SELECT COUNT(*) FROM media.asset", &[])
            .await
            .expect_err("no media read for finance");
        assert_eq!(
            asset.as_db_error().map(|db| db.code().code()),
            Some("42501")
        );
        let job = raw
            .query("SELECT COUNT(*) FROM imagery.image_job", &[])
            .await
            .expect_err("no imagery read for finance");
        assert_eq!(job.as_db_error().map(|db| db.code().code()), Some("42501"));
        let reservation = raw
            .execute(
                "INSERT INTO media.quota_reservation (asset_id, scope_kind, scope_id, \
                 reserved_bytes, counts_upload) VALUES ($1, 'campaign', $2, 1, false)",
                &[&vec![0x90u8; 16], &vec![0x53u8; 16]],
            )
            .await
            .expect_err("no media write for finance");
        assert_eq!(
            reservation.as_db_error().map(|db| db.code().code()),
            Some("42501")
        );
        world.finish().await;
    });
}

#[test]
fn composition_role_cannot_mutate_finance_directly() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        // The role itself is narrow: NOLOGIN, no privilege to escalate with.
        let role = world
            .admin
            .query_one(
                "SELECT rolsuper, rolcanlogin, rolbypassrls FROM pg_roles \
                 WHERE rolname = 'df_imagery_admission'",
                &[],
            )
            .await
            .expect("composition role row");
        assert!(!role.get::<_, bool>("rolsuper"));
        assert!(!role.get::<_, bool>("rolcanlogin"));
        assert!(!role.get::<_, bool>("rolbypassrls"));
        // Nothing for PUBLIC on the composition tables or schemas.
        // information_schema.role_table_grants omits PUBLIC, so read the raw
        // ACLs: grantee 0 is PUBLIC, with the null-ACL default folded in.
        let public_tables: i64 = world
            .admin
            .query_one(
                "SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n \
                 ON n.oid = c.relnamespace \
                 WHERE n.nspname IN ('media', 'imagery') \
                 AND c.relname IN ('asset', 'quota_usage', 'quota_reservation', \
                 'image_job', 'image_interest', 'image_publication_allocation', \
                 'image_finance_admission_binding') \
                 AND EXISTS (SELECT 1 FROM aclexplode(COALESCE(c.relacl, \
                 acldefault('r', c.relowner))) WHERE grantee = 0)",
                &[],
            )
            .await
            .expect("public table grants")
            .get(0);
        assert_eq!(public_tables, 0);
        let public_schemas: i64 = world
            .admin
            .query_one(
                "SELECT COUNT(*) FROM pg_namespace \
                 WHERE nspname IN ('media', 'imagery') \
                 AND EXISTS (SELECT 1 FROM aclexplode(COALESCE(nspacl, \
                 acldefault('n', nspowner))) WHERE grantee = 0)",
                &[],
            )
            .await
            .expect("public schema grants")
            .get(0);
        assert_eq!(public_schemas, 0);
        // The check above catches a real PUBLIC grant: inject one in an owned
        // rollback transaction, observe it, then roll it back.
        let mut probe = world.scratch.connect().await;
        let grant_tx = probe
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await
            .expect("open grant probe");
        grant_tx
            .batch_execute("GRANT SELECT ON media.asset TO PUBLIC")
            .await
            .expect("inject public grant");
        let caught: i64 = grant_tx
            .query_one(
                "SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n \
                 ON n.oid = c.relnamespace \
                 WHERE n.nspname = 'media' AND c.relname = 'asset' \
                 AND EXISTS (SELECT 1 FROM aclexplode(COALESCE(c.relacl, \
                 acldefault('r', c.relowner))) WHERE grantee = 0)",
                &[],
            )
            .await
            .expect("public probe")
            .get(0);
        assert!(caught > 0, "probe sees the injected PUBLIC grant");
        grant_tx.rollback().await.expect("roll back the probe");
        let raw = world.scratch.connect().await;
        raw.batch_execute("SET ROLE df_imagery_admission")
            .await
            .expect("set role");
        // Inherited finance reads work: the composition role is a member of df_finance.
        let receipts: i64 = raw
            .query_one("SELECT COUNT(*) FROM finance.operation_receipt", &[])
            .await
            .expect("inherited finance read")
            .get(0);
        assert_eq!(receipts, 0);
        raw.query_one("SELECT finance.now_ms()", &[])
            .await
            .expect("inherited finance execute");
        // Direct finance writes stay denied: finance rows change only through definers.
        let update = raw
            .execute(
                "UPDATE finance.account_balance SET natural_micro_dollars = 0",
                &[],
            )
            .await
            .expect_err("no direct finance update");
        assert_eq!(
            update.as_db_error().map(|db| db.code().code()),
            Some("42501")
        );
        let insert = raw
            .execute(
                "INSERT INTO finance.operation_receipt (namespace, principal, operation_id, \
                 fingerprint, outcome, created_at_ms) \
                 VALUES ('admission', $1, $2, $3, '{}', 0)",
                &[&vec![0x53u8; 16], &vec![0x40u8; 16], &vec![0u8; 32]],
            )
            .await
            .expect_err("no direct finance insert");
        assert_eq!(
            insert.as_db_error().map(|db| db.code().code()),
            Some("42501")
        );
        let delete = raw
            .execute("DELETE FROM finance.operation_receipt", &[])
            .await
            .expect_err("no direct finance delete");
        assert_eq!(
            delete.as_db_error().map(|db| db.code().code()),
            Some("42501")
        );
        world.finish().await;
    });
}

#[test]
fn admission_role_locks_rows_while_meaningful_writes_stay_denied() {
    runtime().block_on(async {
        let Some(world) = World::open().await else {
            return;
        };
        let shaped = admit_first(&world, 1).await;
        let asset: Vec<u8> = shaped.asset.to_vec();
        let job: Vec<u8> = shaped.job.as_bytes().to_vec();
        let mut raw = world.scratch.connect().await;
        raw.batch_execute("SET ROLE df_imagery_admission")
            .await
            .expect("set role");
        let lock_tx = raw
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await
            .expect("open lock transaction");
        // The admission path locks both rows FOR UPDATE: this must succeed.
        let asset_locked = lock_tx
            .query_one(
                "SELECT asset_id FROM media.asset WHERE asset_id = $1 FOR UPDATE",
                &[&asset],
            )
            .await
            .expect("asset row locks");
        assert_eq!(asset_locked.get::<_, Vec<u8>>(0), asset);
        let job_locked = lock_tx
            .query_one(
                "SELECT job_id FROM imagery.image_job WHERE job_id = $1 FOR UPDATE",
                &[&job],
            )
            .await
            .expect("job row locks");
        assert_eq!(job_locked.get::<_, Vec<u8>>(0), job);
        // The narrow grant covers the timestamp: this write is permitted.
        let touched = lock_tx
            .execute(
                "UPDATE media.asset SET updated_at_ms = updated_at_ms WHERE asset_id = $1",
                &[&asset],
            )
            .await
            .expect("timestamp touch");
        assert_eq!(touched, 1);
        let job_touched = lock_tx
            .execute(
                "UPDATE imagery.image_job SET updated_at_ms = updated_at_ms \
                 WHERE job_id = $1",
                &[&job],
            )
            .await
            .expect("job timestamp touch");
        assert_eq!(job_touched, 1);
        // The lock and timestamp proof is complete: release the transaction
        // so each expected denial runs autocommit and reaches PostgreSQL
        // instead of reporting the aborted-transaction 25P02.
        lock_tx.rollback().await.expect("roll back the locks");
        let zero = vec![0u8; 16];
        // Meaningful asset columns stay ungranted.
        for (statement, with_second) in [
            (
                "UPDATE media.asset SET state = 'rejected' WHERE asset_id = $1",
                false,
            ),
            (
                "UPDATE media.asset SET attempt = $2 WHERE asset_id = $1",
                true,
            ),
            (
                "UPDATE media.asset SET charge_scope_id = $2 WHERE asset_id = $1",
                true,
            ),
        ] {
            let denied = if with_second {
                raw.execute(statement, &[&asset, &zero])
                    .await
                    .expect_err("meaningful asset write denied")
            } else {
                raw.execute(statement, &[&asset])
                    .await
                    .expect_err("meaningful asset write denied")
            };
            assert_eq!(
                denied.as_db_error().map(|db| db.code().code()),
                Some("42501"),
                "denied statement: {statement}"
            );
        }
        // Meaningful job columns stay ungranted.
        for (statement, with_second) in [
            (
                "UPDATE imagery.image_job SET state = 'retired' WHERE job_id = $1",
                false,
            ),
            (
                "UPDATE imagery.image_job SET owner_scope = $2 WHERE job_id = $1",
                true,
            ),
            (
                "UPDATE imagery.image_job SET recipe_bytes = $2 WHERE job_id = $1",
                true,
            ),
        ] {
            let denied = if with_second {
                raw.execute(statement, &[&job, &zero])
                    .await
                    .expect_err("meaningful job write denied")
            } else {
                raw.execute(statement, &[&job])
                    .await
                    .expect_err("meaningful job write denied")
            };
            assert_eq!(
                denied.as_db_error().map(|db| db.code().code()),
                Some("42501"),
                "denied statement: {statement}"
            );
        }
        // Plain finance still cannot lock either row.
        let plain = world.scratch.connect().await;
        plain
            .batch_execute("SET ROLE df_finance")
            .await
            .expect("set role");
        let asset_lock = plain
            .query(
                "SELECT asset_id FROM media.asset WHERE asset_id = $1 FOR UPDATE",
                &[&asset],
            )
            .await
            .expect_err("no finance asset lock");
        assert_eq!(
            asset_lock.as_db_error().map(|db| db.code().code()),
            Some("42501")
        );
        let job_lock = plain
            .query(
                "SELECT job_id FROM imagery.image_job WHERE job_id = $1 FOR UPDATE",
                &[&job],
            )
            .await
            .expect_err("no finance job lock");
        assert_eq!(
            job_lock.as_db_error().map(|db| db.code().code()),
            Some("42501")
        );
        world.finish().await;
    });
}
