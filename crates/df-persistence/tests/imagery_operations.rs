//! The imagery operation journal on real PostgreSQL: replay, join, freeze and withdrawal
//! semantics, plus the races only a real database can show, driven by barriers instead of
//! sleeps. Each test creates its own database `df_imagery_test_<time>_<pid>_<serial>` on the
//! owned scratch server named by `DUNGEONFLUX_RUNTIME_PG_PORT` (host
//! `DUNGEONFLUX_RUNTIME_PG_HOST`, default 127.0.0.1; user `DUNGEONFLUX_RUNTIME_PG_USER`,
//! default postgres) and drops only that database at the end. Without the server every test
//! reports SKIPPED, unless `DUNGEONFLUX_REQUIRE_SCRATCH_PG` is set.

mod runtime_support;

use df_imagery::{
    AcceptCommand, AcceptOutcome, AcceptTarget, FreezeCommand, FreezeOutcome, ImageInterestId,
    ImageJobId, ImageJournal, InterestState, JobState, JoinRefusal, JournalError, SharingIdentity,
    VersionedSnapshot, WithdrawOutcome,
};
use df_imagery::{ImageOwner, OperationIdentity};
use df_persistence::PostgresImageJournal;
use df_persistence::postgres_tls::PostgresTransport;
use df_types::{AccountId, CampaignId, OperationId, RunId};
use runtime_support::run;
use tokio_postgres::{Client, Config, NoTls};

/// Never these ports: the game's and the other owned test servers.
const FORBIDDEN_PORTS: [u16; 4] = [0, 55517, 55439, 63309];
const SNAPSHOT_LIMIT: usize = 64;

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
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let database = format!("df_imagery_test_{nanos}_{}_{serial}", std::process::id());
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

    async fn rows(&self, table: &str) -> i64 {
        let (client, _driver) = raw_client(&pg_config(&self.database).expect("config")).await;
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

fn draft_owner() -> ImageOwner {
    ImageOwner::Draft {
        account: AccountId::from_hex("00000000000000000000000000000011").unwrap(),
    }
}

fn other_owner() -> ImageOwner {
    ImageOwner::Run {
        campaign: CampaignId::from_hex("00000000000000000000000000000033").unwrap(),
        run: RunId::from_hex("00000000000000000000000000000044").unwrap(),
    }
}

fn operation(byte: u8) -> OperationId {
    let mut bytes = [0xcc_u8; 16];
    bytes[15] = byte;
    OperationId::from_bytes(&bytes).unwrap()
}

fn job(byte: u8) -> ImageJobId {
    let mut bytes = [0xaa_u8; 16];
    bytes[15] = byte;
    ImageJobId::from_bytes(&bytes).unwrap()
}

fn interest(byte: u8) -> ImageInterestId {
    let mut bytes = [0xbb_u8; 16];
    bytes[15] = byte;
    ImageInterestId::from_bytes(&bytes).unwrap()
}

fn snapshot(byte: u8) -> VersionedSnapshot {
    VersionedSnapshot::new(1, vec![byte; 16], SNAPSHOT_LIMIT).unwrap()
}

fn sharing(byte: u8) -> SharingIdentity {
    SharingIdentity::from_bytes(&[byte; 32]).unwrap()
}

fn create(
    owner: ImageOwner,
    op: u8,
    fingerprint: [u8; 32],
    intent_byte: u8,
    job_byte: u8,
    interest_byte: u8,
    deadline_ms: i64,
    now_ms: i64,
) -> AcceptCommand {
    AcceptCommand {
        identity: OperationIdentity {
            owner,
            operation: operation(op),
            fingerprint,
        },
        intent: snapshot(intent_byte),
        config: snapshot(0xC0),
        sharing: sharing(0x07),
        interest: interest(interest_byte),
        target: AcceptTarget::Create {
            job: job(job_byte),
            absolute_deadline_ms: deadline_ms,
        },
        now_ms,
    }
}

fn join(
    owner: ImageOwner,
    op: u8,
    fingerprint: [u8; 32],
    intent_byte: u8,
    job_byte: u8,
    interest_byte: u8,
    now_ms: i64,
) -> AcceptCommand {
    let mut command = create(
        owner,
        op,
        fingerprint,
        intent_byte,
        job_byte,
        interest_byte,
        9_000,
        now_ms,
    );
    command.target = AcceptTarget::Join { job: job(job_byte) };
    command
}

fn freeze(
    owner: ImageOwner,
    job_byte: u8,
    expected_revision: u64,
    key_byte: u8,
    recipe_byte: u8,
    now_ms: i64,
) -> FreezeCommand {
    FreezeCommand {
        owner,
        job: job(job_byte),
        expected_revision,
        key: df_assets::MediaRequestKey::from_bytes([key_byte; 32]),
        recipe: VersionedSnapshot::new(2, vec![recipe_byte; 16], SNAPSHOT_LIMIT).unwrap(),
        now_ms,
    }
}

#[test]
fn replay_returns_original_view_without_modifying_anything() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let created = fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        let AcceptOutcome::Created(first) = created else {
            panic!("expected Created");
        };
        let replayed = fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 2, 2, 9_000, 1_500))
            .await
            .expect("replay");
        let AcceptOutcome::Replayed(second) = replayed else {
            panic!("expected Replayed");
        };
        assert_eq!(second.receipt, first.receipt);
        assert_eq!(second.job, first.job);
        assert_eq!(second.interest, first.interest);
        assert_eq!(second.job.revision, 1);
        assert_eq!(second.job.updated_at_ms, 1_000);
        assert_eq!(fixture.rows("image_job").await, 1);
        assert_eq!(fixture.rows("image_operation").await, 1);
        assert_eq!(fixture.rows("image_interest").await, 1);
        fixture.finish().await;
    });
}

