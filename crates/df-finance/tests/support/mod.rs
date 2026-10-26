//! Scratch-database harness for the finance tests. Each test gets a new database named
//! `df_finance_refundstate_<time>_<pid>_<serial>` on the owned scratch server named by
//! `DUNGEONFLUX_FINANCE_PG_PORT` (host `DUNGEONFLUX_FINANCE_PG_HOST`, default 127.0.0.1; user
//! `DUNGEONFLUX_FINANCE_PG_USER`, default postgres) and drops only that database at the end.
//! Without the port the test reports SKIPPED. Tests connect as the server's superuser and act as
//! a finance role with `SET ROLE`, so definer functions and privileges run for real.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the harness"
)]

use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use tokio_postgres::{Client, Config, NoTls, Row, types::ToSql};

pub mod attack;
pub mod catalog;
pub mod invariants;
pub mod mutants;
pub mod scenario;
pub mod world;

/// Never these ports: the game's and the other owned test servers.
const FORBIDDEN_PORTS: [u16; 4] = [0, 55517, 55439, 63309];

pub const MIGRATIONS: [(&str, &str); 6] = [
    (
        "0020",
        include_str!("../../../df-persistence/migrations/0020_finance_journal.sql"),
    ),
    (
        "0021",
        include_str!("../../../df-persistence/migrations/0021_finance_funding.sql"),
    ),
    (
        "0022",
        include_str!("../../../df-persistence/migrations/0022_finance_admission.sql"),
    ),
    (
        "0023",
        include_str!("../../../df-persistence/migrations/0023_finance_refunds_disputes.sql"),
    ),
    (
        "0024",
        include_str!("../../../df-persistence/migrations/0024_finance_boundary.sql"),
    ),
    (
        "0025",
        include_str!(
            "../../../df-persistence/migrations/0025_finance_refunds_disputes_boundary.sql"
        ),
    ),
];

/// Roles are cluster-wide, so concurrent test databases apply the migrations one at a time.
static MIGRATING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn run<T>(test: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(test)
}

pub struct Scratch {
    admin: Config,
    database: String,
}

impl Scratch {
    /// A new migrated database, or `None` (SKIPPED) without a scratch server. PostgreSQL reports
    /// "tuple concurrently updated" when role-level statements of parallel migrations meet; the
    /// database is then dropped and made again, a few times.
    pub async fn open() -> Option<Self> {
        for attempt in 1..=6 {
            let scratch = Self::open_unmigrated().await?;
            match scratch.migrate().await {
                Ok(()) => return Some(scratch),
                Err((_, error))
                    if attempt < 6
                        && error.as_db_error().is_some_and(|db| {
                            db.message().contains("tuple concurrently updated")
                        }) =>
                {
                    scratch.finish().await;
                }
                Err((name, error)) => panic!("migration {name}: {error:?}"),
            }
        }
        None
    }

    async fn migrate(&self) -> Result<(), (&'static str, tokio_postgres::Error)> {
        let client = self.connect().await;
        let _migrating = MIGRATING.lock().await;
        // Roles and their settings are cluster-wide, and test processes run in parallel: one
        // migration at a time per cluster, held on a connection to the maintenance database.
        let gate = connect(&self.admin).await;
        gate.execute("SELECT pg_advisory_lock(7000021)", &[])
            .await
            .expect("migration gate");
        let mut outcome = Ok(());
        for (name, sql) in MIGRATIONS {
            if let Err(error) = client.batch_execute(sql).await {
                outcome = Err((name, error));
                break;
            }
        }
        gate.execute("SELECT pg_advisory_unlock(7000021)", &[])
            .await
            .expect("migration gate release");
        outcome
    }

    pub async fn open_unmigrated() -> Option<Self> {
        let Ok(port) = std::env::var("DUNGEONFLUX_FINANCE_PG_PORT") else {
            eprintln!("SKIPPED: DUNGEONFLUX_FINANCE_PG_PORT must name an owned scratch PostgreSQL");
            return None;
        };
        let port: u16 = port.parse().expect("scratch port");
        assert!(!FORBIDDEN_PORTS.contains(&port));
        let mut admin = Config::new();
        admin
            .host(std::env::var("DUNGEONFLUX_FINANCE_PG_HOST").unwrap_or("127.0.0.1".into()))
            .port(port)
            .user(std::env::var("DUNGEONFLUX_FINANCE_PG_USER").unwrap_or("postgres".into()))
            .dbname("postgres");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let database = format!(
            "df_finance_refundstate_{nanos}_{}_{serial}",
            std::process::id()
        );
        connect(&admin)
            .await
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .expect("create test database");
        Some(Self { admin, database })
    }

    /// The connection settings of the test database, for a test that logs in as another role.
    pub fn config(&self) -> Config {
        let mut config = self.admin.clone();
        config.dbname(&self.database);
        config
    }

    /// A new connection to the test database as the server's superuser.
    pub async fn connect(&self) -> Client {
        let mut config = self.admin.clone();
        config.dbname(&self.database);
        connect(&config).await
    }

    pub async fn finish(self) {
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

async fn connect(config: &Config) -> Client {
    let (client, connection) = config.connect(NoTls).await.expect("scratch server");
    tokio::spawn(async move {
        let _ended = connection.await;
    });
    client
}

/// The SQLSTATE of a database error, or `None` for any other failure.
pub fn sqlstate(error: &tokio_postgres::Error) -> Option<String> {
    error.as_db_error().map(|db| db.code().code().to_string())
}

/// Runs one statement as `role`, then drops back to the superuser.
pub async fn as_role(
    client: &Client,
    role: &str,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, tokio_postgres::Error> {
    client.batch_execute(&format!("SET ROLE {role}")).await?;
    let rows = client.query(sql, params).await;
    client.batch_execute("RESET ROLE").await?;
    rows
}
