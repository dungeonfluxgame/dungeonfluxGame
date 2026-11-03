//! A seam for the host's tests: hold the next application of a direction where the test chooses,
//! and make the next applications fail, so the order of two callers and a failed apply are tested
//! without a sleep.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::http::StatusCode;
use tokio::sync::Notify;

use crate::playground::SceneApplyError;

#[derive(Default)]
pub(super) struct ApplyHook {
    /// Notified when an application is held.
    pub(super) held: Notify,
    /// Lets the held application go on.
    pub(super) release: Notify,
    /// Holds the next application.
    pub(super) hold_next: AtomicBool,
    /// How many of the next applications fail.
    pub(super) fail_next: AtomicUsize,
}

impl ApplyHook {
    pub(super) async fn before_apply(&self) -> Result<(), SceneApplyError> {
        if self.hold_next.swap(false, Ordering::SeqCst) {
            self.held.notify_one();
            self.release.notified().await;
        }
        let failing = self
            .fail_next
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            });
        match failing {
            Ok(_) => Err(SceneApplyError {
                status: StatusCode::CONFLICT,
                message: "the test made this application fail".to_owned(),
            }),
            Err(_) => Ok(()),
        }
    }
}