#[test]
fn fingerprint_or_intent_change_conflicts() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        let changed_fingerprint = fixture
            .journal
            .accept(&create(draft_owner(), 1, [2; 32], 0x10, 2, 2, 9_000, 1_100))
            .await;
        assert_eq!(changed_fingerprint, Err(JournalError::OperationConflict));
        let changed_intent = fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x11, 2, 2, 9_000, 1_100))
            .await;
        assert_eq!(changed_intent, Err(JournalError::OperationConflict));
        assert_eq!(fixture.rows("image_job").await, 1);
        assert_eq!(fixture.rows("image_operation").await, 1);
        assert_eq!(fixture.rows("image_interest").await, 1);
        fixture.finish().await;
    });
}

#[test]
fn same_operation_bytes_across_distinct_owners_stay_scoped() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("first owner");
        fixture
            .journal
            .accept(&create(other_owner(), 1, [1; 32], 0x10, 2, 2, 9_000, 1_000))
            .await
            .expect("second owner");
        let draft_view = fixture
            .journal
            .lookup_operation(&draft_owner(), operation(1))
            .await
            .expect("lookup")
            .expect("draft receipt");
        let run_view = fixture
            .journal
            .lookup_operation(&other_owner(), operation(1))
            .await
            .expect("lookup")
            .expect("run receipt");
        assert_ne!(draft_view.job.job, run_view.job.job);
        assert_eq!(draft_view.job.owner, draft_owner());
        assert_eq!(run_view.job.owner, other_owner());
        // A wrong owner learns nothing: no receipt, no job, and every write is refused.
        assert_eq!(
            fixture
                .journal
                .lookup_operation(&other_owner(), operation(9))
                .await
                .expect("lookup"),
            None
        );
        assert_eq!(
            fixture
                .journal
                .read_job(&other_owner(), job(1))
                .await
                .expect("read"),
            None
        );
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(other_owner(), 1, 1, 0xF0, 0xE0, 1_100))
                .await,
            Err(JournalError::NotFound)
        );
        assert_eq!(
            fixture
                .journal
                .withdraw_interest(&other_owner(), interest(1), 1_100)
                .await,
            Err(JournalError::NotFound)
        );
        assert_eq!(fixture.rows("image_operation").await, 2);
        fixture.finish().await;
    });
}

#[test]
fn two_compatible_operations_join_one_job() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        let joined = fixture
            .journal
            .accept(&join(draft_owner(), 2, [1; 32], 0x10, 1, 2, 1_100))
            .await
            .expect("join");
        let AcceptOutcome::Joined(view) = joined else {
            panic!("expected Joined");
        };
        assert_eq!(view.job.job, job(1));
        assert_eq!(view.job.revision, 2);
        assert_eq!(view.job.updated_at_ms, 1_100);
        assert_eq!(view.receipt.operation, operation(2));
        assert_eq!(view.interest.interest, interest(2));
        assert_eq!(view.interest.state, InterestState::Attached);
        assert_eq!(fixture.rows("image_job").await, 1);
        assert_eq!(fixture.rows("image_operation").await, 2);
        assert_eq!(fixture.rows("image_interest").await, 2);
        fixture.finish().await;
    });
}

#[test]
fn incompatible_join_is_refused_without_fallback_creation() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        let mut wrong_sharing = join(draft_owner(), 2, [1; 32], 0x10, 1, 2, 1_100);
        wrong_sharing.sharing = sharing(0x08);
        assert_eq!(
            fixture.journal.accept(&wrong_sharing).await,
            Err(JournalError::JoinRefused(JoinRefusal::SharingMismatch))
        );
        let mut wrong_intent = join(draft_owner(), 2, [1; 32], 0x10, 1, 2, 1_100);
        wrong_intent.intent = snapshot(0x11);
        assert_eq!(
            fixture.journal.accept(&wrong_intent).await,
            Err(JournalError::JoinRefused(JoinRefusal::IntentMismatch))
        );
        let mut wrong_config = join(draft_owner(), 2, [1; 32], 0x10, 1, 2, 1_100);
        wrong_config.config = snapshot(0xC1);
        assert_eq!(
            fixture.journal.accept(&wrong_config).await,
            Err(JournalError::JoinRefused(JoinRefusal::ConfigMismatch))
        );
        let unknown_job = join(draft_owner(), 2, [1; 32], 0x10, 9, 2, 1_100);
        assert_eq!(
            fixture.journal.accept(&unknown_job).await,
            Err(JournalError::NotFound)
        );
        // Every refusal left the job alone and created no fallback rows.
        let stored = fixture
            .journal
            .read_job(&draft_owner(), job(1))
            .await
            .expect("read")
            .expect("job");
        assert_eq!(stored.revision, 1);
        assert_eq!(fixture.rows("image_job").await, 1);
        assert_eq!(fixture.rows("image_operation").await, 1);
        assert_eq!(fixture.rows("image_interest").await, 1);
        fixture.finish().await;
    });
}

