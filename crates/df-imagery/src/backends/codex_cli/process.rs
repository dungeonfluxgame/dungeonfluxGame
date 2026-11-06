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
//! sessions never retain byte content.

use std::ffi::OsString;
use std::path::PathBuf;
#[cfg(not(windows))]
use std::process::Stdio;

#[cfg(not(windows))]
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt};

#[cfg(windows)]
use super::job;

/// Why the child could not be driven to a protocol exchange.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessError {
    Configuration,
    SpawnFailed { kind: std::io::ErrorKind },
    StreamFailed { kind: std::io::ErrorKind },
    OutputTooLarge,
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration => formatter.write_str("the codex child configuration is invalid"),
            Self::SpawnFailed { kind } => {
                write!(formatter, "the codex child could not start: {kind}")
            }
            Self::StreamFailed { kind } => {
                write!(formatter, "the codex child stream failed: {kind}")
            }
            Self::OutputTooLarge => formatter.write_str("the codex child output is too large"),
        }
    }
}

impl std::error::Error for ProcessError {}

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

/// An interactive child session: line reads interleave with request writes.
pub(crate) trait ProcessSession {
    async fn send(&mut self, bytes: &[u8]) -> Result<(), ProcessError>;
    async fn next_line(&mut self, max_line_bytes: usize) -> Result<Option<Vec<u8>>, ProcessError>;
    /// Closes stdin, drains and discards the remaining output under its bounds,
    /// then reaps the child. No exit status is kept: a decided outcome never
    /// depends on it.
    async fn finish(&mut self) -> Result<(), ProcessError>;
    async fn kill(&mut self) -> Result<(), ProcessError>;
}

/// Spawns one owned child session per call.
pub(crate) trait ProcessRunner {
    type Session: ProcessSession;

    async fn spawn(
        &self,
        spec: &ProcessSpec,
        limits: &StreamLimits,
    ) -> Result<Self::Session, ProcessError>;
}

/// The real runner: an owned child with piped stdio and kill-on-drop.
pub(crate) struct TokioProcessRunner;

#[cfg(not(windows))]
pub(crate) struct TokioSession {
    child: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    stderr_drain: Option<tokio::task::JoinHandle<()>>,
    stdout_total_bytes: u64,
    max_stdout_bytes: u64,
    /// Process group of the helper, retained so a dropped session can signal
    /// the same tree an explicit kill would. The group is created by a
    /// checked `setsid` before exec, so the child is its own group leader.
    #[cfg(unix)]
    process_group: Option<i32>,
}

#[cfg(unix)]
fn enable_process_group(command: &mut tokio::process::Command) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: `setsid` in the child before exec isolates helpers in one group.
    // A failed `setsid` refuses the spawn, so a running child is always its
    // own group leader and the retained group identity below stays valid.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(windows))]
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
        #[cfg(unix)]
        {
            enable_process_group(&mut command);
        }
        let mut child = command
            .spawn()
            .map_err(|error| ProcessError::SpawnFailed { kind: error.kind() })?;
        let stdin = child.stdin.take().ok_or(ProcessError::Configuration)?;
        let stdout = child.stdout.take().ok_or(ProcessError::Configuration)?;
        let stderr = child.stderr.take().ok_or(ProcessError::Configuration)?;
        let stderr_drain = tokio::spawn(discard_stderr(stderr, limits.max_stderr_bytes));
        #[cfg(unix)]
        let process_group = child.id().map(|id| id as i32);
        Ok(TokioSession {
            child,
            stdin: Some(stdin),
            stdout: tokio::io::BufReader::new(stdout),
            stderr_drain: Some(stderr_drain),
            stdout_total_bytes: 0,
            max_stdout_bytes: limits.max_stdout_bytes,
            #[cfg(unix)]
            process_group,
        })
    }
}

#[cfg(windows)]
impl ProcessRunner for TokioProcessRunner {
    type Session = job::JobSession;

    async fn spawn(
        &self,
        spec: &ProcessSpec,
        limits: &StreamLimits,
    ) -> Result<Self::Session, ProcessError> {
        job::spawn(spec, limits)
    }
}

