//! Owned Windows process tree for one interactive Codex child.
//!
//! The helper and every descendant it starts live in a private kill-on-close
//! job created before the first instruction runs: the job travels in the
//! creation attributes, so no helper ever runs outside the job. Closing the
//! last job handle terminates the whole tree, which is what makes
//! cancellation, deadline expiry and session drop leave neither helpers nor
//! stream tasks behind.
//!
//! This module never pins a CLI version, model name or tool policy, never
//! restricts the inherited environment, and never caps the process count:
//! the job carries only `KILL_ON_JOB_CLOSE`. Prompts and server text may flow
//! through these pipes, so finished sessions never retain byte content.

use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::ErrorKind,
    os::windows::ffi::OsStrExt,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
    ptr,
    time::Duration,
};

use tokio::io::AsyncWriteExt;
use windows_sys::Win32::{
    Foundation::{GetLastError, HANDLE, HANDLE_FLAG_INHERIT, STILL_ACTIVE, SetHandleInformation},
    Security::SECURITY_ATTRIBUTES,
    System::{
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Pipes::CreatePipe,
        Threading::{
            CREATE_NO_WINDOW, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
            LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
        },
    },
};

use super::process::{
    KILL_WAIT_TIMEOUT, ProcessError, ProcessSession, ProcessSpec, StreamLimits,
    discard_remaining_stdout, discard_stderr, join_drain, read_framed_line,
};

/// How often a finished child is re-polled while draining a session.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Well-known lookup failures for the child image, kept distinct from
/// resource failures so the caller reports transport honestly.
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_ACCESS_DENIED: u32 = 5;

/// Creates the private kill-on-close job. Only termination on close is set:
/// no process-count cap and no other limit travels with the helper.
fn create_job() -> Result<OwnedHandle, ProcessError> {
    // SAFETY: null attributes and name request a private unnamed job.
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if raw.is_null() {
        return Err(ProcessError::SpawnFailed {
            kind: ErrorKind::Other,
        });
    }
    // SAFETY: CreateJobObjectW returned an owned handle.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the job and properly sized limits are valid for the duration of the call.
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle().cast(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
    {
        return Err(ProcessError::SpawnFailed {
            kind: ErrorKind::Other,
        });
    }
    Ok(job)
}

struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}

impl Attributes {
    fn new() -> Result<Self, ProcessError> {
        let mut size = 0usize;
        // SAFETY: this first call obtains the required size and intentionally has no buffer.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 2, 0, &mut size) };
        if size == 0 {
            return Err(ProcessError::SpawnFailed {
                kind: ErrorKind::Other,
            });
        }
        let mut value = Self {
            storage: vec![0; size.div_ceil(std::mem::size_of::<usize>())],
            initialized: false,
        };
        // SAFETY: storage is properly aligned, large enough and remains owned until Drop.
        if unsafe { InitializeProcThreadAttributeList(value.pointer(), 2, 0, &mut size) } == 0 {
            return Err(ProcessError::SpawnFailed {
                kind: ErrorKind::Other,
            });
        }
        value.initialized = true;
        Ok(value)
    }

    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }

    fn set(
        &mut self,
        attribute: usize,
        data: *const std::ffi::c_void,
        bytes: usize,
    ) -> Result<(), ProcessError> {
        // SAFETY: the initialized list and caller-owned attribute bytes outlive CreateProcessW.
        if unsafe {
            UpdateProcThreadAttribute(
                self.pointer(),
                0,
                attribute,
                data,
                bytes,
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(ProcessError::SpawnFailed {
                kind: ErrorKind::Other,
            });
        }
        Ok(())
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: the list was initialized once and is still backed by storage.
            unsafe { DeleteProcThreadAttributeList(self.pointer()) };
        }
    }
}

fn pipe(child_reads: bool) -> Result<(OwnedHandle, OwnedHandle), ProcessError> {
    let mut read: HANDLE = ptr::null_mut();
    let mut write: HANDLE = ptr::null_mut();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: both output pointers and the security descriptor are valid.
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(ProcessError::SpawnFailed {
            kind: ErrorKind::Other,
        });
    }
    // SAFETY: CreatePipe returned two owned handles.
    let read = unsafe { OwnedHandle::from_raw_handle(read) };
    let write = unsafe { OwnedHandle::from_raw_handle(write) };
    let parent = if child_reads { &write } else { &read };
    // SAFETY: parent is an open pipe handle; clearing inheritance leaves only the child end
    // inherited.
    if unsafe { SetHandleInformation(parent.as_raw_handle().cast(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(ProcessError::SpawnFailed {
            kind: ErrorKind::Other,
        });
    }
    Ok((read, write))
}

fn wide(value: &OsStr) -> Result<Vec<u16>, ProcessError> {
    let value: Vec<u16> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(ProcessError::Configuration);
    }
    Ok(value)
}

