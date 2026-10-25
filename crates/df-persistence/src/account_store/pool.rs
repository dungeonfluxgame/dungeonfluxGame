//! The account store's connections, opening, migration and the `with_client!` checkout.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use df_customer::{StoreError, StoreResult};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_postgres::{Client, Config};

use crate::{
    account_closure::{self, ClosureParticipant, TimeoutPolicy, TimeoutRefusal},
    postgres_tls::{PostgresTransport, TransportRefusal, connect},
};

/// The migrations this store owns, in order (`planning/account-service-research.md` 5.1).
pub(super) const MIGRATIONS: [(i32, &str); 2] = [
    (5, include_str!("../../migrations/0005_accounts.sql")),
    (6, include_str!("../../migrations/0006_account_closure.sql")),
];
pub(super) const CONNECT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);
/// The account tables live in schema `accounts` of the shared hosted database (coordinator
/// launch decision 7: one database, a schema per owner).
pub(super) const SESSION_SETTINGS: &str = "SET statement_timeout = '5s'; SET lock_timeout = '5s'; \
    SET idle_in_transaction_session_timeout = '10s'; SET search_path TO accounts";
/// Postgres SQLSTATE for a unique-constraint violation.
pub(super) const UNIQUE_VIOLATION: &str = "23505";

/// Opening the store failed; nothing was migrated or the database was unreachable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountStoreOpenError {
    /// The transport policy refused the configuration (plaintext to a remote host, bad CA).
    Transport(TransportRefusal),
    Unreachable,
    MigrationFailed,
    /// The role's `transaction_timeout` default is not set, or is longer than the policy allows.
    TransactionTimeout(TimeoutRefusal),
    /// `TimeoutPolicy::NotChecked` was asked for a database that is not on this machine. The
    /// check is skipped only for local development.
    UncheckedTimeoutOnRemoteHost,
}

pub(super) struct Slot {
    pub(super) client: Option<Client>,
    pub(super) driver: Option<JoinHandle<()>>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(driver) = self.driver.take() {
            driver.abort();
        }
    }
}

/// A fixed set of connections, each opened on first use and reopened after it closes.
pub(super) struct Pool {
    config: Config,
    transport: PostgresTransport,
    slots: Vec<Mutex<Slot>>,
    next: AtomicUsize,
}

pub(super) async fn begin_transaction(
    client: &mut Client,
) -> Result<tokio_postgres::Transaction<'_>, tokio_postgres::Error> {
    let transaction = client.transaction().await?;
    transaction
        .batch_execute("SET LOCAL transaction_timeout = '10s'")
        .await?;
    Ok(transaction)
}

impl Pool {
    pub(super) async fn connect(
        config: &Config,
        transport: &PostgresTransport,
    ) -> StoreResult<(Client, JoinHandle<()>)> {
        let (client, connection) =
            tokio::time::timeout(CONNECT_DEADLINE, connect(config, transport))
                .await
                .map_err(|_| StoreError::Unavailable)?
                .map_err(|_| StoreError::Unavailable)?;
        // The driver ends when the connection does; the closed client is replaced at the next
        // checkout. The end is audited (without the error text, which may name the server).
        let driver = tokio::spawn(async move {
            if connection.await.is_err() {
                df_observe::record_audit_line(
                    "account.store_connection_ended",
                    "ACCOUNT store_connection_ended".to_owned(),
                );
            }
        });
        client
            .batch_execute(SESSION_SETTINGS)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok((client, driver))
    }

    pub(super) async fn checkout(&self) -> StoreResult<tokio::sync::MutexGuard<'_, Slot>> {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.slots.len();
        let mut slot = self.slots[index].lock().await;
        if slot.client.as_ref().is_none_or(Client::is_closed) {
            let (client, driver) = Self::connect(&self.config, &self.transport).await?;
            if let Some(old) = slot.driver.replace(driver) {
                old.abort();
            }
            slot.client = Some(client);
        }
        Ok(slot)
    }
}

/// The PostgreSQL account store. Clones share one pool.
#[derive(Clone)]
pub struct PostgresAccountStore {
    pub(super) pool: Arc<Pool>,
    pub(super) closure: Option<Arc<dyn ClosureParticipant>>,
}

