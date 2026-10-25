//! The launcher's only operating-system boundary: checks on the game server process, the
//! single-instance signal, error dialogs and the console for command-line use. Every `unsafe`
//! call lives here.
use std::{io, path::Path};

/// Whether `pid` is alive and still the game server we started (process ids are reused).
#[cfg(windows)]
pub fn process_is(pid: u32, executable: &Path) -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, STILL_ACTIVE},
        System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    };
    // SAFETY: the handle is checked, used for two queries and closed.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut exit_code = 0u32;
        let alive = GetExitCodeProcess(handle, &mut exit_code).is_ok()
            && exit_code == STILL_ACTIVE.0 as u32;
        let mut image_path_utf16 = [0u16; 1024];
        let mut image_path_length_utf16 = image_path_utf16.len() as u32;
        let image_path_was_queried = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(image_path_utf16.as_mut_ptr()),
            &mut image_path_length_utf16,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        let image_path = String::from_utf16_lossy(
            image_path_utf16
                .get(..image_path_length_utf16 as usize)
                .unwrap_or_default(),
        );
        alive && image_path_was_queried && same_file(Path::new(&image_path), executable)
    }
}

#[cfg(not(windows))]
pub fn process_is(pid: u32, executable: &Path) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|image| same_file(&image, executable))
}

fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// Ends the game server at once. Used only after a graceful stop timed out and the player
/// chose Force stop.
#[cfg(windows)]
pub fn terminate(pid: u32, executable: &Path) -> io::Result<()> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
    };
    if !process_is(pid, executable) {
        return Ok(());
    }
    // SAFETY: the handle is checked, used once and closed.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid).map_err(io::Error::other)?;
        let result = TerminateProcess(handle, 1).map_err(io::Error::other);
        let _ = CloseHandle(handle);
        result
    }
}

#[cfg(not(windows))]
pub fn terminate(pid: u32, executable: &Path) -> io::Result<()> {
    if !process_is(pid, executable) {
        return Ok(());
    }
    let status = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("kill failed"))
    }
}

/// The launcher is a Windows (non-console) program, so a command typed in a terminal has
/// nowhere to print. Attach to the parent terminal when standard output is not already a
/// pipe or file (agents capture output through pipes, which keep working unchanged).
#[cfg(windows)]
pub fn attach_parent_console() {
    use windows::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE,
            STD_OUTPUT_HANDLE, SetStdHandle,
        },
    };
    // SAFETY: standard-handle queries and replacement with a console handle we just opened.
    unsafe {
        let present = GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|handle| !handle.is_invalid());
        if present || AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        if let Ok(console) = CreateFileW(
            windows::core::w!("CONOUT$"),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        ) {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, console);
            let _ = SetStdHandle(STD_ERROR_HANDLE, console);
        }
    }
}

#[cfg(not(windows))]
pub fn attach_parent_console() {}

/// Windows children inherit every inheritable handle, not just the ones they are given. The
/// game server and PostgreSQL outlive the launcher, so if they inherited the launcher's own
/// output pipe, a script reading `dungeonflux start | …` would wait for them forever. Children
/// still get the standard handles each spawn passes explicitly.
#[cfg(windows)]
pub fn keep_std_handles_private() {
    use windows::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: only clears the inherit flag on this process's own standard handles.
        unsafe {
            if let Ok(handle) = GetStdHandle(which)
                && !handle.is_invalid()
            {
                let _ = SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
            }
        }
    }
}

#[cfg(not(windows))]
pub fn keep_std_handles_private() {}

/// Saves a JPEG of what `view` shows to `path`, then calls `done` (on the window thread).
/// WebView2 renders the picture itself, so nothing on screen is read or covered.
#[cfg(windows)]
pub fn capture_webview(
    view: &wry::WebView,
    path: std::path::PathBuf,
    done: Box<dyn FnOnce()>,
) -> Result<(), Box<dyn std::error::Error>> {
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_JPEG,
    };
    use windows::Win32::{System::Com::IStream, UI::Shell::SHCreateMemStream};
    use wry::WebViewExtWindows;
    // SAFETY: COM calls on the window thread with interfaces wry keeps alive for the webview.
    unsafe {
        let core = view.controller().CoreWebView2()?;
        let stream: IStream = SHCreateMemStream(None).ok_or("no memory stream")?;
        let reader = stream.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
            if result.is_ok()
                && let Err(error) = save_stream(&reader, &path)
            {
                eprintln!("SCREENSHOT failed error={error}");
            }
            done();
            Ok(())
        }));
        core.CapturePreview(
            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_JPEG,
            &stream,
            &handler,
        )?;
    }
    Ok(())
}