#[test]
fn withdrawn_replay_does_not_reattach() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        let withdrawn = fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_100)
            .await
            .expect("withdraw");
        let WithdrawOutcome::Withdrawn(first) = withdrawn else {
            panic!("expected Withdrawn");
        };
        assert_eq!(first.attached_remaining, 0);
        let replayed = fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 3, 3, 9_000, 1_200))
            .await
            .expect("replay");
        let AcceptOutcome::Replayed(view) = replayed else {
            panic!("expected Replayed");
        };
        assert_eq!(view.interest.state, InterestState::Withdrawn);
        assert_eq!(view.interest.withdrawn_at_ms, Some(1_100));
        assert_eq!(view.job.state, JobState::Retired);
        assert_eq!(fixture.rows("image_interest").await, 1);
        fixture.finish().await;
    });
}

#[test]
fn one_withdrawal_preserves_the_other_interest() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        fixture
            .journal
            .accept(&join(draft_owner(), 2, [1; 32], 0x10, 1, 2, 1_100))
            .await
            .expect("join");
        let first = fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_200)
            .await
            .expect("withdraw one");
        let WithdrawOutcome::Withdrawn(first) = first else {
            panic!("expected Withdrawn");
        };
        assert_eq!(first.attached_remaining, 1);
        let stored = fixture
            .journal
            .read_job(&draft_owner(), job(1))
            .await
            .expect("read")
            .expect("job");
        assert_eq!(stored.state, JobState::Planning);
        let kept = fixture
            .journal
            .lookup_operation(&draft_owner(), operation(2))
            .await
            .expect("lookup")
            .expect("second receipt");
        assert_eq!(kept.interest.state, InterestState::Attached);
        let second = fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_300)
            .await
            .expect("repeat withdraw");
        assert!(matches!(second, WithdrawOutcome::AlreadyWithdrawn(_)));
        fixture.finish().await;
    });
}

#[test]
fn last_withdrawal_retires_but_preserves_receipts_and_frozen_facts() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        let frozen = fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 1, 0xF0, 0xE0, 1_100))
            .await
            .expect("freeze");
        let FreezeOutcome::Frozen { job: frozen_job } = frozen else {
            panic!("expected Frozen");
        };
        assert_eq!(frozen_job.revision, 2);
        let withdrawn = fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_200)
            .await
            .expect("withdraw last");
        let WithdrawOutcome::Withdrawn(last) = withdrawn else {
            panic!("expected Withdrawn");
        };
        assert_eq!(last.attached_remaining, 0);
        assert_eq!(last.job_revision, 3);
        let stored = fixture
            .journal
            .read_job(&draft_owner(), job(1))
            .await
            .expect("read")
            .expect("job");
        assert_eq!(stored.state, JobState::Retired);
        assert_eq!(stored.revision, 3);
        assert_eq!(
            stored.resolved_key,
            Some(df_assets::MediaRequestKey::from_bytes([0xF0; 32]))
        );
        assert_eq!(
            stored.recipe,
            Some(VersionedSnapshot::new(2, vec![0xE0; 16], SNAPSHOT_LIMIT).unwrap())
        );
        let view = fixture
            .journal
            .lookup_operation(&draft_owner(), operation(1))
            .await
            .expect("lookup")
            .expect("receipt survives");
        assert_eq!(view.job.state, JobState::Retired);
        assert_eq!(view.job.resolved_key, stored.resolved_key);
        fixture.finish().await;
    });
}

#[test]
fn simultaneous_creates_leave_single_job_receipt_and_interest() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let spawn = |job_byte: u8, interest_byte: u8| {
            let journal = fixture.journal.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                journal
                    .accept(&create(
                        draft_owner(),
                        1,
                        [1; 32],
                        0x10,
                        job_byte,
                        interest_byte,
                        9_000,
                        1_000,
                    ))
                    .await
                    .expect("accept")
            })
        };
        let first_handle = spawn(1, 1);
        let second_handle = spawn(2, 2);
        let first = first_handle.await.expect("first task");
        let second = second_handle.await.expect("second task");
        let created = [&first, &second]
            .iter()
            .filter(|outcome| matches!(outcome, AcceptOutcome::Created(_)))
            .count();
        assert_eq!(created, 1);
        assert_eq!(fixture.rows("image_job").await, 1);
        assert_eq!(fixture.rows("image_operation").await, 1);
        assert_eq!(fixture.rows("image_interest").await, 1);
        fixture.finish().await;
    });
}

