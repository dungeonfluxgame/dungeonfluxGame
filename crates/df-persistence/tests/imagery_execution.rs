//! Crate-boundary tests for the durable image execution journal: the neutral
//! execution contracts backed by real PostgreSQL rows in one transaction.
//!
//! These tests require a real scratch server: the owned port 55471 (host
//! `DUNGEONFLUX_EXECUTION_PG_HOST`, default 127.0.0.1; user
//! `DUNGEONFLUX_EXECUTION_PG_USER`, default postgres) plus
//! `DUNGEONFLUX_REQUIRE_SCRATCH_PG=1`. Without the flag, or without a
//! server, every test fails loudly: a skipped composition is never counted
//! as a pass. No provider, finance, catalog or network call happens here:
//! finance tuples are opaque test bytes, and transport is never launched.
//!
//! The fixture applies the full composition schema the journal resolves
//! against: finance 0020 through 0025, media 0010 through 0014, imagery
//! 0038, execution 0040, then admission bindings 0039. Published identity
//! resolution reads a real allocation row pointing at a real asset row.
//!
//! Each test fails before the implementation and passes after: they cover
//! the full hosted and local paths, exact claim/prepare replay, two prepared
//! ids under one claim, concurrent prepares and claims, crash replay between
//! prepare and egress, lost-ack reread, expiry and stale claims, expired
//! demand, pending/active capacity with byte bounds and bound reduction,
//! owner isolation, the applied-gate fence and disable, supplier conflicts,
//! versioned supplier checkpoints, unknown-without-resend plus late unknown
//! resolution, conflicting terminal completion, refetch selection, atomic
//! accept-plus-enqueue, cancel-before-send proof, correct stage resolution
//! after a classification touch, and restart durability of checkpoints.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use df_imagery::execution::{
    CancelCommand, CapacityBounds, CapacityScope, CheckpointAlpha, ClaimCommand, ClaimToken,
    CompleteCommand, CompletionOutcome, DispatchId, DispatchMode, DispatchSettlement,
    DispatchState, EnqueueCommand, EnqueueOutcome, ExecutionError, ExecutionIdentity,
    ExecutionStageId, FailureCause, FinanceTuple, ImageExecutionJournal, MarkSentCommand,
    OutputCheckpoint, PrepareCommand, RefetchCommand, ReleaseCommand, StageKind, StageState,
    SupplierCheckpoint, SupplierReport,
};
use df_imagery::{
    AcceptCommand, AcceptOutcome, AcceptTarget, ImageInterestId, ImageJobId, ImageOwner,
    OperationIdentity, SharingIdentity, VersionedSnapshot,
};
use df_persistence::imagery_execution::{
    PostgresImageExecution, accept_with_execution_in, cancel_dispatch_in, claim_stage_in,
    commit_execution, complete_dispatch_in, enqueue_stage_in, mark_sent_in, mirror_settlement_in,
    prepare_dispatch_in, read_dispatch_in, read_execution_gate_in, read_stage_in,
    read_supplier_checkpoint_in, record_supplier_checkpoint_in, record_supplier_in,
    release_stage_in, request_refetch_in, resolve_execution_in, set_capacity_bounds_in,
    set_execution_gate_in,
};
use df_persistence::postgres_tls::PostgresTransport;
use df_types::{AccountId, CampaignId, OperationId, RunId};
use tokio_postgres::{Client, Config, NoTls};

const NOW_MS: i64 = 1_700_000_000_000;
const DEADLINE_MS: i64 = NOW_MS + 50_000;
const SNAPSHOT_BOUND: usize = 1 << 20;
const LEASE_MS: i64 = 10_000;
const GATE_REVISION: u64 = 7;

/// Never these ports: the game's and the other owned test servers.
const FORBIDDEN_PORTS: [u16; 4] = [0, 55517, 55439, 63309];

fn at(offset_ms: i64) -> i64 {
    NOW_MS + offset_ms
}

fn id(byte: u8) -> [u8; 16] {
    [byte; 16]
}

fn owner() -> ImageOwner {
    ImageOwner::Run {
        campaign: CampaignId::from_bytes(&id(0x53)).expect("campaign"),
        run: RunId::from_bytes(&id(0x51)).expect("run"),
    }
}

fn draft_owner() -> ImageOwner {
    ImageOwner::Draft {
        account: AccountId::from_bytes(&id(0x61)).expect("account"),
    }
}

fn snapshot(tag: u8) -> VersionedSnapshot {
    VersionedSnapshot::new(1, vec![tag; 64], SNAPSHOT_BOUND).expect("snapshot")
}

fn accept_command(op: u8, job: u8, interest: u8, deadline_ms: i64, now_ms: i64) -> AcceptCommand {
    AcceptCommand {
        identity: OperationIdentity {
            owner: owner(),
            operation: OperationId::from_bytes(&id(op)).expect("operation"),
            fingerprint: [op; 32],
        },
        intent: snapshot(op),
        config: snapshot(0xc0 | op),
        sharing: SharingIdentity::from_bytes(&[op; 32]).expect("sharing"),
        interest: ImageInterestId::from_bytes(&id(interest)).expect("interest"),
        target: AcceptTarget::Create {
            job: ImageJobId::from_bytes(&id(job)).expect("job"),
            absolute_deadline_ms: deadline_ms,
        },
        now_ms,
    }
}

fn enqueue_command(job: u8, stage: u8, now_ms: i64) -> EnqueueCommand {
    EnqueueCommand::new(
        df_imagery::execution::StageScope {
            owner: owner(),
            job: ImageJobId::from_bytes(&id(job)).expect("job"),
            stage: ExecutionStageId::from_bytes(&id(stage)).expect("stage"),
        },
        df_imagery::execution::StageBasis {
            kind: StageKind::Generation,
            input_revision: 1,
            input_digest: [0x31; 32],
            config_revision: 7,
            config_digest: [0x32; 32],
            parent_dispatch: None,
        },
        df_imagery::execution::ExecutionIdentity {
            asset: id(0x41),
            attempt: id(0x42),
        },
        CapacityScope::new("live/generation".to_owned()).expect("scope"),
        false,
        now_ms,
    )
    .expect("enqueue")
}

/// One opaque finance identity per dispatch: every paid attempt carries a
/// fresh admission, so distinct dispatches never share finance ids.
fn hosted_mode(dispatch: u8) -> DispatchMode {
    DispatchMode::Hosted(
        FinanceTuple::new(id(dispatch), id(dispatch ^ 0x0F), id(dispatch ^ 0xF0)).expect("tuple"),
    )
}

fn claim_command(
    job: u8,
    stage: u8,
    config_revision: u64,
    claim: u8,
    generation: u64,
    now_ms: i64,
) -> ClaimCommand {
    claim_with_lease(
        job,
        stage,
        config_revision,
        claim,
        generation,
        LEASE_MS,
        now_ms,
    )
}

fn claim_with_lease(
    job: u8,
    stage: u8,
    config_revision: u64,
    claim: u8,
    _generation: u64,
    lease_ms: i64,
    now_ms: i64,
) -> ClaimCommand {
    ClaimCommand::new(
        owner(),
        ImageJobId::from_bytes(&id(job)).expect("job"),
        ExecutionStageId::from_bytes(&id(stage)).expect("stage"),
        config_revision,
        ClaimToken::from_bytes(&id(claim)).expect("claim"),
        lease_ms,
        now_ms,
    )
    .expect("claim")
}

fn prepare_command(
    job: u8,
    stage: u8,
    dispatch: u8,
    claim: u8,
    generation: u64,
    mode: DispatchMode,
    now_ms: i64,
) -> PrepareCommand {
    PrepareCommand::new(
        df_imagery::execution::StageClaim {
            scope: df_imagery::execution::StageScope {
                owner: owner(),
                job: ImageJobId::from_bytes(&id(job)).expect("job"),
                stage: ExecutionStageId::from_bytes(&id(stage)).expect("stage"),
            },
            token: df_imagery::execution::ClaimToken::from_bytes(&id(claim)).expect("claim"),
            generation,
        },
        DispatchId::from_bytes(&id(dispatch)).expect("dispatch"),
        mode,
        now_ms,
    )
    .expect("prepare")
}

fn mark_sent_command(
    job: u8,
    stage: u8,
    dispatch: u8,
    claim: u8,
    generation: u64,
    now_ms: i64,
) -> MarkSentCommand {
    MarkSentCommand::new(
        owner(),
        ImageJobId::from_bytes(&id(job)).expect("job"),
        ExecutionStageId::from_bytes(&id(stage)).expect("stage"),
        DispatchId::from_bytes(&id(dispatch)).expect("dispatch"),
        df_imagery::execution::ClaimToken::from_bytes(&id(claim)).expect("claim"),
        generation,
        now_ms,
    )
    .expect("mark sent")
}

fn finish_command(
    job: u8,
    stage: u8,
    dispatch: u8,
    claim: u8,
    generation: u64,
    outcome: CompletionOutcome,
    now_ms: i64,
) -> CompleteCommand {
    CompleteCommand::new(
        df_imagery::execution::StageClaim {
            scope: df_imagery::execution::StageScope {
                owner: owner(),
                job: ImageJobId::from_bytes(&id(job)).expect("job"),
                stage: ExecutionStageId::from_bytes(&id(stage)).expect("stage"),
            },
            token: df_imagery::execution::ClaimToken::from_bytes(&id(claim)).expect("claim"),
            generation,
        },
        DispatchId::from_bytes(&id(dispatch)).expect("dispatch"),
        outcome,
        now_ms,
    )
    .expect("complete")
}

fn success_checkpoint(dispatch: u8, usage_revision: u32) -> OutputCheckpoint {
    OutputCheckpoint::new(
        &DispatchId::from_bytes(&id(dispatch)).expect("dispatch"),
        df_imagery::execution::OutputObject {
            storage_key: format!("staged/{dispatch:02x}/variant"),
            object_version: "etag-3".to_owned(),
            byte_len: 48_128,
            sha256: [0x91; 32],
        },
        df_imagery::execution::OutputImage {
            mime: "image/png".to_owned(),
            width_px: Some(1024),
            height_px: Some(1536),
            alpha: CheckpointAlpha::Opaque,
        },
        [0x92; 32],
        usage_revision,
    )
    .expect("checkpoint")
}

fn supplier_checkpoint(
    job: u8,
    stage: u8,
    dispatch: u8,
    version: u32,
    now_ms: i64,
) -> SupplierCheckpoint {
    SupplierCheckpoint::new(
        df_imagery::execution::ExecutionScope {
            owner: owner(),
            job: job_id(job),
            stage: stage_id(stage),
            dispatch: dispatch_id(dispatch),
        },
        df_imagery::execution::SupplierResume {
            checkpoint_version: version,
            origin: "fal".to_owned(),
            model: "flux-ultra".to_owned(),
            checkpoint_bytes: vec![0x77; 64],
        },
        now_ms,
    )
    .expect("supplier checkpoint")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime")
}

async fn connect(config: &Config) -> Client {
    let (client, connection) = config
        .connect(NoTls)
        .await
        .expect("scratch PostgreSQL is required for execution journal tests");
    tokio::spawn(async move {
        let _ended = connection.await;
    });
    client
}

const MIGRATION_0040: &str = include_str!("../migrations/0040_imagery_execution.sql");

struct Scratch {
    admin: Config,
    database: String,
}

impl Scratch {
    async fn open() -> Self {
        let required = std::env::var("DUNGEONFLUX_REQUIRE_SCRATCH_PG").unwrap_or_default();
        assert_eq!(
            required, "1",
            "execution journal tests need DUNGEONFLUX_REQUIRE_SCRATCH_PG=1 \
             and a scratch server on the owned port"
        );
        let port: u16 = std::env::var("DUNGEONFLUX_EXECUTION_PG_PORT")
            .unwrap_or("55471".to_owned())
            .parse()
            .expect("scratch port");
        assert!(!FORBIDDEN_PORTS.contains(&port));
        let mut admin = Config::new();
        admin
            .host(std::env::var("DUNGEONFLUX_EXECUTION_PG_HOST").unwrap_or("127.0.0.1".into()))
            .port(port)
            .user(std::env::var("DUNGEONFLUX_EXECUTION_PG_USER").unwrap_or("postgres".into()))
            .dbname("postgres");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::SeqCst);
        let database = format!("df_exec_{nanos}_{}_{serial}", std::process::id());
        // Roles are cluster-wide even though each test owns a separate database.
        let installer = connect(&admin).await;
        installer
            .batch_execute("SET statement_timeout='30s'")
            .await
            .expect("installer timeout");
        installer
            .query_one(
                "SELECT pg_advisory_lock(hashtext('df_imagery_test_role_migrations'))",
                &[],
            )
            .await
            .expect("cluster role migration lock");
        installer
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .expect("create test database");
        let scratch = Self { admin, database };
        scratch.migrate().await;
        installer
            .query_one(
                "SELECT pg_advisory_unlock(hashtext('df_imagery_test_role_migrations'))",
                &[],
            )
            .await
            .expect("cluster role migration unlock");
        scratch
    }

    async fn migrate(&self) {
        // The trusted installer owns every migration in order; hand-written lists drift.
        let mut client = connect(&self.config()).await;
        let transaction = client.transaction().await.expect("migration transaction");
        df_persistence::imagery_migrations::migrate_local_in(&transaction)
            .await
            .unwrap_or_else(|error| panic!("imagery migrations: {error:?}"));
        transaction.commit().await.expect("commit migrations");
    }

    fn config(&self) -> Config {
        let mut config = self.admin.clone();
        config.dbname(&self.database);
        config
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

async fn setup_world(client: &mut Client) {
    let bounds = CapacityBounds::new(4, 64, 1 << 20).expect("bounds");
    let tx = client.transaction().await.expect("tx");
    set_capacity_bounds_in(&tx, "live/generation", &bounds, NOW_MS)
        .await
        .expect("capacity bound");
    set_capacity_bounds_in(&tx, "local/transform", &bounds, NOW_MS)
        .await
        .expect("local bound");
    set_execution_gate_in(&tx, "live/generation", GATE_REVISION, true, NOW_MS)
        .await
        .expect("gate");
    set_execution_gate_in(&tx, "local/transform", GATE_REVISION, true, NOW_MS)
        .await
        .expect("local gate");
    tx.commit().await.expect("commit bounds");
}

/// Accepts demand and enqueues its stage atomically: the receipt plus interest
/// plus the durable pending reserve commit together.
async fn accept_and_enqueue(
    client: &mut Client,
    op: u8,
    job: u8,
    interest: u8,
    stage: u8,
    now_ms: i64,
) -> (AcceptOutcome, EnqueueOutcome) {
    let tx = client.transaction().await.expect("tx");
    let outcome = accept_with_execution_in(
        &tx,
        &accept_command(op, job, interest, DEADLINE_MS, now_ms),
        &enqueue_command(job, stage, now_ms),
    )
    .await
    .expect("accept with execution");
    tx.commit().await.expect("commit");
    outcome
}

async fn claim(client: &mut Client, job: u8, stage: u8, claim: u8, generation: u64, now_ms: i64) {
    let tx = client.transaction().await.expect("tx");
    claim_stage_in(
        &tx,
        &claim_command(job, stage, GATE_REVISION, claim, generation, now_ms),
    )
    .await
    .expect("claim");
    tx.commit().await.expect("commit claim");
}

async fn capacity_counts(client: &Client, scope: &str) -> (i64, i64, i64) {
    let row = client
        .query_one(
            "SELECT in_flight, pending_jobs, pending_bytes \
             FROM imagery.image_execution_capacity WHERE capacity_scope = $1",
            &[&scope],
        )
        .await
        .expect("capacity");
    (
        row.get("in_flight"),
        row.get("pending_jobs"),
        row.get("pending_bytes"),
    )
}

async fn dispatch_count(client: &Client, job: u8, stage: u8) -> i64 {
    client.query_one(
        "SELECT COUNT(*) FROM imagery.image_dispatch_attempt WHERE job_id = $1 AND stage_id = $2",
        &[&id(job).as_slice(), &id(stage).as_slice()],
    )
    .await
    .expect("count")
    .get("count")
}

fn job_id(byte: u8) -> ImageJobId {
    ImageJobId::from_bytes(&id(byte)).expect("job")
}

fn stage_id(byte: u8) -> ExecutionStageId {
    ExecutionStageId::from_bytes(&id(byte)).expect("stage")
}

fn dispatch_id(byte: u8) -> DispatchId {
    DispatchId::from_bytes(&id(byte)).expect("dispatch")
}

fn book_call_id(dispatch: DispatchId) -> String {
    df_imagery::execution::book_call_id_for(&dispatch)
}

#[test]
fn hosted_demand_runs_prepare_send_supplier_settle_to_ready() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        let (_receipt, enqueued) =
            accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        let first_revision = enqueued.view().job_revision;
        assert_eq!(enqueued.view().state, StageState::Planned);
        assert_eq!(
            enqueued
                .view()
                .planned_publication
                .expect("media plan")
                .asset,
            id(0x41)
        );
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (0, 1, 128)
        );

        let tx = client.transaction().await.expect("tx");
        let claimed = claim_stage_in(
            &tx,
            &claim_command(0x11, 0x21, GATE_REVISION, 0x71, 0, at(1)),
        )
        .await
        .expect("claim");
        assert_eq!(claimed.claim_generation, 1);
        assert!(claimed.job_revision > first_revision);
        tx.commit().await.expect("commit");
        assert_eq!(capacity_counts(&client, "live/generation").await, (1, 0, 0));

        let tx = client.transaction().await.expect("tx");
        let prepared = prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        assert_eq!(prepared.sequence, 0);
        assert_eq!(prepared.state, DispatchState::Prepared);
        assert_eq!(prepared.settlement, DispatchSettlement::Held);
        assert_eq!(prepared.finance, hosted_mode(0x81).finance());
        assert_eq!(prepared.book_call_id, book_call_id(prepared.dispatch));
        assert_eq!(prepared.lineage_parent, None);
        tx.commit().await.expect("commit");

        let tx = client.transaction().await.expect("tx");
        let sent = mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("mark sent");
        assert_eq!(sent.state, DispatchState::Sent);
        tx.commit().await.expect("commit");

        let tx = client.transaction().await.expect("tx");
        let correlated = record_supplier_in(
            &tx,
            &SupplierReport::new(
                owner(),
                job_id(0x11),
                stage_id(0x21),
                dispatch_id(0x81),
                "supplier-9f".to_owned(),
                at(4),
            )
            .expect("report"),
        )
        .await
        .expect("supplier");
        assert_eq!(correlated.supplier_job_id, Some("supplier-9f".to_owned()));
        tx.commit().await.expect("commit");

        let tx = client.transaction().await.expect("tx");
        let done = complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::Succeeded {
                    usage_revision: 2,
                    checkpoint: success_checkpoint(0x81, 2),
                },
                at(5),
            ),
        )
        .await
        .expect("complete");
        assert_eq!(done.state, DispatchState::Succeeded);
        assert_eq!(done.usage_revision, 2);
        tx.commit().await.expect("commit");

        let tx = client.transaction().await.expect("tx");
        let settled = mirror_settlement_in(
            &tx,
            &owner(),
            &job_id(0x11),
            &stage_id(0x21),
            &dispatch_id(0x81),
            DispatchSettlement::Settled,
            at(6),
        )
        .await
        .expect("mirror");
        assert_eq!(settled.settlement, DispatchSettlement::Settled);
        tx.commit().await.expect("commit");

        let tx = client.transaction().await.expect("tx");
        let resolved = resolve_execution_in(
            &tx,
            &owner(),
            &ExecutionIdentity::new(id(0x41), id(0x42)).expect("identity"),
        )
        .await
        .expect("resolve");
        let current = resolved.current.expect("current dispatch");
        assert_eq!(current.dispatch, dispatch_id(0x81));
        assert_eq!(current.state, DispatchState::Succeeded);
        assert_eq!(current.settlement, DispatchSettlement::Settled);
        assert!(resolved.local_checkpoint.is_none());
        tx.commit().await.expect("read commit");
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        scratch.finish().await;
    });
}

