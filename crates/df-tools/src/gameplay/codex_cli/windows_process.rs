//! Race-free Windows job ownership for the Codex child and its descendants.

use std::{
    ffi::{OsStr, OsString, c_void},
    fs::File,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{HANDLE, HANDLE_FLAG_INHERIT, LocalFree, STILL_ACTIVE, SetHandleInformation},
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::CreateDirectoryW,
    System::{
        Com::CoTaskMemFree,
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Pipes::CreatePipe,
        Threading::{
            CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
        },
    },
    UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath},
};

use super::CodexError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct Output {
    pub(super) code: u32,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

/// Reject system Codex config that could add tools outside this request's control.
pub(super) fn system_config_empty() -> Result<(), CodexError> {
    let mut raw = ptr::null_mut();
    // SAFETY: the system returns an allocated NUL-terminated string, released below.
    if unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, ptr::null_mut(), &mut raw) } < 0
        || raw.is_null()
    {
        return Err(CodexError::Configuration);
    }
    let mut count = 0usize;
    // SAFETY: Windows returns a NUL-terminated string and valid path lengths are bounded.
    while count < 32768 && unsafe { *raw.add(count) } != 0 {
        count += 1;
    }
    if count == 32768 {
        // SAFETY: this allocation was returned by SHGetKnownFolderPath.
        unsafe { CoTaskMemFree(raw.cast()) };
        return Err(CodexError::Configuration);
    }
    // SAFETY: the counted prefix lies in the NUL-terminated Windows allocation.
    let root = std::path::PathBuf::from(OsString::from_wide(unsafe {
        std::slice::from_raw_parts(raw, count)
    }));
    // SAFETY: this allocation was returned by SHGetKnownFolderPath.
    unsafe { CoTaskMemFree(raw.cast()) };
    let directory = root.join("OpenAI/Codex");
    if directory.join("config.toml").exists() || directory.join("requirements.toml").exists() {
        return Err(CodexError::UnsafeTools);
    }
    Ok(())
}

fn wide(value: &OsStr) -> Result<Vec<u16>, CodexError> {
    let value: Vec<u16> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(CodexError::Configuration);
    }
    Ok(value)
}

/// Creates a protected directory whose owner is the only ordinary user allowed access.
pub(super) fn private_directory(path: &Path) -> Result<(), CodexError> {
    let mut name = wide(path.as_os_str())?;
    name.push(0);
    let mut descriptor = ptr::null_mut();
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)\0"
        .encode_utf16()
        .collect();
    // SAFETY: the SDDL string is NUL-terminated and the returned descriptor is released below.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(CodexError::Configuration);
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: path and security attributes remain valid throughout the call.
    let created = unsafe { CreateDirectoryW(name.as_ptr(), &attributes) };
    // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocated this block.
    unsafe { LocalFree(descriptor.cast()) };
    if created == 0 {
        return Err(CodexError::Configuration);
    }
    Ok(())
}

fn quoted(value: &OsStr) -> Result<Vec<u16>, CodexError> {
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

fn command_line(executable: &Path, args: &[OsString]) -> Result<Vec<u16>, CodexError> {
    let mut result = quoted(executable.as_os_str())?;
    for arg in args {
        result.push(b' ' as u16);
        result.extend(quoted(arg)?);
    }
    result.push(0);
    Ok(result)
}

fn environment() -> Result<Vec<u16>, CodexError> {
    let mut entries = Vec::new();
    for name in [
        "PATH",
        "SYSTEMROOT",
        "WINDIR",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "CODEX_HOME",
    ] {
        let value = if name == "CODEX_HOME" {
            std::env::var_os("DF_GAMEPLAY_CODEX_HOME")
        } else {
            std::env::var_os(name)
        };
        if let Some(value) = value {
            let mut item = OsString::from(name);
            item.push("=");
            item.push(value);
            entries.push(item);
        }
    }
    entries.sort_by_key(|entry| entry.to_string_lossy().to_ascii_uppercase());
    let mut block = Vec::new();
    for entry in entries {
        block.extend(wide(&entry)?);
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Ok(block)
}

fn job() -> Result<OwnedHandle, CodexError> {
    // SAFETY: null attributes and name request a private unnamed job.
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if raw.is_null() {
        return Err(CodexError::Interrupted);
    }
    // SAFETY: CreateJobObjectW returned an owned handle.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
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
        return Err(CodexError::Interrupted);
    }
    Ok(job)
}

struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}