#[test]
fn concurrent_incompatible_freeze_has_one_winner() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let spawn = |key_byte: u8| {
            let journal = fixture.journal.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                journal
                    .freeze_recipe(&freeze(draft_owner(), 1, 1, key_byte, key_byte, 1_100))
                    .await
            })
        };
        let first_handle = spawn(0xF0);
        let second_handle = spawn(0xF1);
        let first = first_handle.await.expect("first task");
        let second = second_handle.await.expect("second task");
        let frozen = [&first, &second]
            .iter()
            .filter(|outcome| matches!(outcome, Ok(FreezeOutcome::Frozen { .. })))
            .count();
        let conflicted = [&first, &second]
            .iter()
            .filter(|outcome| matches!(outcome, Err(JournalError::RecipeConflict)))
            .count();
        assert_eq!(frozen, 1);
        assert_eq!(conflicted, 1);
        fixture.finish().await;
    });
}

#[test]
fn identical_freeze_replay_survives_retirement_and_expiry() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 2_000, 1_000))
            .await
            .expect("create");
        let frozen = fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 1, 0xF0, 0xE0, 1_100))
            .await
            .expect("freeze");
        assert!(matches!(frozen, FreezeOutcome::Frozen { .. }));
        let replayed = fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 1, 0xF0, 0xE0, 1_200))
            .await
            .expect("identical replay");
        assert!(matches!(replayed, FreezeOutcome::AlreadyFrozen { .. }));
        fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_300)
            .await
            .expect("withdraw last");
        // Retired and past the deadline, the identical freeze still rereads without reviving.
        let after_retire = fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 9, 0xF0, 0xE0, 5_000))
            .await
            .expect("replay after retire");
        let FreezeOutcome::AlreadyFrozen { job } = after_retire else {
            panic!("expected AlreadyFrozen");
        };
        assert_eq!(job.state, JobState::Retired);
        assert_eq!(job.revision, 3);
        fixture.finish().await;
    });
}

#[test]
fn different_facts_conflict_after_retirement() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 1, 0xF0, 0xE0, 1_100))
            .await
            .expect("freeze");
        fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_200)
            .await
            .expect("retire");
        // A retired job keeps its first decision: a different key conflicts.
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(draft_owner(), 1, 3, 0xF1, 0xE0, 1_300))
                .await,
            Err(JournalError::RecipeConflict)
        );
        // A different recipe with the same key conflicts as well.
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(draft_owner(), 1, 3, 0xF0, 0xE1, 1_300))
                .await,
            Err(JournalError::RecipeConflict)
        );
        // The identical replay still rereads without reviving the job.
        let replayed = fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 3, 0xF0, 0xE0, 1_300))
            .await
            .expect("identical replay after retire");
        let FreezeOutcome::AlreadyFrozen { job } = replayed else {
            panic!("expected AlreadyFrozen");
        };
        assert_eq!(job.state, JobState::Retired);
        assert_eq!(job.revision, 3);
        fixture.finish().await;
    });
}

#[test]
fn different_facts_conflict_after_expiry() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 2_000, 1_000))
            .await
            .expect("create");
        fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 1, 0xF0, 0xE0, 1_100))
            .await
            .expect("freeze");
        // Past the deadline the frozen job still defends its first decision.
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(draft_owner(), 1, 2, 0xF1, 0xE0, 5_000))
                .await,
            Err(JournalError::RecipeConflict)
        );
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(draft_owner(), 1, 2, 0xF0, 0xE1, 5_000))
                .await,
            Err(JournalError::RecipeConflict)
        );
        let replayed = fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 2, 0xF0, 0xE0, 5_000))
            .await
            .expect("identical replay after expiry");
        assert!(matches!(replayed, FreezeOutcome::AlreadyFrozen { .. }));
        fixture.finish().await;
    });
}

#[test]
fn stale_revision_is_refused_and_mismatched_facts_conflict() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(draft_owner(), 1, 7, 0xF0, 0xE0, 1_100))
                .await,
            Err(JournalError::Stale {
                current_revision: 1
            })
        );
        fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 1, 0xF0, 0xE0, 1_100))
            .await
            .expect("freeze");
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(draft_owner(), 1, 2, 0xF1, 0xE1, 1_200))
                .await,
            Err(JournalError::RecipeConflict)
        );
        fixture.finish().await;
    });
}

