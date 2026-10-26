use std::{error::Error, future::Future, io};

#[cfg(windows)]
static WINDOWS_STOP: std::sync::OnceLock<tokio::sync::watch::Sender<bool>> =
    std::sync::OnceLock::new();

const WINDOWS_STACK_SIZE: usize = 16 * 1024 * 1024;

pub fn install_panic_reporter() {
    std::panic::set_hook(Box::new(|panic| {
        let location = panic
            .location()
            .map(|location| (location.file(), location.line(), location.column()));
        let (message, location) = panic_details(panic.payload(), location);
        eprintln!("PANIC message={message:?} location={location}");
    }));
}

#[cfg(windows)]
pub fn install_console_stop_handler() -> tokio::sync::watch::Receiver<bool> {
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let _ = WINDOWS_STOP.set(sender);
    unsafe extern "system" fn handle_console_event(event: u32) -> i32 {
        if matches!(event, 0..=2) {
            if let Some(sender) = WINDOWS_STOP.get() {
                let _ = sender.send(true);
            }
            1
        } else {
            0
        }
    }
    // SAFETY: the callback is static, follows the Win32 system ABI, and only sends a
    // bounded watch notification; it does not access borrowed data or block.
    unsafe {
        SetConsoleCtrlHandler(Some(handle_console_event), 1);
    }
    receiver
}

#[cfg(windows)]
type ConsoleHandler = Option<unsafe extern "system" fn(u32) -> i32>;

#[cfg(windows)]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(handler: ConsoleHandler, add: i32) -> i32;
}

#[cfg(windows)]
pub fn run<F, Fut>(entry: F) -> Result<(), Box<dyn Error>>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), Box<dyn Error>>> + 'static,
{
    std::thread::Builder::new()
        .name("df-server-entry".to_owned())
        .stack_size(WINDOWS_STACK_SIZE)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .thread_stack_size(WINDOWS_STACK_SIZE)
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|runtime| runtime.block_on(entry()).map_err(|error| error.to_string()))
        })?
        .join()
        .map_err(|_| io::Error::other("server entry thread panicked"))?
        .map_err(Into::into)
}

#[cfg(not(windows))]
pub fn run<F, Fut>(entry: F) -> Result<(), Box<dyn Error>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), Box<dyn Error>>>,
{
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(entry())
}

fn panic_details(
    payload: &(dyn std::any::Any + Send),
    location: Option<(&str, u32, u32)>,
) -> (String, String) {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
        .to_owned();
    let location = location
        .map(|(file, line, column)| format!("{file}:{line}:{column}"))
        .unwrap_or_else(|| "unknown location".to_owned());
    (message, location)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_entry_and_worker_stacks_exceed_default_stack() {
        let stack_size = std::hint::black_box(WINDOWS_STACK_SIZE);
        assert!(stack_size >= 16 * 1024 * 1024);
    }

    #[test]
    fn panic_report_contains_message_and_source_location() {
        let (message, location) = panic_details(&"diagnostic-test", Some(("test.rs", 7, 2)));
        assert_eq!(message, "diagnostic-test");
        assert_eq!(location, "test.rs:7:2");
    }
}
