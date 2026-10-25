//! Native connection ownership, finite transaction deadline and confirmed rollback boundaries.
//! Source draft awaiting native driver/actor bridge/source claim. No runtime is constructed here.
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout, timeout_at};
use tokio_postgres::{Client, Connection, Socket, Transaction};

use df_session::submission::{CommitOutcome, RepositoryError};

/// Positive production bounds are supplied by the root-owned native runtime.
#[derive(Clone, Copy)]
pub struct TransactionBounds {
    pub transaction: Duration,
    pub rollback: Duration,
    pub driver_join: Duration,
}
impl TransactionBounds {
    pub(crate) fn validate(self) -> Result<Self, RepositoryError> {
        if self.transaction.is_zero()
            || self.rollback.is_zero()
            || self.driver_join.is_zero()
            || Instant::now().checked_add(self.transaction).is_none()
            || Instant::now().checked_add(self.rollback).is_none()
            || Instant::now().checked_add(self.driver_join).is_none()
        {
            return Err(RepositoryError::Capacity);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum DiscardState {
    NotStarted,
    Joined,
    JoinedWithDriverError,
    JoinedCancelled,
    JoinedFailed,
    JoinPending,
}

// Test-only scheduling control around the ACTUAL Connection future. This
// barrier confers no scope authority and never replaces the connection task.
#[cfg(test)]
pub(crate) struct FixtureConnectionPollBarrier {
    armed: std::sync::atomic::AtomicBool,
    waiting: std::sync::atomic::AtomicBool,
    failed: std::sync::atomic::AtomicBool,
    waker: std::sync::Mutex<Option<std::task::Waker>>,
    entered: std::sync::mpsc::SyncSender<std::thread::ThreadId>,
    release: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    maximum_hold: Duration,
}
#[cfg(test)]
pub(crate) struct FixturePollBarrierControl {
    pub(crate) barrier: std::sync::Arc<FixtureConnectionPollBarrier>,
    pub(crate) entered: std::sync::mpsc::Receiver<std::thread::ThreadId>,
    pub(crate) release: std::sync::mpsc::Sender<()>,
}
#[cfg(test)]
impl FixturePollBarrierControl {
    pub(crate) fn new(maximum_hold: Duration) -> Result<Self, RepositoryError> {
        if maximum_hold.is_zero()
            || std::time::Instant::now()
                .checked_add(maximum_hold)
                .is_none()
        {
            return Err(RepositoryError::Capacity);
        }
        let (entered_tx, entered) = std::sync::mpsc::sync_channel(1);
        let (release, release_rx) = std::sync::mpsc::channel();
        Ok(Self {
            barrier: std::sync::Arc::new(FixtureConnectionPollBarrier {
                armed: std::sync::atomic::AtomicBool::new(false),
                waiting: std::sync::atomic::AtomicBool::new(false),
                failed: std::sync::atomic::AtomicBool::new(false),
                waker: std::sync::Mutex::new(None),
                entered: entered_tx,
                release: std::sync::Mutex::new(Some(release_rx)),
                maximum_hold,
            }),
            entered,
            release,
        })
    }
}
#[cfg(test)]
impl FixtureConnectionPollBarrier {
    pub(crate) fn arm(&self) -> Result<(), RepositoryError> {
        let waker = self
            .waker
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        let waker = waker.as_ref().ok_or(RepositoryError::Unavailable)?;
        self.armed.store(true, std::sync::atomic::Ordering::Release);
        waker.wake_by_ref();
        Ok(())
    }
    pub(crate) fn waiting(&self) -> bool {
        self.waiting.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn failed(&self) -> bool {
        self.failed.load(std::sync::atomic::Ordering::Acquire)
    }
    fn before_actual_poll(&self, waker: &std::task::Waker) {
        let Ok(mut stored) = self.waker.lock() else {
            self.failed
                .store(true, std::sync::atomic::Ordering::Release);
            return;
        };
        *stored = Some(waker.clone());
        drop(stored);
        if !self.armed.swap(false, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        self.waiting
            .store(true, std::sync::atomic::Ordering::Release);
        if self.entered.try_send(std::thread::current().id()).is_err() {
            self.failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        let released = self
            .release
            .lock()
            .ok()
            .and_then(|mut receiver| receiver.take())
            .is_some_and(|receiver| receiver.recv_timeout(self.maximum_hold).is_ok());
        if !released {
            self.failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        self.waiting
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

/// The repository retains every connection task until joined, including poisoned cleanup.
pub(crate) struct OwnedConnection {
    client: Option<Client>,
    driver: Option<JoinHandle<Result<(), tokio_postgres::Error>>>,
    bounds: TransactionBounds,
    usable: bool,
    discard_state: DiscardState,
}
impl OwnedConnection {
    pub(crate) fn from_connected(
        runtime: &Handle,
        client: Client,
        connection: Connection<Socket, tokio_postgres::tls::NoTlsStream>,
        bounds: TransactionBounds,
    ) -> Result<Self, RepositoryError> {
        let bounds = bounds.validate()?;
        // This named task is stored here and joined on explicit close/discard.
        let driver = runtime.spawn(connection);
        Ok(Self {
            client: Some(client),
            driver: Some(driver),
            bounds,
            usable: true,
            discard_state: DiscardState::NotStarted,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_connected_with_poll_barrier(
        runtime: &Handle,
        client: Client,
        connection: Connection<Socket, tokio_postgres::tls::NoTlsStream>,
        bounds: TransactionBounds,
        barrier: std::sync::Arc<FixtureConnectionPollBarrier>,
    ) -> Result<Self, RepositoryError> {
        let bounds = bounds.validate()?;
        // Real membership queries and commits poll this real connection normally
        // until the owner arms the single, bounded worker scheduling barrier.
        let driver = runtime.spawn(async move {
            let mut connection = std::pin::pin!(connection);
            std::future::poll_fn(move |context| {
                barrier.before_actual_poll(context.waker());
                std::future::Future::poll(connection.as_mut(), context)
            })
            .await
        });
        Ok(Self {
            client: Some(client),
            driver: Some(driver),
            bounds,
            usable: true,
            discard_state: DiscardState::NotStarted,
        })
    }

    pub(crate) fn take_client(&mut self) -> Result<Client, RepositoryError> {
        if !self.usable || !self.driver_live() {
            return Err(RepositoryError::Unavailable);
        }
        self.client.take().ok_or(RepositoryError::Unavailable)
    }

    pub(crate) fn return_client(&mut self, client: Client) -> Result<(), RepositoryError> {
        if !self.usable || self.client.is_some() {
            return Err(RepositoryError::Unavailable);
        }
        self.client = Some(client);
        Ok(())
    }

    pub(crate) fn ready_for_lookup(&self) -> bool {
        self.usable && self.client.is_some() && self.driver_live()
    }

    fn driver_live(&self) -> bool {
        self.driver
            .as_ref()
            .is_some_and(|driver| !driver.is_finished())
    }

    pub(crate) fn usable(&self) -> bool {
        self.usable
    }
    pub(crate) fn discard_state(&self) -> DiscardState {
        self.discard_state
    }

    /// Physical discard never turns a possibly committed transaction into a known rollback.
    /// A join timeout keeps the aborted handle owned and permanently refuses connection reuse.
    pub(crate) async fn discard(&mut self) -> Result<(), RepositoryError> {
        self.usable = false;
        self.client.take();
        if let Some(driver) = self.driver.as_mut() {
            driver.abort();
            match timeout(self.bounds.driver_join, driver).await {
                Ok(Ok(Ok(()))) => {
                    self.driver.take();
                    self.discard_state = DiscardState::Joined;
                }
                Ok(Ok(Err(_))) => {
                    // Physical join completed; the driver's I/O failure is separately observed.
                    self.driver.take();
                    self.discard_state = DiscardState::JoinedWithDriverError;
                }
                Ok(Err(error)) if error.is_cancelled() => {
                    self.driver.take();
                    self.discard_state = DiscardState::JoinedCancelled;
                }
                Ok(Err(_)) => {
                    self.driver.take();
                    self.discard_state = DiscardState::JoinedFailed;
                    return Err(RepositoryError::Unavailable);
                }
                Err(_) => {
                    self.discard_state = DiscardState::JoinPending;
                    return Err(RepositoryError::Unavailable);
                }
            }
        }
        Ok(())
    }

    // Private fixture final-administrator shutdown lets the ACTUAL connection
    // flush its normal Terminate path. Abort remains an explicit bounded fallback
    // through close/discard if this join times out; the pending handle stays owned.
    // Driver completion alone does not prove the PostgreSQL backend disappeared.
    #[cfg(test)]
    pub(crate) async fn fixture_close_gracefully(&mut self) -> Result<(), RepositoryError> {
        self.usable = false;
        self.client.take();
        if let Some(driver) = self.driver.as_mut() {
            match timeout(self.bounds.driver_join, driver).await {
                Ok(Ok(Ok(()))) => {
                    self.driver.take();
                    self.discard_state = DiscardState::Joined;
                }
                Ok(Ok(Err(_))) => {
                    self.driver.take();
                    self.discard_state = DiscardState::JoinedWithDriverError;
                    return Err(RepositoryError::Unavailable);
                }
                Ok(Err(error)) if error.is_cancelled() => {
                    self.driver.take();
                    self.discard_state = DiscardState::JoinedCancelled;
                    return Err(RepositoryError::Unavailable);
                }
                Ok(Err(_)) => {
                    self.driver.take();
                    self.discard_state = DiscardState::JoinedFailed;
                    return Err(RepositoryError::Unavailable);
                }
                Err(_) => {
                    self.discard_state = DiscardState::JoinPending;
                    return Err(RepositoryError::Unavailable);
                }
            }
        }
        Ok(())
    }

    /// Explicit shutdown is joined. Drop is an emergency abort, never a successful close proof.
    pub(crate) async fn close(&mut self) -> Result<(), RepositoryError> {
        self.discard().await
    }
}
impl Drop for OwnedConnection {
    fn drop(&mut self) {
        self.usable = false;
        self.client.take();
        if let Some(driver) = &self.driver {
            driver.abort();
        }
    }
}

/// Only the joined dedicated inbox actor thread may bridge this synchronous consumer port.
/// Calling this from an asynchronous runtime worker refuses before starting database work.
pub(crate) fn actor_block_on<F: std::future::Future>(
    runtime: &Handle,
    future: F,
) -> Result<F::Output, RepositoryError> {
    if Handle::try_current().is_ok() {
        return Err(RepositoryError::Unavailable);
    }
    Ok(runtime.block_on(future))
}

/// The COMMIT future and its receipt wait have one absolute transaction deadline.
/// Losing the acknowledgement is uncertainty even if cancellation/discard later succeeds.
pub(crate) async fn acknowledge_commit(
    transaction: Transaction<'_>,
    receipt: df_session::submission::DecisionReceipt,
    deadline: Instant,
) -> CommitOutcome {
    match timeout_at(deadline, transaction.commit()).await {
        Ok(Ok(())) => CommitOutcome::Confirmed(receipt),
        Ok(Err(_)) | Err(_) => CommitOutcome::Indeterminate,
    }
}

/// Once stage work has begun, all errors/cancellation before COMMIT must explicitly
/// rollback within a cleanup bound; failed cleanup must discard the owned connection.
/// Query CancelToken alone is insufficient: its separate request can race another query.
/// Receipt receiver cancellation does not enter this function or abort accepted work.
pub(crate) async fn known_rollback(
    transaction: Transaction<'_>,
    bounds: TransactionBounds,
) -> Result<(), RepositoryError> {
    match timeout(bounds.rollback, transaction.rollback()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) | Err(_) => Err(RepositoryError::Unavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::time::Duration;
    use tokio::runtime::Builder;

    #[test]
    fn runtime_worker_refuses_sync_port_without_polling_database_work() {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let polled = Cell::new(false);
        runtime.block_on(async {
            assert_eq!(
                actor_block_on(runtime.handle(), async {
                    polled.set(true);
                    7
                }),
                Err(RepositoryError::Unavailable)
            );
        });
        assert!(!polled.get());
    }

    #[test]
    fn dedicated_actor_bridge_uses_its_injected_runtime_without_detaching_work() {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let completed = Cell::new(false);
        let actor_result = actor_block_on(runtime.handle(), async {
            completed.set(true);
            7
        });
        assert_eq!(actor_result, Ok(7));
        assert!(completed.get());
    }

    #[test]
    fn zero_or_unrepresentable_deadlines_refuse_before_connection_ownership() {
        for bad in [Duration::ZERO, Duration::MAX] {
            let bounds = TransactionBounds {
                transaction: bad,
                rollback: Duration::from_secs(1),
                driver_join: Duration::from_secs(1),
            };
            assert!(matches!(bounds.validate(), Err(RepositoryError::Capacity)));
        }
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    #[test]
    fn finished_driver_is_not_ready_and_remains_owned_until_joined_close() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (release, wait) = tokio::sync::oneshot::channel();
            let driver = tokio::spawn(async {
                wait.await.unwrap();
                Ok(())
            });
            let mut connection = OwnedConnection {
                client: None,
                driver: Some(driver),
                bounds: TransactionBounds {
                    transaction: Duration::from_secs(1),
                    rollback: Duration::from_secs(1),
                    driver_join: Duration::from_secs(1),
                },
                usable: true,
                discard_state: DiscardState::NotStarted,
            };
            // Exercise driver liveness independently of the absent SQL client.
            assert!(connection.driver_live());
            release.send(()).unwrap();
            tokio::task::yield_now().await;
            assert!(!connection.driver_live());
            assert!(!connection.ready_for_lookup());
            assert!(connection.usable());
            assert!(
                connection
                    .driver
                    .as_ref()
                    .is_some_and(JoinHandle::is_finished)
            );
            connection.close().await.unwrap();
            assert!(connection.driver.is_none());
            assert!(matches!(connection.discard_state(), DiscardState::Joined));
        });
    }
}