#[test]
fn exact_claim_and_prepare_replay_returns_the_original_rows() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;

        let tx = client.transaction().await.expect("tx");
        let claimed = claim_stage_in(
            &tx,
            &claim_command(0x11, 0x21, GATE_REVISION, 0x71, 0, at(1)),
        )
        .await
        .expect("claim");
        assert_eq!(claimed.claim_generation, 1);
        let replayed = claim_stage_in(
            &tx,
            &claim_command(0x11, 0x21, GATE_REVISION, 0x71, 0, at(2)),
        )
        .await
        .expect("claim replay");
        assert_eq!(replayed.claim_generation, 1);
        assert_eq!(replayed.lease_until_ms, claimed.lease_until_ms);
        tx.commit().await.expect("commit");

        let tx = client.transaction().await.expect("tx");
        let prepared = prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(3)),
        )
        .await
        .expect("prepare");
        let replayed = prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(4)),
        )
        .await
        .expect("prepare replay");
        assert_eq!(replayed.dispatch, prepared.dispatch);
        assert_eq!(replayed.sequence, 0);
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x82), at(5)),
            )
            .await,
            Err(ExecutionError::IdentityConflict)
        );
        tx.commit().await.expect("commit");
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn committed_prepare_survives_a_lost_ack_by_reread() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;

        // The worker prepares and commits, then the ack is lost on the way
        // back: the worker keeps only the dispatch id and rereads it.
        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        tx.commit().await.expect("commit");

        let tx = client.transaction().await.expect("tx");
        let reread = read_dispatch_in(&tx, &owner(), &dispatch_id(0x81))
            .await
            .expect("reread one identity");
        assert_eq!(reread.dispatch, dispatch_id(0x81));
        assert_eq!(reread.sequence, 0);
        assert_eq!(reread.state, DispatchState::Prepared);
        assert_eq!(reread.finance, hosted_mode(0x81).finance());
        mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("recovery continues from the same identity");
        tx.commit().await.expect("commit");
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn expired_claims_stale_tokens_and_oversized_leases_refuse() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;

        let tx = client.transaction().await.expect("tx");
        claim_stage_in(
            &tx,
            &claim_with_lease(0x11, 0x21, GATE_REVISION, 0x71, 0, 2_000, at(1)),
        )
        .await
        .expect("short claim");
        tx.commit().await.expect("commit");
        // A write under another token while the lease lives is lost.
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x81, 0x70, 0, hosted_mode(0x81), at(2)),
            )
            .await,
            Err(ExecutionError::LostClaim)
        );
        // A lease window reaching past the retained job deadline refuses:
        // no fixed ceiling, just the deadline the job carries.
        assert_eq!(
            claim_stage_in(
                &tx,
                &claim_with_lease(0x11, 0x21, GATE_REVISION, 0x72, 0, 60_000, at(0))
            )
            .await,
            Err(ExecutionError::Expired)
        );
        tx.rollback().await.expect("rollback");
        // The lease lapses; a fresh token supersedes to the next generation.
        let tx = client.transaction().await.expect("tx");
        let advanced = claim_stage_in(
            &tx,
            &claim_command(0x11, 0x21, GATE_REVISION, 0x72, 1, at(11_000)),
        )
        .await
        .expect("supersede");
        assert_eq!(advanced.claim_generation, 2);
        tx.commit().await.expect("commit");

        // Past the job deadline, claims and enqueues both refuse.
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            claim_stage_in(
                &tx,
                &claim_command(0x11, 0x21, GATE_REVISION, 0x73, 2, at(60_000))
            )
            .await,
            Err(ExecutionError::Expired)
        );
        assert_eq!(
            enqueue_stage_in(&tx, &enqueue_command(0x11, 0x22, at(60_000))).await,
            Err(ExecutionError::Expired)
        );
        scratch.finish().await;
    });
}

#[test]
fn a_second_prepare_while_one_is_current_refuses_without_writing() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;

        let tx = client.transaction().await.expect("tx");
        let first = prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("first");
        assert_eq!(first.sequence, 0);
        // A second id while the first is still prepared never supersedes it.
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x82, 0x71, 1, hosted_mode(0x82), at(3)),
            )
            .await,
            Err(ExecutionError::Superseded)
        );
        tx.commit().await.expect("commit");

        // The refused id left no row behind.
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            read_dispatch_in(&tx, &owner(), &dispatch_id(0x82)).await,
            Err(ExecutionError::NotFound)
        );
        // The refused id was never prepared, so it cannot send either.
        assert_eq!(
            mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x82, 0x71, 1, at(4))).await,
            Err(ExecutionError::NotFound)
        );
        scratch.finish().await;
    });
}

#[test]
fn concurrent_prepares_of_one_identity_agree_without_a_second_send() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;

        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let left_config = scratch.config();
        let right_config = scratch.config();
        let left_barrier = barrier.clone();
        let left = tokio::spawn(async move {
            let mut client = connect(&left_config).await;
            left_barrier.wait().await;
            let tx = client.transaction().await.expect("tx");
            let prepared = prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
            )
            .await
            .expect("left prepare");
            tx.commit().await.expect("commit");
            prepared
        });
        let right = tokio::spawn(async move {
            let mut client = connect(&right_config).await;
            barrier.wait().await;
            let tx = client.transaction().await.expect("tx");
            let prepared = prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
            )
            .await
            .expect("right prepare");
            tx.commit().await.expect("commit");
            prepared
        });
        let (left, right) = tokio::join!(left, right);
        let left = left.expect("left task").sequence;
        let right = right.expect("right task").sequence;
        assert_eq!((left, right), (0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        assert_eq!(capacity_counts(&client, "live/generation").await, (1, 0, 0));
        scratch.finish().await;
    });
}

#[test]
fn concurrent_claims_leave_exactly_one_winner() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;

        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let left_config = scratch.config();
        let right_config = scratch.config();
        let left_barrier = barrier.clone();
        let left = tokio::spawn(async move {
            let mut client = connect(&left_config).await;
            left_barrier.wait().await;
            let tx = client.transaction().await.expect("tx");
            let outcome = claim_stage_in(
                &tx,
                &claim_command(0x11, 0x21, GATE_REVISION, 0x71, 0, at(1)),
            )
            .await;
            tx.commit().await.expect("commit");
            outcome.map(|view| view.claim_generation)
        });
        let right = tokio::spawn(async move {
            let mut client = connect(&right_config).await;
            barrier.wait().await;
            let tx = client.transaction().await.expect("tx");
            let outcome = claim_stage_in(
                &tx,
                &claim_command(0x11, 0x21, GATE_REVISION, 0x72, 0, at(1)),
            )
            .await;
            // A lost claim rolls back; committing would be harmless but the
            // rollback proves nothing was written by the loser.
            if outcome.is_err() {
                tx.rollback().await.expect("rollback");
            } else {
                tx.commit().await.expect("commit");
            }
            outcome.map(|view| view.claim_generation)
        });
        let (left, right) = tokio::join!(left, right);
        let mut outcomes = [left.expect("left task"), right.expect("right task")];
        outcomes.sort_by_key(|result| result.is_err());
        assert_eq!(outcomes[0], Ok(1));
        assert_eq!(outcomes[1], Err(ExecutionError::LostClaim));
        assert_eq!(capacity_counts(&client, "live/generation").await, (1, 0, 0));
        scratch.finish().await;
    });
}

#[test]
fn retry_after_failure_needs_a_fresh_finance_identity() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;

        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("mark sent");
        let failed = complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::Failed {
                    cause: FailureCause::TransportLost,
                },
                at(4),
            ),
        )
        .await
        .expect("complete");
        assert_eq!(failed.state, DispatchState::Failed);
        assert_eq!(failed.cause, Some(FailureCause::TransportLost));
        // An identical terminal replay returns the same outcome.
        let replayed = complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::Failed {
                    cause: FailureCause::TransportLost,
                },
                at(5),
            ),
        )
        .await
        .expect("identical replay");
        assert_eq!(replayed.state, DispatchState::Failed);
        // A conflicting outcome over the terminal row is refused.
        assert_eq!(
            complete_dispatch_in(
                &tx,
                &finish_command(
                    0x11,
                    0x21,
                    0x81,
                    0x71,
                    1,
                    CompletionOutcome::Failed {
                        cause: FailureCause::Refused,
                    },
                    at(5),
                ),
            )
            .await,
            Err(ExecutionError::OperationConflict)
        );
        assert_eq!(
            complete_dispatch_in(
                &tx,
                &finish_command(
                    0x11,
                    0x21,
                    0x81,
                    0x71,
                    1,
                    CompletionOutcome::Succeeded {
                        usage_revision: 1,
                        checkpoint: success_checkpoint(0x81, 1),
                    },
                    at(5),
                ),
            )
            .await,
            Err(ExecutionError::OperationConflict)
        );
        // A retry reusing the spent finance identity is refused.
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x82, 0x71, 1, hosted_mode(0x81), at(6)),
            )
            .await,
            Err(ExecutionError::IdentityConflict)
        );
        // A retry with a fresh finance admission links to its parent.
        let retry = prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x82, 0x71, 1, hosted_mode(0x82), at(6)),
        )
        .await
        .expect("fresh retry");
        assert_eq!(retry.sequence, 1);
        assert_eq!(retry.lineage_parent, Some(dispatch_id(0x81)));
        tx.commit().await.expect("commit");

        // The cursor moved: the old dispatch can no longer send.
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(7))).await,
            Err(ExecutionError::Superseded)
        );
        mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x82, 0x71, 1, at(7)))
            .await
            .expect("retry sends");
        tx.commit().await.expect("commit");
        scratch.finish().await;
    });
}

#[test]
fn pending_capacity_refuses_over_reserve_and_release_frees_it() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        // One job's retained intent (64) plus config (64) bytes fits exactly.
        let bounds = CapacityBounds::new(4, 64, 128).expect("bounds");
        let tx = client.transaction().await.expect("tx");
        set_capacity_bounds_in(&tx, "live/generation", &bounds, NOW_MS)
            .await
            .expect("bound");
        set_execution_gate_in(&tx, "live/generation", GATE_REVISION, true, NOW_MS)
            .await
            .expect("gate");
        tx.commit().await.expect("commit");

        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (0, 1, 128)
        );
        // A second job's bytes no longer fit the pending queue.
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            accept_with_execution_in(
                &tx,
                &accept_command(0x20, 0x22, 0x23, DEADLINE_MS, at(1)),
                &enqueue_command(0x22, 0x24, at(1)),
            )
            .await,
            Err(ExecutionError::CapacityFull)
        );
        tx.rollback().await.expect("rollback");

        // Claiming moves pending to active; the waiting stage enqueues next.
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(2)).await;
        assert_eq!(capacity_counts(&client, "live/generation").await, (1, 0, 0));
        accept_and_enqueue(&mut client, 0x20, 0x22, 0x23, 0x24, at(3)).await;
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (1, 1, 128)
        );
        scratch.finish().await;
    });
}

#[test]
fn lowering_the_active_bound_stops_new_claims_and_keeps_running_work() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        let bounds = CapacityBounds::new(2, 64, 1 << 20).expect("bounds");
        let tx = client.transaction().await.expect("tx");
        set_capacity_bounds_in(&tx, "live/generation", &bounds, NOW_MS)
            .await
            .expect("bound");
        set_execution_gate_in(&tx, "live/generation", GATE_REVISION, true, NOW_MS)
            .await
            .expect("gate");
        tx.commit().await.expect("commit");

        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        accept_and_enqueue(&mut client, 0x20, 0x22, 0x23, 0x24, at(2)).await;
        claim(&mut client, 0x22, 0x24, 0x72, 0, at(3)).await;
        assert_eq!(capacity_counts(&client, "live/generation").await, (2, 0, 0));

        // The bound drops below current usage: allowed, and the running
        // stages keep their liability.
        let reduced = CapacityBounds::new(1, 64, 1 << 20).expect("reduced");
        let tx = client.transaction().await.expect("tx");
        set_capacity_bounds_in(&tx, "live/generation", &reduced, at(4))
            .await
            .expect("bound reduction keeps existing liability");
        tx.commit().await.expect("commit");
        assert_eq!(capacity_counts(&client, "live/generation").await, (2, 0, 0));

        // New work stops at the reduced bound.
        accept_and_enqueue(&mut client, 0x30, 0x33, 0x34, 0x25, at(5)).await;
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            claim_stage_in(
                &tx,
                &claim_command(0x33, 0x25, GATE_REVISION, 0x73, 0, at(6))
            )
            .await,
            Err(ExecutionError::CapacityFull)
        );
        tx.rollback().await.expect("rollback");

        // Releasing the running stages drains below the bound; the queued
        // stage claims next without losing its pending reserve.
        for (job, stage, token) in [(0x11, 0x21, 0x71), (0x22, 0x24, 0x72)] {
            let tx = client.transaction().await.expect("tx");
            release_stage_in(
                &tx,
                &ReleaseCommand::new(
                    owner(),
                    job_id(job),
                    stage_id(stage),
                    ClaimToken::from_bytes(&id(token)).expect("claim"),
                    1,
                    at(7),
                )
                .expect("release"),
            )
            .await
            .expect("release");
            tx.commit().await.expect("commit");
        }
        claim(&mut client, 0x33, 0x25, 0x73, 0, at(8)).await;
        assert_eq!(capacity_counts(&client, "live/generation").await, (1, 0, 0));
        scratch.finish().await;
    });
}

#[test]
fn wrong_owner_learns_nothing() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            read_stage_in(&tx, &draft_owner(), &job_id(0x11), &stage_id(0x21)).await,
            Err(ExecutionError::NotFound)
        );
        assert_eq!(
            resolve_execution_in(
                &tx,
                &draft_owner(),
                &ExecutionIdentity::new(id(0x41), id(0x42)).expect("identity"),
            )
            .await,
            Err(ExecutionError::NotFound)
        );
        assert_eq!(
            read_supplier_checkpoint_in(
                &tx,
                &draft_owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await,
            Err(ExecutionError::NotFound)
        );
        scratch.finish().await;
    });
}

#[test]
fn applied_gate_fences_claims_and_missing_gates_fence_at_zero() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            claim_stage_in(
                &tx,
                &claim_command(0x11, 0x21, GATE_REVISION + 1, 0x71, 0, at(1))
            )
            .await,
            Err(ExecutionError::ConfigFenced {
                current_revision: GATE_REVISION
            })
        );
        claim_stage_in(
            &tx,
            &claim_command(0x11, 0x21, GATE_REVISION, 0x71, 0, at(1)),
        )
        .await
        .expect("applied revision claims");
        assert_eq!(
            read_execution_gate_in(&tx, "live/generation")
                .await
                .expect("gate")
                .applied_revision,
            GATE_REVISION
        );
        // No operator revision was ever applied for this scope.
        assert_eq!(
            read_execution_gate_in(&tx, "unowned/scope").await,
            Err(ExecutionError::ConfigFenced {
                current_revision: 0
            })
        );
        tx.commit().await.expect("commit");
        scratch.finish().await;
    });
}

