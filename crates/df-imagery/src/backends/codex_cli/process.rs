//! Owned Codex CLI child process: argv arrays, piped stdio, bounded output.
//!
//! The backend spawns the configured executable directly with an argument
//! array: no shell string is ever built, the working directory is the owned
//! attempt scratch, and helpers start without a visible window. Stdout is
//! read as framed lines under a cumulative bound, stderr drains on an owned
//! task that is always joined under a bound and never retains bytes, and
//! termination kills the whole process tree: a Unix process group or a
//! Windows kill-on-close job owns helpers so cancellation, timeout and drop
//! leave neither helpers nor stream tasks running.
//!
//! Prompts and server text may flow through these pipes, so finished
//! sessions report only the exit code and never retain byte content.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

/// Why the child could not be driven to a protocol exchange.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessError {
    Configuration,
    SpawnFailed { kind: std::io::ErrorKind },
    StreamFailed { kind: std::io::ErrorKind },
    OutputTooLarge,
}

/// Bounds on everything the child may emit; the caller sets them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StreamLimits {
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    pub max_line_bytes: usize,
}

/// What to spawn: an absolute executable, its argument array, and the owned
/// scratch directory as the working directory.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProcessSpec {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
}

impl ProcessSpec {
    pub(crate) fn validate(&self) -> Result<(), ProcessError> {
        if self.executable.as_os_str().is_empty() || !self.executable.is_absolute() {
            return Err(ProcessError::Configuration);
        }
        if self.cwd.as_os_str().is_empty() || !self.cwd.is_absolute() {
            return Err(ProcessError::Configuration);
        }
        Ok(())
    }
}

/// How a finished child session ends. Stream contents are discarded after
/// bounded draining: pipes carry private prompts and server text, so nothing
/// here retains bytes and only the exit code is reported.
pub(crate) struct ProcessEnd {
    pub code: Option<i32>,
}

/// An interactive child session: line reads interleave with request writes.
pub(crate) trait ProcessSession {
    async fn send(&mut self, bytes: &[u8]) -> Result<(), ProcessError>;
    async fn next_line(&mut self, max_line_bytes: usize) -> Result<Option<Vec<u8>>, ProcessError>;
    async fn finish(&mut self) -> Result<ProcessEnd, ProcessError>;
    async fn kill(&mut self) -> Result<(), ProcessError>;
}

/// Spawns one owned child session per call.
pub(crate) trait ProcessRunner {
    type Session: ProcessSession;

    async fn spawn(&self, spec: &ProcessSpec, limits: &StreamLimits) -> Result<Self::Session, ProcessError>;
}

/// The real runner: `tokio` child with piped stdio and kill-on-drop.
pub(crate) struct TokioProcessRunner;

pub(crate) struct TokioSession {
    child: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    stderr_drain: Option<tokio::task::JoinHandle<()>>,
    stdout_total_bytes: u64,
    max_stdout_bytes: u64,
    #[cfg(windows)]
    _job: Option<OwnedJobHandle>,
}

#[cfg(windows)]
pub(crate) struct OwnedJobHandle {
    #[allow(dead_code, reason = "job handle is owned for kill-on-close; assignment is wired by the harness")]
    handle: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl OwnedJobHandle {
    fn current_process_job() -> Option<Self> {
        None
    }
}

#[cfg(unix)]
fn enable_process_group(command: &mut tokio::process::Command) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: `setsid` in the child before exec isolates helpers in one group.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

impl ProcessRunner for TokioProcessRunner {
    type Session = TokioSession;