impl PostgresAccountStore {
    /// Connects under `transport`, applies the account migrations once (serialized by an
    /// advisory lock), checks the role's `transaction_timeout` default against `timeout` and
    /// keeps at most `connections` connections.
    pub async fn open(
        config: Config,
        transport: PostgresTransport,
        connections: usize,
        timeout: TimeoutPolicy,
    ) -> Result<Self, AccountStoreOpenError> {
        crate::postgres_tls::check_transport(&config, &transport)
            .map_err(AccountStoreOpenError::Transport)?;
        if timeout == TimeoutPolicy::NotChecked && !crate::postgres_tls::all_hosts_loopback(&config)
        {
            return Err(AccountStoreOpenError::UncheckedTimeoutOnRemoteHost);
        }
        let pool = Pool {
            config,
            transport,
            slots: (0..connections.max(1))
                .map(|_| {
                    Mutex::new(Slot {
                        client: None,
                        driver: None,
                    })
                })
                .collect(),
            next: AtomicUsize::new(0),
        };
        {
            let mut slot = pool
                .checkout()
                .await
                .map_err(|_| AccountStoreOpenError::Unreachable)?;
            let client = slot
                .client
                .as_mut()
                .ok_or(AccountStoreOpenError::Unreachable)?;
            // A runtime role cannot (and need not) migrate: when every migration is recorded
            // the DDL is skipped, and the migrator role applied it.
            if !is_migrated(client)
                .await
                .map_err(|_| AccountStoreOpenError::Unreachable)?
            {
                migrate(client)
                    .await
                    .map_err(|_| AccountStoreOpenError::MigrationFailed)?;
            }
            // The role default, read on a fresh connection: a setting changed in a session or a
            // transaction does not count.
            let reported: String = client
                .query_one("SELECT current_setting('transaction_timeout')", &[])
                .await
                .map_err(|_| AccountStoreOpenError::Unreachable)?
                .try_get(0)
                .map_err(|_| AccountStoreOpenError::Unreachable)?;
            account_closure::check_timeout(timeout, &reported)
                .map_err(AccountStoreOpenError::TransactionTimeout)?;
        }
        Ok(Self {
            pool: Arc::new(pool),
            closure: None,
        })
    }

    /// Registers the campaign service's side of account closure. Hosted composition always
    /// does; without it no campaign exists as far as closure is concerned.
    pub fn with_closure_participant(mut self, participant: Arc<dyn ClosureParticipant>) -> Self {
        self.closure = Some(participant);
        self
    }

    pub(super) fn participant(&self) -> Option<&dyn ClosureParticipant> {
        self.closure.as_deref()
    }
}

async fn is_migrated(client: &Client) -> Result<bool, tokio_postgres::Error> {
    let table = client
        .query_one(
            "SELECT to_regclass('accounts.schema_migration') IS NOT NULL",
            &[],
        )
        .await?
        .try_get::<_, bool>(0)?;
    if !table {
        return Ok(false);
    }
    let applied: i64 = client
        .query_one("SELECT count(*) FROM accounts.schema_migration", &[])
        .await?
        .try_get(0)?;
    Ok(usize::try_from(applied).is_ok_and(|applied| applied >= MIGRATIONS.len()))
}

async fn migrate(client: &mut Client) -> Result<(), tokio_postgres::Error> {
    let transaction = client.transaction().await?;
    transaction
        .batch_execute(
            "SELECT pg_advisory_xact_lock(hashtext('df_accounts_migration')); \
             CREATE SCHEMA IF NOT EXISTS accounts; \
             CREATE TABLE IF NOT EXISTS accounts.schema_migration ( \
                 version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())",
        )
        .await?;
    for (version, sql) in MIGRATIONS {
        let applied = transaction
            .query_opt(
                "SELECT 1 FROM schema_migration WHERE version = $1",
                &[&version],
            )
            .await?
            .is_some();
        if !applied {
            transaction.batch_execute(sql).await?;
            transaction
                .execute(
                    "INSERT INTO schema_migration (version) VALUES ($1)",
                    &[&version],
                )
                .await?;
        }
    }
    transaction.commit().await
}

macro_rules! with_client {
    ($store:expr, $client:ident => $body:expr) => {{
        let mut slot = $store.pool.checkout().await?;
        let $client = slot
            .client
            .as_mut()
            .ok_or(df_customer::StoreError::Unavailable)?;
        $body
    }};
}
pub(super) use with_client;