#[test]
fn committed_disable_refuses_new_work_and_keeps_sent_liability() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("mark sent");
        tx.commit().await.expect("commit");

        // The operator commits a disable for the scope.
        let tx = client.transaction().await.expect("tx");
        set_execution_gate_in(&tx, "live/generation", GATE_REVISION + 1, false, at(4))
            .await
            .expect("disable");
        tx.commit().await.expect("commit");

        // Claims, prepares and sends refuse under the disable.
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            claim_stage_in(
                &tx,
                &claim_command(0x11, 0x21, GATE_REVISION + 1, 0x72, 1, at(5))
            )
            .await,
            Err(ExecutionError::Disabled)
        );
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x82, 0x71, 1, hosted_mode(0x82), at(5)),
            )
            .await,
            Err(ExecutionError::Disabled)
        );
        // Replaying an already committed marker reads history; it grants no new send.
        assert_eq!(
            mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(5)))
                .await
                .expect("historical sent replay")
                .state,
            DispatchState::Sent
        );
        // The already-sent dispatch keeps its outstanding liability.
        let sent = read_dispatch_in(&tx, &owner(), &dispatch_id(0x81))
            .await
            .expect("sent liability retained");
        assert_eq!(sent.state, DispatchState::Sent);
        assert_eq!(sent.settlement, DispatchSettlement::Held);
        tx.rollback().await.expect("rollback");

        // Re-enabling resumes new work under the new revision.
        let tx = client.transaction().await.expect("tx");
        set_execution_gate_in(&tx, "live/generation", GATE_REVISION + 2, true, at(6))
            .await
            .expect("re-enable");
        tx.commit().await.expect("commit");
        let tx = client.transaction().await.expect("tx");
        let reclaimed = claim_stage_in(
            &tx,
            &claim_command(0x11, 0x21, GATE_REVISION + 2, 0x72, 0, at(20_000)),
        )
        .await
        .expect("re-enabled claim");
        assert_eq!(reclaimed.claim_generation, 2);
        tx.commit().await.expect("commit");
        scratch.finish().await;
    });
}

#[test]
fn lost_commit_keeps_ids_for_reread_and_retry() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;

        // A deferred journal link dangles until commit, which must then fail:
        // the execution writes may or may not have taken effect.
        let tx = client.transaction().await.expect("tx");
        accept_with_execution_in(
            &tx,
            &accept_command(0x10, 0x11, 0x12, DEADLINE_MS, at(0)),
            &enqueue_command(0x11, 0x21, at(0)),
        )
        .await
        .expect("accept with execution");
        let scope = df_imagery::owner_scope_bytes(&owner());
        tx.execute(
            "INSERT INTO imagery.image_operation (owner_scope, operation_id, fingerprint, \
             job_id, interest_id, created_at_ms) VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &scope,
                &id(0x60).as_slice(),
                &[0x61u8; 32].as_slice(),
                &id(0x11).as_slice(),
                &id(0x99).as_slice(),
                &at(0),
            ],
        )
        .await
        .expect("dangling link");
        assert_eq!(commit_execution(tx).await, Err(ExecutionError::Unknown));

        // Reread by id finds nothing committed, and the retry enqueues cleanly.
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            read_stage_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21)).await,
            Err(ExecutionError::NotFound)
        );
        accept_with_execution_in(
            &tx,
            &accept_command(0x10, 0x11, 0x12, DEADLINE_MS, at(1)),
            &enqueue_command(0x11, 0x21, at(1)),
        )
        .await
        .expect("retry after unknown");
        tx.commit().await.expect("commit");
        scratch.finish().await;
    });
}

#[test]
fn supplier_ids_are_conflict_checked_idempotent_updates() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("mark sent");
        let report = |supplier: &str, now_ms: i64| {
            SupplierReport::new(
                owner(),
                job_id(0x11),
                stage_id(0x21),
                dispatch_id(0x81),
                supplier.to_owned(),
                now_ms,
            )
            .expect("report")
        };
        record_supplier_in(&tx, &report("supplier-a", at(4)))
            .await
            .expect("first");
        record_supplier_in(&tx, &report("supplier-a", at(5)))
            .await
            .expect("idempotent replay");
        assert_eq!(
            record_supplier_in(&tx, &report("supplier-b", at(6))).await,
            Err(ExecutionError::SupplierConflict)
        );
        scratch.finish().await;
    });
}

#[test]
fn supplier_checkpoints_replay_identically_and_conflict_on_rewrite() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");

        let first =
            record_supplier_checkpoint_in(&tx, &supplier_checkpoint(0x11, 0x21, 0x81, 1, at(3)))
                .await
                .expect("record");
        assert_eq!(first.checkpoint_version, 1);
        assert_eq!(first.origin, "fal");
        assert_eq!(first.model, "flux-ultra");
        let replayed =
            record_supplier_checkpoint_in(&tx, &supplier_checkpoint(0x11, 0x21, 0x81, 1, at(4)))
                .await
                .expect("identical replay");
        assert_eq!(replayed.checkpoint_bytes, first.checkpoint_bytes);
        let mut rewritten = supplier_checkpoint(0x11, 0x21, 0x81, 1, at(5));
        rewritten.checkpoint_bytes = vec![0x79; 64];
        assert_eq!(
            record_supplier_checkpoint_in(&tx, &rewritten).await,
            Err(ExecutionError::SupplierConflict)
        );
        record_supplier_checkpoint_in(&tx, &supplier_checkpoint(0x11, 0x21, 0x81, 2, at(6)))
            .await
            .expect("next version");
        let latest = read_supplier_checkpoint_in(
            &tx,
            &owner(),
            &job_id(0x11),
            &stage_id(0x21),
            &dispatch_id(0x81),
        )
        .await
        .expect("read")
        .expect("latest");
        assert_eq!(latest.checkpoint_version, 2);
        tx.commit().await.expect("commit");
        scratch.finish().await;
    });
}

#[test]
fn unknown_keeps_liability_and_never_resends_from_here() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;

        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("mark sent");
        let unknown = complete_dispatch_in(
            &tx,
            &finish_command(0x11, 0x21, 0x81, 0x71, 1, CompletionOutcome::Unknown, at(4)),
        )
        .await
        .expect("unknown");
        assert_eq!(unknown.state, DispatchState::Unknown);
        assert_eq!(unknown.settlement, DispatchSettlement::UnknownOutstanding);
        tx.commit().await.expect("commit");

        // No resend path exists while the unknown dispatch stands: a fresh
        // id refuses, and marking the unknown sent refuses too.
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x82, 0x71, 1, hosted_mode(0x82), at(5)),
            )
            .await,
            Err(ExecutionError::Superseded)
        );
        assert_eq!(
            mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(5))).await,
            Err(ExecutionError::Superseded)
        );
        let old = read_dispatch_in(&tx, &owner(), &dispatch_id(0x81))
            .await
            .expect("old row intact");
        assert_eq!(old.state, DispatchState::Unknown);
        tx.commit().await.expect("read commit");
        assert_eq!(capacity_counts(&client, "live/generation").await, (1, 0, 0));
        scratch.finish().await;
    });
}

#[test]
fn unknown_accepts_a_late_correlated_result_without_a_new_send() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;

        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("mark sent");
        complete_dispatch_in(
            &tx,
            &finish_command(0x11, 0x21, 0x81, 0x71, 1, CompletionOutcome::Unknown, at(4)),
        )
        .await
        .expect("unknown");
        tx.commit().await.expect("commit");

        // The lease lapsed while the supplier worked: a fresh claim takes
        // over, then the late receipt and recovered output resolve the same
        // dispatch with no new send.
        claim(&mut client, 0x11, 0x21, 0x72, 0, at(11_000)).await;
        let tx = client.transaction().await.expect("tx");
        record_supplier_in(
            &tx,
            &SupplierReport::new(
                owner(),
                job_id(0x11),
                stage_id(0x21),
                dispatch_id(0x81),
                "supplier-9f".to_owned(),
                at(12_000),
            )
            .expect("report"),
        )
        .await
        .expect("late receipt");
        record_supplier_checkpoint_in(&tx, &supplier_checkpoint(0x11, 0x21, 0x81, 1, at(12_000)))
            .await
            .expect("resume bytes");
        let resolved = complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x72,
                2,
                CompletionOutcome::Succeeded {
                    usage_revision: 2,
                    checkpoint: success_checkpoint(0x81, 2),
                },
                at(12_000),
            ),
        )
        .await
        .expect("late resolution");
        assert_eq!(resolved.state, DispatchState::Succeeded);
        assert_eq!(resolved.supplier_job_id, Some("supplier-9f".to_owned()));
        assert_eq!(resolved.settlement, DispatchSettlement::UnknownOutstanding);
        // A conflicting late outcome over the resolved row is refused.
        assert_eq!(
            complete_dispatch_in(
                &tx,
                &finish_command(
                    0x11,
                    0x21,
                    0x81,
                    0x72,
                    2,
                    CompletionOutcome::Failed {
                        cause: FailureCause::TransportLost,
                    },
                    at(13_000),
                ),
            )
            .await,
            Err(ExecutionError::OperationConflict)
        );
        tx.commit().await.expect("commit");
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn refetch_selection_persists_its_dispatch() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        request_refetch_in(
            &tx,
            &RefetchCommand::new(
                owner(),
                job_id(0x11),
                stage_id(0x21),
                dispatch_id(0x81),
                at(3),
            )
            .expect("refetch"),
        )
        .await
        .expect("refetch");
        tx.commit().await.expect("commit");
        let tx = client.transaction().await.expect("tx");
        let resolved = resolve_execution_in(
            &tx,
            &owner(),
            &ExecutionIdentity::new(id(0x41), id(0x42)).expect("identity"),
        )
        .await
        .expect("resolve");
        assert_eq!(resolved.refetch, Some(dispatch_id(0x81)));
        scratch.finish().await;
    });
}

#[test]
fn local_work_resolves_to_its_checkpoint_with_no_finance_tuple() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.expect("tx");
        let bounds = CapacityBounds::new(4, 64, 1 << 20).expect("bounds");
        set_capacity_bounds_in(&tx, "local/transform", &bounds, NOW_MS)
            .await
            .expect("bound");
        set_execution_gate_in(&tx, "local/transform", GATE_REVISION, true, NOW_MS)
            .await
            .expect("gate");
        tx.commit().await.expect("commit");

        let mut local = enqueue_command(0x11, 0x21, at(0));
        local.kind = StageKind::LocalTransform;
        local.local_accounting = true;
        local.capacity = CapacityScope::new("local/transform".to_owned()).expect("scope");
        let tx = client.transaction().await.expect("tx");
        accept_with_execution_in(
            &tx,
            &accept_command(0x10, 0x11, 0x12, DEADLINE_MS, at(0)),
            &local,
        )
        .await
        .expect("accept local");
        tx.commit().await.expect("commit");
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;

        let tx = client.transaction().await.expect("tx");
        let prepared = prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, DispatchMode::Local, at(2)),
        )
        .await
        .expect("prepare local");
        assert_eq!(prepared.finance, None);
        assert!(prepared.local_accounting);
        let done = complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::Succeeded {
                    usage_revision: 0,
                    checkpoint: success_checkpoint(0x81, 0),
                },
                at(3),
            ),
        )
        .await
        .expect("complete local");
        assert_eq!(done.finance, None);
        tx.commit().await.expect("commit");

        let tx = client.transaction().await.expect("tx");
        let resolved = resolve_execution_in(
            &tx,
            &owner(),
            &ExecutionIdentity::new(id(0x41), id(0x42)).expect("identity"),
        )
        .await
        .expect("resolve");
        let checkpoint = resolved.local_checkpoint.expect("local checkpoint");
        assert_eq!(checkpoint, success_checkpoint(0x81, 0));
        assert_eq!(resolved.current.map(|view| view.finance), Some(None));
        scratch.finish().await;
    });
}

#[test]
fn conflicting_bases_replay_or_refuse_without_double_reserve() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        let (_receipt, first) =
            accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        assert!(matches!(first, EnqueueOutcome::Enqueued(_)));

        // An identical enqueue replays without reserving again.
        let tx = client.transaction().await.expect("tx");
        let replayed = enqueue_stage_in(&tx, &enqueue_command(0x11, 0x21, at(1)))
            .await
            .expect("replay");
        assert!(matches!(replayed, EnqueueOutcome::Existing(_)));
        tx.commit().await.expect("commit");
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (0, 1, 128)
        );

        // A different immutable basis conflicts; another planned identity too.
        let tx = client.transaction().await.expect("tx");
        let mut other_kind = enqueue_command(0x11, 0x21, at(2));
        other_kind.kind = StageKind::Edit;
        assert_eq!(
            enqueue_stage_in(&tx, &other_kind).await,
            Err(ExecutionError::BasisConflict)
        );
        let mut other_planned = enqueue_command(0x11, 0x21, at(2));
        other_planned
            .planned_publication
            .as_mut()
            .expect("media plan")
            .asset = id(0x43);
        assert_eq!(
            enqueue_stage_in(&tx, &other_planned).await,
            Err(ExecutionError::PlannedIdentityConflict)
        );
        tx.rollback().await.expect("rollback");
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (0, 1, 128)
        );
        scratch.finish().await;
    });
}

#[test]
fn conflicting_accept_rolls_back_demand_and_reserve_together() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;

        // Same operation under another fingerprint: the whole transaction,
        // demand and reserve alike, must roll back.
        let tx = client.transaction().await.expect("tx");
        let mut conflict = accept_command(0x10, 0x11, 0x13, DEADLINE_MS, at(1));
        conflict.identity.fingerprint = [0x99; 32];
        assert_eq!(
            accept_with_execution_in(&tx, &conflict, &enqueue_command(0x11, 0x21, at(1))).await,
            Err(ExecutionError::OperationConflict)
        );
        tx.rollback().await.expect("rollback");
        let tx = client.transaction().await.expect("tx");
        let stages: i64 = tx
            .query_one("SELECT COUNT(*) FROM imagery.image_execution_stage", &[])
            .await
            .expect("count")
            .get("count");
        assert_eq!(stages, 1);
        tx.commit().await.expect("read commit");
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (0, 1, 128)
        );
        scratch.finish().await;
    });
}

#[test]
fn cancel_before_send_releases_proven_unsent_work() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;

        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        let cancelled = cancel_dispatch_in(
            &tx,
            &CancelCommand::new(
                owner(),
                job_id(0x11),
                stage_id(0x21),
                dispatch_id(0x81),
                ClaimToken::from_bytes(&id(0x71)).expect("claim"),
                1,
                at(3),
            )
            .expect("cancel"),
        )
        .await
        .expect("cancel");
        assert_eq!(cancelled.state, DispatchState::Released);
        assert_eq!(cancelled.settlement, DispatchSettlement::Released);
        tx.commit().await.expect("commit");
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));

        // A cancelled dispatch never sends, and its stage still releases.
        let tx = client.transaction().await.expect("tx");
        assert_eq!(
            mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(4))).await,
            Err(ExecutionError::Superseded)
        );
        let released = release_stage_in(
            &tx,
            &ReleaseCommand::new(
                owner(),
                job_id(0x11),
                stage_id(0x21),
                ClaimToken::from_bytes(&id(0x71)).expect("claim"),
                1,
                at(4),
            )
            .expect("release"),
        )
        .await
        .expect("release after cancel");
        assert_eq!(released.state, StageState::Released);
        tx.commit().await.expect("commit");
        scratch.finish().await;
    });
}

/// Inserts a real asset row plus a real 0039 publication allocation naming
/// one stage, against the full composition schema.
async fn insert_asset_and_allocation(
    client: &mut Client,
    asset: u8,
    publication: u8,
    job: u8,
    stage: u8,
) {
    client
        .execute(
            "INSERT INTO media.asset (asset_id, kind, provenance, charge_scope_kind, \
             charge_scope_id, audience_kind, audience_account, dedupe_scope, state, \
             attempt, retention_class, created_at_ms, updated_at_ms) \
             VALUES ($1, 'image', 'generated', 'campaign', $2, 'owner_private', $3, \
             $2, 'uploading', $4, 'campaign_media', $5, $5)",
            &[
                &id(asset).as_slice(),
                &id(0x53).as_slice(),
                &id(0x61).as_slice(),
                &id(publication).as_slice(),
                &NOW_MS,
            ],
        )
        .await
        .expect("asset");
    let scope = df_imagery::owner_scope_bytes(&owner());
    client
        .execute(
            "INSERT INTO imagery.image_publication_allocation (publication_attempt, asset_id, \
             job_id, stage_id, owner_scope, original_recipe_key, original_recipe_revision, \
             original_recipe_bytes, original_config_revision, original_config_bytes, \
             created_at_ms) VALUES ($1, $2, $3, $4, $5, $6, 1, $7, 1, $7, $8)",
            &[
                &id(publication).as_slice(),
                &id(asset).as_slice(),
                &id(job).as_slice(),
                &id(stage).as_slice(),
                &scope,
                &[0x31u8; 32].as_slice(),
                &vec![0x32u8; 16],
                &NOW_MS,
            ],
        )
        .await
        .expect("allocation");
}

#[test]
fn published_resolution_follows_the_allocation_stage_after_a_classification_touch() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        // One job with an output stage, then a classification stage under the
        // same planned identity. The second accept replays the same receipt.
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x22, at(0)).await;
        let tx = client.transaction().await.expect("tx");
        let replayed = accept_with_execution_in(
            &tx,
            &accept_command(0x10, 0x11, 0x12, DEADLINE_MS, at(1)),
            &{
                let mut classification = enqueue_command(0x11, 0x21, at(1));
                classification.kind = StageKind::Classification;
                classification
            },
        )
        .await
        .expect("classification stage");
        assert!(matches!(replayed.1, EnqueueOutcome::Enqueued(_)));
        tx.commit().await.expect("commit");

        let tx = client.transaction().await.expect("inspection tx");
        let mut inspection = enqueue_command(0x11, 0x23, at(2));
        inspection.kind = StageKind::Inspection;
        enqueue_stage_in(&tx, &inspection)
            .await
            .expect("later inspection");
        tx.commit().await.expect("inspection commit");
        // Touching classification last must not reroute recovery.
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(2)).await;
        insert_asset_and_allocation(&mut client, 0x41, 0x42, 0x11, 0x22).await;

        let tx = client.transaction().await.expect("tx");
        let published = resolve_execution_in(
            &tx,
            &owner(),
            &ExecutionIdentity::new(id(0x41), id(0x42)).expect("identity"),
        )
        .await
        .expect("published resolve");
        assert_eq!(published.job, job_id(0x11));
        assert_eq!(published.stage, stage_id(0x22));
        let planned = resolve_execution_in(
            &tx,
            &owner(),
            &ExecutionIdentity::new(id(0x41), id(0x42)).expect("identity"),
        )
        .await
        .expect("planned resolve");
        assert_eq!(planned.job, job_id(0x11));
        assert_eq!(planned.stage, stage_id(0x22));
        assert_eq!(
            resolve_execution_in(
                &tx,
                &owner(),
                &ExecutionIdentity::new(id(0x47), id(0x48)).expect("identity"),
            )
            .await,
            Err(ExecutionError::NotFound)
        );
        scratch.finish().await;
    });
}