#[test]
fn first_freeze_refuses_retired_and_expired_jobs() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_100)
            .await
            .expect("retire");
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(draft_owner(), 1, 2, 0xF0, 0xE0, 1_200))
                .await,
            Err(JournalError::Retired)
        );
        fixture
            .journal
            .accept(&create(draft_owner(), 2, [2; 32], 0x10, 2, 2, 2_000, 1_000))
            .await
            .expect("create expiring job");
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(draft_owner(), 2, 1, 0xF0, 0xE0, 2_500))
                .await,
            Err(JournalError::Expired)
        );
        // A join past the deadline is refused the same way, without creating anything.
        assert_eq!(
            fixture
                .journal
                .accept(&join(draft_owner(), 3, [2; 32], 0x10, 2, 3, 2_500))
                .await,
            Err(JournalError::JoinRefused(JoinRefusal::Expired))
        );
        assert_eq!(fixture.rows("image_operation").await, 2);
        fixture.finish().await;
    });
}

#[test]
fn reopened_adapter_retains_original_snapshots_key_and_receipt() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 1, 0xF0, 0xE0, 1_100))
            .await
            .expect("freeze");
        let before = fixture
            .journal
            .lookup_operation(&draft_owner(), operation(1))
            .await
            .expect("lookup")
            .expect("receipt");
        let reopened = PostgresImageJournal::open(
            pg_config(&fixture.database).expect("config"),
            PostgresTransport::PlaintextLoopbackOnly,
            2,
        )
        .await
        .expect("reopen journal");
        let after = reopened
            .read_job(&draft_owner(), job(1))
            .await
            .expect("read")
            .expect("job");
        assert_eq!(after, before.job);
        assert_eq!(after.intent.bytes(), snapshot(0x10).bytes());
        assert_eq!(after.config.bytes(), snapshot(0xC0).bytes());
        let view = reopened
            .lookup_operation(&draft_owner(), operation(1))
            .await
            .expect("lookup")
            .expect("receipt");
        assert_eq!(view, before);
        Fixture {
            journal: reopened,
            ..fixture
        }
        .finish()
        .await;
    });
}

#[test]
fn no_public_access_to_journal_tables() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let (client, _driver) = raw_client(&pg_config(&fixture.database).expect("config")).await;
        // No ACL entry grants the PUBLIC pseudo-role (grantee 0): an empty grantee prefix in
        // any relacl item means PUBLIC. Tables are created grant-free, so relacl stays null.
        for table in ["image_job", "image_operation", "image_interest"] {
            let relacl: Option<Vec<String>> = client
                .query_one(
                    "SELECT relacl FROM pg_class WHERE oid = $1::text::regclass",
                    &[&format!("imagery.{table}")],
                )
                .await
                .expect("relacl read")
                .get(0);
            let items = relacl.unwrap_or_default();
            assert!(
                items.iter().all(|item| !item.starts_with('=')),
                "PUBLIC pseudo-role must hold no grant on {table}: {items:?}"
            );
        }
        // An actual unprivileged login is denied every statement for real.
        let login = format!("df_imagery_noaccess_{}", std::process::id());
        client
            .batch_execute(&format!("DROP ROLE IF EXISTS {login}"))
            .await
            .expect("clear stale unprivileged login");
        client
            .batch_execute(&format!("CREATE ROLE {login} LOGIN"))
            .await
            .expect("create unprivileged login");
        let denied = async {
            let mut denied_config = pg_config(&fixture.database).expect("config");
            denied_config.user(&login);
            let (denied_client, _denied_driver) = raw_client(&denied_config).await;
            for probe in [
                "SELECT COUNT(*) FROM imagery.image_job",
                "INSERT INTO imagery.image_job (job_id) VALUES ('\\x00')",
                "UPDATE imagery.image_job SET state = 'planning' WHERE false",
                "DELETE FROM imagery.image_job WHERE false",
                "SELECT COUNT(*) FROM imagery.image_operation",
                "SELECT COUNT(*) FROM imagery.image_interest",
            ] {
                let error = denied_client
                    .query(probe, &[])
                    .await
                    .expect_err("unprivileged login must be denied");
                let state = error
                    .as_db_error()
                    .map(|db| db.code().code().to_owned());
                assert_eq!(
                    state.as_deref(),
                    Some("42501"),
                    "expected insufficient_privilege for {probe}"
                );
            }
        };
        denied.await;
        client
            .batch_execute(&format!("DROP ROLE IF EXISTS {login}"))
            .await
            .expect("drop unprivileged login");
        fixture.finish().await;
    });
}

#[test]
fn debug_output_never_carries_payload_bytes() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        let stored = fixture
            .journal
            .read_job(&draft_owner(), job(1))
            .await
            .expect("read")
            .expect("job");
        let marker = vec![0x10; 16];
        let rendered = format!("{stored:?} {:?} {:?}", stored.intent, stored.config);
        assert!(!rendered.contains(&format!("{marker:?}")));
        assert!(rendered.contains("revision"));
        fixture.finish().await;
    });
}

