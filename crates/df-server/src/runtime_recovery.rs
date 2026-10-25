//! Hosted PostgreSQL startup guard and owned service-lock recovery task.

use std::{future::Future, pin::Pin, time::Duration};

use df_persistence::postgres_tls::{self, PostgresTransport};
use tokio::sync::mpsc;

const MAXIMUM_BREAK_ATTEMPTS_PER_WINDOW: u8 = 3;
const BREAK_ATTEMPT_WINDOW: Duration = Duration::from_secs(60);
const BREAK_BACKOFF: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(250),
    Duration::from_secs(1),
];
const BREAK_LOCK_TIMEOUT_MS: i32 = 2_000;

pub type RecoveryFuture<'a, T = ()> =
    Pin<Box<dyn Future<Output = Result<T, RecoveryFailure>> + Send + 'a>>;

/// Operations required by the server recovery owner; tests supply a deterministic fake.
pub trait RecoveryStore: Send + Sync + 'static {
    fn require_no_prepared_transactions(&self) -> RecoveryFuture<'_>;
    fn service_is_halted(&self) -> RecoveryFuture<'_, bool>;
    fn break_service_lock(&self) -> RecoveryFuture<'_>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryFailure {
    DatabaseUnavailable,
    PreparedTransactionsEnabled,
    ProcedureFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupRefusal {
    DatabaseUnavailable,
    PreparedTransactionsEnabled,
    RecoveryCheckFailed,
}

/// Owns the bounded DF002 event queue and the cancellation handle for its worker.
pub struct RecoveryHandle {
    reports: mpsc::Sender<()>,
    worker: tokio::task::JoinHandle<()>,
    connection: Option<tokio::task::JoinHandle<()>>,
}

impl RecoveryHandle {
    /// Reports only the fenced-call SQLSTATE; a full queue is safely coalesced by dropping it.
    pub fn report_fenced_sqlstate(&self, sqlstate: &str) {
        if sqlstate == "DF002" && self.reports.try_send(()).is_err() {
            df_observe::record_audit_line(
                "server.runtime_recovery",
                "SERVICE_LOCK recovery_queue_full".to_owned(),
            );
        }
    }
}

impl Drop for RecoveryHandle {
    fn drop(&mut self) {
        self.worker.abort();
        if let Some(connection) = &self.connection {
            connection.abort();
        }
    }
}

/// Runs the prepared-transaction check before returning an owned DF002 recovery supervisor.
pub async fn start_hosted<S: RecoveryStore>(store: S) -> Result<RecoveryHandle, StartupRefusal> {
    start_hosted_with_connection(store, None).await
}

/// Local development skips the hosted recovery connection; hosted configs run it before service setup.
pub async fn start_configured(
    config: &crate::config::ServerConfig,
) -> Result<Option<RecoveryHandle>, StartupRefusal> {
    if !config.instance.environment.is_hosted() {
        return Ok(None);
    }
    let (store, driver) = match PostgresRecoveryStore::connect(&config.database).await {
        Ok(connection) => connection,
        Err(_) => {
            df_observe::record_audit_line(
                "server.runtime_recovery",
                "STARTUP refused reason=control_database_unavailable".to_owned(),
            );
            return Err(StartupRefusal::DatabaseUnavailable);
        }
    };
    let connection = tokio::spawn(async move {
        if driver.await.is_err() {
            df_observe::record_audit_line(
                "server.runtime_recovery",
                "DATABASE connection_driver_failed".to_owned(),
            );
        }
    });
    start_hosted_with_connection(store, Some(connection))
        .await
        .map(Some)
}

async fn start_hosted_with_connection<S: RecoveryStore>(
    store: S,
    connection: Option<tokio::task::JoinHandle<()>>,
) -> Result<RecoveryHandle, StartupRefusal> {
    match store.require_no_prepared_transactions().await {
        Ok(()) => {}
        Err(RecoveryFailure::PreparedTransactionsEnabled) => {
            if let Some(connection) = &connection {
                connection.abort();
            }
            df_observe::record_audit_line(
                "server.runtime_recovery",
                "STARTUP refused reason=prepared_transactions_enabled".to_owned(),
            );
            return Err(StartupRefusal::PreparedTransactionsEnabled);
        }
        Err(RecoveryFailure::DatabaseUnavailable) => {
            if let Some(connection) = &connection {
                connection.abort();
            }
            df_observe::record_audit_line(
                "server.runtime_recovery",
                "STARTUP refused reason=control_database_unavailable".to_owned(),
            );
            return Err(StartupRefusal::DatabaseUnavailable);
        }
        Err(RecoveryFailure::ProcedureFailed) => {
            if let Some(connection) = &connection {
                connection.abort();
            }
            df_observe::record_audit_line(
                "server.runtime_recovery",
                "STARTUP refused reason=prepared_transaction_check_failed".to_owned(),
            );
            return Err(StartupRefusal::RecoveryCheckFailed);
        }
    }

    let (reports, events) = mpsc::channel(32);
    let worker = tokio::spawn(run_sweeper(store, events));
    Ok(RecoveryHandle {
        reports,
        worker,
        connection,
    })
}