impl Attributes {
    fn new() -> Result<Self, CodexError> {
        let mut size = 0usize;
        // SAFETY: this first call obtains the required size and intentionally has no buffer.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 2, 0, &mut size) };
        if size == 0 {
            return Err(CodexError::Interrupted);
        }
        let mut value = Self {
            storage: vec![0; size.div_ceil(std::mem::size_of::<usize>())],
            initialized: false,
        };
        // SAFETY: storage is properly aligned, large enough and remains owned until Drop.
        if unsafe { InitializeProcThreadAttributeList(value.pointer(), 2, 0, &mut size) } == 0 {
            return Err(CodexError::Interrupted);
        }
        value.initialized = true;
        Ok(value)
    }

    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }

    fn set(&mut self, attribute: u32, data: *const c_void, bytes: usize) -> Result<(), CodexError> {
        // SAFETY: the initialized list and caller-owned attribute bytes outlive CreateProcessW.
        if unsafe {
            UpdateProcThreadAttribute(
                self.pointer(),
                0,
                attribute as usize,
                data,
                bytes,
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(CodexError::Interrupted);
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

struct Process {
    _job: OwnedHandle,
    process: OwnedHandle,
}

fn pipe(child_reads: bool) -> Result<(OwnedHandle, OwnedHandle), CodexError> {
    let mut read: HANDLE = ptr::null_mut();
    let mut write: HANDLE = ptr::null_mut();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: both output pointers and the security descriptor are valid.
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(CodexError::Interrupted);
    }
    // SAFETY: CreatePipe returned two owned handles.
    let read = unsafe { OwnedHandle::from_raw_handle(read) };
    let write = unsafe { OwnedHandle::from_raw_handle(write) };
    let parent = if child_reads { &write } else { &read };
    // SAFETY: parent is an open pipe handle; clearing inheritance leaves only the child end inherited.
    if unsafe { SetHandleInformation(parent.as_raw_handle().cast(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(CodexError::Interrupted);
    }
    Ok((read, write))
}

fn spawn(
    executable: &Path,
    args: &[OsString],
    directory: &Path,
    input: &OwnedHandle,
    stdout: &OwnedHandle,
    stderr: &OwnedHandle,
) -> Result<Process, CodexError> {
    let handles: [HANDLE; 3] = [
        input.as_raw_handle().cast(),
        stdout.as_raw_handle().cast(),
        stderr.as_raw_handle().cast(),
    ];
    let job = job()?;
    let jobs: [HANDLE; 1] = [job.as_raw_handle().cast()];
    let mut attributes = Attributes::new()?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        handles.as_ptr().cast(),
        std::mem::size_of_val(&handles),
    )?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_JOB_LIST,
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
    let environment = environment()?;
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: all pointers reference live NUL-terminated inputs, the handle list contains only
    // our inherited stdio handles, and JOB_LIST assigns the child before its first instruction.
    if unsafe {
        CreateProcessW(
            program.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    } == 0
    {
        return Err(CodexError::Interrupted);
    }
    // SAFETY: successful CreateProcessW transfers both non-null handles to this process.
    let child = unsafe { OwnedHandle::from_raw_handle(process.hProcess) };
    let _thread = unsafe { OwnedHandle::from_raw_handle(process.hThread) };
    Ok(Process {
        _job: job,
        process: child,
    })
}

/// Runs a CLI child in a job that closes on cancellation, timeout, or parent exit.
pub(super) async fn run(
    executable: &Path,
    args: &[OsString],
    directory: &Path,
    input: &[u8],
    _label: &str,
    maximum: u64,
) -> Result<Output, CodexError> {
    let (stdin_read, stdin_write) = pipe(true)?;
    let (stdout_read, stdout_write) = pipe(false)?;
    let (stderr_read, stderr_write) = pipe(false)?;
    let child = spawn(
        executable,
        args,
        directory,
        &stdin_read,
        &stdout_write,
        &stderr_write,
    )?;
    drop((stdin_read, stdout_write, stderr_write));
    let wait = async move {
        let code = loop {
            let mut code = 0u32;
            // SAFETY: the process handle remains owned and code points to writable memory.
            if unsafe { GetExitCodeProcess(child.process.as_raw_handle().cast(), &mut code) } == 0 {
                return Err(CodexError::Interrupted);
            }
            if code != STILL_ACTIVE as u32 {
                break code;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        // Closing the job also terminates descendants that outlived the CLI.
        drop(child);
        Ok::<u32, CodexError>(code)
    };
    let read = |handle: OwnedHandle| async move {
        let file: File = handle.into();
        let mut pipe = tokio::fs::File::from_std(file);
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let count = pipe
                .read(&mut chunk)
                .await
                .map_err(|_| CodexError::Interrupted)?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) as u64 > maximum {
                return Err(CodexError::Output);
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        Ok::<Vec<u8>, CodexError>(bytes)
    };
    let write = async move {
        let file: File = stdin_write.into();
        let mut pipe = tokio::fs::File::from_std(file);
        pipe.write_all(input)
            .await
            .map_err(|_| CodexError::Interrupted)?;
        pipe.shutdown().await.map_err(|_| CodexError::Interrupted)?;
        Ok::<(), CodexError>(())
    };
    let (code, stdout, stderr, ()) =
        tokio::try_join!(wait, read(stdout_read), read(stderr_read), write)?;
    Ok(Output {
        code,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    fn powershell() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os("SYSTEMROOT").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe")
    }

    fn directory(name: &str) -> std::path::PathBuf {
        let root = std::env::current_dir()
            .unwrap()
            .join("artifacts/tmp/codex-cli-tests");
        std::fs::create_dir_all(&root).unwrap();
        let mut random = [0u8; 8];
        getrandom::getrandom(&mut random).unwrap();
        let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = root.join(format!("{name}-{id}"));
        std::fs::create_dir(&path).unwrap();
        path
    }

    fn args(script: &str) -> Vec<OsString> {
        ["-NoProfile", "-NonInteractive", "-Command", script]
            .into_iter()
            .map(OsString::from)
            .collect()
    }

    fn running(pid: u32) -> bool {
        // SAFETY: querying an existing process requires no inherited or privileged handle.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return false;
        }
        // SAFETY: OpenProcess returned an owned process handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut code = 0u32;
        // SAFETY: the handle is live and code is a valid writable pointer.
        unsafe {
            GetExitCodeProcess(handle.as_raw_handle().cast(), &mut code) != 0
                && code == STILL_ACTIVE as u32
        }
    }

    async fn cleanup(path: &Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while std::fs::remove_dir_all(path).is_err() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("test directory remained locked");
    }

    #[tokio::test]
    async fn transports_stdin_and_separate_bounded_outputs() {
        let dir = directory("io");
        let script = "$value=[Console]::In.ReadToEnd(); [Console]::Out.Write('OUT:'+ $value); [Console]::Error.Write('ERR')";
        let result = run(&powershell(), &args(script), &dir, b"hello", "io", 4096)
            .await
            .unwrap();
        assert_eq!(result.code, 0);
        assert_eq!(result.stdout, b"OUT:hello");
        assert_eq!(result.stderr, b"ERR");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        cleanup(&dir).await;
    }

    #[tokio::test]
    async fn oversized_output_terminates_the_writer() {
        for (name, stream) in [("stdout", "Out"), ("stderr", "Error")] {
            let dir = directory(name);
            let marker = dir.join("parent.pid");
            let path = marker.to_string_lossy().replace('\'', "''");
            let script = format!(
                "[IO.File]::WriteAllText('{path}', $PID); [Console]::{stream}.Write(('x' * 100000)); Start-Sleep -Seconds 60"
            );
            let result = tokio::time::timeout(
                Duration::from_secs(8),
                run(&powershell(), &args(&script), &dir, b"", name, 4096),
            )
            .await
            .unwrap();
            assert!(matches!(result, Err(CodexError::Output)));
            let pid: u32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while running(pid) {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .expect("output writer survived cancellation");
            cleanup(&dir).await;
        }
    }

    #[tokio::test]
    async fn owned_job_rejects_child_processes() {
        let dir = directory("tree");
        let script = "try { Start-Process -FilePath (Join-Path $env:SYSTEMROOT 'System32/WindowsPowerShell/v1.0/powershell.exe') -ArgumentList '-NoProfile -Command exit' -WindowStyle Hidden -ErrorAction Stop | Out-Null; [Console]::Out.Write('UNEXPECTED') } catch { [Console]::Out.Write('BLOCKED') }";
        let output = tokio::time::timeout(
            Duration::from_secs(8),
            run(&powershell(), &args(script), &dir, b"", "tree", 4096),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(output.stdout, b"BLOCKED");
        cleanup(&dir).await;
    }
}