#[test]
fn restart_keeps_the_committed_local_checkpoint() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.expect("tx");
        let bounds = CapacityBounds::new(4, 64, 1 << 20).expect("bounds");
        set_capacity_bounds_in(&tx, "local/transform", &bounds, NOW_MS)
            .await
            .expect("bound");
        set_execution_gate_in(&tx, "local/transform", GATE_REVISION, true, NOW_MS)
            .await
            .expect("gate");
        tx.commit().await.expect("commit");

        let mut local = enqueue_command(0x11, 0x21, at(0));
        local.kind = StageKind::LocalTransform;
        local.local_accounting = true;
        local.capacity = CapacityScope::new("local/transform".to_owned()).expect("scope");
        let tx = client.transaction().await.expect("tx");
        accept_with_execution_in(
            &tx,
            &accept_command(0x10, 0x11, 0x12, DEADLINE_MS, at(0)),
            &local,
        )
        .await
        .expect("accept local");
        tx.commit().await.expect("commit");
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        let tx = client.transaction().await.expect("tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, DispatchMode::Local, at(2)),
        )
        .await
        .expect("prepare local");
        complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::Succeeded {
                    usage_revision: 0,
                    checkpoint: success_checkpoint(0x81, 0),
                },
                at(3),
            ),
        )
        .await
        .expect("complete local");
        tx.commit().await.expect("commit");
        drop(client);

        // A new connection after restart reads the same checkpoint bytes.
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.expect("tx");
        let resolved = resolve_execution_in(
            &tx,
            &owner(),
            &ExecutionIdentity::new(id(0x41), id(0x42)).expect("identity"),
        )
        .await
        .expect("resolve after restart");
        let checkpoint = resolved.local_checkpoint.expect("local checkpoint");
        assert_eq!(checkpoint, success_checkpoint(0x81, 0));
        let current = resolved.current.expect("current dispatch");
        assert_eq!(current.dispatch, dispatch_id(0x81));
        assert_eq!(current.state, DispatchState::Succeeded);
        scratch.finish().await;
    });
}

#[test]
fn journal_port_serves_a_stage_end_to_end() {
    let runtime = runtime();
    runtime.block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;

        let journal = PostgresImageExecution::open(
            scratch.config(),
            PostgresTransport::PlaintextLoopbackOnly,
            2,
        )
        .await
        .expect("open journal");
        let queued = journal
            .scan_queue("live/generation", 10, at(0))
            .await
            .expect("scan");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].stage, stage_id(0x21));
        assert_eq!(queued[0].state, StageState::Planned);

        let claimed = journal
            .claim(&claim_command(0x11, 0x21, GATE_REVISION, 0x71, 0, at(1)))
            .await
            .expect("claim");
        assert_eq!(claimed.claim_generation, 1);
        let prepared = journal
            .prepare(&prepare_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                hosted_mode(0x81),
                at(2),
            ))
            .await
            .expect("prepare");
        assert_eq!(prepared.sequence, 0);
        let sent = journal
            .mark_sent(&mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("mark sent");
        assert_eq!(sent.state, DispatchState::Sent);
        journal
            .report_supplier(
                &SupplierReport::new(
                    owner(),
                    job_id(0x11),
                    stage_id(0x21),
                    dispatch_id(0x81),
                    "supplier-9f".to_owned(),
                    at(4),
                )
                .expect("report"),
            )
            .await
            .expect("supplier");
        let checkpoint = journal
            .record_supplier_checkpoint(&supplier_checkpoint(0x11, 0x21, 0x81, 1, at(4)))
            .await
            .expect("supplier checkpoint");
        assert_eq!(checkpoint.checkpoint_version, 1);
        let reread = journal
            .read_supplier_checkpoint(&owner(), &job_id(0x11), &stage_id(0x21), &dispatch_id(0x81))
            .await
            .expect("read checkpoint")
            .expect("checkpoint");
        assert_eq!(reread.checkpoint_bytes, checkpoint.checkpoint_bytes);
        let done = journal
            .complete(&finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::Succeeded {
                    usage_revision: 2,
                    checkpoint: success_checkpoint(0x81, 2),
                },
                at(5),
            ))
            .await
            .expect("complete");
        assert_eq!(done.state, DispatchState::Succeeded);
        let resolved = journal
            .resolve(
                &owner(),
                &ExecutionIdentity::new(id(0x41), id(0x42)).expect("identity"),
            )
            .await
            .expect("resolve");
        assert_eq!(
            resolved.current.expect("current").dispatch,
            dispatch_id(0x81)
        );
        let stage = journal
            .read_stage(&owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .expect("read stage");
        assert_eq!(stage.state, StageState::Admitted);
        let dispatch = journal
            .read_dispatch(&owner(), &dispatch_id(0x81))
            .await
            .expect("read dispatch");
        assert_eq!(dispatch.state, DispatchState::Succeeded);

        journal
            .set_execution_gate("live/generation", GATE_REVISION + 1, false, at(6))
            .await
            .expect("disable");
        assert_eq!(
            journal
                .read_execution_gate("live/generation")
                .await
                .expect("gate")
                .enabled,
            false
        );
        assert_eq!(
            journal
                .claim(&claim_command(
                    0x11,
                    0x21,
                    GATE_REVISION + 1,
                    0x72,
                    1,
                    at(7)
                ))
                .await,
            Err(ExecutionError::Disabled)
        );
        scratch.finish().await;
    });
}

#[test]
fn gate_disable_waits_for_the_transaction_that_committed_sent() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut sender = connect(&scratch.config()).await;
        setup_world(&mut sender).await;
        accept_and_enqueue(&mut sender, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut sender, 0x11, 0x21, 0x71, 0, at(1)).await;
        let tx = sender.transaction().await.expect("prepare tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        tx.commit().await.expect("prepare commit");
        let tx = sender.transaction().await.expect("sent tx");
        mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("sent holds gate lock");
        let mut operator = connect(&scratch.config()).await;
        let operator_pid: i32 = operator
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .expect("operator pid")
            .get(0);
        let observer = connect(&scratch.config()).await;
        // The handle owns the operator transaction until its gate update and commit finish.
        let disable = tokio::spawn(async move {
            let tx = operator.transaction().await.expect("disable tx");
            set_execution_gate_in(&tx, "live/generation", GATE_REVISION + 1, false, at(4))
                .await
                .expect("disable");
            tx.commit().await.expect("disable commit");
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let blocked: bool = observer
                    .query_one(
                        "SELECT cardinality(pg_blocking_pids($1)) > 0",
                        &[&operator_pid],
                    )
                    .await
                    .expect("blocking evidence")
                    .get(0);
                if blocked {
                    break;
                }
                assert!(
                    !disable.is_finished(),
                    "disable acknowledged before Sent committed"
                );
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("operator must block on held gate lock");
        tx.commit().await.expect("sent commit");
        disable.await.expect("operator joined");
        let tx = sender.transaction().await.expect("read tx");
        let gate = read_execution_gate_in(&tx, "live/generation")
            .await
            .expect("gate");
        assert!(!gate.enabled);
        assert_eq!(gate.applied_revision, GATE_REVISION + 1);
        assert_eq!(
            read_dispatch_in(&tx, &owner(), &dispatch_id(0x81))
                .await
                .expect("retained sent")
                .state,
            DispatchState::Sent
        );
        tx.commit().await.expect("read commit");
        scratch.finish().await;
    });
}

#[test]
fn changed_enabled_revision_fences_new_prepare_and_send_but_exact_prepare_replays() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        accept_and_enqueue(&mut client, 0x20, 0x22, 0x23, 0x24, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        claim(&mut client, 0x22, 0x24, 0x72, 0, at(1)).await;
        let tx = client.transaction().await.expect("prepare tx");
        let prepared = prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        tx.commit().await.expect("prepare commit");
        let tx = client.transaction().await.expect("apply tx");
        set_execution_gate_in(&tx, "live/generation", GATE_REVISION + 1, true, at(3))
            .await
            .expect("new enabled revision");
        tx.commit().await.expect("apply commit");
        let tx = client.transaction().await.expect("fence tx");
        let fenced = Err(ExecutionError::ConfigFenced {
            current_revision: GATE_REVISION + 1,
        });
        assert_eq!(
            mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(4))).await,
            fenced
        );
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x22, 0x24, 0x82, 0x72, 1, hosted_mode(0x82), at(4))
            )
            .await,
            Err(ExecutionError::ConfigFenced {
                current_revision: GATE_REVISION + 1
            })
        );
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(4))
            )
            .await
            .expect("exact replay"),
            prepared
        );
        assert_eq!(
            set_execution_gate_in(&tx, "live/generation", GATE_REVISION, true, at(5)).await,
            Err(ExecutionError::ConfigFenced {
                current_revision: GATE_REVISION + 1
            })
        );
        assert_eq!(
            set_execution_gate_in(&tx, "live/generation", GATE_REVISION + 1, false, at(5)).await,
            Err(ExecutionError::ConfigFenced {
                current_revision: GATE_REVISION + 1
            })
        );
        tx.commit().await.expect("read commit");
        assert_eq!(dispatch_count(&client, 0x22, 0x24).await, 0);
        scratch.finish().await;
    });
}

#[test]
fn retained_replay_and_unknown_recovery_survive_deadline_retirement_and_disable() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        let tx = client.transaction().await.expect("send tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare");
        mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
            .await
            .expect("sent");
        complete_dispatch_in(
            &tx,
            &finish_command(0x11, 0x21, 0x81, 0x71, 1, CompletionOutcome::Unknown, at(4)),
        )
        .await
        .expect("unknown");
        tx.commit().await.expect("send commit");
        client
            .execute(
                "UPDATE imagery.image_job SET state = 'retired' WHERE job_id = $1",
                &[&id(0x11).as_slice()],
            )
            .await
            .expect("retire");
        let tx = client.transaction().await.expect("disable tx");
        set_execution_gate_in(&tx, "live/generation", GATE_REVISION + 1, false, at(60_000))
            .await
            .expect("disable");
        tx.commit().await.expect("disable commit");
        let tx = client.transaction().await.expect("recovery tx");
        assert!(matches!(
            enqueue_stage_in(&tx, &enqueue_command(0x11, 0x21, at(60_001))).await,
            Ok(EnqueueOutcome::Existing(_))
        ));
        let historical_claim = claim_stage_in(
            &tx,
            &claim_command(0x11, 0x21, GATE_REVISION, 0x71, 1, at(60_001)),
        )
        .await
        .expect("claim history");
        assert_eq!(historical_claim.claim_generation, 1);
        assert_eq!(historical_claim.lease_until_ms, at(1) + LEASE_MS);
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(60_001))
            )
            .await
            .expect("prepare history")
            .state,
            DispatchState::Unknown
        );
        let completion = finish_command(
            0x11,
            0x21,
            0x81,
            0x71,
            1,
            CompletionOutcome::Succeeded {
                usage_revision: 2,
                checkpoint: success_checkpoint(0x81, 2),
            },
            at(60_001),
        );
        let recovered = complete_dispatch_in(&tx, &completion)
            .await
            .expect("late result");
        assert_eq!(recovered.state, DispatchState::Succeeded);
        assert_eq!(recovered.settlement, DispatchSettlement::UnknownOutstanding);
        assert_eq!(
            complete_dispatch_in(&tx, &completion)
                .await
                .expect("result replay"),
            recovered
        );
        let checkpoint = df_persistence::imagery_execution::read_output_checkpoint_in(
            &tx,
            &owner(),
            &job_id(0x11),
            &stage_id(0x21),
            &dispatch_id(0x81),
        )
        .await
        .expect("hosted checkpoint")
        .expect("retained");
        assert_eq!(checkpoint.object_version, "etag-3");
        tx.commit().await.expect("recovery commit");
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn releasing_an_unclaimed_stage_returns_its_pending_bytes_exactly_once() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        let tx = client.transaction().await.expect("release tx");
        let command = ReleaseCommand::new(
            owner(),
            job_id(0x11),
            stage_id(0x21),
            ClaimToken::from_bytes(&id(0x21)).expect("initial token"),
            0,
            at(60_000),
        )
        .expect("release");
        let released = release_stage_in(&tx, &command)
            .await
            .expect("release pending");
        assert_eq!(released.state, StageState::Released);
        assert_eq!(
            release_stage_in(&tx, &command)
                .await
                .expect("release replay"),
            released
        );
        tx.commit().await.expect("release commit");
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        scratch.finish().await;
    });
}

#[test]
fn retained_terminal_stages_do_not_starve_the_first_runnable_queue_page() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        for offset in 0..4u8 {
            let job = 0x30 + offset;
            let stage = 0x40 + offset;
            let dispatch = 0x80 + offset;
            let token = 0x70 + offset;
            accept_and_enqueue(&mut client, 0x10 + offset, job, 0x50 + offset, stage, at(0)).await;
            claim(&mut client, job, stage, token, 0, at(1)).await;
            let tx = client.transaction().await.expect("terminal tx");
            prepare_dispatch_in(
                &tx,
                &prepare_command(job, stage, dispatch, token, 1, hosted_mode(dispatch), at(2)),
            )
            .await
            .expect("prepare");
            mark_sent_in(
                &tx,
                &mark_sent_command(job, stage, dispatch, token, 1, at(3)),
            )
            .await
            .expect("sent");
            complete_dispatch_in(
                &tx,
                &finish_command(
                    job,
                    stage,
                    dispatch,
                    token,
                    1,
                    CompletionOutcome::Failed {
                        cause: FailureCause::Refused,
                    },
                    at(4),
                ),
            )
            .await
            .expect("terminal");
            tx.commit().await.expect("terminal commit");
        }
        accept_and_enqueue(&mut client, 0x20, 0x61, 0x62, 0x63, at(20_000)).await;
        let tx = client.transaction().await.expect("scan tx");
        let queue =
            df_persistence::imagery_execution::scan_queue_in(&tx, "live/generation", 1, at(20_000))
                .await
                .expect("runnable queue");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].job, job_id(0x61));
        tx.commit().await.expect("scan commit");
        scratch.finish().await;
    });
}

#[test]
fn global_dispatch_and_finance_collisions_preserve_the_other_job_and_transaction() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        accept_and_enqueue(&mut client, 0x20, 0x22, 0x23, 0x24, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        claim(&mut client, 0x22, 0x24, 0x72, 0, at(1)).await;
        let tx = client.transaction().await.expect("prepare tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("first prepare");
        tx.commit().await.expect("prepare commit");
        let tx = client.transaction().await.expect("collision tx");
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x22, 0x24, 0x81, 0x72, 1, hosted_mode(0x81), at(3))
            )
            .await,
            Err(ExecutionError::NotFound)
        );
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(0x22, 0x24, 0x82, 0x72, 1, hosted_mode(0x81), at(3))
            )
            .await,
            Err(ExecutionError::IdentityConflict)
        );
        let live: i32 = tx
            .query_one("SELECT 1::integer", &[])
            .await
            .expect("not aborted")
            .get(0);
        assert_eq!(live, 1);
        tx.rollback().await.expect("collision rollback");
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        assert_eq!(dispatch_count(&client, 0x22, 0x24).await, 0);
        assert_eq!(capacity_counts(&client, "live/generation").await, (2, 0, 0));
        scratch.finish().await;
    });
}

fn recovery_command(
    job: u8,
    stage: u8,
    dispatch: u8,
    token: u8,
    now_ms: i64,
) -> df_imagery::execution::ClaimRecoveryCommand {
    df_imagery::execution::ClaimRecoveryCommand {
        owner: owner(),
        job: job_id(job),
        stage: stage_id(stage),
        dispatch: dispatch_id(dispatch),
        claim: ClaimToken::from_bytes(&id(token)).expect("recovery token"),
        lease_ms: 4_000,
        now_ms,
    }
}

async fn retain_sent_stage(client: &mut Client, index: u8, outcome: Option<CompletionOutcome>) {
    let job = 0x30 + index;
    let stage = 0x40 + index;
    let dispatch = 0x80 + index;
    let token = 0x70 + index;
    accept_and_enqueue(client, 0x10 + index, job, 0x50 + index, stage, at(0)).await;
    claim(client, job, stage, token, 0, at(1)).await;
    let tx = client.transaction().await.expect("retained tx");
    prepare_dispatch_in(
        &tx,
        &prepare_command(job, stage, dispatch, token, 1, hosted_mode(dispatch), at(2)),
    )
    .await
    .expect("retained prepare");
    mark_sent_in(
        &tx,
        &mark_sent_command(job, stage, dispatch, token, 1, at(3)),
    )
    .await
    .expect("retained sent");
    if let Some(outcome) = outcome {
        complete_dispatch_in(
            &tx,
            &finish_command(job, stage, dispatch, token, 1, outcome, at(4)),
        )
        .await
        .expect("retained outcome");
    }
    tx.commit().await.expect("retained commit");
}

