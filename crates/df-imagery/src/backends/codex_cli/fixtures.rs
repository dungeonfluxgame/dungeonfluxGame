//! Benign Windows helpers shared by imagery backend tests.
//!
//! Nothing here launches the configured CLI or spends anything: powershell
//! scripts drive tree-lifetime tests through the real session, and
//! `findstr.exe` is the argv-agnostic sleeper for `execute()`-level tests
//! whose argument array is fixed to `app-server`. No helper touches the
//! network.

use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{INVALID_HANDLE_VALUE, STILL_ACTIVE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// How often a test re-checks helper state.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// How long a test waits for a helper to appear or disappear.
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long each scripted helper sleeps unless the job kills it first.
const HELPER_SLEEP_SECONDS: u64 = 60;

pub(crate) fn powershell_path() -> PathBuf {
    PathBuf::from(std::env::var_os("SYSTEMROOT").expect("SYSTEMROOT is set on Windows"))
        .join("System32/WindowsPowerShell/v1.0/powershell.exe")
}

/// `findstr` with a pattern argument blocks reading piped stdin, so it is a
/// deterministic sleeper for argument-fixed `execute()` tests.
pub(crate) fn findstr_path() -> PathBuf {
    PathBuf::from(std::env::var_os("SYSTEMROOT").expect("SYSTEMROOT is set on Windows"))
        .join("System32/findstr.exe")
}

fn base_dir() -> PathBuf {
    // An explicitly configured TMPDIR keeps test artifacts in the repository;
    // the OS default is only the fallback (notably on Windows, where
    // `temp_dir` never consults TMPDIR).
    if let Some(dir) = std::env::var_os("TMPDIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    std::env::temp_dir()
}

pub(crate) fn scratch_dir(name: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let dir = base_dir().join(format!("df-codex-job-{name}-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the test scratch directory is creatable");
    dir
}

pub(crate) fn cleanup_dir(dir: &Path) {
    // Best-effort cleanup; the process tree is already proven dead above.
    let _ = std::fs::remove_dir_all(dir);
}

/// A powershell script that starts a grandchild sleeper, records both pids in
/// `marker`, emits `extra` lines, then sleeps. The grandchild joins the same
/// job automatically, so its death proves tree cleanup.
pub(crate) fn tree_script(marker: &Path, extra: &str) -> String {
    let marker_text = marker.to_string_lossy().replace('\'', "''");
    let helper = powershell_path().to_string_lossy().replace('\'', "''");
    format!(
        // Out-File defaults to UTF-16LE, which `read_to_string` cannot decode;
        // ASCII keeps the pid pair readable as UTF-8 without changing it.
        "$helper = '{helper}'; $child = Start-Process -FilePath $helper -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep {HELPER_SLEEP_SECONDS}' -WindowStyle Hidden -PassThru; \"$PID $($child.Id)\" | Out-File -FilePath '{marker_text}' -NoNewline -Encoding ascii; {extra}Start-Sleep {HELPER_SLEEP_SECONDS}"
    )
}

pub(crate) fn read_pid_pair(text: &str) -> (u32, u32) {
    let mut parts = text.split_whitespace();
    let helper: u32 = parts
        .next()
        .expect("the helper wrote its pid")
        .parse()
        .expect("pids are numeric");
    let grandchild: u32 = parts
        .next()
        .expect("the helper wrote the grandchild pid")
        .parse()
        .expect("pids are numeric");
    assert!(parts.next().is_none(), "the marker holds exactly two pids");
    (helper, grandchild)
}

pub(crate) fn running(pid: u32) -> bool {
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

pub(crate) async fn wait_for_file(path: &Path) -> String {
    let start = std::time::Instant::now();
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => return text,
            _ => {
                assert!(
                    start.elapsed() < WAIT_TIMEOUT,
                    "the helper wrote {}",
                    path.display()
                );
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }
}

pub(crate) async fn wait_until_gone(pid: u32) {
    let start = std::time::Instant::now();
    while running(pid) {
        assert!(
            start.elapsed() < WAIT_TIMEOUT,
            "process {pid} survived its job"
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Pids of our direct children with the given image name.
pub(crate) fn child_process_ids(image: &str) -> Vec<u32> {
    // SAFETY: a process snapshot needs no target handle and is closed below.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    // SAFETY: the snapshot handle is owned and closed when it drops.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..PROCESSENTRY32W::default()
    };
    let mut found = Vec::new();
    // SAFETY: the snapshot is live and entry points to writable memory.
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle().cast(), &mut entry) };
    while more != 0 {
        if entry.th32ParentProcessID == std::process::id()
            && exe_name(&entry).eq_ignore_ascii_case(image)
        {
            found.push(entry.th32ProcessID);
        }
        // SAFETY: same live snapshot and writable entry as above.
        more = unsafe { Process32NextW(snapshot.as_raw_handle().cast(), &mut entry) };
    }
    found
}

fn exe_name(entry: &PROCESSENTRY32W) -> String {
    let len = entry
        .szExeFile
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(entry.szExeFile.len());
    String::from_utf16_lossy(&entry.szExeFile[..len])
}

/// Waits for one helper child beyond `baseline`, proving the real spawn ran.
pub(crate) async fn wait_for_new_child(image: &str, baseline: &[u32]) -> u32 {
    let start = std::time::Instant::now();
    loop {
        let mut fresh: Vec<u32> = child_process_ids(image)
            .into_iter()
            .filter(|pid| !baseline.contains(pid))
            .collect();
        fresh.sort_unstable();
        if let Some(first) = fresh.first() {
            return *first;
        }
        assert!(
            start.elapsed() < WAIT_TIMEOUT,
            "no new {image} child appeared"
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Waits until no helper child beyond `baseline` remains.
pub(crate) async fn wait_until_no_new_child(image: &str, baseline: &[u32]) {
    let start = std::time::Instant::now();
    loop {
        if child_process_ids(image)
            .into_iter()
            .all(|pid| baseline.contains(&pid))
        {
            return;
        }
        assert!(
            start.elapsed() < WAIT_TIMEOUT,
            "a spawned {image} child survived"
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
