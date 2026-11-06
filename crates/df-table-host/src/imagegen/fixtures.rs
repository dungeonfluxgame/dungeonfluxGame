//! A scripted backend for tests: it answers as told, after a delay when told, and remembers what it
//! was asked.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use df_table::testkit::ManualClock;
use futures_util::future::BoxFuture;

use super::backend::{BackendError, ImageBackend};
use super::model::{BackendRequest, GeneratedImage};

/// What the fake does when asked to generate.
#[derive(Clone)]
pub(super) enum Script {
    Picture(GeneratedImage),
    Fail(BackendError),
}

pub(super) struct FakeBackend {
    script: Script,
    delay: Duration,
    check: Result<(), BackendError>,
    /// A clock the check moves on by this many milliseconds, standing in for a slow check.
    check_takes: Option<(Arc<ManualClock>, u64)>,
    asked: Mutex<Vec<(BackendRequest, Duration)>>,
}

impl FakeBackend {
    pub(super) fn new(script: Script) -> Arc<Self> {
        Arc::new(Self {
            script,
            delay: Duration::ZERO,
            check: Ok(()),
            check_takes: None,
            asked: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn slow(script: Script, delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            script,
            delay,
            check: Ok(()),
            check_takes: None,
            asked: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn failing_check(error: BackendError) -> Arc<Self> {
        Arc::new(Self {
            script: Script::Fail(BackendError::Internal),
            delay: Duration::ZERO,
            check: Err(error),
            check_takes: None,
            asked: Mutex::new(Vec::new()),
        })
    }

    /// A backend that draws `script` and whose check moves `clock` on by `check_ms`.
    pub(super) fn checking_for(
        script: Script,
        clock: Arc<ManualClock>,
        check_ms: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            script,
            delay: Duration::ZERO,
            check: Ok(()),
            check_takes: Some((clock, check_ms)),
            asked: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn asked(&self) -> Vec<(BackendRequest, Duration)> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ImageBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn check(&self) -> Result<(), BackendError> {
        if let Some((clock, check_ms)) = &self.check_takes {
            clock.advance_ms(*check_ms);
        }
        self.check.clone()
    }

    fn generate<'a>(
        &'a self,
        request: &'a BackendRequest,
        deadline: Duration,
    ) -> BoxFuture<'a, Result<GeneratedImage, BackendError>> {
        Box::pin(async move {
            self.asked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((request.clone(), deadline));
            tokio::time::sleep(self.delay).await;
            match &self.script {
                Script::Picture(image) => Ok(image.clone()),
                Script::Fail(error) => Err(error.clone()),
            }
        })
    }
}

/// The bytes of a small file that starts like a PNG: enough for the host's checks, which look at
/// the signature and the size the backend reports.
pub(super) fn png_bytes() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 13]);
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&1672_u32.to_be_bytes());
    bytes.extend_from_slice(&941_u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes
}

pub(super) fn picture() -> GeneratedImage {
    let bytes = png_bytes();
    GeneratedImage {
        bytes,
        mime: "image/png".to_owned(),
        width_px: 1672,
        height_px: 941,
        sha256_hex: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        duration: Duration::from_millis(4200),
        input_tokens: Some(10),
        output_tokens: Some(20),
        cleanup: None,
    }
}