async fn run_sweeper<S: RecoveryStore>(store: S, mut events: mpsc::Receiver<()>) {
    let mut window_started = tokio::time::Instant::now();
    let mut attempts_in_window = 0_u8;
    while events.recv().await.is_some() {
        if window_started.elapsed() >= BREAK_ATTEMPT_WINDOW {
            window_started = tokio::time::Instant::now();
            attempts_in_window = 0;
        }
        if attempts_in_window >= MAXIMUM_BREAK_ATTEMPTS_PER_WINDOW {
            continue;
        }
        for backoff in BREAK_BACKOFF.iter().skip(usize::from(attempts_in_window)) {
            if !backoff.is_zero() {
                tokio::time::sleep(*backoff).await;
            }
            match store.service_is_halted().await {
                Ok(true) => break,
                Ok(false) => {}
                Err(_) => {
                    df_observe::record_audit_line(
                        "server.runtime_recovery",
                        "SERVICE_LOCK state_check_failed".to_owned(),
                    );
                    break;
                }
            }
            attempts_in_window = attempts_in_window.saturating_add(1);
            df_observe::record_audit_line(
                "server.runtime_recovery",
                format!("SERVICE_LOCK break_attempt attempt={attempts_in_window}"),
            );
            match store.break_service_lock().await {
                Ok(()) => break,
                Err(_) => df_observe::record_audit_line(
                    "server.runtime_recovery",
                    format!("SERVICE_LOCK break_failed attempt={attempts_in_window}"),
                ),
            }
            if attempts_in_window >= MAXIMUM_BREAK_ATTEMPTS_PER_WINDOW {
                break;
            }
        }
    }
}

/// PostgreSQL implementation for the server's typed database configuration.
pub struct PostgresRecoveryStore {
    client: tokio_postgres::Client,
}

impl PostgresRecoveryStore {
    /// Wraps an established connection, mainly for integration with an owned connection pool.
    #[cfg(test)]
    pub(crate) fn from_client(client: tokio_postgres::Client) -> Self {
        Self { client }
    }

    /// Opens a control-plane connection and returns the driver future that must be polled.
    pub async fn connect(
        database: &crate::config::DatabaseSection,
    ) -> Result<(Self, postgres_tls::Driver), RecoveryFailure> {
        let password = std::fs::read(&database.password_file)
            .map_err(|_| RecoveryFailure::DatabaseUnavailable)?;
        let password =
            String::from_utf8(password).map_err(|_| RecoveryFailure::DatabaseUnavailable)?;
        let password = password.trim_end_matches(['\r', '\n']);
        let mut config = tokio_postgres::Config::new();
        config
            .host(&database.host)
            .port(database.port)
            .dbname(&database.name)
            .user(&database.user)
            .password(password)
            .options(&format!(
                "-c statement_timeout={}",
                database.statement_timeout_ms
            ));
        let transport = match database.tls.mode {
            crate::config::TlsMode::VerifyFull => PostgresTransport::VerifiedTls {
                ca_file: database
                    .tls
                    .ca_file
                    .clone()
                    .ok_or(RecoveryFailure::DatabaseUnavailable)?,
            },
            crate::config::TlsMode::LoopbackPlaintext => PostgresTransport::PlaintextLoopbackOnly,
        };
        let (client, driver) = postgres_tls::connect(&config, &transport)
            .await
            .map_err(|_| RecoveryFailure::DatabaseUnavailable)?;
        Ok((Self { client }, driver))
    }
}

impl RecoveryStore for PostgresRecoveryStore {
    fn require_no_prepared_transactions(&self) -> RecoveryFuture<'_> {
        Box::pin(async move {
            self.client
                .simple_query("SELECT df_runtime.require_no_prepared_transactions()")
                .await
                .map_err(|error| {
                    if error.code().is_some_and(|code| code.code() == "DF006") {
                        RecoveryFailure::PreparedTransactionsEnabled
                    } else {
                        RecoveryFailure::ProcedureFailed
                    }
                })?;
            Ok(())
        })
    }

    fn service_is_halted(&self) -> RecoveryFuture<'_, bool> {
        Box::pin(async move {
            let row = self
                .client
                .query_one(
                    "SELECT out_serving = 'halted_regression' FROM df_runtime.service_state()",
                    &[],
                )
                .await
                .map_err(|_| RecoveryFailure::ProcedureFailed)?;
            Ok(row.get(0))
        })
    }

    fn break_service_lock(&self) -> RecoveryFuture<'_> {
        Box::pin(async move {
            self.client
                .simple_query(&format!(
                    "SELECT df_runtime.break_service_lock({BREAK_LOCK_TIMEOUT_MS}, 3)"
                ))
                .await
                .map_err(|_| RecoveryFailure::ProcedureFailed)?;
            Ok(())
        })
    }
}