#[cfg(not(windows))]
impl Drop for TokioSession {
    fn drop(&mut self) {
        if let Some(handle) = self.stderr_drain.take() {
            handle.abort();
        }
        // The direct child also dies through kill_on_drop, but descendants
        // joined only the process group: signal the whole group so a dropped
        // session owns the same tree cleanup as an explicit kill. The group
        // identity was retained at spawn from a checked `setsid`, so it names
        // exactly this helper tree; ESRCH only means the tree already exited.
        #[cfg(unix)]
        if let Some(group) = self.process_group {
            // SAFETY: `killpg` with SIGKILL on our own process group.
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
        }
    }
}

/// How long `kill` waits for the killed tree to exit. Expiry is harmless: the
/// tree was already signalled, and the remaining drops only release handles.
pub(crate) const KILL_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Size of the reusable buffer for stream bytes that are read and discarded.
const DISCARD_CHUNK_BYTES: usize = 8192;

/// Owned discard drain for stderr: a bounded cumulative read that never
/// retains bytes, because the pipe may carry prompts and server text. A read
/// error ends the drain; nothing is reported because nothing is kept.
pub(crate) async fn discard_stderr<R>(mut stderr: R, max_stderr_bytes: u64)
where
    R: AsyncRead + Unpin,
{
    let mut total_bytes: u64 = 0;
    let mut chunk = [0u8; DISCARD_CHUNK_BYTES];
    loop {
        let remaining_bytes = max_stderr_bytes.saturating_sub(total_bytes);
        if remaining_bytes == 0 {
            break;
        }
        let take = remaining_bytes.min(chunk.len() as u64) as usize;
        match stderr.read(&mut chunk[..take]).await {
            Ok(0) | Err(_) => break,
            Ok(count) => total_bytes = total_bytes.saturating_add(count as u64),
        }
    }
}

/// Reads one newline-framed stdout line under the per-line bound and the
/// cumulative stdout bound, then strips the LF or CRLF frame.
pub(crate) async fn read_framed_line<R>(
    stdout: &mut R,
    stdout_total_bytes: &mut u64,
    max_stdout_bytes: u64,
    max_line_bytes: usize,
) -> Result<Option<Vec<u8>>, ProcessError>
where
    R: AsyncBufRead + Unpin,
{
    let remaining_total = max_stdout_bytes.saturating_sub(*stdout_total_bytes);
    let cap = (max_line_bytes as u64)
        .saturating_add(1)
        .min(remaining_total);
    if cap == 0 {
        return Err(ProcessError::OutputTooLarge);
    }
    let mut line = Vec::new();
    let count = (&mut *stdout)
        .take(cap)
        .read_until(b'\n', &mut line)
        .await
        .map_err(|error| ProcessError::StreamFailed { kind: error.kind() })?;
    if count == 0 {
        return Ok(None);
    }
    *stdout_total_bytes = stdout_total_bytes.saturating_add(line.len() as u64);
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

/// Reads and discards the rest of stdout under the cumulative bound, so a
/// finishing child is never blocked on a full pipe.
pub(crate) async fn discard_remaining_stdout<R>(
    stdout: &mut R,
    stdout_total_bytes: &mut u64,
    max_stdout_bytes: u64,
) -> Result<(), ProcessError>
where
    R: AsyncRead + Unpin,
{
    let mut chunk = [0u8; DISCARD_CHUNK_BYTES];
    loop {
        let remaining_bytes = max_stdout_bytes.saturating_sub(*stdout_total_bytes);
        if remaining_bytes == 0 {
            return Err(ProcessError::OutputTooLarge);
        }
        let count = (&mut *stdout)
            .take(remaining_bytes.min(chunk.len() as u64))
            .read(&mut chunk)
            .await
            .map_err(|error| ProcessError::StreamFailed { kind: error.kind() })?;
        if count == 0 {
            return Ok(());
        }
        *stdout_total_bytes = stdout_total_bytes.saturating_add(count as u64);
    }
}

/// How long the owner waits for the owned stderr drain to finish before it
/// aborts that task and rejoins it. The absolute turn deadline always
/// outranks this grace; this constant only bounds the drain itself.
pub(crate) const DRAIN_JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Joins an owned stderr drain under a bound without ever detaching it: the
/// handle stays borrowed while the timeout runs, and a slow drain is aborted
/// and rejoined instead of dropped into the background.
pub(crate) async fn join_drain(slot: &mut Option<tokio::task::JoinHandle<()>>) {
    join_drain_timeout(slot, DRAIN_JOIN_TIMEOUT).await;
}

pub(crate) async fn join_drain_timeout(
    slot: &mut Option<tokio::task::JoinHandle<()>>,
    timeout: std::time::Duration,
) {
    let Some(handle) = slot.as_mut() else {
        return;
    };
    if tokio::time::timeout(timeout, &mut *handle).await.is_ok() {
        let _ = slot.take();
        return;
    }
    handle.abort();
    let _ = (&mut *handle).await;
    let _ = slot.take();
}

#[cfg(not(windows))]
impl ProcessSession for TokioSession {
    async fn send(&mut self, bytes: &[u8]) -> Result<(), ProcessError> {
        let stdin = self.stdin.as_mut().ok_or(ProcessError::StreamFailed {
            kind: std::io::ErrorKind::BrokenPipe,
        })?;
        stdin
            .write_all(bytes)
            .await
            .map_err(|error| ProcessError::StreamFailed { kind: error.kind() })
    }

    async fn next_line(&mut self, max_line_bytes: usize) -> Result<Option<Vec<u8>>, ProcessError> {
        read_framed_line(
            &mut self.stdout,
            &mut self.stdout_total_bytes,
            self.max_stdout_bytes,
            max_line_bytes,
        )
        .await
    }

    async fn finish(&mut self) -> Result<(), ProcessError> {
        self.stdin.take();
        discard_remaining_stdout(
            &mut self.stdout,
            &mut self.stdout_total_bytes,
            self.max_stdout_bytes,
        )
        .await?;
        join_drain(&mut self.stderr_drain).await;
        self.child
            .wait()
            .await
            .map_err(|error| ProcessError::StreamFailed { kind: error.kind() })?;
        Ok(())
    }

    async fn kill(&mut self) -> Result<(), ProcessError> {
        self.stdin.take();
        kill_tree(&mut self.child);
        // Expiry or a wait error is harmless: the tree was already signalled.
        let _ = tokio::time::timeout(KILL_WAIT_TIMEOUT, self.child.wait()).await;
        join_drain(&mut self.stderr_drain).await;
        Ok(())
    }
}

#[cfg(not(windows))]
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
    // A failed signal means the direct child already exited; the group kill
    // above covers its helpers either way.
    let _ = child.start_kill();
}

