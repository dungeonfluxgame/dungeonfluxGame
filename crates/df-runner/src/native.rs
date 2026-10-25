//! Starts the runtime as a child process of the runner (local development). The child gets an
//! empty environment and the descriptor as its only input. A monitor task owns the `Child`; the
//! handle owns the task, so dropping the handle kills the runtime.

use std::{io, path::PathBuf, time::Duration};

use tokio::{
    process::Child,
    sync::{oneshot, watch},
    task::JoinHandle,
    time::timeout,
};

use crate::{
    launcher::{ExitReport, LaunchError, RuntimeLauncher},
    process,
    spec::{PlacementId, RuntimeSpec},
};

type Outcome = Option<Result<ExitReport, io::ErrorKind>>;

pub struct NativeLauncher {
    program: PathBuf,
}

impl NativeLauncher {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }
}

pub struct NativeRuntime {
    placement_id: PlacementId,
    outcome: watch::Receiver<Outcome>,
    kill: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    monitor: JoinHandle<()>,
}

impl Drop for NativeRuntime {
    fn drop(&mut self) {
        // Aborting the monitor drops the `Child`, whose `kill_on_drop` ends the process.
        self.monitor.abort();
    }
}

async fn monitor(mut child: Child, kill: oneshot::Receiver<()>, outcome: watch::Sender<Outcome>) {
    let status = tokio::select! {
        status = child.wait() => status,
        _ = kill => match child.kill().await {
            Ok(()) => child.wait().await,
            Err(error) => Err(error),
        },
    };
    let report = status
        .map(|status| ExitReport {
            exit_code: status.code(),
        })
        .map_err(|error| error.kind());
    outcome.send_replace(Some(report));
}

impl NativeRuntime {
    async fn exit(&self) -> Result<ExitReport, LaunchError> {
        let mut outcome = self.outcome.clone();
        let finished = outcome
            .wait_for(Option::is_some)
            .await
            .map_err(|_| LaunchError::Supervise(io::ErrorKind::BrokenPipe))?;
        match *finished {
            Some(Ok(report)) => Ok(report),
            Some(Err(kind)) => Err(LaunchError::Supervise(kind)),
            None => Err(LaunchError::Supervise(io::ErrorKind::Other)),
        }
    }

    fn request_kill(&self) {
        let sender = self
            .kill
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(sender) = sender {
            // The monitor may already have seen the exit; then there is nothing to kill.
            let _already_exited = sender.send(());
        }
    }
}

impl RuntimeLauncher for NativeLauncher {
    type Handle = NativeRuntime;

    async fn start(&self, spec: &RuntimeSpec) -> Result<NativeRuntime, LaunchError> {
        let mut command = process::command(&self.program);
        command
            .arg("campaign-runtime")
            .arg("--descriptor")
            .arg(&spec.descriptor_path);
        let child = command
            .spawn()
            .map_err(|error| LaunchError::Spawn(error.kind()))?;
        let (kill, kill_requested) = oneshot::channel();
        let (outcome_sender, outcome) = watch::channel(None);
        let monitor = tokio::spawn(monitor(child, kill_requested, outcome_sender));
        df_observe::record_audit_line(
            "RUNNER",
            format!(
                "RUNNER launch status=started launcher=native placement={}",
                spec.placement_id
            ),
        );
        Ok(NativeRuntime {
            placement_id: spec.placement_id,
            outcome,
            kill: std::sync::Mutex::new(Some(kill)),
            monitor,
        })
    }

    async fn wait(&self, handle: &NativeRuntime) -> Result<ExitReport, LaunchError> {
        handle.exit().await
    }

    async fn stop(
        &self,
        handle: &NativeRuntime,
        grace: Duration,
    ) -> Result<ExitReport, LaunchError> {
        // A monitor failure is an error, not a graceful exit; only the timeout kills.
        let graceful = match timeout(grace, handle.exit()).await {
            Ok(Ok(_)) => true,
            Ok(Err(error)) => return Err(error),
            Err(_elapsed) => false,
        };
        if !graceful {
            handle.request_kill();
        }
        let report = handle.exit().await?;
        df_observe::record_audit_line(
            "RUNNER",
            format!(
                "RUNNER stop status={} placement={} exit_code={}",
                if graceful { "graceful" } else { "killed" },
                handle.placement_id,
                report
                    .exit_code
                    .map_or_else(|| "none".to_string(), |code| code.to_string()),
            ),
        );
        Ok(report)
    }
}
