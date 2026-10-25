//! Connection drivers remain owned; migrations are serialized and recorded transactionally.

use std::time::Duration;

use df_persistence::postgres_tls::{PostgresTransport, connect};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_postgres::{Client, Config};

pub(crate) struct Connection {
    pub(crate) client: Client,
    pub(crate) driver: Driver,
}

pub(crate) struct Driver(JoinHandle<()>);

impl Drop for Driver {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Connection {
    pub(crate) async fn open(
        config: &Config,
        transport: &PostgresTransport,
    ) -> Result<Self, DatabaseError> {
        let (client, connection) =
            tokio::time::timeout(Duration::from_secs(5), connect(config, transport))
                .await
                .map_err(|_| DatabaseError::Connect)?
                .map_err(|_| DatabaseError::Connect)?;
        let driver = tokio::spawn(async move {
            if connection.await.is_err() {
                df_observe::record_audit_line(
                    "hosted.database_connection_ended",
                    "HOSTED database_connection_ended".to_owned(),
                );
            }
        });
        let owned = Self {
            client,
            driver: Driver(driver),
        };
        owned
            .client
            .batch_execute(
                "SET statement_timeout = '5s'; SET lock_timeout = '5s'; \
             SET idle_in_transaction_session_timeout = '10s'",
            )
            .await
            .map_err(DatabaseError::Query)?;
        Ok(owned)
    }
}

#[derive(Debug)]
pub(crate) enum DatabaseError {
    Connect,
    Query(tokio_postgres::Error),
    Migration {
        number: i32,
        cause: tokio_postgres::Error,
    },
}

#[derive(Clone)]
pub(crate) struct Connector {
    pub(crate) config: Config,
    pub(crate) transport: PostgresTransport,
}

impl crate::campaign_store::Connect for Connector {
    fn connect(&self) -> crate::campaign_store::ConnectFuture<'_> {
        Box::pin(async move {
            let (client, connection) = tokio::time::timeout(
                Duration::from_secs(5),
                connect(&self.config, &self.transport),
            )
            .await
            .map_err(|_| df_campaign::errors::StoreError::Unavailable)?
            .map_err(|_| df_campaign::errors::StoreError::Unavailable)?;
            let driver = tokio::spawn(async move {
                if connection.await.is_err() {
                    df_observe::record_audit_line(
                        "hosted.campaign_connection_ended",
                        "HOSTED campaign_connection_ended".to_owned(),
                    );
                }
            });
            Ok((client, driver))
        })
    }
}

/// This process serializes its startup; the database lock also protects concurrent instances.
static MIGRATION_LOCK: Mutex<()> = Mutex::const_new(());

pub(crate) async fn migrate(connection: &mut Connection) -> Result<(), DatabaseError> {
    apply(connection, MIGRATIONS).await
}

pub(crate) async fn apply(
    connection: &mut Connection,
    migrations: &[(i32, &str)],
) -> Result<(), DatabaseError> {
    let _local = MIGRATION_LOCK.lock().await;
    let transaction = connection
        .client
        .transaction()
        .await
        .map_err(DatabaseError::Query)?;
    transaction.batch_execute(
        "SELECT pg_advisory_xact_lock(7000021); \
         CREATE SCHEMA IF NOT EXISTS hosted; \
         CREATE TABLE IF NOT EXISTS hosted.migration (number integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT clock_timestamp())",
    ).await.map_err(DatabaseError::Query)?;
    for (number, sql) in migrations {
        if transaction
            .query_opt(
                "SELECT number FROM hosted.migration WHERE number = $1",
                &[number],
            )
            .await
            .map_err(DatabaseError::Query)?
            .is_some()
        {
            continue;
        }
        // AccountStore owns its migration ledger; applying accounts here would create two ledgers.
        let payload = if *number == 1 {
            sql.strip_prefix("-- UNAPPLIED SOURCE DRAFT: consumer contracts, limits, driver and scope binding pending.\n-- No runtime policy grants access until the approved database scope resolver exists.\nBEGIN;\n")
                .and_then(|body| body.trim_end().strip_suffix("COMMIT;"))
                .ok_or(DatabaseError::Connect)?
        } else {
            sql
        };
        transaction
            .batch_execute(payload)
            .await
            .map_err(|cause| DatabaseError::Migration {
                number: *number,
                cause,
            })?;
        transaction
            .execute(
                "INSERT INTO hosted.migration(number) VALUES ($1)",
                &[number],
            )
            .await
            .map_err(DatabaseError::Query)?;
    }
    transaction.commit().await.map_err(DatabaseError::Query)
}

const MIGRATIONS: &[(i32, &str)] = &[
    (
        1,
        include_str!("../../df-persistence/migrations/0001_game_decisions.sql"),
    ),
    (
        3,
        include_str!("../../df-persistence/migrations/0003_story_memory.sql"),
    ),
    (
        8,
        include_str!("../../df-persistence/migrations/0008_campaigns.sql"),
    ),
    (
        9,
        include_str!("../../df-persistence/migrations/0009_lobby.sql"),
    ),
    (
        10,
        include_str!("../../df-persistence/migrations/0010_media_catalog.sql"),
    ),
    (
        11,
        include_str!("../../df-persistence/migrations/0011_character_library.sql"),
    ),
    (
        12,
        include_str!("../../df-persistence/migrations/0012_media_exports.sql"),
    ),
    (
        13,
        include_str!("../../df-persistence/migrations/0013_recovery_journal.sql"),
    ),
    (
        14,
        include_str!("../../df-persistence/migrations/0014_media_sweep.sql"),
    ),
    (
        15,
        include_str!("../../df-persistence/migrations/0015_runtime_directory.sql"),
    ),
    (
        16,
        include_str!("../../df-persistence/migrations/0016_table_sessions.sql"),
    ),
    (
        18,
        include_str!("../../df-persistence/migrations/0018_runtime_roles.sql"),
    ),
    (
        20,
        include_str!("../../df-persistence/migrations/0020_finance_journal.sql"),
    ),
    (
        21,
        include_str!("../../df-persistence/migrations/0021_finance_funding.sql"),
    ),
    (
        22,
        include_str!("../../df-persistence/migrations/0022_finance_admission.sql"),
    ),
    (
        23,
        include_str!("../../df-persistence/migrations/0023_finance_refunds_disputes.sql"),
    ),
    (
        24,
        include_str!("../../df-persistence/migrations/0024_finance_boundary.sql"),
    ),
];
