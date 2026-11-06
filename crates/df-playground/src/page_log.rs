//! The page's one `PageLog` and its clock: every part of the page notes what it showed, played
//! and met here, and the report carries it to the host.
//!
//! It is reached with `try_borrow_mut`, so a note made while the log is already in use (a panic in
//! the middle of a note) is dropped instead of panicking again.

use std::cell::RefCell;

use crate::timeline::{ErrorSource, PageLog};

thread_local! {
    static LOG: RefCell<PageLog> = RefCell::new(PageLog::default());
}

/// Milliseconds on the page's clock: since the page began loading. A page without a window or a
/// clock says 0, and its times then all read 0.
pub fn now_ms() -> u64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0, |performance| performance.now().max(0.0) as u64)
}

/// Runs `change` on the page's log, or `None` when the log is in use.
pub fn with_log<R>(change: impl FnOnce(&mut PageLog) -> R) -> Option<R> {
    LOG.with(|log| log.try_borrow_mut().ok().map(|mut log| change(&mut log)))
}

/// Notes an error for the report. Returns whether it should be reported at once.
pub fn note_error(source: ErrorSource, message: &str) -> bool {
    let now = now_ms();
    with_log(|log| log.note_error(source, message, now)).unwrap_or(false)
}