#[test]
fn invalid_demand_is_rejected_before_any_write() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let mut timeless = create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 0);
        assert_eq!(
            fixture.journal.accept(&timeless).await,
            Err(JournalError::InvalidRecord)
        );
        timeless.now_ms = 1_000;
        timeless.target = AcceptTarget::Create {
            job: job(1),
            absolute_deadline_ms: 1_000,
        };
        assert_eq!(
            fixture.journal.accept(&timeless).await,
            Err(JournalError::InvalidRecord)
        );
        assert_eq!(
            fixture
                .journal
                .accept(&join(draft_owner(), 1, [1; 32], 0x10, 9, 1, 0))
                .await,
            Err(JournalError::InvalidRecord)
        );
        assert_eq!(
            fixture
                .journal
                .freeze_recipe(&freeze(draft_owner(), 9, 1, 0xF0, 0xE0, 1_100))
                .await,
            Err(JournalError::NotFound)
        );
        assert_eq!(
            fixture
                .journal
                .withdraw_interest(&draft_owner(), interest(9), 1_100)
                .await,
            Err(JournalError::NotFound)
        );
        assert_eq!(
            fixture
                .journal
                .lookup_operation(&draft_owner(), operation(9))
                .await
                .expect("lookup"),
            None
        );
        assert_eq!(
            fixture
                .journal
                .read_job(&draft_owner(), job(9))
                .await
                .expect("read"),
            None
        );
        assert_eq!(fixture.rows("image_job").await, 0);
        fixture.finish().await;
    });
}

#[test]
fn full_u32_snapshot_revisions_round_trip() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let intent = VersionedSnapshot::new(3_000_000_000, vec![0xA5; 16], SNAPSHOT_LIMIT)
            .expect("intent above i32 range");
        let config = VersionedSnapshot::new(u32::MAX, vec![0xC0; 16], SNAPSHOT_LIMIT)
            .expect("config at u32::MAX");
        let mut demand = create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000);
        demand.intent = intent.clone();
        demand.config = config.clone();
        let created = fixture
            .journal
            .accept(&demand)
            .await
            .expect("create with full u32 revisions");
        let AcceptOutcome::Created(view) = created else {
            panic!("expected Created");
        };
        assert_eq!(view.job.intent.revision(), 3_000_000_000);
        assert_eq!(view.job.config.revision(), u32::MAX);
        let recipe = VersionedSnapshot::new(u32::MAX, vec![0xE0; 16], SNAPSHOT_LIMIT)
            .expect("recipe at u32::MAX");
        let mut freeze_command = freeze(draft_owner(), 1, 1, 0xF0, 0xE0, 1_100);
        freeze_command.recipe = recipe.clone();
        let frozen = fixture
            .journal
            .freeze_recipe(&freeze_command)
            .await
            .expect("freeze with u32::MAX recipe revision");
        let FreezeOutcome::Frozen { job: frozen_job } = frozen else {
            panic!("expected Frozen");
        };
        assert_eq!(frozen_job.recipe.expect("recipe").revision(), u32::MAX);
        let stored = fixture
            .journal
            .read_job(&draft_owner(), job(1))
            .await
            .expect("read")
            .expect("job");
        assert_eq!(stored.intent.revision(), 3_000_000_000);
        assert_eq!(stored.config.revision(), u32::MAX);
        assert_eq!(stored.recipe.expect("stored recipe").revision(), u32::MAX);
        fixture.finish().await;
    });
}

#[test]
fn retired_partial_frozen_facts_rejected() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        fixture
            .journal
            .freeze_recipe(&freeze(draft_owner(), 1, 1, 0xF0, 0xE0, 1_100))
            .await
            .expect("freeze");
        fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_200)
            .await
            .expect("retire");
        let (client, _driver) = raw_client(&pg_config(&fixture.database).expect("config")).await;
        let job_id = job(1).as_bytes().to_vec();
        for probe in [
            "UPDATE imagery.image_job SET resolved_recipe_key = NULL WHERE job_id = $1",
            "UPDATE imagery.image_job SET recipe_bytes = NULL WHERE job_id = $1",
            "UPDATE imagery.image_job SET recipe_revision = NULL WHERE job_id = $1",
        ] {
            let error = client
                .execute(probe, &[&job_id])
                .await
                .expect_err("partial retired facts must fail");
            assert_eq!(
                error.as_db_error().map(|db| db.code().code()),
                Some("23514"),
                "expected check_violation for {probe}"
            );
        }
        let stored = fixture
            .journal
            .read_job(&draft_owner(), job(1))
            .await
            .expect("read")
            .expect("job");
        assert_eq!(stored.state, JobState::Retired);
        assert!(stored.resolved_key.is_some());
        assert!(stored.recipe.is_some());
        fixture.finish().await;
    });
}

