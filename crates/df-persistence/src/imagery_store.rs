//! PostgreSQL adapter for `df_imagery::ImageJournal` (migration 0038). PostgreSQL is
//! the authority for whether an operation, job or interest exists. Every trait method is one
//! transaction; queries are parameterized; a failure before commit is `Unavailable` (or the
//! typed refusal the row state demands) and a failed commit is `Unknown`. The
//! statements live in `imagery_sql`, taking the caller's transaction, so later catalog and
//! finance composition can call the functions below in its own transaction.

use std::sync::Arc;

use df_imagery::{
    AcceptCommand, AcceptOutcome, FreezeCommand, FreezeOutcome, ImageJobId, ImageJournal,
    JournalError, OperationView, WithdrawOutcome,
};
use df_types::OperationId;
use tokio_postgres::{Config, Transaction};

use crate::imagery_sql as sql;
use crate::media_pool::Pool;
use crate::postgres_tls::PostgresTransport;

const MIGRATIONS: [(i32, &str); 1] = [(
    38,
    include_str!("../migrations/0038_imagery_operations.sql"),
)];

/// Opening the journal failed; nothing was served.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageJournalOpenError {
    Transport(crate::postgres_tls::TransportRefusal),
    Unreachable,
    MigrationFailed,
}

/// Atomically accepts demand in the caller's transaction: the receipt lookup or creation plus
/// the interest row. The caller retains this transaction until it commits or rolls back.
pub async fn accept_in(
    tx: &Transaction<'_>,
    command: &AcceptCommand,
) -> Result<AcceptOutcome, JournalError> {
    sql::accept_in(tx, command).await
}

/// Reads the receipt with its current job and interest in the caller's transaction.
pub async fn lookup_in(
    tx: &Transaction<'_>,
    owner: &df_imagery::ImageOwner,
    operation: OperationId,
) -> Result<Option<OperationView>, JournalError> {
    sql::lookup_in(tx, owner, operation).await
}

/// Reads the stored job in the caller's transaction.
pub async fn read_in(
    tx: &Transaction<'_>,
    owner: &df_imagery::ImageOwner,
    job: ImageJobId,
) -> Result<Option<df_imagery::ImageJob>, JournalError> {
    sql::read_in(tx, owner, &job).await
}

/// Freezes the job's recipe in the caller's transaction.
pub async fn freeze_in(
    tx: &Transaction<'_>,
    command: &FreezeCommand,
) -> Result<FreezeOutcome, JournalError> {
    sql::freeze_in(tx, command).await
}

/// Withdraws the interest in the caller's transaction, retiring the job on the last one.
pub async fn withdraw_in(
    tx: &Transaction<'_>,
    owner: &df_imagery::ImageOwner,
    interest: df_imagery::ImageInterestId,
    now_ms: i64,
) -> Result<WithdrawOutcome, JournalError> {
    sql::withdraw_in(tx, owner, &interest, now_ms).await
}

/// The PostgreSQL imagery journal. Clones share one connection pool.
#[derive(Clone)]
pub struct PostgresImageJournal {
    pool: Arc<Pool>,
}