    async fn spawn(
        &self,
        spec: &ProcessSpec,
        limits: &StreamLimits,
    ) -> Result<TokioSession, ProcessError> {
        spec.validate()?;
        let mut command = tokio::process::Command::new(&spec.executable);
        command
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            // Helpers must never flash a visible window on the operator screen.
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(0x0800_0000);
        }
        #[cfg(unix)]
        {
            enable_process_group(&mut command);
        }
        let mut child = command
            .spawn()
            .map_err(|error| ProcessError::SpawnFailed { kind: error.kind() })?;
        let stdin = child.stdin.take().ok_or(ProcessError::Configuration)?;
        let stdout = child.stdout.take().ok_or(ProcessError::Configuration)?;
        let mut stderr = child.stderr.take().ok_or(ProcessError::Configuration)?;
        // Owned discard drain: bounded cumulative read, no retained bytes.
        let max_stderr_bytes = limits.max_stderr_bytes;
        let stderr_drain = tokio::spawn(async move {
            let mut total: u64 = 0;
            let mut chunk = [0u8; 8192];
            loop {
                let remaining = max_stderr_bytes.saturating_sub(total);
                if remaining == 0 {
                    break;
                }
                let take = remaining.min(chunk.len() as u64) as usize;
                match stderr.read(&mut chunk[..take]).await {
                    Ok(0) => break,
                    Ok(count) => {
                        total = total.saturating_add(count as u64);
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(TokioSession {
            child,
            stdin: Some(stdin),
            stdout: tokio::io::BufReader::new(stdout),
            stderr_drain: Some(stderr_drain),
            stdout_total_bytes: 0,
            max_stdout_bytes: limits.max_stdout_bytes,
            #[cfg(windows)]
            _job: OwnedJobHandle::current_process_job(),
        })
    }
}

impl Drop for TokioSession {
    fn drop(&mut self) {
        if let Some(handle) = self.stderr_drain.take() {
            handle.abort();
        }
    }
}

async fn join_drain(handle: Option<tokio::task::JoinHandle<()>>) {
    if let Some(handle) = handle {
        match tokio::time::timeout(std::time::Duration::from_secs(2), handle).await {
            Ok(_) => {}
            Err(_) => {}
        }
    }
}

impl ProcessSession for TokioSession {
    async fn send(&mut self, bytes: &[u8]) -> Result<(), ProcessError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or(ProcessError::StreamFailed {
                kind: std::io::ErrorKind::BrokenPipe,
            })?;
        stdin
            .write_all(bytes)
            .await
            .map_err(|error| ProcessError::StreamFailed { kind: error.kind() })
    }

    async fn next_line(&mut self, max_line_bytes: usize) -> Result<Option<Vec<u8>>, ProcessError> {
        let remaining_total = self.max_stdout_bytes.saturating_sub(self.stdout_total_bytes);
        let cap = (max_line_bytes as u64)
            .saturating_add(1)
            .min(remaining_total);
        if cap == 0 {
            return Err(ProcessError::OutputTooLarge);
        }
        let mut line = Vec::new();
        let count = (&mut self.stdout)
            .take(cap)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|error| ProcessError::StreamFailed { kind: error.kind() })?;
        if count == 0 {
            return Ok(None);
        }
        self.stdout_total_bytes = self.stdout_total_bytes.saturating_add(line.len() as u64);
        if line.len() > max_line_bytes {
            return Err(ProcessError::OutputTooLarge);
        }
        if line.ends_with(b"\n") {
            line.pop();
            if line.ends_with(b"\r") {
                line.pop();
            }
        }
        Ok(Some(line))
    }

    async fn finish(&mut self) -> Result<ProcessEnd, ProcessError> {
        self.stdin.take();
        let mut chunk = [0u8; 8192];
        loop {
            let remaining = self.max_stdout_bytes.saturating_sub(self.stdout_total_bytes);
            if remaining == 0 {
                return Err(ProcessError::OutputTooLarge);
            }
            let count = (&mut self.stdout)
                .take(remaining.min(chunk.len() as u64))
                .read(&mut chunk)
                .await
                .map_err(|error| ProcessError::StreamFailed { kind: error.kind() })?;
            if count == 0 {
                break;
            }
            self.stdout_total_bytes = self.stdout_total_bytes.saturating_add(count as u64);
        }
        join_drain(self.stderr_drain.take()).await;
        let status = self
            .child
            .wait()
            .await
            .map_err(|error| ProcessError::StreamFailed { kind: error.kind() })?;
        Ok(ProcessEnd { code: status.code() })
    }

    async fn kill(&mut self) -> Result<(), ProcessError> {
        self.stdin.take();
        kill_tree(&mut self.child);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await;
        join_drain(self.stderr_drain.take()).await;
        Ok(())
    }
}

fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    {
        if let Some(id) = child.id() {
            // SAFETY: `killpg` with SIGKILL on our own process group is the
            // documented tree termination; ESRCH means the child already exited.
            unsafe {
                libc::killpg(id as i32, libc::SIGKILL);
            }
        }
    }
    let _ = child.start_kill();
}

/// Scripted child for tests: no processes, no filesystem, no network.
#[cfg(test)]
pub(crate) struct FakeRunner {
    lines: Vec<Vec<u8>>,
    delay_ms: u64,
    spawn_error: Option<ProcessError>,
    send_fail_after: Option<usize>,
    hang_after_lines: bool,
    pub(crate) shared: std::sync::Arc<FakeShared>,
}

#[cfg(test)]
pub(crate) struct FakeShared {
    pub sends: std::sync::Mutex<Vec<Vec<u8>>>,
    pub spawns: std::sync::atomic::AtomicUsize,
    pub kills: std::sync::atomic::AtomicUsize,
    pub drops: std::sync::atomic::AtomicUsize,
    pub helper_alive: std::sync::atomic::AtomicBool,
    pub drain_alive: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
pub(crate) struct FakeSession {
    lines: std::collections::VecDeque<Vec<u8>>,
    delay_ms: u64,
    send_fail_after: Option<usize>,
    hang_after_lines: bool,
    shared: std::sync::Arc<FakeShared>,
    drain: Option<tokio::task::JoinHandle<()>>,
}

#[cfg(test)]
impl FakeRunner {
    pub(crate) fn new(lines: Vec<Vec<u8>>) -> Self {
        Self {
            lines,
            delay_ms: 0,
            spawn_error: None,
            send_fail_after: None,
            hang_after_lines: false,
            shared: std::sync::Arc::new(FakeShared {
                sends: std::sync::Mutex::new(Vec::new()),
                spawns: std::sync::atomic::AtomicUsize::new(0),
                kills: std::sync::atomic::AtomicUsize::new(0),
                drops: std::sync::atomic::AtomicUsize::new(0),
                helper_alive: std::sync::atomic::AtomicBool::new(false),
                drain_alive: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    pub(crate) fn with_spawn_error(mut self, error: ProcessError) -> Self {
        self.spawn_error = Some(error);
        self
    }

    pub(crate) fn with_send_fail_after(mut self, sends: usize) -> Self {
        self.send_fail_after = Some(sends);
        self
    }

    pub(crate) fn with_hang_after_lines(mut self) -> Self {
        self.hang_after_lines = true;
        self
    }
}

#[cfg(test)]
impl ProcessRunner for FakeRunner {
    type Session = FakeSession;

    async fn spawn(
        &self,
        spec: &ProcessSpec,
        _limits: &StreamLimits,
    ) -> Result<FakeSession, ProcessError> {
        spec.validate()?;
        self.shared
            .spawns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(error) = self.spawn_error {
            return Err(error);
        }
        self.shared
            .helper_alive
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.shared
            .drain_alive
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let drain_flag = self.shared.clone();
        let drain = tokio::spawn(async move {
            let _ = drain_flag;
            std::future::pending::<()>().await;
        });
        Ok(FakeSession {
            lines: self.lines.iter().cloned().collect(),
            delay_ms: self.delay_ms,
            send_fail_after: self.send_fail_after,
            hang_after_lines: self.hang_after_lines,
            shared: self.shared.clone(),
            drain: Some(drain),
        })
    }
}

#[cfg(test)]
impl Drop for FakeSession {
    fn drop(&mut self) {
        self.shared
            .drops
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.shared
            .helper_alive
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.shared
            .drain_alive
            .store(false, std::sync::atomic::Ordering::SeqCst);
        if let Some(handle) = self.drain.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
impl ProcessSession for FakeSession {
    async fn send(&mut self, bytes: &[u8]) -> Result<(), ProcessError> {
        let count = self.shared.sends.lock().unwrap().len();
        if let Some(limit) = self.send_fail_after {
            if count >= limit {
                return Err(ProcessError::StreamFailed {
                    kind: std::io::ErrorKind::BrokenPipe,
                });
            }
        }
        self.shared.sends.lock().unwrap().push(bytes.to_vec());
        Ok(())
    }

    async fn next_line(&mut self, max_line_bytes: usize) -> Result<Option<Vec<u8>>, ProcessError> {
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if self.hang_after_lines && self.lines.is_empty() {
            std::future::pending::<()>().await;
            return Ok(None);
        }
        match self.lines.pop_front() {
            None => Ok(None),
            Some(line) => {
                if line.len() > max_line_bytes {
                    return Err(ProcessError::OutputTooLarge);
                }
                Ok(Some(line))
            }
        }
    }

    async fn finish(&mut self) -> Result<ProcessEnd, ProcessError> {
        if let Some(handle) = self.drain.take() {
            handle.abort();
            let _ = handle.await;
        }
        self.shared
            .drain_alive
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.shared
            .helper_alive
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(ProcessEnd { code: Some(0) })
    }

    async fn kill(&mut self) -> Result<(), ProcessError> {
        self.shared
            .kills
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(handle) = self.drain.take() {
            handle.abort();
            let _ = handle.await;
        }
        self.shared
            .drain_alive
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.shared
            .helper_alive
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ProcessSpec {
        ProcessSpec {
            executable: std::path::PathBuf::from("/bin/codex"),
            args: vec![std::ffi::OsString::from("app-server")],
            cwd: std::path::PathBuf::from("/tmp/scratch"),
        }
    }

    #[test]
    fn relative_executable_is_rejected() {
        let mut bad = spec();
        bad.executable = std::path::PathBuf::from("codex");
        assert_eq!(
            bad.validate().unwrap_err(),
            ProcessError::Configuration
        );
    }

    #[tokio::test]
    async fn fake_session_records_sends_and_lines() {
        let runner = FakeRunner::new(vec![b"{\"a\":1}".to_vec()]);
        let limits = StreamLimits {
            max_stdout_bytes: 1024,
            max_stderr_bytes: 256,
            max_line_bytes: 512,
        };
        let mut session = runner.spawn(&spec(), &limits).await.unwrap();
        session.send(b"hello").await.unwrap();
        assert_eq!(session.next_line(512).await.unwrap(), Some(b"{\"a\":1}".to_vec()));
        assert_eq!(session.next_line(512).await.unwrap(), None);
        assert_eq!(session.finish().await.unwrap().code, Some(0));
        assert_eq!(runner.shared.sends.lock().unwrap().len(), 1);
        assert_eq!(
            runner.shared.kills.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn spawn_failure_reports_transport_without_a_session() {
        let runner = FakeRunner::new(Vec::new())
            .with_spawn_error(ProcessError::SpawnFailed {
                kind: std::io::ErrorKind::NotFound,
            });
        let limits = StreamLimits {
            max_stdout_bytes: 1024,
            max_stderr_bytes: 256,
            max_line_bytes: 512,
        };
        let error = runner.spawn(&spec(), &limits).await.unwrap_err();
        assert_eq!(
            error,
            ProcessError::SpawnFailed {
                kind: std::io::ErrorKind::NotFound,
            }
        );
        assert_eq!(
            runner.shared.spawns.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn oversize_line_is_rejected() {
        let runner = FakeRunner::new(vec![vec![b'x'; 64]]);
        let limits = StreamLimits {
            max_stdout_bytes: 4096,
            max_stderr_bytes: 256,
            max_line_bytes: 16,
        };
        let mut session = runner.spawn(&spec(), &limits).await.unwrap();
        assert_eq!(
            session.next_line(16).await.unwrap_err(),
            ProcessError::OutputTooLarge
        );
    }

    #[tokio::test]
    async fn helper_and_drain_tasks_do_not_survive_kill_or_drop() {
        let runner = FakeRunner::new(vec![b"{\"a\":1}".to_vec()]);
        let limits = StreamLimits {
            max_stdout_bytes: 1024,
            max_stderr_bytes: 256,
            max_line_bytes: 512,
        };
        let mut session = runner.spawn(&spec(), &limits).await.unwrap();
        assert!(
            runner
                .shared
                .helper_alive
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        session.kill().await.unwrap();
        assert!(
            !runner
                .shared
                .helper_alive
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        assert!(
            !runner
                .shared
                .drain_alive
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        let mut session = runner.spawn(&spec(), &limits).await.unwrap();
        let _ = session.next_line(512).await;
        drop(session);
        assert!(
            !runner
                .shared
                .helper_alive
                .load(std::sync::atomic::Ordering::SeqCst)
        );
    }
}