#[test]
fn recovery_discovers_sent_unknown_and_unpublished_success_after_retirement_with_pagination() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::{claim_recovery_in, scan_recovery_in};
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        retain_sent_stage(&mut client, 0, None).await;
        retain_sent_stage(&mut client, 1, Some(CompletionOutcome::Unknown)).await;
        retain_sent_stage(
            &mut client,
            2,
            Some(CompletionOutcome::Succeeded {
                usage_revision: 2,
                checkpoint: success_checkpoint(0x82, 2),
            }),
        )
        .await;
        let tx = client.transaction().await.expect("supplier checkpoint tx");
        record_supplier_checkpoint_in(&tx, &supplier_checkpoint(0x31, 0x41, 0x81, 1, at(5)))
            .await
            .expect("supplier resume checkpoint");
        tx.commit().await.expect("supplier checkpoint commit");
        drop(client);
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.expect("active scan");
        assert!(
            scan_recovery_in(&tx, "live/generation", None, 10, at(5))
                .await
                .expect("scan")
                .is_empty()
        );
        tx.commit().await.expect("active commit");
        client
            .execute("UPDATE imagery.image_job SET state='retired'", &[])
            .await
            .expect("retire");
        let tx = client.transaction().await.expect("disabled scan");
        set_execution_gate_in(&tx, "live/generation", GATE_REVISION + 1, false, at(60_000))
            .await
            .expect("disable");
        let first = scan_recovery_in(&tx, "live/generation", None, 2, at(60_000))
            .await
            .expect("first page");
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].current.state, DispatchState::Sent);
        assert_eq!(first[1].current.state, DispatchState::Unknown);
        let next = scan_recovery_in(
            &tx,
            "live/generation",
            Some(first[1].current.dispatch),
            2,
            at(60_000),
        )
        .await
        .expect("second page");
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].current.state, DispatchState::Succeeded);
        assert_eq!(next[0].claim_generation, 1);
        assert_eq!(
            next[0].claim,
            ClaimToken::from_bytes(&id(0x72)).expect("original claim")
        );
        tx.commit().await.expect("scan commit");
        let tx = client.transaction().await.expect("recovery tx");
        let lease = claim_recovery_in(&tx, &recovery_command(0x31, 0x41, 0x81, 0x91, at(60_001)))
            .await
            .expect("post deadline recovery");
        assert!(lease.lease_until_ms > DEADLINE_MS);
        let supplier = read_supplier_checkpoint_in(
            &tx,
            &lease.stage.owner,
            &lease.stage.job,
            &lease.stage.stage,
            &lease.stage.current.dispatch,
        )
        .await
        .expect("resume read")
        .expect("retained resume");
        assert_eq!(supplier, supplier_checkpoint(0x31, 0x41, 0x81, 1, at(5)));

        let result = complete_dispatch_in(
            &tx,
            &finish_command(
                0x31,
                0x41,
                0x81,
                0x71,
                1,
                CompletionOutcome::Succeeded {
                    usage_revision: 2,
                    checkpoint: success_checkpoint(0x81, 2),
                },
                at(60_002),
            ),
        )
        .await
        .expect("late original receipt");
        assert_eq!(result.state, DispatchState::Succeeded);
        assert_eq!(result.settlement, DispatchSettlement::UnknownOutstanding);
        tx.commit().await.expect("recovery commit");
        assert_eq!(dispatch_count(&client, 0x31, 0x41).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn recovery_lease_never_replaces_a_stage_claim_or_grants_fresh_send_authority() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::{claim_recovery_in, validate_recovery_in};
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        retain_sent_stage(&mut client, 0, Some(CompletionOutcome::Unknown)).await;
        let tx = client.transaction().await.expect("early tx");
        assert_eq!(
            claim_recovery_in(&tx, &recovery_command(0x30, 0x40, 0x80, 0x90, at(5))).await,
            Err(ExecutionError::LostClaim)
        );
        tx.rollback().await.expect("early rollback");
        let tx = client.transaction().await.expect("recovery tx");
        let mut wrong_owner = recovery_command(0x30, 0x40, 0x80, 0x90, at(11_000));
        wrong_owner.owner = draft_owner();
        assert_eq!(
            claim_recovery_in(&tx, &wrong_owner).await,
            Err(ExecutionError::NotFound)
        );
        let lease = claim_recovery_in(&tx, &recovery_command(0x30, 0x40, 0x80, 0x90, at(11_000)))
            .await
            .expect("claim recovery");
        validate_recovery_in(&tx, &lease, at(11_001))
            .await
            .expect("exclusive recovery");
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &prepare_command(
                    0x30,
                    0x40,
                    0x85,
                    0x90,
                    lease.generation,
                    hosted_mode(0x85),
                    at(11_001)
                )
            )
            .await,
            Err(ExecutionError::LostClaim)
        );
        assert_eq!(
            mark_sent_in(
                &tx,
                &mark_sent_command(0x30, 0x40, 0x80, 0x90, lease.generation, at(11_001))
            )
            .await,
            Err(ExecutionError::LostClaim)
        );
        assert_eq!(
            claim_stage_in(
                &tx,
                &claim_command(0x30, 0x40, GATE_REVISION, 0x75, 1, at(11_001))
            )
            .await,
            Err(ExecutionError::LostClaim)
        );
        let historical = read_stage_in(&tx, &owner(), &job_id(0x30), &stage_id(0x40))
            .await
            .expect("stage");
        assert_eq!(historical.claim_generation, 1);
        assert_eq!(historical.lease_until_ms, at(1) + LEASE_MS);
        tx.commit().await.expect("recovery commit");
        assert_eq!(dispatch_count(&client, 0x30, 0x40).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn concurrent_recovery_claims_have_one_winner_and_lost_ack_replay_does_not_renew() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::{claim_recovery_in, validate_recovery_in};
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        retain_sent_stage(&mut client, 0, None).await;
        let mut left = connect(&scratch.config()).await;
        let mut right = connect(&scratch.config()).await;
        let (left_result, right_result) = tokio::join!(
            async {
                let tx = left.transaction().await.expect("left tx");
                let result =
                    claim_recovery_in(&tx, &recovery_command(0x30, 0x40, 0x80, 0x90, at(11_000)))
                        .await;
                tx.commit().await.expect("left commit");
                result
            },
            async {
                let tx = right.transaction().await.expect("right tx");
                let result =
                    claim_recovery_in(&tx, &recovery_command(0x30, 0x40, 0x80, 0x91, at(11_000)))
                        .await;
                tx.commit().await.expect("right commit");
                result
            }
        );
        let winner = match (left_result, right_result) {
            (Ok(winner), Err(ExecutionError::LostClaim))
            | (Err(ExecutionError::LostClaim), Ok(winner)) => winner,
            result => panic!("one recovery winner required: {result:?}"),
        };
        let tx = client.transaction().await.expect("replay tx");
        let mut replay = recovery_command(0x30, 0x40, 0x80, 0x90, at(20_000));
        replay.claim = winner.token;
        let historical = claim_recovery_in(&tx, &replay)
            .await
            .expect("lost ack replay");
        assert_eq!(historical.generation, winner.generation);
        assert_eq!(historical.lease_until_ms, winner.lease_until_ms);
        assert_eq!(
            validate_recovery_in(&tx, &winner, at(20_000)).await,
            Err(ExecutionError::LostClaim)
        );
        let new = claim_recovery_in(&tx, &recovery_command(0x30, 0x40, 0x80, 0x92, at(20_000)))
            .await
            .expect("fresh recovery lease");
        assert_eq!(new.generation, winner.generation + 1);
        tx.commit().await.expect("replay commit");
        scratch.finish().await;
    });
}

#[test]
fn recovery_acknowledgment_and_due_retry_prevent_terminal_and_unknown_busy_loops() {
    runtime().block_on(async {
        use df_imagery::execution::{FinishRecoveryCommand, RecoveryDisposition};
        use df_persistence::imagery_execution::{
            claim_recovery_in, finish_recovery_in, scan_recovery_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        for index in 0..3u8 {
            retain_sent_stage(
                &mut client,
                index,
                Some(CompletionOutcome::Failed {
                    cause: FailureCause::Refused,
                }),
            )
            .await;
            let tx = client.transaction().await.expect("settle tx");
            mirror_settlement_in(
                &tx,
                &owner(),
                &job_id(0x30 + index),
                &stage_id(0x40 + index),
                &dispatch_id(0x80 + index),
                DispatchSettlement::UnusableSettled,
                at(5),
            )
            .await
            .expect("settle");
            tx.commit().await.expect("settle commit");
        }
        retain_sent_stage(&mut client, 3, Some(CompletionOutcome::Unknown)).await;
        let tx = client.transaction().await.expect("ack tx");
        for index in 0..3u8 {
            let lease = claim_recovery_in(
                &tx,
                &recovery_command(
                    0x30 + index,
                    0x40 + index,
                    0x80 + index,
                    0x90 + index,
                    at(11_000),
                ),
            )
            .await
            .expect("terminal recovery");
            let command = FinishRecoveryCommand {
                owner: owner(),
                job: lease.stage.job,
                stage: lease.stage.stage,
                dispatch: lease.stage.current.dispatch,
                claim: lease.token,
                claim_generation: lease.generation,
                disposition: RecoveryDisposition::Completed,
                now_ms: at(11_001),
            };
            finish_recovery_in(&tx, &command)
                .await
                .expect("terminal ack");
            finish_recovery_in(&tx, &command)
                .await
                .expect("lost ack replay");
        }
        let page = scan_recovery_in(&tx, "live/generation", None, 1, at(11_001))
            .await
            .expect("after ack");
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].current.dispatch, dispatch_id(0x83));
        let lease = claim_recovery_in(&tx, &recovery_command(0x33, 0x43, 0x83, 0x93, at(11_001)))
            .await
            .expect("unknown recovery");
        let mut command = FinishRecoveryCommand {
            owner: owner(),
            job: lease.stage.job,
            stage: lease.stage.stage,
            dispatch: lease.stage.current.dispatch,
            claim: lease.token,
            claim_generation: lease.generation,
            disposition: RecoveryDisposition::Completed,
            now_ms: at(11_002),
        };
        assert_eq!(
            finish_recovery_in(&tx, &command).await,
            Err(ExecutionError::OperationConflict)
        );
        command.disposition = RecoveryDisposition::RetryAt {
            next_at_ms: at(20_000),
        };
        finish_recovery_in(&tx, &command)
            .await
            .expect("defer unknown");
        assert!(
            scan_recovery_in(&tx, "live/generation", None, 1, at(19_999))
                .await
                .expect("not due")
                .is_empty()
        );
        command.now_ms = at(20_001);
        finish_recovery_in(&tx, &command)
            .await
            .expect("deferred receipt replays after due time");
        assert_eq!(
            scan_recovery_in(&tx, "live/generation", None, 1, at(20_000))
                .await
                .expect("due")
                .len(),
            1
        );
        tx.commit().await.expect("ack commit");
        scratch.finish().await;
    });
}

#[test]
fn unpublished_success_cannot_ack_recovery_and_cancelled_unsent_work_is_never_recovered() {
    runtime().block_on(async {
        use df_imagery::execution::{FinishRecoveryCommand, RecoveryDisposition};
        use df_persistence::imagery_execution::{
            claim_recovery_in, finish_recovery_in, scan_recovery_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        retain_sent_stage(
            &mut client,
            0,
            Some(CompletionOutcome::Succeeded {
                usage_revision: 2,
                checkpoint: success_checkpoint(0x80, 2),
            }),
        )
        .await;
        let tx = client.transaction().await.expect("success tx");
        mirror_settlement_in(
            &tx,
            &owner(),
            &job_id(0x30),
            &stage_id(0x40),
            &dispatch_id(0x80),
            DispatchSettlement::Settled,
            at(5),
        )
        .await
        .expect("settle");
        let lease = claim_recovery_in(&tx, &recovery_command(0x30, 0x40, 0x80, 0x90, at(11_000)))
            .await
            .expect("unpublished recovery");
        assert_eq!(
            finish_recovery_in(
                &tx,
                &FinishRecoveryCommand {
                    owner: owner(),
                    job: lease.stage.job,
                    stage: lease.stage.stage,
                    dispatch: lease.stage.current.dispatch,
                    claim: lease.token,
                    claim_generation: lease.generation,
                    disposition: RecoveryDisposition::Completed,
                    now_ms: at(11_001)
                }
            )
            .await,
            Err(ExecutionError::OperationConflict)
        );
        tx.commit().await.expect("success commit");
        accept_and_enqueue(&mut client, 0x20, 0x31, 0x51, 0x41, at(0)).await;
        claim(&mut client, 0x31, 0x41, 0x71, 0, at(1)).await;
        let tx = client.transaction().await.expect("cancel tx");
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x31, 0x41, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepare unsent");
        cancel_dispatch_in(
            &tx,
            &CancelCommand::new(
                owner(),
                job_id(0x31),
                stage_id(0x41),
                dispatch_id(0x81),
                ClaimToken::from_bytes(&id(0x71)).expect("claim"),
                1,
                at(3),
            )
            .expect("cancel command"),
        )
        .await
        .expect("cancel before send");
        set_execution_gate_in(&tx, "live/generation", GATE_REVISION + 1, false, at(4))
            .await
            .expect("disable");
        let due = scan_recovery_in(&tx, "live/generation", None, 10, at(20_000))
            .await
            .expect("scan");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].current.dispatch, dispatch_id(0x80));
        assert_eq!(
            claim_recovery_in(&tx, &recovery_command(0x31, 0x41, 0x81, 0x91, at(20_000))).await,
            Err(ExecutionError::Superseded)
        );
        tx.commit().await.expect("cancel commit");
        scratch.finish().await;
    });
}

#[test]
fn recovery_ownership_cannot_follow_a_cursor_to_a_new_explicit_dispatch() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::{claim_recovery_in, validate_recovery_in};
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        retain_sent_stage(&mut client, 0, Some(CompletionOutcome::Unknown)).await;
        let tx = client.transaction().await.expect("recovery tx");
        let lease = claim_recovery_in(&tx, &recovery_command(0x30, 0x40, 0x80, 0x90, at(11_000)))
            .await
            .expect("claim recovery");
        complete_dispatch_in(
            &tx,
            &finish_command(
                0x30,
                0x40,
                0x80,
                0x70,
                1,
                CompletionOutcome::Failed {
                    cause: FailureCause::Refused,
                },
                at(11_001),
            ),
        )
        .await
        .expect("proven original failure");
        mirror_settlement_in(
            &tx,
            &owner(),
            &job_id(0x30),
            &stage_id(0x40),
            &dispatch_id(0x80),
            DispatchSettlement::UnusableSettled,
            at(11_002),
        )
        .await
        .expect("original settlement");
        let mut acknowledged = df_imagery::execution::FinishRecoveryCommand {
            owner: owner(),
            job: job_id(0x30),
            stage: stage_id(0x40),
            dispatch: dispatch_id(0x80),
            claim: lease.token,
            claim_generation: lease.generation,
            disposition: df_imagery::execution::RecoveryDisposition::Completed,
            now_ms: at(11_003),
        };
        df_persistence::imagery_execution::finish_recovery_in(&tx, &acknowledged)
            .await
            .expect("original ack");
        tx.commit().await.expect("recovery commit");
        let tx = client.transaction().await.expect("explicit retry tx");
        let claimed = claim_stage_in(
            &tx,
            &claim_command(0x30, 0x40, GATE_REVISION, 0x75, 1, at(20_000)),
        )
        .await
        .expect("fresh normal claim after recovery expires");
        assert_eq!(claimed.claim_generation, 2);
        prepare_dispatch_in(
            &tx,
            &prepare_command(0x30, 0x40, 0x85, 0x75, 2, hosted_mode(0x85), at(20_001)),
        )
        .await
        .expect("new explicit attempt with fresh finance identity");
        assert_eq!(
            validate_recovery_in(&tx, &lease, at(20_002)).await,
            Err(ExecutionError::Superseded)
        );
        acknowledged.now_ms = at(20_003);
        df_persistence::imagery_execution::finish_recovery_in(&tx, &acknowledged)
            .await
            .expect("lost original ack replays after cursor advances");
        tx.commit().await.expect("explicit retry commit");
        scratch.finish().await;
    });
}

async fn prepare_decision_stage(client: &mut Client, kind: StageKind) {
    let mut enqueue = enqueue_command(0x11, 0x21, at(0));
    enqueue.kind = kind;
    let tx = client.transaction().await.expect("decision enqueue tx");
    accept_with_execution_in(
        &tx,
        &accept_command(0x10, 0x11, 0x12, DEADLINE_MS, at(0)),
        &enqueue,
    )
    .await
    .expect("decision enqueue");
    tx.commit().await.expect("decision enqueue commit");
    claim(client, 0x11, 0x21, 0x71, 0, at(1)).await;
    let tx = client.transaction().await.expect("decision sent tx");
    prepare_dispatch_in(
        &tx,
        &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
    )
    .await
    .expect("decision prepare");
    mark_sent_in(&tx, &mark_sent_command(0x11, 0x21, 0x81, 0x71, 1, at(3)))
        .await
        .expect("decision sent");
    tx.commit().await.expect("decision sent commit");
}

fn observed_decision(
    basis: df_imagery::execution::StageBasis,
    payload: Vec<u8>,
    version: u32,
) -> df_imagery::execution::DecisionObservation {
    use df_imagery::execution::{
        DecisionBasis, DecisionKind, DecisionObservation, DecisionReceipt, DecisionScope,
    };
    let kind = match basis.kind {
        StageKind::Classification => DecisionKind::Classification,
        StageKind::Inspection => DecisionKind::Inspection,
        _ => panic!("a real decision stage is required"),
    };
    DecisionObservation::new(
        DecisionScope {
            owner: owner(),
            job: job_id(0x11),
            stage: stage_id(0x21),
            dispatch: dispatch_id(0x81),
        },
        kind,
        version,
        DecisionBasis {
            schema_revision: 2,
            validator_revision: 3,
            stage: basis,
        },
        payload,
        DecisionReceipt {
            usage_revision: 1,
            observed_at_ms: at(4),
        },
        1 << 20,
    )
    .expect("decision observation")
}

