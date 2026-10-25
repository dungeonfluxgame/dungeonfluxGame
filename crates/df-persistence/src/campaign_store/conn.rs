//! Connections and transactions for the campaign store.
//!
//! A transaction is either the store's own (one pooled connection, explicit BEGIN and COMMIT) or
//! the caller's (B's close transaction, which the campaign owner's closure functions run inside).
//! Both give the same `Tx`, so the rules are written once. Queries are parameterized and
//! schema-qualified. A failure before commit is `Unavailable` or `Retryable` (nothing changed); a
//! failed or lost COMMIT is `Unknown`, never reported as failure or success.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use df_campaign::errors::{StoreError, StoreResult};
use tokio::{
    sync::{Mutex, OwnedMutexGuard},
    task::JoinHandle,
};
use tokio_postgres::{Client, Row, Transaction, types::ToSql};

pub(super) type Params<'a> = &'a [&'a (dyn ToSql + Sync)];

/// SQLSTATE of a deadlock.
const DEADLOCK: &str = "40P01";
/// SQLSTATE of a lock timeout.
const LOCK_NOT_AVAILABLE: &str = "55P03";
/// SQLSTATE of a statement timeout.
const QUERY_CANCELED: &str = "57014";
/// The code O1's `enter_service()` and `require_service()` raise during a halt.
const SERVICE_HALTING: &str = "DF002";
/// The commit-time check of the event transaction bound (migration 0008).
const EVENT_TRANSACTION_TOO_OLD: &str = "DF003";
/// The database refused an event row or a counter move that would break the feed's invariants.
const FEED_INVARIANT: &str = "DF004";

/// Settings applied to every pooled connection when it opens: statement and lock timeouts and an
/// idle-in-transaction bound. The transaction bound itself is the role default, checked at open.
fn session_settings(statement_ms: u32, lock_ms: u32) -> String {
    format!(
        "SET statement_timeout = '{statement_ms}ms'; SET lock_timeout = '{lock_ms}ms'; \n         SET idle_in_transaction_session_timeout = '10s'"
    )
}

/// The future a connector returns: the client and the task that drives its connection.
pub type ConnectFuture<'a> =
    Pin<Box<dyn Future<Output = StoreResult<(Client, JoinHandle<()>)>> + Send + 'a>>;

/// Opens one connection to the hosted database. The composition supplies the transport policy
/// (verified TLS, or loopback plaintext for development); this module does not choose it.
pub trait Connect: Send + Sync + 'static {
    fn connect(&self) -> ConnectFuture<'_>;
}

pub(super) struct Slot {
    client: Option<Client>,
    driver: Option<JoinHandle<()>>,
    /// Set between BEGIN and COMMIT or ROLLBACK. A slot found still open (a cancelled future)
    /// is discarded instead of reused.
    in_transaction: bool,
    /// Set when the server ended the session (a transaction timeout, a shutdown): the connection
    /// is replaced at the next checkout even if the driver has not noticed yet.
    broken: AtomicBool,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(driver) = self.driver.take() {
            driver.abort();
        }
    }
}

/// A fixed set of connections, each opened on first use and replaced after it closes. A
/// connection whose session the server terminated (a `transaction_timeout`) reports itself
/// closed and is discarded at the next checkout.
pub struct Pool {
    connector: Box<dyn Connect>,
    settings: String,
    slots: Vec<Arc<Mutex<Slot>>>,
    next: AtomicUsize,
}

impl Pool {
    /// The pool the write transactions use: statements may wait five seconds for a lock.
    pub fn new(connector: Box<dyn Connect>, size: usize) -> Self {
        Self::with_settings(connector, size, session_settings(5_000, 5_000))
    }

    /// The pool of the membership reads: a statement over `deadline_ms` fails (the reads fail
    /// closed) and a lock wait is short, so a read never queues behind a write's lock wait.
    pub fn for_reads(connector: Box<dyn Connect>, size: usize, deadline_ms: u32) -> Self {
        Self::with_settings(connector, size, session_settings(deadline_ms, deadline_ms))
    }

    fn with_settings(connector: Box<dyn Connect>, size: usize, settings: String) -> Self {
        Self {
            connector,
            settings,
            slots: (0..size.max(1))
                .map(|_| {
                    Arc::new(Mutex::new(Slot {
                        client: None,
                        driver: None,
                        in_transaction: false,
                        broken: AtomicBool::new(false),
                    }))
                })
                .collect(),
            next: AtomicUsize::new(0),
            // Slots are tried in turn so concurrent callers use different connections.
        }
    }