impl PostgresImageJournal {
    pub async fn open(
        config: Config,
        transport: PostgresTransport,
        connections: usize,
    ) -> Result<Self, ImageJournalOpenError> {
        crate::postgres_tls::check_transport(&config, &transport)
            .map_err(ImageJournalOpenError::Transport)?;
        let pool = Pool::new(config, transport, connections);
        {
            let mut slot = pool
                .checkout()
                .await
                .map_err(|_| ImageJournalOpenError::Unreachable)?;
            let client = slot
                .client
                .as_mut()
                .ok_or(ImageJournalOpenError::Unreachable)?;
            migrate(client)
                .await
                .map_err(|_| ImageJournalOpenError::MigrationFailed)?;
        }
        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// A failed commit is `Unknown`: the writes may or may not have taken effect, so the
    /// caller must reread before retrying. Every statement error beforehand is already mapped by
    /// the statements to `Unavailable` or a typed refusal, which changed nothing. This covers
    /// only the store commit, not duplicate dispatch safety beyond this store.
    async fn commit(transaction: Transaction<'_>) -> Result<(), JournalError> {
        transaction.commit().await.map_err(|error| {
            sql::record_failure(&error);
            JournalError::Unknown
        })
    }
}

/// Applies the journal migration once, serialised by an advisory lock.
async fn migrate(client: &mut tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    let transaction = client.transaction().await?;
    transaction
        .batch_execute(
            "SELECT pg_advisory_xact_lock(hashtext('df_imagery_migration')); \
             CREATE SCHEMA IF NOT EXISTS imagery; \
             CREATE TABLE IF NOT EXISTS imagery.schema_migration ( \
                 version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())",
        )
        .await?;
    for (version, script) in MIGRATIONS {
        let applied = transaction
            .query_opt(
                "SELECT 1 FROM imagery.schema_migration WHERE version = $1",
                &[&version],
            )
            .await?
            .is_some();
        if !applied {
            transaction.batch_execute(script).await?;
            transaction
                .execute(
                    "INSERT INTO imagery.schema_migration (version) VALUES ($1)",
                    &[&version],
                )
                .await?;
        }
    }
    transaction.commit().await
}

/// Runs `$body` (an expression using `.await` and `?`) in one transaction bound to `$tx`; an early
/// return drops the transaction, which rolls it back. The builder names READ COMMITTED
/// explicitly so an altered session or database default cannot silently escalate isolation.
macro_rules! in_transaction {
    ($self:expr, $tx:ident => $body:expr) => {{
        let mut slot = $self
            .pool
            .checkout()
            .await
            .map_err(|_| JournalError::Unavailable)?;
        let client = slot.client.as_mut().ok_or(JournalError::Unavailable)?;
        let $tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(|_| JournalError::Unavailable)?;
        let value = $body;
        Self::commit($tx).await?;
        Ok(value)
    }};
}

impl ImageJournal for PostgresImageJournal {
    async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
        in_transaction!(self, tx => accept_in(&tx, command).await?)
    }

    async fn lookup_operation(
        &self,
        owner: &df_imagery::ImageOwner,
        operation: OperationId,
    ) -> Result<Option<OperationView>, JournalError> {
        in_transaction!(self, tx => lookup_in(&tx, owner, operation).await?)
    }

    async fn read_job(
        &self,
        owner: &df_imagery::ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<df_imagery::ImageJob>, JournalError> {
        in_transaction!(self, tx => read_in(&tx, owner, job).await?)
    }

    async fn freeze_recipe(&self, command: &FreezeCommand) -> Result<FreezeOutcome, JournalError> {
        in_transaction!(self, tx => freeze_in(&tx, command).await?)
    }

    async fn withdraw_interest(
        &self,
        owner: &df_imagery::ImageOwner,
        interest: df_imagery::ImageInterestId,
        now_ms: i64,
    ) -> Result<WithdrawOutcome, JournalError> {
        in_transaction!(self, tx => withdraw_in(&tx, owner, interest, now_ms).await?)
    }
}

/// The pre-commit kill path: a backend terminated after staging but before the commit is
/// sent must surface `Unknown` through the adapter's own `commit`, and the staged rows must be
/// gone on a fresh connection. Termination plus a bounded poll keeps this deterministic; there
/// are no sleeps and no application fault-injection seam. This proves only the store commit
/// mapping, not applied/acknowledged-loss recovery or duplicate dispatch safety.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod commit_tests {
    use super::*;
    use df_imagery::{AcceptTarget, ImageInterestId, SharingIdentity, VersionedSnapshot};
    use df_types::{AccountId, OperationId};
    use tokio_postgres::{Client, NoTls};

    const MAXIMUM_POLLS: usize = 200_000;