#[test]
fn zero_ids_rejected_before_any_write() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        assert!(df_imagery::ImageJobId::from_bytes(&[0; 16]).is_err());
        assert!(df_imagery::ImageInterestId::from_bytes(&[0; 16]).is_err());
        assert!(OperationId::from_bytes(&[0; 16]).is_err());
        let (client, _driver) = raw_client(&pg_config(&fixture.database).expect("config")).await;
        let scope = df_imagery::owner_scope_bytes(&draft_owner());
        let sharing = vec![7u8; 32];
        let error = client
            .execute(
                "INSERT INTO imagery.image_job (job_id, owner_scope, sharing_identity, \
                 intent_revision, intent_bytes, config_revision, config_bytes, \
                 absolute_deadline_ms, state, revision, created_at_ms, updated_at_ms) \
                 VALUES ('\\x00000000000000000000000000000000', $1, $2, 1, '\\x01', 1, \
                 '\\x01', 9000, 'planning', 1, 1000, 1000)",
                &[&scope, &sharing],
            )
            .await
            .expect_err("zero job_id must fail");
        assert_eq!(
            error.as_db_error().map(|db| db.code().code()),
            Some("23514"),
            "expected check_violation for zero job_id"
        );
        assert_eq!(fixture.rows("image_job").await, 0);
        fixture.finish().await;
    });
}

#[test]
fn schema_rejects_dangling_cross_owner_and_wrong_job_links() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create job one");
        fixture
            .journal
            .accept(&create(draft_owner(), 2, [2; 32], 0x10, 2, 2, 9_000, 1_000))
            .await
            .expect("create job two");
        let (mut client, _driver) =
            raw_client(&pg_config(&fixture.database).expect("config")).await;
        let scope = df_imagery::owner_scope_bytes(&draft_owner());
        let other_scope = df_imagery::owner_scope_bytes(&other_owner());
        let job1 = job(1).as_bytes().to_vec();
        let unknown_job = job(9).as_bytes().to_vec();
        let unknown_interest = interest(9).as_bytes().to_vec();
        let unknown_operation = operation(9).as_bytes().to_vec();
        // Dangling job reference fails.
        let dangling = client
            .execute(
                "INSERT INTO imagery.image_operation (owner_scope, operation_id, fingerprint, \
                 job_id, interest_id, created_at_ms) VALUES ($1, $2, '\\x0101010101010101010101010101010101010101010101010101010101010101', $3, $4, 1000)",
                &[&scope, &unknown_operation, &unknown_job, &unknown_interest],
            )
            .await
            .expect_err("dangling job link must fail");
        assert_eq!(
            dangling.as_db_error().map(|db| db.code().code()),
            Some("23503"),
            "expected foreign_key_violation for dangling job"
        );
        // Cross-owner job reference fails: job one belongs to the draft scope.
        let cross_owner = client
            .execute(
                "INSERT INTO imagery.image_operation (owner_scope, operation_id, fingerprint, \
                 job_id, interest_id, created_at_ms) VALUES ($1, $2, '\\x0101010101010101010101010101010101010101010101010101010101010101', $3, $4, 1000)",
                &[&other_scope, &unknown_operation, &job1, &unknown_interest],
            )
            .await
            .expect_err("cross-owner job link must fail");
        assert_eq!(
            cross_owner.as_db_error().map(|db| db.code().code()),
            Some("23503"),
            "expected foreign_key_violation for cross-owner link"
        );
        // Wrong-job pairing fails at commit: the operation names job one while its interest
        // row names job two, so the both-directions link cannot match.
        let job2 = job(2).as_bytes().to_vec();
        let tx = client.transaction().await.expect("transaction");
        tx.execute(
            "INSERT INTO imagery.image_operation (owner_scope, operation_id, fingerprint, \
             job_id, interest_id, created_at_ms) VALUES ($1, $2, '\\x0101010101010101010101010101010101010101010101010101010101010101', $3, $4, 1000)",
            &[&scope, &unknown_operation, &job1, &unknown_interest],
        )
        .await
        .expect("stage operation");
        tx.execute(
            "INSERT INTO imagery.image_interest (interest_id, owner_scope, job_id, \
             operation_id, state, created_at_ms) VALUES ($1, $2, $3, $4, 'attached', 1000)",
            &[&unknown_interest, &scope, &job2, &unknown_operation],
        )
        .await
        .expect("stage interest");
        let commit = tx.commit().await.expect_err("wrong-job commit must fail");
        assert_eq!(
            commit.as_db_error().map(|db| db.code().code()),
            Some("23503"),
            "expected foreign_key_violation for wrong-job link"
        );
        assert_eq!(fixture.rows("image_operation").await, 2);
        assert_eq!(fixture.rows("image_interest").await, 2);
        fixture.finish().await;
    });
}