/// Scripted child for tests: no processes, no filesystem, no network.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct FakeRunner {
    lines: Vec<Vec<u8>>,
    delay_ms: u64,
    spawn_error: Option<ProcessError>,
    send_fail_after: Option<usize>,
    hang_after_lines: bool,
    hang_after_sends: Option<usize>,
    pub(crate) shared: std::sync::Arc<FakeShared>,
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct FakeShared {
    pub sends: std::sync::Mutex<Vec<Vec<u8>>>,
    pub turn_dispatched: tokio::sync::Notify,
    pub spawns: std::sync::atomic::AtomicUsize,
    pub kills: std::sync::atomic::AtomicUsize,
    pub drops: std::sync::atomic::AtomicUsize,
    pub helper_alive: std::sync::atomic::AtomicBool,
    pub drain_alive: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct FakeSession {
    lines: std::collections::VecDeque<Vec<u8>>,
    delay_ms: u64,
    send_fail_after: Option<usize>,
    hang_after_lines: bool,
    hang_after_sends: Option<usize>,
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
            hang_after_sends: None,
            shared: std::sync::Arc::new(FakeShared {
                sends: std::sync::Mutex::new(Vec::new()),
                turn_dispatched: tokio::sync::Notify::new(),
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

    /// Makes request writes stall once `sends` writes already succeeded, so
    /// cancellation and deadline paths can prove the owned tree still dies
    /// with a blocked stdin pipe. Zero stalls every write.
    pub(crate) fn with_hang_after_sends(mut self, sends: usize) -> Self {
        self.hang_after_sends = Some(sends);
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
            hang_after_sends: self.hang_after_sends,
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
        if let Some(limit) = self.hang_after_sends
            && count >= limit
        {
            std::future::pending::<()>().await;
            return Ok(());
        }
        if let Some(limit) = self.send_fail_after
            && count >= limit
        {
            return Err(ProcessError::StreamFailed {
                kind: std::io::ErrorKind::BrokenPipe,
            });
        }
        self.shared.sends.lock().unwrap().push(bytes.to_vec());
        if serde_json::from_slice::<serde_json::Value>(bytes)
            .is_ok_and(|value| value["method"] == "turn/start")
        {
            self.shared.turn_dispatched.notify_one();
        }
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

    async fn finish(&mut self) -> Result<(), ProcessError> {
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
#[path = "process_tests.rs"]
mod tests;