#[cfg(windows)]
fn save_stream(
    stream: &windows::Win32::System::Com::IStream,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::Win32::System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};
    let mut stat = STATSTG::default();
    // SAFETY: reads the finished capture out of the memory stream into an owned buffer.
    let bytes = unsafe {
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        let capture_size_bytes = u32::try_from(stat.cbSize)?;
        if capture_size_bytes == 0 || capture_size_bytes > 32 * 1024 * 1024 {
            return Err("capture size out of range".into());
        }
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut bytes = vec![0u8; capture_size_bytes as usize];
        let mut bytes_read = 0u32;
        stream
            .Read(
                bytes.as_mut_ptr().cast(),
                capture_size_bytes,
                Some(&mut bytes_read),
            )
            .ok()?;
        bytes.truncate(bytes_read as usize);
        bytes
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::files::write_atomic(path, &bytes)?;
    Ok(())
}

/// A blocking error dialog for failures before any window exists.
#[cfg(windows)]
pub fn error_box(message: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let text = windows::core::HSTRING::from(message);
    // SAFETY: both strings outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            None,
            &text,
            windows::core::w!("DungeonFlux"),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
pub fn error_box(_message: &str) {}

/// One launcher per data folder. A second copy asks the first to show its window and exits.
pub enum Instance {
    First(InstanceGuard),
    AlreadyRunning,
}

pub struct InstanceGuard {
    #[cfg(windows)]
    mutex: windows::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    event: windows::Win32::Foundation::HANDLE,
}

// SAFETY: kernel handles may be used from any thread.
unsafe impl Send for InstanceGuard {}

#[cfg(windows)]
pub fn claim_instance(key: &str) -> io::Result<Instance> {
    use windows::Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError},
        System::Threading::{CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, OpenEventW, SetEvent},
        UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow},
    };
    let mutex_name = windows::core::HSTRING::from(format!("Local\\DungeonFluxLauncher-{key}"));
    let event_name = windows::core::HSTRING::from(format!("Local\\DungeonFluxLauncherShow-{key}"));
    // SAFETY: named kernel objects; every handle is owned by the guard or closed here.
    unsafe {
        let mutex = CreateMutexW(None, false, &mutex_name).map_err(io::Error::other)?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, &event_name) {
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
            }
            return Ok(Instance::AlreadyRunning);
        }
        let event = CreateEventW(None, false, false, &event_name).map_err(io::Error::other)?;
        Ok(Instance::First(InstanceGuard { mutex, event }))
    }
}

#[cfg(not(windows))]
pub fn claim_instance(_key: &str) -> io::Result<Instance> {
    Ok(Instance::First(InstanceGuard {}))
}

impl InstanceGuard {
    /// Blocks until another copy of the launcher asks this one to show its window.
    #[cfg(windows)]
    pub fn wait_for_show_request(&self) -> bool {
        use windows::Win32::{Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject};
        // SAFETY: the event handle lives as long as the guard.
        unsafe { WaitForSingleObject(self.event, u32::MAX) == WAIT_OBJECT_0 }
    }

    #[cfg(not(windows))]
    pub fn wait_for_show_request(&self) -> bool {
        false
    }
}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        // SAFETY: both handles were created by `claim_instance` and are closed once.
        unsafe {
            let _ = CloseHandle(self.event);
            let _ = CloseHandle(self.mutex);
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_process_is_recognised_only_with_its_own_executable() {
        let current = std::env::current_exe().unwrap();
        assert!(process_is(std::process::id(), &current));
        assert!(!process_is(
            std::process::id(),
            Path::new("C:/Windows/notepad.exe")
        ));
    }
}