async fn record_decision_usage(
    config: &Config,
    observation: &df_imagery::execution::DecisionObservation,
) {
    use df_imagery::cost::{CostRoute, CostUnits, UsageEvidence, UsageRevision};
    use df_imagery::runtime::{ImageUsageJournal, RuntimeUsage};
    let usage = df_persistence::imagery_runtime::PostgresImageUsage::open_existing(
        config.clone(),
        PostgresTransport::PlaintextLoopbackOnly,
        2,
    )
    .expect("raw usage journal");
    usage
        .record_usage(&RuntimeUsage {
            owner: observation.scope.owner.clone(),
            job: observation.scope.job,
            stage: observation.scope.stage,
            dispatch: observation.scope.dispatch,
            revision: UsageRevision(u64::from(observation.receipt.usage_revision)),
            route: CostRoute {
                provider: "fixture".to_owned(),
                model: "fixture-decision".to_owned(),
                cause: "imagery".to_owned(),
                detail: "decision".to_owned(),
                maxima: vec![CostUnits {
                    category: "input_tokens".to_owned(),
                    quantity: 100,
                }],
            },
            evidence: UsageEvidence::Reported,
            units: vec![CostUnits {
                category: "input_tokens".to_owned(),
                quantity: 10,
            }],
            usable: true,
            at_ms: u64::try_from(observation.receipt.observed_at_ms).expect("observation time"),
            raw_output: Some(observation.payload().to_vec()),
        })
        .await
        .expect("durable raw usage");
}

#[test]
fn decision_raw_observation_precedes_checkpoint_and_terminal_replay_compares_exact_version() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::{
            read_decision_checkpoint_in, read_decision_observation_in, read_output_checkpoint_in,
            read_stage_basis_in, record_decision_observation_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        prepare_decision_stage(&mut client, StageKind::Classification).await;
        let tx = client.transaction().await.expect("ordering tx");
        let basis = read_stage_basis_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .expect("basis");
        let observation =
            observed_decision(basis, br#"{"class":"private-fixture-class"}"#.to_vec(), 1);
        let command = finish_command(
            0x11,
            0x21,
            0x81,
            0x71,
            1,
            CompletionOutcome::Decision {
                observation_version: 1,
                usage_revision: 1,
            },
            at(5),
        );
        assert_eq!(
            complete_dispatch_in(&tx, &command).await,
            Err(ExecutionError::ObservationMissing)
        );
        assert_eq!(
            record_decision_observation_in(&tx, &observation).await,
            Err(ExecutionError::ObservationMissing)
        );
        tx.rollback().await.expect("ordering rollback");
        record_decision_usage(&scratch.config(), &observation).await;
        let tx = client.transaction().await.expect("decision tx");
        assert_eq!(
            record_decision_observation_in(&tx, &observation)
                .await
                .expect("observe"),
            observation
        );
        assert_eq!(
            record_decision_observation_in(&tx, &observation)
                .await
                .expect("observe replay"),
            observation
        );
        assert!(
            read_decision_checkpoint_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await
            .expect("before selection")
            .is_none()
        );
        assert!(
            df_persistence::imagery_execution::read_decision_reference_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await
            .expect("unselected metadata")
            .is_none()
        );
        let mut wrong_basis = observation.basis.stage.clone();
        wrong_basis.input_digest = [0x98; 32];
        assert_eq!(
            record_decision_observation_in(&tx, &observed_decision(wrong_basis, b"{}".to_vec(), 2))
                .await,
            Err(ExecutionError::BasisConflict)
        );
        assert_eq!(
            record_decision_observation_in(
                &tx,
                &observed_decision(observation.basis.stage.clone(), b"{}".to_vec(), 1)
            )
            .await,
            Err(ExecutionError::OperationConflict)
        );
        let done = complete_dispatch_in(&tx, &command)
            .await
            .expect("decision completes");
        assert_eq!(done.state, DispatchState::Succeeded);
        assert_eq!(done.finance, hosted_mode(0x81).finance());
        assert_eq!(
            complete_dispatch_in(&tx, &command)
                .await
                .expect("terminal replay"),
            done
        );
        let reference = df_persistence::imagery_execution::read_decision_reference_in(
            &tx,
            &owner(),
            &job_id(0x11),
            &stage_id(0x21),
            &dispatch_id(0x81),
        )
        .await
        .expect("cheap selected metadata")
        .expect("selected reference");
        assert_eq!(reference.scope, observation.scope);
        assert_eq!(reference.basis, observation.basis);
        assert_eq!(reference.observation_version, 1);
        assert_eq!(reference.receipt, observation.receipt);
        assert_eq!(reference.payload_sha256, *observation.payload_sha256());
        assert_eq!(
            df_persistence::imagery_execution::read_decision_reference_in(
                &tx,
                &draft_owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await,
            Err(ExecutionError::NotFound)
        );
        let second = observed_decision(observation.basis.stage.clone(), b"{}".to_vec(), 2);
        record_decision_observation_in(&tx, &second)
            .await
            .expect("later observed evidence");
        assert_eq!(
            complete_dispatch_in(
                &tx,
                &finish_command(
                    0x11,
                    0x21,
                    0x81,
                    0x71,
                    1,
                    CompletionOutcome::Decision {
                        observation_version: 2,
                        usage_revision: 1
                    },
                    at(6)
                )
            )
            .await,
            Err(ExecutionError::OperationConflict)
        );
        assert_eq!(
            read_decision_checkpoint_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await
            .expect("winning read"),
            Some(observation.clone())
        );
        assert_eq!(
            read_decision_observation_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81),
                None
            )
            .await
            .expect("latest read"),
            Some(second)
        );
        assert!(
            read_output_checkpoint_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await
            .expect("no image checkpoint")
            .is_none()
        );
        assert_eq!(
            read_decision_checkpoint_in(
                &tx,
                &draft_owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await,
            Err(ExecutionError::NotFound)
        );
        let assets: i64 = tx
            .query_one("SELECT count(*) FROM media.asset", &[])
            .await
            .expect("asset count")
            .get(0);
        assert_eq!(assets, 0);
        tx.commit().await.expect("decision commit");
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn inspection_unknown_recovery_selects_same_decision_and_acknowledges_without_a_catalog_asset() {
    runtime().block_on(async {
        use df_imagery::execution::{FinishRecoveryCommand, RecoveryDisposition};
        use df_persistence::imagery_execution::{
            claim_recovery_in, finish_recovery_in, read_decision_checkpoint_in,
            read_stage_basis_in, record_decision_observation_in, scan_recovery_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        prepare_decision_stage(&mut client, StageKind::Inspection).await;
        let tx = client.transaction().await.expect("unknown tx");
        complete_dispatch_in(
            &tx,
            &finish_command(0x11, 0x21, 0x81, 0x71, 1, CompletionOutcome::Unknown, at(4)),
        )
        .await
        .expect("unknown");
        let basis = read_stage_basis_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .expect("basis");
        tx.commit().await.expect("unknown commit");
        let observation = observed_decision(
            basis,
            br#"{"acceptance":"private-fixture-result"}"#.to_vec(),
            1,
        );
        record_decision_usage(&scratch.config(), &observation).await;
        client
            .execute(
                "UPDATE imagery.image_job SET state='retired' WHERE job_id=$1",
                &[&id(0x11).as_slice()],
            )
            .await
            .expect("retire");
        let tx = client.transaction().await.expect("recovery tx");
        set_execution_gate_in(&tx, "live/generation", GATE_REVISION + 1, false, at(60_000))
            .await
            .expect("disable");
        let lease = claim_recovery_in(&tx, &recovery_command(0x11, 0x21, 0x81, 0x91, at(60_001)))
            .await
            .expect("recovery lease");
        record_decision_observation_in(&tx, &observation)
            .await
            .expect("observed after deadline");
        let result = complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::Decision {
                    observation_version: 1,
                    usage_revision: 1,
                },
                at(60_002),
            ),
        )
        .await
        .expect("correlated decision recovery");
        assert_eq!(result.state, DispatchState::Succeeded);
        assert_eq!(result.settlement, DispatchSettlement::UnknownOutstanding);
        mirror_settlement_in(
            &tx,
            &owner(),
            &job_id(0x11),
            &stage_id(0x21),
            &dispatch_id(0x81),
            DispatchSettlement::Settled,
            at(60_003),
        )
        .await
        .expect("finance-owned settlement mirror");
        finish_recovery_in(
            &tx,
            &FinishRecoveryCommand {
                owner: owner(),
                job: job_id(0x11),
                stage: stage_id(0x21),
                dispatch: dispatch_id(0x81),
                claim: lease.token,
                claim_generation: lease.generation,
                disposition: RecoveryDisposition::Completed,
                now_ms: at(60_004),
            },
        )
        .await
        .expect("decision acknowledgment");
        assert_eq!(
            read_decision_checkpoint_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await
            .expect("recovered decision"),
            Some(observation)
        );
        assert!(
            scan_recovery_in(&tx, "live/generation", None, 10, at(60_005))
                .await
                .expect("acked scan")
                .is_empty()
        );
        let assets: i64 = tx
            .query_one("SELECT count(*) FROM media.asset", &[])
            .await
            .expect("asset count")
            .get(0);
        assert_eq!(assets, 0);
        tx.commit().await.expect("recovery commit");
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        scratch.finish().await;
    });
}

#[test]
fn decision_host_payload_policy_is_configurable_and_reduction_preserves_exact_historical_replay() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::{
            read_stage_basis_in, record_decision_observation_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        prepare_decision_stage(&mut client, StageKind::Classification).await;
        let tx = client.transaction().await.expect("basis tx");
        let basis = read_stage_basis_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .expect("basis");
        tx.commit().await.expect("basis commit");
        let observation = observed_decision(basis, b"private-observed-decision".to_vec(), 1);
        record_decision_usage(&scratch.config(), &observation).await;
        let tx = client.transaction().await.expect("policy tx");
        let bound = CapacityBounds::new(4, 64, 1 << 20).expect("bounds");
        set_capacity_bounds_in(
            &tx,
            "live/generation",
            &bound.with_decision_payload_bytes(2).expect("small bound"),
            at(5),
        )
        .await
        .expect("apply small bound");
        assert_eq!(
            record_decision_observation_in(&tx, &observation).await,
            Err(ExecutionError::DecisionPayloadTooLarge { maximum_bytes: 2 })
        );
        set_capacity_bounds_in(
            &tx,
            "live/generation",
            &bound
                .with_decision_payload_bytes(1 << 20)
                .expect("raised bound"),
            at(6),
        )
        .await
        .expect("raise bound");
        record_decision_observation_in(&tx, &observation)
            .await
            .expect("configured larger observation");
        set_capacity_bounds_in(
            &tx,
            "live/generation",
            &bound.with_decision_payload_bytes(1).expect("reduced bound"),
            at(7),
        )
        .await
        .expect("reduce bound");
        assert_eq!(
            record_decision_observation_in(&tx, &observation)
                .await
                .expect("historical replay"),
            observation
        );
        let second = observed_decision(
            observation.basis.stage.clone(),
            observation.payload().to_vec(),
            2,
        );
        assert_eq!(
            record_decision_observation_in(&tx, &second).await,
            Err(ExecutionError::DecisionPayloadTooLarge { maximum_bytes: 1 })
        );
        tx.commit().await.expect("policy commit");
        scratch.finish().await;
    });
}

#[test]
fn concurrent_child_enqueue_replays_one_immutable_parent_dependency_without_duplicate_reserves() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::read_stage_basis_in;
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        retain_sent_stage(
            &mut client,
            0,
            Some(CompletionOutcome::Succeeded {
                usage_revision: 1,
                checkpoint: success_checkpoint(0x80, 1),
            }),
        )
        .await;
        let child = |candidate: u8| {
            let mut command = enqueue_command(0x30, candidate, at(5));
            command.kind = StageKind::Inspection;
            command.with_parent(dispatch_id(0x80))
        };
        let mut left = connect(&scratch.config()).await;
        let mut right = connect(&scratch.config()).await;
        let (a, b) = tokio::join!(
            async {
                let tx = left.transaction().await.expect("left child tx");
                let result = enqueue_stage_in(&tx, &child(0x41))
                    .await
                    .expect("left child");
                tx.commit().await.expect("left child commit");
                result
            },
            async {
                let tx = right.transaction().await.expect("right child tx");
                let result = enqueue_stage_in(&tx, &child(0x42))
                    .await
                    .expect("right child");
                tx.commit().await.expect("right child commit");
                result
            }
        );
        let (created, replayed) = match (a, b) {
            (EnqueueOutcome::Enqueued(created), EnqueueOutcome::Existing(replayed))
            | (EnqueueOutcome::Existing(replayed), EnqueueOutcome::Enqueued(created)) => {
                (created, replayed)
            }
            pair => panic!("one child creation required: {pair:?}"),
        };
        assert_eq!(created.stage, replayed.stage);
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (0, 1, 128)
        );
        let tx = client.transaction().await.expect("dependency tx");
        let basis = read_stage_basis_in(&tx, &owner(), &job_id(0x30), &created.stage)
            .await
            .expect("child basis");
        assert_eq!(basis.parent_dispatch, Some(dispatch_id(0x80)));
        let found = df_persistence::imagery_execution::read_child_stage_in(
            &tx,
            &owner(),
            &job_id(0x30),
            &dispatch_id(0x80),
            StageKind::Inspection,
        )
        .await
        .expect("canonical child lookup")
        .expect("child exists");
        assert_eq!(found.stage, created.stage);
        assert_eq!(
            df_persistence::imagery_execution::read_child_stage_in(
                &tx,
                &owner(),
                &job_id(0x30),
                &dispatch_id(0x80),
                StageKind::LocalTransform
            )
            .await
            .expect("absent other child"),
            None
        );
        assert_eq!(
            df_persistence::imagery_execution::read_child_stage_in(
                &tx,
                &draft_owner(),
                &job_id(0x30),
                &dispatch_id(0x80),
                StageKind::Inspection
            )
            .await,
            Err(ExecutionError::NotFound)
        );
        let mut conflicting = child(0x43);
        conflicting.input_digest = [0x93; 32];
        assert_eq!(
            enqueue_stage_in(&tx, &conflicting).await,
            Err(ExecutionError::BasisConflict)
        );
        tx.commit().await.expect("dependency commit");
        retain_sent_stage(&mut client, 1, Some(CompletionOutcome::Unknown)).await;
        let tx = client.transaction().await.expect("unknown parent tx");
        let mut unresolved = enqueue_command(0x31, 0x44, at(5));
        unresolved.kind = StageKind::Inspection;
        assert_eq!(
            enqueue_stage_in(&tx, &unresolved.with_parent(dispatch_id(0x81))).await,
            Err(ExecutionError::BasisConflict)
        );
        let mut cross_owner = child(0x45);
        cross_owner.job = job_id(0x31);
        assert_eq!(
            enqueue_stage_in(&tx, &cross_owner).await,
            Err(ExecutionError::BasisConflict)
        );
        tx.rollback().await.expect("dependency rollback");
        client
            .execute(
                "UPDATE imagery.image_job SET state='retired' WHERE job_id=$1",
                &[&id(0x30).as_slice()],
            )
            .await
            .expect("retire parent job");
        let tx = client.transaction().await.expect("child historical tx");
        let mut replay = child(0x46);
        replay.now_ms = at(60_000);
        assert!(
            matches!(enqueue_stage_in(&tx,&replay).await.expect("child historical replay"),
            EnqueueOutcome::Existing(view) if view.stage==created.stage)
        );
        tx.commit().await.expect("child historical commit");
        scratch.finish().await;
    });
}