    fn scratch_config(dbname: &str) -> Option<Config> {
        let Ok(port) = std::env::var("DUNGEONFLUX_RUNTIME_PG_PORT") else {
            assert!(
                std::env::var_os("DUNGEONFLUX_REQUIRE_SCRATCH_PG").is_none(),
                "DUNGEONFLUX_REQUIRE_SCRATCH_PG is set but DUNGEONFLUX_RUNTIME_PG_PORT is not"
            );
            eprintln!("SKIPPED: DUNGEONFLUX_RUNTIME_PG_PORT must name an owned scratch PostgreSQL");
            return None;
        };
        let mut config = Config::new();
        config
            .host(std::env::var("DUNGEONFLUX_RUNTIME_PG_HOST").unwrap_or("127.0.0.1".into()))
            .port(port.parse().expect("scratch port"))
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

    fn demand() -> AcceptCommand {
        let owner = df_imagery::ImageOwner::Draft {
            account: AccountId::from_hex("00000000000000000000000000000011").unwrap(),
        };
        AcceptCommand {
            identity: df_imagery::OperationIdentity {
                owner,
                operation: OperationId::from_hex("000000000000000000000000000000cc").unwrap(),
                fingerprint: [5; 32],
            },
            intent: VersionedSnapshot::new(1, vec![9; 16], 64).unwrap(),
            config: VersionedSnapshot::new(1, vec![8; 16], 64).unwrap(),
            sharing: SharingIdentity::from_bytes(&[7; 32]).unwrap(),
            interest: ImageInterestId::from_hex("000000000000000000000000000000bb").unwrap(),
            target: AcceptTarget::Create {
                job: ImageJobId::from_hex("000000000000000000000000000000aa").unwrap(),
                absolute_deadline_ms: 9_000,
            },
            now_ms: 1_000,
        }
    }

    #[test]
    fn severed_commit_surfaces_unknown_and_rolls_back() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database = format!("df_imagery_commit_{nanos}_{}", std::process::id());
        let Some(admin) = scratch_config("postgres") else {
            return;
        };
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let (admin_client, _admin_driver) = raw_client(&admin).await;
                admin_client
                    .batch_execute(&format!("CREATE DATABASE {database}"))
                    .await
                    .expect("create test database");
                let config = scratch_config(&database).expect("scratch config");
                let (setup_client, _setup_driver) = raw_client(&config).await;
                setup_client
                    .batch_execute(include_str!("../migrations/0038_imagery_operations.sql"))
                    .await
                    .expect("imagery migration");
                let journal = PostgresImageJournal::open(
                    config.clone(),
                    PostgresTransport::PlaintextLoopbackOnly,
                    1,
                )
                .await
                .expect("open journal");
                let mut slot = journal.pool.checkout().await.expect("pool checkout");
                let client = slot.client.as_mut().expect("pooled client");
                let tx = client.transaction().await.expect("transaction");
                let staged = accept_in(&tx, &demand()).await.expect("stage demand");
                assert!(matches!(staged, AcceptOutcome::Created(_)));
                let pid: i32 = tx
                    .query_one("SELECT pg_backend_pid()", &[])
                    .await
                    .expect("backend pid")
                    .get(0);
                setup_client
                    .query_one("SELECT pg_terminate_backend($1)", &[&pid])
                    .await
                    .expect("terminate the staging backend");
                let mut gone = false;
                for _ in 0..MAXIMUM_POLLS {
                    let count: i64 = setup_client
                        .query_one(
                            "SELECT COUNT(*) FROM pg_stat_activity WHERE pid = $1",
                            &[&pid],
                        )
                        .await
                        .expect("poll backends")
                        .get(0);
                    if count == 0 {
                        gone = true;
                        break;
                    }
                }
                assert!(gone, "terminated backend stayed visible");
                let committed = PostgresImageJournal::commit(tx).await;
                assert_eq!(committed, Err(JournalError::Unknown));
                drop(slot);
                drop(journal);
                let jobs: i64 = setup_client
                    .query_one("SELECT COUNT(*) FROM imagery.image_job", &[])
                    .await
                    .expect("reread jobs")
                    .get(0);
                assert_eq!(jobs, 0);
                admin_client
                    .batch_execute(&format!("DROP DATABASE IF EXISTS {database} WITH (FORCE)"))
                    .await
                    .expect("drop test database");
            });
    }
}
