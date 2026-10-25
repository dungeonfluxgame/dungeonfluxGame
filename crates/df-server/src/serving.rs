//! Hosted serving entry points: startup checks run before any listener binds, the recovery
//! handle is kept for the process lifetime, and readiness flips only after those checks pass.

use std::{future::Future, net::SocketAddr, pin::Pin, sync::Arc};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::{
    config::{Role, ServerConfig},
    exporter::ExporterHandle,
    health::{HealthBoard, Signal, SignalState},
    metrics::MetricsRegistry,
    runtime_recovery::{RecoveryHandle, StartupRefusal, start_configured},
};

#[cfg(test)]
use crate::runtime_recovery::{RecoveryStore, start_hosted};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServingError {
    StartupRefused(StartupRefusal),
    WrongRole { expected: Role, found: Role },
    BindFailed,
    MaintenanceInvalid,
    MaintenanceUnavailable,
}

impl std::fmt::Display for ServingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartupRefused(refusal) => write!(formatter, "startup refused: {refusal:?}"),
            Self::WrongRole { expected, found } => {
                write!(formatter, "role {found:?} cannot serve as {expected:?}")
            }
            Self::BindFailed => write!(formatter, "listener bind failed"),
            Self::MaintenanceInvalid => write!(formatter, "maintenance configuration invalid"),
            Self::MaintenanceUnavailable => write!(formatter, "maintenance unavailable"),
        }
    }
}
impl std::error::Error for ServingError {}

/// One owned maintenance worker: the control plane starts it and stops it on shutdown.
pub trait MaintenanceControl: Send {
    fn runs(&self) -> u32;
    fn failures(&self) -> u32;
    fn is_running(&self) -> bool;
    fn stop(self: Box<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

impl MaintenanceControl for df_finance::maintenance::MaintenanceHandle {
    fn runs(&self) -> u32 {
        df_finance::maintenance::MaintenanceHandle::runs(self)
    }
    fn failures(&self) -> u32 {
        df_finance::maintenance::MaintenanceHandle::failures(self)
    }
    fn is_running(&self) -> bool {
        df_finance::maintenance::MaintenanceHandle::is_running(self)
    }
    fn stop(self: Box<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { (*self).stop().await })
    }
}

type MaintenanceStarter =
    Pin<Box<dyn Future<Output = Result<Option<Box<dyn MaintenanceControl>>, ServingError>> + Send>>;

/// A serving process: every owned handle lives here until `shutdown` (or drop).
pub struct Serving {
    recovery: Option<RecoveryHandle>,
    exporter: Option<ExporterHandle>,
    board: Arc<HealthBoard>,
    maintenance: Option<Box<dyn MaintenanceControl>>,
}

impl Serving {
    pub fn local_address(&self) -> SocketAddr {
        self.exporter
            .as_ref()
            .map(ExporterHandle::local_address)
            .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 0)))
    }

    pub fn board(&self) -> Arc<HealthBoard> {
        Arc::clone(&self.board)
    }

    pub fn is_ready(&self) -> bool {
        self.board.state(Signal::Ready) == SignalState::Healthy
    }

    /// Routes a fenced database call's SQLSTATE to the DF002 sweeper (critic-2 item 1).
    pub fn report_fenced_sqlstate(&self, sqlstate: &str) {
        if let Some(recovery) = &self.recovery {
            recovery.report_fenced_sqlstate(sqlstate);
        }
    }

    pub fn maintenance_runs(&self) -> u32 {
        self.maintenance.as_ref().map_or(0, |worker| worker.runs())
    }

    pub fn maintenance_is_running(&self) -> bool {
        self.maintenance
            .as_ref()
            .is_some_and(|worker| worker.is_running())
    }

    pub async fn shutdown(mut self) {
        self.board
            .set(Signal::Ready, SignalState::Unhealthy("draining"));
        if let Some(maintenance) = self.maintenance.take() {
            maintenance.stop().await;
        }
        self.exporter.take();
        self.recovery.take();
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        self.exporter.take();
        self.recovery.take();
        // MaintenanceHandle's watch sender drops with the box, which ends its task.
        self.maintenance.take();
    }
}