    async fn checkout(&self) -> StoreResult<OwnedMutexGuard<Slot>> {
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        let count = self.slots.len();
        let mut guard = None;
        for offset in 0..count {
            if let Ok(free) = self.slots[(start + offset) % count]
                .clone()
                .try_lock_owned()
            {
                guard = Some(free);
                break;
            }
        }
        let mut slot = match guard {
            Some(slot) => slot,
            None => self.slots[start % count].clone().lock_owned().await,
        };
        if slot.in_transaction
            || slot.broken.load(Ordering::Relaxed)
            || slot.client.as_ref().is_none_or(Client::is_closed)
        {
            if let Some(old) = slot.driver.take() {
                old.abort();
            }
            slot.client = None;
            slot.in_transaction = false;
            slot.broken.store(false, Ordering::Relaxed);
            let (client, driver) = self.connector.connect().await?;
            client
                .batch_execute(&self.settings)
                .await
                .map_err(|error| map_error(&error, false))?;
            slot.client = Some(client);
            slot.driver = Some(driver);
        }
        Ok(slot)
    }

    /// One statement outside a transaction block, for the single-statement reads (a snapshot read
    /// is one MVCC snapshot because it is one statement). The role default bounds it.
    pub(super) async fn read(&self, sql: &str, params: Params<'_>) -> StoreResult<Vec<Row>> {
        let slot = self.checkout().await?;
        client_of(&slot)
            .query(sql, params)
            .await
            .map_err(|error| map_error(&error, false))
    }

    /// Opens a fresh connection that is not pooled (for tests that need an independent session).
    pub async fn fresh_client(&self) -> StoreResult<(Client, JoinHandle<()>)> {
        self.connector.connect().await
    }
}

enum Link<'a> {
    Owned(OwnedMutexGuard<Slot>),
    Borrowed(&'a Transaction<'a>),
}

/// One transaction. The store's own is committed or rolled back by `commit` and `rollback`; the
/// caller's is committed by the caller and `commit` here does nothing.
pub(super) struct Tx<'a> {
    link: Link<'a>,
    /// Test only: take multi-campaign locks in descending order, the order that must deadlock
    /// against the specified one, so a test can show it would catch the mistake.
    #[cfg(test)]
    reversed_locks: bool,
}

fn client_of(slot: &Slot) -> &Client {
    // A slot is checked out with a client present; it is replaced before the guard is returned.
    slot.client
        .as_ref()
        .unwrap_or_else(|| unreachable!("a checked-out slot holds a client"))
}

impl Tx<'static> {
    /// Begins the store's own transaction on a pooled connection. `timeout_ms` is the second
    /// guard behind the role default (a per-transaction setting does not shorten a timer that
    /// is already running, so the startup check of the role default is the guarantee).
    pub(super) async fn begin(pool: &Pool, timeout_ms: u32) -> StoreResult<Self> {
        let mut slot = pool.checkout().await?;
        let client = client_of(&slot);
        // The second guard only ever shortens: a role default already at or below the bound (and
        // above zero) is kept, anything else (unset, or longer) becomes the bound.
        let statement = format!(
            "BEGIN; SELECT set_config('transaction_timeout', \
             least(nullif((SELECT setting::bigint FROM pg_settings \
             WHERE name = 'transaction_timeout'), 0), {timeout_ms})::text || 'ms', true)"
        );
        client
            .batch_execute(&statement)
            .await
            .map_err(|error| map_error(&error, false))?;
        slot.in_transaction = true;
        Ok(Self {
            link: Link::Owned(slot),
            #[cfg(test)]
            reversed_locks: false,
        })
    }
}

impl<'a> Tx<'a> {
    /// Runs inside the caller's transaction (B's close transaction).
    pub(super) fn borrowed(transaction: &'a Transaction<'a>) -> Self {
        Self {
            link: Link::Borrowed(transaction),
            #[cfg(test)]
            reversed_locks: false,
        }
    }

    /// Test only: the caller's transaction with campaign locks taken in the wrong order.
    #[cfg(test)]
    pub(super) fn borrowed_with_reversed_locks(transaction: &'a Transaction<'a>) -> Self {
        let mut tx = Self::borrowed(transaction);
        tx.reversed_locks = true;
        tx
    }