fn candidate_png(changed: bool) -> Vec<u8> {
    if changed {
        vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 96, 248,
            207, 240, 31, 0, 4, 1, 1, 255, 113, 235, 71, 229, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ]
    } else {
        vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207,
            192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ]
    }
}
fn candidate_observation(
    basis: df_imagery::execution::StageBasis,
    bytes: &[u8],
    version: u32,
    usage_revision: u32,
) -> df_imagery::execution::CandidateObservation {
    use df_imagery::execution::{CandidateObservation, ExecutionScope, ObservationReceipt};
    CandidateObservation::inspect(
        ExecutionScope {
            owner: owner(),
            job: job_id(0x11),
            stage: stage_id(0x21),
            dispatch: dispatch_id(0x81),
        },
        basis,
        version,
        ObservationReceipt {
            usage_revision,
            observed_at_ms: at(4),
        },
        bytes,
        df_assets::ImageProofLimits::new(1024, 1, 16).expect("bounded proof"),
        df_assets::ImageIntent::new("image/png", 1, 1, df_assets::ImageAlpha::Opaque)
            .expect("intent"),
    )
    .expect("actual decoded candidate")
}
async fn record_raw_usage(
    config: &Config,
    scope: &df_imagery::execution::ExecutionScope,
    revision: u32,
    bytes: Option<Vec<u8>>,
    evidence: df_imagery::cost::UsageEvidence,
    usable: bool,
) {
    use df_imagery::cost::{CostRoute, CostUnits, UsageRevision};
    use df_imagery::runtime::{ImageUsageJournal, RuntimeUsage};
    let journal = df_persistence::imagery_runtime::PostgresImageUsage::open_existing(
        config.clone(),
        PostgresTransport::PlaintextLoopbackOnly,
        2,
    )
    .expect("usage journal");
    journal
        .record_usage(&RuntimeUsage {
            owner: scope.owner.clone(),
            job: scope.job,
            stage: scope.stage,
            dispatch: scope.dispatch,
            revision: UsageRevision(u64::from(revision)),
            route: CostRoute {
                provider: "fixture".to_owned(),
                model: "actual-candidate".to_owned(),
                cause: "imagery".to_owned(),
                detail: "candidate".to_owned(),
                maxima: vec![CostUnits {
                    category: "input_tokens".to_owned(),
                    quantity: 100,
                }],
            },
            evidence,
            units: vec![CostUnits {
                category: "input_tokens".to_owned(),
                quantity: if evidence == df_imagery::cost::UsageEvidence::Unknown {
                    100
                } else {
                    10
                },
            }],
            usable,
            at_ms: u64::try_from(at(4)).expect("time"),
            raw_output: bytes,
        })
        .await
        .expect("raw usage record");
}
async fn retain_candidate(
    client: &mut Client,
    config: &Config,
) -> df_imagery::execution::CandidateReceipt {
    use df_persistence::imagery_execution::{read_stage_basis_in, record_candidate_observation_in};
    prepare_decision_stage(client, StageKind::Generation).await;
    let tx = client.transaction().await.expect("basis");
    let basis = read_stage_basis_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
        .await
        .expect("basis");
    tx.commit().await.expect("basis commit");
    let bytes = candidate_png(false);
    let observation = candidate_observation(basis, &bytes, 1, 1);
    record_raw_usage(
        config,
        &observation.as_receipt().scope,
        1,
        Some(bytes),
        df_imagery::cost::UsageEvidence::Reported,
        true,
    )
    .await;
    let tx = client.transaction().await.expect("candidate");
    let receipt = record_candidate_observation_in(&tx, &observation)
        .await
        .expect("observation");
    complete_dispatch_in(
        &tx,
        &finish_command(
            0x11,
            0x21,
            0x81,
            0x71,
            1,
            CompletionOutcome::Candidate {
                observation_version: 1,
                usage_revision: 1,
            },
            at(5),
        ),
    )
    .await
    .expect("candidate complete");
    mirror_settlement_in(
        &tx,
        &owner(),
        &job_id(0x11),
        &stage_id(0x21),
        &dispatch_id(0x81),
        DispatchSettlement::Settled,
        at(6),
    )
    .await
    .expect("actual settlement mirror");
    tx.commit().await.expect("candidate commit");
    receipt
}

#[test]
fn candidate_completion_requires_exact_durable_bytes_and_keeps_winning_receipt_immutable() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::{
            read_candidate_checkpoint_in, read_candidate_observation_in, read_output_checkpoint_in,
            read_stage_basis_in, record_candidate_observation_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        prepare_decision_stage(&mut client, StageKind::Generation).await;
        let tx = client.transaction().await.expect("basis");
        let basis = read_stage_basis_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .expect("basis");
        tx.commit().await.expect("basis commit");
        let bytes = candidate_png(false);
        let candidate = candidate_observation(basis.clone(), &bytes, 1, 1);
        let tx = client.transaction().await.expect("missing raw");
        assert_eq!(
            record_candidate_observation_in(&tx, &candidate).await,
            Err(ExecutionError::ObservationMissing)
        );
        assert_eq!(
            complete_dispatch_in(
                &tx,
                &finish_command(
                    0x11,
                    0x21,
                    0x81,
                    0x71,
                    1,
                    CompletionOutcome::Candidate {
                        observation_version: 1,
                        usage_revision: 1
                    },
                    at(5)
                )
            )
            .await,
            Err(ExecutionError::ObservationMissing)
        );
        complete_dispatch_in(
            &tx,
            &finish_command(0x11, 0x21, 0x81, 0x71, 1, CompletionOutcome::Unknown, at(5)),
        )
        .await
        .expect("retain uncertain supplier liability");
        tx.commit().await.expect("unknown before raw observation");
        record_raw_usage(
            &scratch.config(),
            &candidate.as_receipt().scope,
            1,
            Some(candidate_png(true)),
            df_imagery::cost::UsageEvidence::Reported,
            true,
        )
        .await;
        let tx = client.transaction().await.expect("different bytes");
        assert_eq!(
            record_candidate_observation_in(&tx, &candidate).await,
            Err(ExecutionError::ObservationMissing)
        );
        tx.rollback().await.expect("rollback");
        let candidate = candidate_observation(basis.clone(), &bytes, 1, 2);
        record_raw_usage(
            &scratch.config(),
            &candidate.as_receipt().scope,
            2,
            Some(bytes.clone()),
            df_imagery::cost::UsageEvidence::Reported,
            true,
        )
        .await;
        let tx = client.transaction().await.expect("valid candidate");
        assert_eq!(
            record_candidate_observation_in(&tx, &candidate)
                .await
                .expect("record"),
            *candidate.as_receipt()
        );
        assert_eq!(
            record_candidate_observation_in(&tx, &candidate)
                .await
                .expect("lost ack"),
            *candidate.as_receipt()
        );
        let mut foreign = candidate.as_receipt().scope.clone();
        foreign.owner = draft_owner();
        let foreign = df_imagery::execution::CandidateObservation::inspect(
            foreign,
            basis.clone(),
            1,
            candidate.as_receipt().receipt,
            &bytes,
            df_assets::ImageProofLimits::default(),
            df_assets::ImageIntent::new("image/png", 1, 1, df_assets::ImageAlpha::Opaque)
                .expect("intent"),
        )
        .expect("proof");
        assert_eq!(
            record_candidate_observation_in(&tx, &foreign).await,
            Err(ExecutionError::NotFound)
        );
        let completed = complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::Candidate {
                    observation_version: 1,
                    usage_revision: 2,
                },
                at(5),
            ),
        )
        .await
        .expect("complete");
        assert_eq!(completed.usage_revision, 2);
        assert_eq!(completed.state, DispatchState::Succeeded);
        assert_eq!(completed.settlement, DispatchSettlement::UnknownOutstanding);
        assert_eq!(
            read_candidate_checkpoint_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await
            .expect("checkpoint"),
            Some(candidate.as_receipt().clone())
        );
        assert!(
            read_output_checkpoint_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81)
            )
            .await
            .expect("no object")
            .is_none()
        );
        tx.commit().await.expect("commit");
        let second = candidate_observation(basis, &bytes, 2, 2);
        let tx = client.transaction().await.expect("version conflict");
        record_candidate_observation_in(&tx, &second)
            .await
            .expect("later observation");
        assert_eq!(
            complete_dispatch_in(
                &tx,
                &finish_command(
                    0x11,
                    0x21,
                    0x81,
                    0x71,
                    1,
                    CompletionOutcome::Candidate {
                        observation_version: 2,
                        usage_revision: 2
                    },
                    at(6)
                )
            )
            .await,
            Err(ExecutionError::OperationConflict)
        );
        let changed = candidate_observation(
            second.as_receipt().basis.clone(),
            &candidate_png(true),
            1,
            2,
        );
        assert_eq!(
            record_candidate_observation_in(&tx, &changed).await,
            Err(ExecutionError::OperationConflict)
        );
        assert_eq!(
            read_candidate_observation_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81),
                None
            )
            .await
            .expect("latest"),
            Some(second.as_receipt().clone())
        );
        tx.commit().await.expect("commit");
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM media.asset", &[])
                .await
                .expect("assets")
                .get::<_, i64>(0),
            0
        );
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn definitive_unusable_completion_requires_known_scoped_usage_without_fabricated_image() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        prepare_decision_stage(&mut client, StageKind::Generation).await;
        let scope = df_imagery::execution::ExecutionScope {
            owner: owner(),
            job: job_id(0x11),
            stage: stage_id(0x21),
            dispatch: dispatch_id(0x81),
        };
        let command = finish_command(
            0x11,
            0x21,
            0x81,
            0x71,
            1,
            CompletionOutcome::Unusable {
                cause: FailureCause::Refused,
                usage_revision: 1,
            },
            at(5),
        );
        let tx = client.transaction().await.expect("missing usage");
        assert_eq!(
            complete_dispatch_in(&tx, &command).await,
            Err(ExecutionError::ObservationMissing)
        );
        tx.rollback().await.expect("rollback");
        record_raw_usage(
            &scratch.config(),
            &scope,
            1,
            None,
            df_imagery::cost::UsageEvidence::Unknown,
            false,
        )
        .await;
        let tx = client.transaction().await.expect("unknown usage");
        assert_eq!(
            complete_dispatch_in(&tx, &command).await,
            Err(ExecutionError::ObservationMissing)
        );
        complete_dispatch_in(
            &tx,
            &finish_command(0x11, 0x21, 0x81, 0x71, 1, CompletionOutcome::Unknown, at(6)),
        )
        .await
        .expect("unknown retained");
        tx.commit().await.expect("unknown commit");
        record_raw_usage(
            &scratch.config(),
            &scope,
            2,
            None,
            df_imagery::cost::UsageEvidence::Reported,
            false,
        )
        .await;
        let mut known = command;
        known.outcome = CompletionOutcome::Unusable {
            cause: FailureCause::Refused,
            usage_revision: 2,
        };
        known.now_ms = at(7);
        let tx = client.transaction().await.expect("known unusable");
        let failed = complete_dispatch_in(&tx, &known)
            .await
            .expect("known failure");
        assert_eq!(failed.state, DispatchState::Failed);
        assert_eq!(failed.usage_revision, 2);
        assert_eq!(failed.settlement, DispatchSettlement::UnknownOutstanding);
        assert_eq!(
            complete_dispatch_in(&tx, &known)
                .await
                .expect("exact replay"),
            failed
        );
        known.outcome = CompletionOutcome::Unusable {
            cause: FailureCause::Refused,
            usage_revision: 1,
        };
        assert_eq!(
            complete_dispatch_in(&tx, &known).await,
            Err(ExecutionError::OperationConflict)
        );
        tx.commit().await.expect("commit");
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn expired_candidate_local_recovery_waits_for_sent_and_replays_one_canonical_child() {
    runtime().block_on(async {
        use df_imagery::execution::{ReconciliationCommand,FinishRecoveryCommand,RecoveryDisposition};
        use df_persistence::imagery_execution::{claim_recovery_in,prepare_reconciliation_in,
            mark_local_sent_in,LocalSentOutcome,finish_recovery_in,read_stage_basis_in,read_current_dispatch_in};
        let scratch=Scratch::open().await;let mut client=connect(&scratch.config()).await;
        setup_world(&mut client).await;let candidate=retain_candidate(&mut client,&scratch.config()).await;
        client.execute("UPDATE imagery.image_job SET state='retired'",&[]).await.expect("retire");
        let tx=client.transaction().await.expect("recovery");
        set_execution_gate_in(&tx,"local/transform",GATE_REVISION+1,false,at(60_000)).await.expect("disabled");
        let lease=claim_recovery_in(&tx,&recovery_command(0x11,0x21,0x81,0x91,at(60_001))).await.expect("paid recovery");
        let mut forged=lease.clone();forged.lease_until_ms=at(900_000);
        let command=ReconciliationCommand {candidate_dispatch:candidate.scope.dispatch,child:stage_id(0x22),
            dispatch:dispatch_id(0x82),claim:ClaimToken::from_bytes(&id(0x72)).expect("child claim"),
            capacity:CapacityScope::new("local/transform".to_owned()).expect("local scope"),
            recipe_revision:4,recipe_digest:[0xa7;32],now_ms:at(60_002)};
        let child=prepare_reconciliation_in(&tx,&forged,&command).await.expect("local child");
        assert_eq!(child.current.state,DispatchState::Prepared);assert!(child.current.local_accounting);
        assert!(child.current.finance.is_none());assert_eq!(child.stage.lease_until_ms,lease.lease_until_ms);
        let basis=read_stage_basis_in(&tx,&owner(),&job_id(0x11),&child.stage.stage).await.expect("basis");
        assert_eq!(basis.parent_dispatch,Some(candidate.scope.dispatch));assert_eq!(basis.input_revision,4);
        assert_eq!(basis.input_digest,[0xa7;32]);assert_ne!(basis.input_digest,candidate.descriptor.sha256);
        assert_eq!(basis.config_revision,candidate.basis.config_revision);
        let ack=FinishRecoveryCommand {owner:owner(),job:job_id(0x11),stage:stage_id(0x21),dispatch:dispatch_id(0x81),
            claim:lease.token,claim_generation:lease.generation,disposition:RecoveryDisposition::Completed,now_ms:at(60_003)};
        assert_eq!(finish_recovery_in(&tx,&ack).await,Err(ExecutionError::OperationConflict));
        tx.commit().await.expect("prepared crash boundary");drop(client);
        let mut client=connect(&scratch.config()).await;let tx=client.transaction().await.expect("restart replay");
        let mut replay=command.clone();replay.child=stage_id(0x23);replay.dispatch=dispatch_id(0x83);
        replay.claim=ClaimToken::from_bytes(&id(0x73)).expect("fresh candidate claim");replay.now_ms=at(60_004);
        let restored=prepare_reconciliation_in(&tx,&lease,&replay).await.expect("canonical replay");
        assert_eq!(restored.stage.stage,child.stage.stage);assert_eq!(restored.current,child.current);assert_eq!(restored.claim,child.claim);
        let mark=MarkSentCommand::new(owner(),job_id(0x11),child.stage.stage,child.current.dispatch,
            child.claim.token,child.claim.generation,at(60_005)).expect("local marker");
        assert_eq!(mark_local_sent_in(&tx,&mark).await.expect("source transaction gate"),LocalSentOutcome::New);
        tx.commit().await.expect("Sent crash boundary");drop(client);
        let mut client=connect(&scratch.config()).await;let tx=client.transaction().await.expect("lost sent ack");
        assert_eq!(mark_local_sent_in(&tx,&mark).await.expect("no second egress"),LocalSentOutcome::AlreadySent);
        assert_eq!(read_current_dispatch_in(&tx,&owner(),&job_id(0x11),&child.stage.stage).await.expect("exact child cursor").expect("child").state,DispatchState::Sent);
        finish_recovery_in(&tx,&FinishRecoveryCommand{now_ms:at(60_006),..ack}).await.expect("discoverable sent child allows ack");
        tx.commit().await.expect("ack commit");
        let tx=client.transaction().await.expect("old proof");
        assert_eq!(prepare_reconciliation_in(&tx,&lease,&replay).await,Err(ExecutionError::LostClaim));tx.rollback().await.expect("rollback");
        assert_eq!(dispatch_count(&client,0x11,0x21).await,1);assert_eq!(dispatch_count(&client,0x11,0x22).await,1);
        assert_eq!(client.query_one("SELECT count(*) FROM imagery.image_dispatch_attempt WHERE NOT local_accounting",&[]).await.expect("paid sends").get::<_,i64>(0),1);
        assert_eq!(client.query_one("SELECT count(*) FROM media.asset",&[]).await.expect("no assets").get::<_,i64>(0),0);
        assert_eq!(client.query_one("SELECT count(*) FROM imagery.image_interest",&[]).await.expect("no new demand").get::<_,i64>(0),1);
        scratch.finish().await;
    });
}