async fn start_after_checks(
    config: &ServerConfig,
    expected: Role,
    recovery: Option<RecoveryHandle>,
    starter: impl FnOnce() -> MaintenanceStarter,
) -> Result<Serving, ServingError> {
    if config.instance.role != expected {
        return Err(ServingError::WrongRole {
            expected,
            found: config.instance.role,
        });
    }
    let registry = Arc::new(MetricsRegistry::new());
    let board = Arc::new(HealthBoard::starting(&registry).map_err(|_| ServingError::BindFailed)?);
    // Binding happens only after `start_configured` has succeeded in the caller.
    let listener = TcpListener::bind(config.listen.metrics_address)
        .await
        .map_err(|_| ServingError::BindFailed)?;
    let exporter = ExporterHandle::serve(listener, registry, Arc::clone(&board))
        .map_err(|_| ServingError::BindFailed)?;
    board.set(Signal::Database, SignalState::Healthy);
    let maintenance = starter().await?;
    let serving = Serving {
        recovery,
        exporter: Some(exporter),
        board,
        maintenance,
    };
    serving.board.set(Signal::Ready, SignalState::Healthy);
    df_observe::record_audit_line("server.serving", format!("SERVING ready role={expected:?}"));
    Ok(serving)
}

fn no_maintenance() -> MaintenanceStarter {
    Box::pin(async { Ok(None) })
}

/// Connects the finance maintenance connection through the shared TLS policy.
async fn finance_maintenance(
    config: ServerConfig,
) -> Result<Option<Box<dyn MaintenanceControl>>, ServingError> {
    let database = config.database.clone();
    let password =
        std::fs::read(&database.password_file).map_err(|_| ServingError::MaintenanceUnavailable)?;
    let password = String::from_utf8(password).map_err(|_| ServingError::MaintenanceUnavailable)?;
    let mut pg = tokio_postgres::Config::new();
    pg.host(&database.host)
        .port(database.port)
        .dbname(&database.name)
        .user(&database.user)
        .password(password.trim_end_matches(['\r', '\n']))
        .options(&format!(
            "-c statement_timeout={}",
            database.statement_timeout_ms
        ));
    let transport = match database.tls.mode {
        crate::config::TlsMode::VerifyFull => {
            df_persistence::postgres_tls::PostgresTransport::VerifiedTls {
                ca_file: database
                    .tls
                    .ca_file
                    .clone()
                    .ok_or(ServingError::MaintenanceUnavailable)?,
            }
        }
        crate::config::TlsMode::LoopbackPlaintext => {
            df_persistence::postgres_tls::PostgresTransport::PlaintextLoopbackOnly
        }
    };
    let (client, driver) = df_persistence::postgres_tls::connect(&pg, &transport)
        .await
        .map_err(|_| ServingError::MaintenanceUnavailable)?;
    let driver_handle = tokio::spawn(async move {
        let _ = driver.await;
    });
    struct WithDriver {
        inner: Option<Box<dyn MaintenanceControl>>,
        driver: Option<tokio::task::JoinHandle<()>>,
    }
    impl MaintenanceControl for WithDriver {
        fn runs(&self) -> u32 {
            self.inner.as_ref().map_or(0, |inner| inner.runs())
        }
        fn failures(&self) -> u32 {
            self.inner.as_ref().map_or(0, |inner| inner.failures())
        }
        fn is_running(&self) -> bool {
            self.inner.as_ref().is_some_and(|inner| inner.is_running())
        }
        fn stop(mut self: Box<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
            Box::pin(async move {
                if let Some(driver) = self.driver.take() {
                    driver.abort();
                }
                if let Some(inner) = self.inner.take() {
                    inner.stop().await;
                }
            })
        }
    }
    impl Drop for WithDriver {
        fn drop(&mut self) {
            if let Some(driver) = self.driver.take() {
                driver.abort();
            }
        }
    }
    let worker = df_finance::maintenance::MaintenanceWorker::new(
        client,
        df_finance::maintenance::MaintenanceConfig::default(),
    )
    .map_err(|_| ServingError::MaintenanceInvalid)?;
    let handle: Box<dyn MaintenanceControl> = Box::new(worker.spawn());
    Ok(Some(Box::new(WithDriver {
        inner: Some(handle),
        driver: Some(driver_handle),
    })))
}