fn quoted(value: &OsStr) -> Result<Vec<u16>, ProcessError> {
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for character in wide(value)? {
        if character == b'\\' as u16 {
            slashes += 1;
        } else {
            let count = if character == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            };
            result.extend(std::iter::repeat_n(b'\\' as u16, count));
            result.push(character);
            slashes = 0;
        }
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    Ok(result)
}

fn command_line(executable: &Path, args: &[OsString]) -> Result<Vec<u16>, ProcessError> {
    let mut result = quoted(executable.as_os_str())?;
    for arg in args {
        result.push(b' ' as u16);
        result.extend(quoted(arg)?);
    }
    result.push(0);
    Ok(result)
}

fn spawn_failed() -> ProcessError {
    // SAFETY: reads the calling thread's last Win32 error.
    let kind = match unsafe { GetLastError() } {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => ErrorKind::NotFound,
        ERROR_ACCESS_DENIED => ErrorKind::PermissionDenied,
        _ => ErrorKind::Other,
    };
    ProcessError::SpawnFailed { kind }
}

struct SpawnedChild {
    job: OwnedHandle,
    process: OwnedHandle,
}

fn spawn_in_job(
    executable: &Path,
    args: &[OsString],
    directory: &Path,
    input: &OwnedHandle,
    stdout: &OwnedHandle,
    stderr: &OwnedHandle,
) -> Result<SpawnedChild, ProcessError> {
    let handles: [HANDLE; 3] = [
        input.as_raw_handle().cast(),
        stdout.as_raw_handle().cast(),
        stderr.as_raw_handle().cast(),
    ];
    let job = create_job()?;
    let jobs: [HANDLE; 1] = [job.as_raw_handle().cast()];
    let mut attributes = Attributes::new()?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        handles.as_ptr().cast(),
        std::mem::size_of_val(&handles),
    )?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
        jobs.as_ptr().cast(),
        std::mem::size_of_val(&jobs),
    )?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = handles[0];
    startup.StartupInfo.hStdOutput = handles[1];
    startup.StartupInfo.hStdError = handles[2];
    startup.lpAttributeList = attributes.pointer();
    let mut program = wide(executable.as_os_str())?;
    program.push(0);
    let mut cwd = wide(directory.as_os_str())?;
    cwd.push(0);
    let mut command = command_line(executable, args)?;
    let mut process = PROCESS_INFORMATION::default();
    // The child inherits the ambient environment so the CLI keeps its
    // configured authentication; assignment below is atomic, so a helper
    // that starts before it cannot exist.
    // SAFETY: all pointers reference live NUL-terminated inputs, the handle list contains only
    // our inherited stdio handles, and JOB_LIST assigns the child before its first instruction.
    if unsafe {
        CreateProcessW(
            program.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
            ptr::null(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    } == 0
    {
        return Err(spawn_failed());
    }
    // SAFETY: successful CreateProcessW transfers both non-null handles to this process.
    let child = unsafe { OwnedHandle::from_raw_handle(process.hProcess) };
    let thread = unsafe { OwnedHandle::from_raw_handle(process.hThread) };
    drop(thread);
    Ok(SpawnedChild {
        job,
        process: child,
    })
}

/// Spawns one owned helper inside its own kill-on-close job. The job is
/// assigned atomically at creation; any failure before the session returns
/// drops every handle, so no unowned helper survives a failed spawn.
pub(crate) fn spawn(spec: &ProcessSpec, limits: &StreamLimits) -> Result<JobSession, ProcessError> {
    spec.validate()?;
    let (stdin_read, stdin_write) = pipe(true)?;
    let (stdout_read, stdout_write) = pipe(false)?;
    let (stderr_read, stderr_write) = pipe(false)?;
    let child = spawn_in_job(
        &spec.executable,
        &spec.args,
        &spec.cwd,
        &stdin_read,
        &stdout_write,
        &stderr_write,
    )?;
    drop((stdin_read, stdout_write, stderr_write));
    let stdin_file: File = stdin_write.into();
    let stdout_file: File = stdout_read.into();
    let stderr_file: File = stderr_read.into();
    let stderr_drain = tokio::spawn(discard_stderr(
        tokio::fs::File::from_std(stderr_file),
        limits.max_stderr_bytes,
    ));
    Ok(JobSession {
        process: child.process,
        stdin: Some(tokio::fs::File::from_std(stdin_file)),
        stdout: tokio::io::BufReader::new(tokio::fs::File::from_std(stdout_file)),
        stderr_drain: Some(stderr_drain),
        stdout_total_bytes: 0,
        max_stdout_bytes: limits.max_stdout_bytes,
        job: Some(child.job),
    })
}

fn exit_code(process: &OwnedHandle) -> Result<Option<u32>, ProcessError> {
    let mut code = 0u32;
    // SAFETY: the process handle remains owned and code points to writable memory.
    if unsafe { GetExitCodeProcess(process.as_raw_handle().cast(), &mut code) } == 0 {
        return Err(ProcessError::StreamFailed {
            kind: ErrorKind::Other,
        });
    }
    if code == STILL_ACTIVE as u32 {
        Ok(None)
    } else {
        Ok(Some(code))
    }
}

async fn wait_for_exit(process: &OwnedHandle) {
    loop {
        match exit_code(process) {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => tokio::time::sleep(POLL_INTERVAL).await,
        }
    }
}

/// An interactive child session: line reads interleave with request writes,
/// and the job is closed last so drop always terminates the whole tree.
#[derive(Debug)]
pub(crate) struct JobSession {
    process: OwnedHandle,
    stdin: Option<tokio::fs::File>,
    stdout: tokio::io::BufReader<tokio::fs::File>,
    stderr_drain: Option<tokio::task::JoinHandle<()>>,
    stdout_total_bytes: u64,
    max_stdout_bytes: u64,
    job: Option<OwnedHandle>,
}

impl Drop for JobSession {
    fn drop(&mut self) {
        if let Some(handle) = self.stderr_drain.take() {
            handle.abort();
        }
        // The job handle drops here: kill-on-close terminates the whole tree,
        // so a dropped session owns the same cleanup as an explicit kill.
    }
}

impl ProcessSession for JobSession {
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
        wait_for_exit(&self.process).await;
        Ok(())
    }

    async fn kill(&mut self) -> Result<(), ProcessError> {
        self.stdin.take();
        // SAFETY: best-effort stop of the direct child; closing the job below
        // terminates any helper the child already started.
        unsafe {
            TerminateProcess(self.process.as_raw_handle().cast(), 1);
        }
        drop(self.job.take());
        // A bounded wait: expiry is harmless because the closed job already
        // terminated every member; the remaining drops only release handles.
        let _ = tokio::time::timeout(KILL_WAIT_TIMEOUT, wait_for_exit(&self.process)).await;
        join_drain(&mut self.stderr_drain).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::Path};

    use crate::backends::codex_cli::fixtures::{
        cleanup_dir, powershell_path, read_pid_pair, running, scratch_dir, tree_script,
        wait_for_file, wait_until_gone,
    };
    use crate::backends::codex_cli::process::{
        ProcessError, ProcessRunner, ProcessSession, ProcessSpec, StreamLimits, TokioProcessRunner,
    };

    fn limits() -> StreamLimits {
        StreamLimits {
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 256 * 1024,
            max_line_bytes: 4096,
        }
    }

    fn helper_spec(program: &Path, script: &str, dir: &Path) -> ProcessSpec {
        ProcessSpec {
            executable: program.to_owned(),
            args: vec![
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-Command"),
                OsString::from(script),
            ],
            cwd: dir.to_owned(),
        }
    }

    #[tokio::test]
    async fn dropped_session_kills_the_helper_tree() {
        let dir = scratch_dir("drop");
        let marker = dir.join("pids.txt");
        let runner = TokioProcessRunner;
        let session = runner
            .spawn(
                &helper_spec(&powershell_path(), &tree_script(&marker, ""), &dir),
                &limits(),
            )
            .await
            .expect("the benign helper spawns inside its job");
        let (helper, grandchild) = read_pid_pair(&wait_for_file(&marker).await);
        assert_ne!(helper, grandchild);
        assert!(running(helper), "the helper is alive before the drop");
        assert!(running(grandchild), "the grandchild joined before the drop");
        drop(session);
        wait_until_gone(helper).await;
        wait_until_gone(grandchild).await;
        cleanup_dir(&dir);
    }

    #[tokio::test]
    async fn kill_terminates_the_helper_tree() {
        let dir = scratch_dir("kill");
        let marker = dir.join("pids.txt");
        let runner = TokioProcessRunner;
        let mut session = runner
            .spawn(
                &helper_spec(&powershell_path(), &tree_script(&marker, ""), &dir),
                &limits(),
            )
            .await
            .expect("the benign helper spawns inside its job");
        let (helper, grandchild) = read_pid_pair(&wait_for_file(&marker).await);
        assert_ne!(helper, grandchild);
        session.kill().await.expect("kill stops the owned tree");
        wait_until_gone(helper).await;
        wait_until_gone(grandchild).await;
        session
            .finish()
            .await
            .expect("finish after kill reports the exit");
        cleanup_dir(&dir);
    }

    #[tokio::test]
    async fn missing_helper_image_is_a_transport_failure() {
        let dir = scratch_dir("missing-exe");
        let runner = TokioProcessRunner;
        let spec = ProcessSpec {
            executable: dir.join("missing-codex.exe"),
            args: vec![OsString::from("app-server")],
            cwd: dir.clone(),
        };
        let error = runner.spawn(&spec, &limits()).await.unwrap_err();
        assert_eq!(
            error,
            ProcessError::SpawnFailed {
                kind: std::io::ErrorKind::NotFound,
            }
        );
        cleanup_dir(&dir);
    }
}