#[test]
fn expired_recovery_proof_cannot_mark_prepared_local_child_or_create_paid_child() {
    runtime().block_on(async {
        use df_imagery::execution::ReconciliationCommand;
        use df_persistence::imagery_execution::{
            claim_recovery_in, mark_local_sent_in, prepare_reconciliation_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        retain_candidate(&mut client, &scratch.config()).await;
        let tx = client.transaction().await.expect("recovery");
        let lease = claim_recovery_in(&tx, &recovery_command(0x11, 0x21, 0x81, 0x91, at(60_001)))
            .await
            .expect("lease");
        let command = ReconciliationCommand {
            candidate_dispatch: dispatch_id(0x81),
            child: stage_id(0x22),
            dispatch: dispatch_id(0x82),
            claim: ClaimToken::from_bytes(&id(0x72)).expect("claim"),
            capacity: CapacityScope::new("local/transform".to_owned()).expect("scope"),
            recipe_revision: 1,
            recipe_digest: [0xa7; 32],
            now_ms: at(60_002),
        };
        let child = prepare_reconciliation_in(&tx, &lease, &command)
            .await
            .expect("prepared");
        let verdict: String = tx
            .query_one(
                "SELECT verdict FROM imagery.mark_local_dispatch_sent($1,$2,$3,$4,NULL,NULL,$5)",
                &[
                    &df_imagery::owner_scope_bytes(&owner()),
                    &job_id(0x11).as_bytes().as_slice(),
                    &child.stage.stage.as_bytes().as_slice(),
                    &child.current.dispatch.as_bytes().as_slice(),
                    &at(60_003),
                ],
            )
            .await
            .expect("NULL claim refusal")
            .get(0);
        assert_eq!(verdict, "invalid");
        assert_eq!(
            read_dispatch_in(&tx, &owner(), &child.current.dispatch)
                .await
                .expect("unchanged marker")
                .state,
            DispatchState::Prepared
        );
        let mut paid = enqueue_command(0x11, 0x25, at(60_003)).with_parent(dispatch_id(0x81));
        paid.kind = StageKind::Inspection;
        assert_eq!(
            enqueue_stage_in(&tx, &paid).await,
            Err(ExecutionError::Expired)
        );
        tx.commit().await.expect("prepared commit");
        let tx = client.transaction().await.expect("expired gate");
        assert_eq!(
            mark_local_sent_in(
                &tx,
                &MarkSentCommand::new(
                    owner(),
                    job_id(0x11),
                    child.stage.stage,
                    child.current.dispatch,
                    child.claim.token,
                    child.claim.generation,
                    lease.lease_until_ms
                )
                .expect("mark")
            )
            .await,
            Err(ExecutionError::LostClaim)
        );
        let mut expired = command;
        expired.now_ms = lease.lease_until_ms;
        assert_eq!(
            prepare_reconciliation_in(&tx, &lease, &expired).await,
            Err(ExecutionError::LostClaim)
        );
        tx.rollback().await.expect("rollback");
        assert_eq!(dispatch_count(&client, 0x11, 0x22).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn urgency_is_immutable_per_interest_and_age_promotes_fifo_without_changing_content_identity() {
    runtime().block_on(async {
        use df_imagery::Urgency;
        use df_persistence::imagery_execution::{
            read_capacity_bounds_in, read_current_dispatch_in, record_urgency_in, scan_queue_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        accept_and_enqueue(&mut client, 0x30, 0x31, 0x32, 0x41, at(100)).await;
        let tx = client.transaction().await.expect("hints");
        let bounds = CapacityBounds::new(4, 64, 1 << 20)
            .expect("bounds")
            .with_urgency_promotion_ms(1000)
            .expect("age");
        set_capacity_bounds_in(&tx, "live/generation", &bounds, at(101))
            .await
            .expect("install");
        assert_eq!(
            read_capacity_bounds_in(&tx, "live/generation")
                .await
                .expect("read bounds"),
            bounds
        );
        assert_eq!(
            read_capacity_bounds_in(&tx, "absent").await,
            Err(ExecutionError::NotFound)
        );
        assert_eq!(
            read_current_dispatch_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
                .await
                .expect("not prepared"),
            None
        );
        let interest = ImageInterestId::from_bytes(&id(0x12)).expect("interest");
        assert_eq!(
            record_urgency_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &interest,
                Urgency::Speculative,
                at(101)
            )
            .await
            .expect("speculative"),
            Urgency::Speculative
        );
        assert_eq!(
            record_urgency_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &interest,
                Urgency::Visible,
                at(102)
            )
            .await
            .expect("replay preserves first"),
            Urgency::Speculative
        );
        record_urgency_in(
            &tx,
            &owner(),
            &job_id(0x31),
            &ImageInterestId::from_bytes(&id(0x32)).expect("interest"),
            Urgency::Visible,
            at(101),
        )
        .await
        .expect("visible");
        assert_eq!(
            scan_queue_in(&tx, "live/generation", 2, at(103))
                .await
                .expect("priority")
                .iter()
                .map(|row| row.stage)
                .collect::<Vec<_>>(),
            vec![stage_id(0x41), stage_id(0x21)]
        );
        assert_eq!(
            scan_queue_in(&tx, "live/generation", 2, at(2000))
                .await
                .expect("age fairness")
                .iter()
                .map(|row| row.stage)
                .collect::<Vec<_>>(),
            vec![stage_id(0x21), stage_id(0x41)]
        );
        assert_eq!(
            record_urgency_in(
                &tx,
                &draft_owner(),
                &job_id(0x11),
                &interest,
                Urgency::Visible,
                at(104)
            )
            .await,
            Err(ExecutionError::NotFound)
        );
        tx.commit().await.expect("hints commit");
        let journal = PostgresImageExecution::open_existing(
            scratch.config(),
            PostgresTransport::PlaintextLoopbackOnly,
            2,
        )
        .await
        .expect("noDDL execution startup");
        assert_eq!(
            journal
                .read_capacity_bounds("live/generation")
                .await
                .expect("retained bounds"),
            bounds
        );
        scratch.finish().await;
    });
}

#[test]
fn direct_winning_inspection_recovery_finalizes_exact_candidate_without_reopening_paid_work() {
    runtime().block_on(async {
        use df_imagery::execution::{DecisionBasis,DecisionKind,DecisionObservation,ObservationReceipt,ExecutionScope,ReconciliationCommand};
        use df_persistence::imagery_execution::{read_stage_basis_in,record_decision_observation_in,
            claim_recovery_in,prepare_reconciliation_in,mark_local_sent_in,LocalSentOutcome};
        let scratch=Scratch::open().await;let mut client=connect(&scratch.config()).await;
        setup_world(&mut client).await;let candidate=retain_candidate(&mut client,&scratch.config()).await;
        let tx=client.transaction().await.expect("inspection child");
        let mut enqueue=enqueue_command(0x11,0x22,at(7)).with_parent(candidate.scope.dispatch);enqueue.kind=StageKind::Inspection;
        enqueue_stage_in(&tx,&enqueue).await.expect("real child");tx.commit().await.expect("enqueue commit");
        claim(&mut client,0x11,0x22,0x72,0,at(8)).await;
        let tx=client.transaction().await.expect("inspection Sent");
        prepare_dispatch_in(&tx,&prepare_command(0x11,0x22,0x82,0x72,1,hosted_mode(0x82),at(9))).await.expect("prepare");
        mark_sent_in(&tx,&mark_sent_command(0x11,0x22,0x82,0x72,1,at(10))).await.expect("inspection send marker");
        let basis=read_stage_basis_in(&tx,&owner(),&job_id(0x11),&stage_id(0x22)).await.expect("actual parent basis");
        tx.commit().await.expect("Sent commit");
        let scope=ExecutionScope{owner:owner(),job:job_id(0x11),stage:stage_id(0x22),dispatch:dispatch_id(0x82)};
        let payload=br#"{"accepted":true}"#.to_vec();
        record_raw_usage(&scratch.config(),&scope,1,Some(payload.clone()),df_imagery::cost::UsageEvidence::Reported,true).await;
        let observation=DecisionObservation::new(scope,DecisionKind::Inspection,1,
            DecisionBasis{schema_revision:1,validator_revision:1,stage:basis},payload,
            ObservationReceipt{usage_revision:1,observed_at_ms:at(11)},1024).expect("decision evidence");
        let tx=client.transaction().await.expect("inspection decision");
        record_decision_observation_in(&tx,&observation).await.expect("durable decision");
        complete_dispatch_in(&tx,&finish_command(0x11,0x22,0x82,0x72,1,CompletionOutcome::Decision{observation_version:1,usage_revision:1},at(12))).await.expect("inspection win");
        mirror_settlement_in(&tx,&owner(),&job_id(0x11),&stage_id(0x22),&dispatch_id(0x82),DispatchSettlement::Settled,at(13)).await.expect("inspection settlement");
        tx.commit().await.expect("decision commit");
        client.execute("UPDATE imagery.image_job SET state='retired'",&[]).await.expect("retire");
        let tx=client.transaction().await.expect("inspection recovery");
        let lease=claim_recovery_in(&tx,&recovery_command(0x11,0x22,0x82,0x92,at(60_001))).await.expect("direct inspection recovery");
        let command=ReconciliationCommand{candidate_dispatch:candidate.scope.dispatch,child:stage_id(0x23),dispatch:dispatch_id(0x83),
            claim:ClaimToken::from_bytes(&id(0x73)).expect("claim"),capacity:CapacityScope::new("local/transform".to_owned()).expect("scope"),
            recipe_revision:4,recipe_digest:[0xa8;32],now_ms:at(60_002)};
        let child=prepare_reconciliation_in(&tx,&lease,&command).await.expect("one-hop local child");
        assert_eq!(df_persistence::imagery_execution::finish_recovery_in(&tx,
            &df_imagery::execution::FinishRecoveryCommand{owner:owner(),job:job_id(0x11),stage:stage_id(0x22),dispatch:dispatch_id(0x82),
                claim:lease.token,claim_generation:lease.generation,disposition:df_imagery::execution::RecoveryDisposition::Completed,now_ms:at(60_003)}).await,
            Err(ExecutionError::OperationConflict));
        let final_basis=read_stage_basis_in(&tx,&owner(),&job_id(0x11),&child.stage.stage).await.expect("final basis");
        assert_eq!(final_basis.parent_dispatch,Some(candidate.scope.dispatch));assert_ne!(final_basis.parent_dispatch,Some(lease.stage.current.dispatch));
        assert_eq!(child.current.state,DispatchState::Prepared);
        assert_eq!(mark_local_sent_in(&tx,&MarkSentCommand::new(owner(),job_id(0x11),child.stage.stage,
            child.current.dispatch,child.claim.token,child.claim.generation,at(60_003)).expect("marker")).await.expect("exact actor gate"),LocalSentOutcome::New);
        tx.commit().await.expect("local commit");
        assert_eq!(client.query_one("SELECT count(*) FROM imagery.image_dispatch_attempt WHERE NOT local_accounting",&[]).await.expect("original paid count").get::<_,i64>(0),2);
        assert_eq!(client.query_one("SELECT count(*) FROM media.asset",&[]).await.expect("journal never publishes").get::<_,i64>(0),0);
        scratch.finish().await;
    });
}

#[test]
fn execution_startup_without_migration_refuses_missing_schema_and_zero_connections() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::ImageExecutionOpenError;
        let scratch = Scratch::open().await;
        assert!(matches!(
            PostgresImageExecution::open_existing(
                scratch.config(),
                PostgresTransport::PlaintextLoopbackOnly,
                0
            )
            .await,
            Err(ImageExecutionOpenError::InvalidConnections)
        ));
        let empty_database = format!("{}_empty", scratch.database);
        let admin = connect(&scratch.admin).await;
        admin
            .batch_execute(&format!("CREATE DATABASE {empty_database}"))
            .await
            .expect("owned empty database");
        let mut empty_config = scratch.config();
        empty_config.dbname(&empty_database);
        assert!(matches!(
            PostgresImageExecution::open_existing(
                empty_config.clone(),
                PostgresTransport::PlaintextLoopbackOnly,
                1
            )
            .await,
            Err(ImageExecutionOpenError::MissingSchema)
        ));
        let client = connect(&empty_config).await;
        assert_eq!(
            client
                .query_one("SELECT to_regnamespace('imagery')::text", &[])
                .await
                .expect("schema absence")
                .get::<_, Option<String>>(0),
            None
        );
        drop(client);
        admin
            .batch_execute(&format!("DROP DATABASE {empty_database} WITH (FORCE)"))
            .await
            .expect("drop owned empty database");
        scratch.finish().await;
    });
}

#[test]
fn successful_candidate_with_unknown_billing_requires_its_exact_selected_raw_usage_revision() {
    runtime().block_on(async {
        use df_imagery::cost::UsageEvidence;
        use df_persistence::imagery_execution::{
            read_stage_basis_in, record_candidate_observation_in,
        };
        for evidence in [UsageEvidence::Reported, UsageEvidence::Unknown] {
            let scratch = Scratch::open().await;
            let mut client = connect(&scratch.config()).await;
            setup_world(&mut client).await;
            prepare_decision_stage(&mut client, StageKind::Generation).await;
            let tx = client.transaction().await.expect("basis");
            let basis = read_stage_basis_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
                .await
                .expect("basis");
            tx.commit().await.expect("basis commit");
            let bytes = candidate_png(false);
            let observation = candidate_observation(basis, &bytes, 1, 1);
            record_raw_usage(
                &scratch.config(),
                &observation.as_receipt().scope,
                1,
                Some(bytes.clone()),
                evidence,
                true,
            )
            .await;
            let other = if evidence == UsageEvidence::Unknown {
                UsageEvidence::Reported
            } else {
                UsageEvidence::Unknown
            };
            record_raw_usage(
                &scratch.config(),
                &observation.as_receipt().scope,
                2,
                Some(bytes),
                other,
                true,
            )
            .await;
            let tx = client.transaction().await.expect("complete candidate");
            record_candidate_observation_in(&tx, &observation)
                .await
                .expect("durable bytes");
            let completed = complete_dispatch_in(
                &tx,
                &finish_command(
                    0x11,
                    0x21,
                    0x81,
                    0x71,
                    1,
                    CompletionOutcome::Candidate {
                        observation_version: 1,
                        usage_revision: 1,
                    },
                    at(5),
                ),
            )
            .await
            .expect("semantic success");
            assert_eq!(completed.state, DispatchState::Succeeded);
            assert_eq!(completed.usage_revision, 1);
            let mirrored = mirror_settlement_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81),
                DispatchSettlement::UnknownOutstanding,
                at(6),
            )
            .await;
            if evidence == UsageEvidence::Unknown {
                assert_eq!(
                    mirrored.expect("exact Unknown receipt").settlement,
                    DispatchSettlement::UnknownOutstanding
                );
            } else {
                assert_eq!(mirrored, Err(ExecutionError::ObservationMissing));
            }
            let settled = mirror_settlement_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                &dispatch_id(0x81),
                DispatchSettlement::Settled,
                at(7),
            )
            .await
            .expect("finance terminal settlement");
            assert_eq!(settled.settlement, DispatchSettlement::Settled);
            assert_eq!(
                mirror_settlement_in(
                    &tx,
                    &owner(),
                    &job_id(0x11),
                    &stage_id(0x21),
                    &dispatch_id(0x81),
                    DispatchSettlement::UnknownOutstanding,
                    at(8)
                )
                .await,
                Err(ExecutionError::OperationConflict)
            );
            tx.commit().await.expect("commit");
            assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
            scratch.finish().await;
        }
    });
}

#[test]
fn source_denied_queue_deferral_returns_expired_unsent_capacity_without_losing_or_starving_demand()
{
    runtime().block_on(async {
        use df_persistence::imagery_execution::scan_queue_in;
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        let original = CapacityBounds::new(1, 2, 256).expect("bounded queue");
        let tx = client.transaction().await.expect("install queue");
        set_capacity_bounds_in(&tx, "live/generation", &original, at(0))
            .await
            .expect("bounds");
        tx.commit().await.expect("bounds commit");
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        accept_and_enqueue(&mut client, 0x30, 0x31, 0x32, 0x41, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (1, 1, 128)
        );
        let tx = client.transaction().await.expect("live claim denial");
        assert_eq!(
            defer_queued_in(
                &tx,
                &owner(),
                &job_id(0x11),
                &stage_id(0x21),
                at(20_000),
                at(2)
            )
            .await,
            Err(ExecutionError::LostClaim)
        );
        tx.rollback().await.expect("rollback");
        let tx = client.transaction().await.expect("expired denial");
        set_capacity_bounds_in(
            &tx,
            "live/generation",
            &CapacityBounds::new(1, 1, 128).expect("lowered"),
            at(11_000),
        )
        .await
        .expect("lowered bounds");
        defer_queued_in(
            &tx,
            &owner(),
            &job_id(0x11),
            &stage_id(0x21),
            at(20_000),
            at(11_000),
        )
        .await
        .expect("return existing active reserve");
        let before = read_stage_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .expect("deferred stage");
        assert_eq!(before.state, StageState::Planned);
        assert_eq!(before.lease_until_ms, 0);
        defer_queued_in(
            &tx,
            &owner(),
            &job_id(0x11),
            &stage_id(0x21),
            at(20_000),
            at(11_001),
        )
        .await
        .expect("idempotent repeat");
        assert_eq!(
            read_stage_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
                .await
                .expect("same row"),
            before
        );
        assert_eq!(
            claim_stage_in(
                &tx,
                &claim_command(0x11, 0x21, GATE_REVISION, 0x72, 0, at(11_002))
            )
            .await,
            Err(ExecutionError::Deferred {
                not_before_ms: at(20_000)
            })
        );
        assert_eq!(
            claim_stage_in(
                &tx,
                &claim_command(0x31, 0x41, GATE_REVISION, 0x73, 0, at(11_002))
            )
            .await,
            Err(ExecutionError::CapacityFull)
        );
        tx.commit().await.expect("deferral commit");
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (0, 2, 256)
        );
        let tx = client.transaction().await.expect("resume other owner work");
        set_capacity_bounds_in(&tx, "live/generation", &original, at(11_003))
            .await
            .expect("bounds restored");
        let queued = scan_queue_in(&tx, "live/generation", 10, at(11_004))
            .await
            .expect("due filter");
        assert_eq!(
            queued.iter().map(|stage| stage.stage).collect::<Vec<_>>(),
            vec![stage_id(0x41)]
        );
        claim_stage_in(
            &tx,
            &claim_command(0x31, 0x41, GATE_REVISION, 0x73, 0, at(11_004)),
        )
        .await
        .expect("other job not starved");
        assert_eq!(
            defer_queued_in(
                &tx,
                &draft_owner(),
                &job_id(0x11),
                &stage_id(0x21),
                at(20_000),
                at(11_004)
            )
            .await,
            Err(ExecutionError::NotFound)
        );
        tx.commit().await.expect("other claim commit");
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (1, 1, 128)
        );
        let tx = client.transaction().await.expect("due time");
        assert_eq!(
            scan_queue_in(&tx, "live/generation", 10, at(20_000))
                .await
                .expect("due")
                .iter()
                .map(|stage| stage.stage)
                .collect::<Vec<_>>(),
            vec![stage_id(0x21)]
        );
        tx.commit().await.expect("due commit");
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_interest", &[])
                .await
                .expect("original demand")
                .get::<_, i64>(0),
            2
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
                .await
                .expect("no send or finance")
                .get::<_, i64>(0),
            0
        );
        scratch.finish().await;
    });
}

#[path = "imagery_execution/terminal.rs"]
mod terminal_regressions;

#[path = "imagery_execution/local_inspection.rs"]
mod local_inspection_regressions;

#[path = "imagery_execution/queue.rs"]
mod queue_regressions;

#[path = "imagery_execution/terminal_pending.rs"]
mod terminal_pending_regressions;

async fn defer_queued_in(
    tx: &tokio_postgres::Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    not_before_ms: i64,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    df_persistence::imagery_execution::defer_queued_in(
        tx,
        &df_imagery::execution::DeferQueuedCommand {
            scope: df_imagery::execution::StageScope {
                owner: owner.clone(),
                job: *job,
                stage: *stage,
            },
            not_before_ms,
            notice: None,
            now_ms,
        },
    )
    .await
}

#[path = "imagery_execution/outcomes.rs"]
mod outcome_regressions;

#[path = "imagery_execution/decline.rs"]
mod decline_regressions;