/// `df-server control-plane --config <path>`: recovery first, then the finance worker.
pub async fn start_control_plane(config: ServerConfig) -> Result<Serving, ServingError> {
    if config.instance.role != Role::ControlPlane {
        return Err(ServingError::WrongRole {
            expected: Role::ControlPlane,
            found: config.instance.role,
        });
    }
    let recovery = start_configured(&config)
        .await
        .map_err(ServingError::StartupRefused)?;
    let maintenance_config = config.clone();
    start_after_checks(
        &config,
        Role::ControlPlane,
        recovery,
        move || -> MaintenanceStarter { Box::pin(finance_maintenance(maintenance_config)) },
    )
    .await
}

/// `df-server campaign-runtime --descriptor <path>`: hosted settings, no dev surfaces.
pub async fn start_campaign_runtime(config: ServerConfig) -> Result<Serving, ServingError> {
    if config.instance.role != Role::CampaignRuntime {
        return Err(ServingError::WrongRole {
            expected: Role::CampaignRuntime,
            found: config.instance.role,
        });
    }
    if config.instance.environment.is_hosted() && config.local_dev.is_some() {
        return Err(ServingError::StartupRefused(
            StartupRefusal::RecoveryCheckFailed,
        ));
    }
    let recovery = start_configured(&config)
        .await
        .map_err(ServingError::StartupRefused)?;
    start_after_checks(&config, Role::CampaignRuntime, recovery, no_maintenance).await
}

/// `df-server egress --config <path>`.
pub async fn start_egress(config: ServerConfig) -> Result<Serving, ServingError> {
    if config.instance.role != Role::Egress {
        return Err(ServingError::WrongRole {
            expected: Role::Egress,
            found: config.instance.role,
        });
    }
    let recovery = start_configured(&config)
        .await
        .map_err(ServingError::StartupRefused)?;
    start_after_checks(&config, Role::Egress, recovery, no_maintenance).await
}

/// Test seam: identical ordering (store check, then bind, then maintenance, then ready)
/// with a fake recovery store and a fake maintenance worker.
#[cfg(test)]
pub(crate) async fn start_with_store<S: RecoveryStore>(
    config: &ServerConfig,
    expected: Role,
    store: S,
    starter: impl FnOnce() -> MaintenanceStarter,
) -> Result<Serving, ServingError> {
    let recovery = start_hosted(store)
        .await
        .map_err(ServingError::StartupRefused)?;
    start_after_checks(config, expected, Some(recovery), starter).await
}

#[cfg(test)]
pub(crate) fn fake_maintenance_starter(
    worker: Box<dyn MaintenanceControl>,
) -> impl FnOnce() -> MaintenanceStarter {
    move || -> MaintenanceStarter { Box::pin(async move { Ok(Some(worker)) }) }
}

/// One healthcheck probe (`df-server healthcheck --address <a> --signal <name>`).
pub async fn check_health(address: SocketAddr, signal: Signal) -> std::io::Result<bool> {
    let mut stream = TcpStream::connect(address).await?;
    let request = format!(
        "GET /healthz/{} HTTP/1.1\r\nHost: healthcheck\r\n\r\n",
        signal.name()
    );
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response.starts_with("HTTP/1.1 200"))
}