    /// Whether multi-campaign locks are taken in descending order (never outside tests).
    pub(super) fn locks_reversed(&self) -> bool {
        #[cfg(test)]
        {
            self.reversed_locks
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    pub(super) async fn query(&self, sql: &str, params: Params<'_>) -> StoreResult<Vec<Row>> {
        let result = match &self.link {
            Link::Owned(slot) => client_of(slot).query(sql, params).await,
            Link::Borrowed(transaction) => transaction.query(sql, params).await,
        };
        self.observe(result, false)
    }

    pub(super) async fn query_opt(
        &self,
        sql: &str,
        params: Params<'_>,
    ) -> StoreResult<Option<Row>> {
        let result = match &self.link {
            Link::Owned(slot) => client_of(slot).query_opt(sql, params).await,
            Link::Borrowed(transaction) => transaction.query_opt(sql, params).await,
        };
        self.observe(result, false)
    }

    pub(super) async fn query_one(&self, sql: &str, params: Params<'_>) -> StoreResult<Row> {
        let result = match &self.link {
            Link::Owned(slot) => client_of(slot).query_one(sql, params).await,
            Link::Borrowed(transaction) => transaction.query_one(sql, params).await,
        };
        self.observe(result, false)
    }

    pub(super) async fn execute(&self, sql: &str, params: Params<'_>) -> StoreResult<u64> {
        let result = match &self.link {
            Link::Owned(slot) => client_of(slot).execute(sql, params).await,
            Link::Borrowed(transaction) => transaction.execute(sql, params).await,
        };
        self.observe(result, false)
    }

    /// Maps a driver error and, when the server ended the session, marks the connection so the
    /// next checkout replaces it.
    fn observe<T>(
        &self,
        result: Result<T, tokio_postgres::Error>,
        committing: bool,
    ) -> StoreResult<T> {
        result.map_err(|error| {
            if let Link::Owned(slot) = &self.link {
                note_broken(slot, &error);
            }
            map_error(&error, committing)
        })
    }

    /// The effective `transaction_timeout` in milliseconds (0 means disabled).
    pub(super) async fn effective_timeout_ms(&self) -> StoreResult<u64> {
        let row = self
            .query_one("SELECT current_setting('transaction_timeout')", &[])
            .await?;
        let text: String = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
        timeout_ms(&text).ok_or(StoreError::InvalidRecord)
    }

    /// Whether this is the caller's transaction (whose timeout this crate cannot set).
    pub(super) fn is_borrowed(&self) -> bool {
        matches!(self.link, Link::Borrowed(_))
    }

    /// A method called inside a caller's transaction cannot raise the caller's limit: it refuses
    /// unless the effective timeout is positive and at most `maximum_ms`.
    pub(super) async fn require_timeout(&self, maximum_ms: u64) -> StoreResult<()> {
        if !self.is_borrowed() {
            return Ok(());
        }
        match self.effective_timeout_ms().await? {
            0 => Err(StoreError::TimeoutNotSet),
            ms if ms > maximum_ms => Err(StoreError::TimeoutNotSet),
            _ => Ok(()),
        }
    }

    pub(super) async fn commit(self) -> StoreResult<()> {
        match self.link {
            Link::Borrowed(_) => Ok(()),
            Link::Owned(mut slot) => {
                let result = client_of(&slot).batch_execute("COMMIT").await;
                slot.in_transaction = false;
                if let Err(error) = &result {
                    note_broken(&slot, error);
                }
                result.map_err(|error| map_error(&error, true))
            }
        }
    }

    pub(super) async fn rollback(self) {
        if let Link::Owned(mut slot) = self.link {
            // A failed rollback leaves a broken connection, which the next checkout replaces.
            let _ = client_of(&slot).batch_execute("ROLLBACK").await;
            slot.in_transaction = false;
        }
    }
}

fn note_broken(slot: &Slot, error: &tokio_postgres::Error) {
    let fatal = error.is_closed()
        || error
            .as_db_error()
            .is_some_and(|db| matches!(db.severity(), "FATAL" | "PANIC"));
    if fatal {
        slot.broken.store(true, Ordering::Relaxed);
    }
}

/// SQLSTATE and connection state to a store error. `committing` makes a lost connection
/// `Unknown`: the COMMIT may have been applied.
pub(super) fn map_error(error: &tokio_postgres::Error, committing: bool) -> StoreError {
    if let Some(db) = error.as_db_error() {
        let code = db.code().code();
        // The server ended the session while COMMIT was running (a transaction timeout, a
        // shutdown): the commit may or may not have happened.
        if committing && matches!(db.severity(), "FATAL" | "PANIC") {
            return StoreError::Unknown;
        }
        return match code {
            DEADLOCK | LOCK_NOT_AVAILABLE | QUERY_CANCELED | EVENT_TRANSACTION_TOO_OLD => {
                StoreError::Retryable
            }
            FEED_INVARIANT => StoreError::InvalidRecord,
            SERVICE_HALTING => StoreError::ServiceHalting,
            _ => StoreError::Unavailable,
        };
    }
    if committing {
        StoreError::Unknown
    } else {
        StoreError::Unavailable
    }
}

/// Milliseconds in a `SHOW transaction_timeout` answer: `0`, `10s`, `5min`, `250ms`, `1h`, `1d`.
pub(super) fn timeout_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let (number, unit) = text.split_at(digits);
    let number: u64 = number.parse().ok()?;
    let factor = match unit.trim() {
        "" | "ms" => 1,
        "s" => 1_000,
        "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    number.checked_mul(factor)
}

#[cfg(test)]
mod tests {
    use super::timeout_ms;

    #[test]
    fn timeouts_are_normalized_to_milliseconds() {
        assert_eq!(timeout_ms("0"), Some(0));
        assert_eq!(timeout_ms("10s"), Some(10_000));
        assert_eq!(timeout_ms("10000ms"), Some(10_000));
        assert_eq!(timeout_ms("1min"), Some(60_000));
        assert_eq!(timeout_ms("60s"), Some(60_000));
        assert_eq!(timeout_ms("1h"), Some(3_600_000));
        assert_eq!(timeout_ms("junk"), None);
        assert_eq!(timeout_ms(""), None);
    }
}
