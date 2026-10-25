//! The launcher port: how the runner starts, waits for and stops one runtime. Adapters are the
//! native child process (local development) and the Docker proxy (staging and production).

use std::{future::Future, io, time::Duration};

use crate::{docker::ProxyError, spec::RuntimeSpec};

/// How a runtime ended. `exit_code` is `None` when it was killed rather than exiting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitReport {
    pub exit_code: Option<i32>,
}

impl ExitReport {
    /// Only an exit with code 0 is a clean exit; the runner releases the placement as unclean
    /// otherwise (design 6.5).
    pub const fn is_clean(self) -> bool {
        matches!(self.exit_code, Some(0))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchError {
    /// The operating system refused to start the process.
    Spawn(io::ErrorKind),
    /// Waiting for or killing the process failed.
    Supervise(io::ErrorKind),
    /// The slot is outside the range the proxy admits.
    SlotOutOfRange,
    /// The Docker proxy refused or could not carry out a request.
    Proxy(ProxyError),
}

pub trait RuntimeLauncher {
    type Handle: Send + Sync;

    fn start(
        &self,
        spec: &RuntimeSpec,
    ) -> impl Future<Output = Result<Self::Handle, LaunchError>> + Send;

    /// Resolves when the runtime has exited.
    fn wait(
        &self,
        handle: &Self::Handle,
    ) -> impl Future<Output = Result<ExitReport, LaunchError>> + Send;

    /// Gives the runtime `grace` to exit by itself (after a drain request), then kills it.
    fn stop(
        &self,
        handle: &Self::Handle,
        grace: Duration,
    ) -> impl Future<Output = Result<ExitReport, LaunchError>> + Send;
}