#[test]
fn phantom_extra_interest_for_committed_operation_same_job_fails_commit() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create");
        let (mut client, _driver) =
            raw_client(&pg_config(&fixture.database).expect("config")).await;
        let scope = df_imagery::owner_scope_bytes(&draft_owner());
        let job1 = job(1).as_bytes().to_vec();
        let op1 = operation(1).as_bytes().to_vec();
        let phantom = interest(9).as_bytes().to_vec();
        let tx = client.transaction().await.expect("transaction");
        tx.execute(
            "INSERT INTO imagery.image_interest (interest_id, owner_scope, job_id, \
             operation_id, state, created_at_ms) VALUES ($1, $2, $3, $4, 'attached', 1000)",
            &[&phantom, &scope, &job1, &op1],
        )
        .await
        .expect("stage phantom interest");
        let commit = tx.commit().await.expect_err("phantom interest commit must fail");
        assert_eq!(
            commit.as_db_error().map(|db| db.code().code()),
            Some("23503"),
            "expected foreign_key_violation for phantom interest"
        );
        assert_eq!(fixture.rows("image_operation").await, 1);
        assert_eq!(fixture.rows("image_interest").await, 1);
        let withdrawn = fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_100)
            .await
            .expect("withdraw true interest");
        let WithdrawOutcome::Withdrawn(last) = withdrawn else {
            panic!("expected Withdrawn");
        };
        assert_eq!(last.attached_remaining, 0);
        fixture.finish().await;
    });
}

#[test]
fn phantom_extra_interest_for_committed_operation_other_job_fails_commit() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        fixture
            .journal
            .accept(&create(draft_owner(), 1, [1; 32], 0x10, 1, 1, 9_000, 1_000))
            .await
            .expect("create job one");
        fixture
            .journal
            .accept(&create(draft_owner(), 2, [2; 32], 0x10, 2, 2, 9_000, 1_000))
            .await
            .expect("create job two");
        let (mut client, _driver) =
            raw_client(&pg_config(&fixture.database).expect("config")).await;
        let scope = df_imagery::owner_scope_bytes(&draft_owner());
        let job2 = job(2).as_bytes().to_vec();
        let op1 = operation(1).as_bytes().to_vec();
        let phantom = interest(9).as_bytes().to_vec();
        let tx = client.transaction().await.expect("transaction");
        tx.execute(
            "INSERT INTO imagery.image_interest (interest_id, owner_scope, job_id, \
             operation_id, state, created_at_ms) VALUES ($1, $2, $3, $4, 'attached', 1000)",
            &[&phantom, &scope, &job2, &op1],
        )
        .await
        .expect("stage phantom interest");
        let commit = tx.commit().await.expect_err("phantom interest commit must fail");
        assert_eq!(
            commit.as_db_error().map(|db| db.code().code()),
            Some("23503"),
            "expected foreign_key_violation for phantom interest"
        );
        assert_eq!(fixture.rows("image_operation").await, 2);
        assert_eq!(fixture.rows("image_interest").await, 2);
        let first = fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(1), 1_100)
            .await
            .expect("withdraw job one");
        let WithdrawOutcome::Withdrawn(first) = first else {
            panic!("expected Withdrawn");
        };
        assert_eq!(first.attached_remaining, 0);
        let second = fixture
            .journal
            .withdraw_interest(&draft_owner(), interest(2), 1_100)
            .await
            .expect("withdraw job two");
        let WithdrawOutcome::Withdrawn(second) = second else {
            panic!("expected Withdrawn");
        };
        assert_eq!(second.attached_remaining, 0);
        fixture.finish().await;
    });
}

#[test]
fn journal_statements_require_read_committed() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let (mut client, _driver) =
            raw_client(&pg_config(&fixture.database).expect("config")).await;
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::Serializable)
            .start()
            .await
            .expect("serializable transaction");
        let demand = create(draft_owner(), 9, [9; 32], 0x10, 9, 9, 9_000, 1_000);
        let refused = df_persistence::accept_in(&tx, &demand).await;
        assert_eq!(refused, Err(JournalError::Unavailable));
        drop(tx);
        // The public adapter still serves at READ COMMITTED on the same database.
        fixture
            .journal
            .accept(&create(draft_owner(), 9, [9; 32], 0x10, 9, 9, 9_000, 1_000))
            .await
            .expect("adapter stays read committed");
        fixture.finish().await;
    });
}

#[test]
fn explicit_read_committed_survives_altered_default() {
    run(async {
        let Some(fixture) = Fixture::open().await else {
            return;
        };
        let (client, _driver) = raw_client(&pg_config(&fixture.database).expect("config")).await;
        client
            .batch_execute(&format!(
                "ALTER DATABASE {} SET DEFAULT_TRANSACTION_ISOLATION TO 'serializable'",
                fixture.database
            ))
            .await
            .expect("alter default isolation");
        // A fresh adapter with fresh sessions still serves: its builder names READ COMMITTED.
        let reopened = PostgresImageJournal::open(
            pg_config(&fixture.database).expect("config"),
            PostgresTransport::PlaintextLoopbackOnly,
            1,
        )
        .await
        .expect("reopen journal");
        reopened
            .accept(&create(draft_owner(), 7, [7; 32], 0x10, 7, 7, 9_000, 1_000))
            .await
            .expect("accept under altered default");
        client
            .batch_execute(&format!(
                "ALTER DATABASE {} RESET DEFAULT_TRANSACTION_ISOLATION",
                fixture.database
            ))
            .await
            .expect("reset default isolation");
        fixture.finish().await;
    });
}
