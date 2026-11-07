//! The two pieces every blocking file task of the host shares: running the work on the blocking
//! pool, and a write lock for the work that reads then writes one folder.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Runs blocking `work` on the blocking pool. A task that stopped without an answer (it panicked
/// or the runtime is stopping) is `stopped`.
pub(crate) async fn run_blocking<T, E>(
    work: impl FnOnce() -> Result<T, E> + Send + 'static,
    stopped: E,
) -> Result<T, E>
where
    T: Send + 'static,
    E: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or(Err(stopped))
}

/// Runs `work`, a paid call, on a task of its own and waits for it. A request that is dropped (the
/// client left, or shutdown's grace ended) cannot cancel it, so an answer already paid for is
/// stored and recorded. A task that stopped without an answer is `stopped`.
pub(crate) async fn run_owned<T, E>(
    work: impl Future<Output = Result<T, E>> + Send + 'static,
    stopped: E,
) -> Result<T, E>
where
    T: Send + 'static,
    E: Send + 'static,
{
    tokio::spawn(work).await.unwrap_or(Err(stopped))
}

/// A lock held around the work that reads then writes one folder of files, so one never overwrites
/// what another stored meanwhile. Taken only inside blocking work, never across an `.await`. A
/// panic of an earlier holder left no file half written (every write of the host is whole or not
/// at all), so a poisoned lock is still good.
pub(crate) struct WriteLock(Mutex<()>);

impl WriteLock {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(()))
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, ()> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
