//! The maintenance worker: the housekeeping the finance database needs on a schedule, run by one
//! owned task with its own connection, not by the flows (planning/finance-ledger-research.md 17.8,
//! 19.1). A flow's latency never pays for it, and a stuck sweep never holds a customer's request.
//!
//! One run sweeps the lock and marker bookkeeping of finished transactions, keeps the accounting
//! periods created ahead of the clock, and, every few runs, reads the charged-total invariant
//! (a session's running total equals the retail of its usage events). The host starts the worker
//! with [`MaintenanceWorker::spawn`] and keeps the [`MaintenanceHandle`] to read its counters and to
//! stop it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_postgres::Client;

use crate::service::FinanceError;

/// The most months ahead one call may create (the database refuses more).
pub const MAX_PERIOD_HORIZON_MONTHS: i32 = 60;

/// The tunables of the worker.
#[derive(Clone, Copy, Debug)]
pub struct MaintenanceConfig {
    /// Time between runs.
    pub every: Duration,
    /// Accounting periods are kept created this many months ahead (0 to 60).
    pub period_horizon_months: i32,
    /// The charged-total invariant is read on every this-many-th run; 0 never reads it.
    pub drift_check_every_runs: u32,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            every: Duration::from_secs(60),
            period_horizon_months: 24,
            drift_check_every_runs: 60,
        }
    }
}

/// The configuration was refused before any connection was used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidMaintenanceConfig(pub &'static str);

impl MaintenanceConfig {
    /// Refuses a configuration the database would refuse anyway, at start-up and not at the first run.
    pub fn validate(&self) -> Result<(), InvalidMaintenanceConfig> {
        if self.every.is_zero() {
            return Err(InvalidMaintenanceConfig("the interval is zero"));
        }
        if !(0..=MAX_PERIOD_HORIZON_MONTHS).contains(&self.period_horizon_months) {
            return Err(InvalidMaintenanceConfig(
                "the period horizon is outside 0 to 60 months",
            ));
        }
        Ok(())
    }
}

/// What one maintenance run did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaintenanceReport {
    pub swept_flow_locks: i64,
    pub periods_created: i32,
    /// Sessions whose charged total differs from their usage events' retail; `None` when this run
    /// did not read the invariant.
    pub drifted_sessions: Option<i64>,
}

/// One owned connection and the schedule it works to.
pub struct MaintenanceWorker {
    client: Client,
    config: MaintenanceConfig,
    runs: u32,
}

impl MaintenanceWorker {
    pub fn new(
        client: Client,
        config: MaintenanceConfig,
    ) -> Result<Self, InvalidMaintenanceConfig> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            runs: 0,
        })
    }

    /// One run, now.
    pub async fn run_once(&mut self) -> Result<MaintenanceReport, FinanceError> {
        self.runs = self.runs.saturating_add(1);
        let swept = self
            .client
            .query_one("SELECT finance.sweep_flow_state()", &[])
            .await?
            .get::<_, i64>(0);
        let created = self
            .client
            .query_one(
                "SELECT finance.extend_period_horizon($1)",
                &[&self.config.period_horizon_months],
            )
            .await?
            .get::<_, i32>(0);
        let every = self.config.drift_check_every_runs;
        let drifted = if every > 0 && (self.runs - 1).is_multiple_of(every) {
            Some(
                self.client
                    .query_one("SELECT count(*) FROM finance.session_charge_drift()", &[])
                    .await?
                    .get::<_, i64>(0),
            )
        } else {
            None
        };
        Ok(MaintenanceReport {
            swept_flow_locks: swept,
            periods_created: created,
            drifted_sessions: drifted,
        })
    }

    /// Starts the worker on the current Tokio runtime. The first run is immediate.
    pub fn spawn(mut self) -> MaintenanceHandle {
        let (stop, mut stopped) = watch::channel(false);
        let counters = Arc::new(Counters::default());
        let shared = Arc::clone(&counters);
        let every = self.config.every;
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(every);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {}
                    _ = stopped.changed() => break,
                }
                match self.run_once().await {
                    Ok(report) => {
                        shared.runs.fetch_add(1, Ordering::Relaxed);
                        if let Some(drifted) = report.drifted_sessions {
                            shared.drifted_sessions.store(
                                u64::try_from(drifted).unwrap_or(u64::MAX),
                                Ordering::Relaxed,
                            );
                        }
                    }
                    Err(_) => {
                        shared.failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        MaintenanceHandle {
            stop,
            counters,
            task,
        }
    }
}

#[derive(Default)]
struct Counters {
    runs: AtomicU32,
    failures: AtomicU32,
    drifted_sessions: AtomicU64,
}

/// The host's view of a running worker.
pub struct MaintenanceHandle {
    stop: watch::Sender<bool>,
    counters: Arc<Counters>,
    task: JoinHandle<()>,
}

impl MaintenanceHandle {
    /// Runs that finished without an error.
    pub fn runs(&self) -> u32 {
        self.counters.runs.load(Ordering::Relaxed)
    }

    /// Runs that failed; the host alerts on a rising count.
    pub fn failures(&self) -> u32 {
        self.counters.failures.load(Ordering::Relaxed)
    }

    /// Sessions the last invariant read found out of step; the host alerts on anything above zero.
    pub fn drifted_sessions(&self) -> u64 {
        self.counters.drifted_sessions.load(Ordering::Relaxed)
    }

    /// Whether the task is still running (it ends only when stopped or when the runtime closes).
    pub fn is_running(&self) -> bool {
        !self.task.is_finished()
    }

    /// Stops the worker after the run in progress and waits for it.
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}
