//! What every route shares: the canvas under its lock, the event sink and log, the terminal echo
//! and the places the host reads files from.
//!
//! It owns the lock discipline in one place. A route takes the canvas lock briefly and never across
//! an `.await`, and queues new audit lines for the terminal only after the lock is released (the
//! echo never waits for the terminal). It also owns what is shared between readers: the frame, built
//! once for each change and handed to every page as the same bytes, and the signal that wakes the
//! host's clock when something starts to move. It decides nothing about requests: `server` and
//! `playground` own their routes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::{Notify, watch};

use df_table::canvas::Canvas;
use df_table::element::ElementId;
use df_table::event::{EffectKind, RejectionCause, TableEvent};
use df_table::ports::EventSink;

use crate::event_log::EventLog;
use crate::playground::{PlaygroundFiles, PlaygroundState};
use crate::sink::{HostSink, LineEcho};
use crate::view::{FrameCanvas, ViewError};
use crate::wire::Frame;

/// The frame as bytes for the change count it was built at, shared by every reader of it.
#[derive(Debug, Clone)]
pub(crate) struct BuiltFrame {
    pub(crate) count: u64,
    pub(crate) json: Arc<[u8]>,
}

/// Why a frame could not be built.
#[derive(Debug)]
pub(crate) enum FrameError {
    View(ViewError),
    Json(serde_json::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::View(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "the frame could not be written: {error}"),
        }
    }
}

/// A name for this run of the host: the time it started and the process, so two runs never share
/// one and a page can tell a restarted host from the same one. A clock before 1970 reads as zero,
/// and the process number still tells the runs apart.
fn new_boot_id() -> String {
    let nanoseconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    static STARTED_IN_THIS_PROCESS: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let ordinal = STARTED_IN_THIS_PROCESS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{nanoseconds:x}-{:x}-{ordinal:x}", std::process::id())
}

/// Everything the handlers share.
pub struct AppState {
    canvas: Mutex<Canvas>,
    sink: HostSink,
    echo: Option<LineEcho>,
    kind_directory: PathBuf,
    playground: PlaygroundState,
    /// Counts the host's changes. A reader waiting for news waits on this.
    changes: watch::Sender<u64>,
    /// The frame last built, for the change count it was built at.
    frame: Mutex<Option<BuiltFrame>>,
    /// Signalled when an element starts to travel, so the clock can sleep while nothing moves.
    motion: Notify,
    /// How many ticks took the canvas lock, so a test can see that an idle clock does not.
    ticks: AtomicU64,
}

impl AppState {
    /// `echo` prints new audit lines to the terminal; `None` keeps the log quiet.
    pub fn new(
        canvas: Canvas,
        sink: HostSink,
        echo: Option<LineEcho>,
        kind_directory: PathBuf,
        playground_files: PlaygroundFiles,
    ) -> Self {
        Self {
            canvas: Mutex::new(canvas),
            sink,
            echo,
            kind_directory,
            playground: PlaygroundState::new(playground_files, new_boot_id()),
            changes: watch::channel(0).0,
            frame: Mutex::new(None),
            motion: Notify::new(),
            ticks: AtomicU64::new(0),
        }
    }

    /// Says that something a reader could see has changed. A reader waiting on `changes` wakes.
    pub(crate) fn announce_change(&self) {
        self.changes.send_modify(|count| *count += 1);
    }

    /// The count of changes so far.
    pub(crate) fn change_count(&self) -> u64 {
        *self.changes.borrow()
    }

    /// A receiver that sees every later announcement.
    pub(crate) fn watch_changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Wakes the clock. Call it after an effect that may have started a travel; a signal with
    /// nothing waiting is kept, so one made before the clock parks is not lost.
    pub(crate) fn wake_clock(&self) {
        self.motion.notify_one();
    }

    /// Waits until the clock is woken.
    pub(crate) async fn clock_woken(&self) {
        self.motion.notified().await;
    }

    /// Whether any element is travelling.
    pub(crate) fn is_moving(&self) -> bool {
        self.canvas().is_moving()
    }

    /// How many ticks have taken the canvas lock.
    pub fn tick_count(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }

    /// The frame for the current change count, as bytes. It is built once per count: the first
    /// reader builds it (the canvas is copied under its lock, and written as JSON after the lock
    /// is released) and every other reader of that count gets the same bytes.
    pub(crate) fn frame(&self) -> Result<BuiltFrame, FrameError> {
        let mut cached = self.frame.lock().unwrap_or_else(PoisonError::into_inner);
        let count = self.change_count();
        if let Some(built) = cached.as_ref().filter(|built| built.count == count) {
            return Ok(built.clone());
        }
        let canvas = FrameCanvas::from_canvas(&self.canvas()).map_err(FrameError::View)?;
        let frame = Frame {
            frame: count,
            canvas,
            controls: self.playground.snapshot(),
        };
        let json: Arc<[u8]> = serde_json::to_vec(&frame).map_err(FrameError::Json)?.into();
        let built = BuiltFrame { count, json };
        *cached = Some(built.clone());
        Ok(built)
    }

    /// Advances every travelling element by `seconds` of the host's clock. Idle, it takes the lock
    /// and returns. A tick that moved something is announced, and an arrival is echoed.
    pub fn tick(&self, seconds: f32) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        let report = {
            let mut canvas = self.canvas();
            if !canvas.is_moving() {
                return;
            }
            canvas.tick(seconds, &self.sink)
        };
        if report.moved > 0 || !report.arrived.is_empty() {
            self.announce_change();
        }
        if !report.arrived.is_empty() {
            self.echo_new_lines();
        }
    }

    pub(crate) fn canvas(&self) -> MutexGuard<'_, Canvas> {
        self.canvas.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn sink(&self) -> &HostSink {
        &self.sink
    }

    pub(crate) fn log(&self) -> &Arc<EventLog> {
        self.sink.log()
    }

    pub(crate) fn kind_directory(&self) -> &Path {
        &self.kind_directory
    }

    pub(crate) fn playground(&self) -> &PlaygroundState {
        &self.playground
    }

    /// Records a rejection that happened before a request became an effect.
    pub(crate) fn record_rejection(
        &self,
        effect: Option<EffectKind>,
        element: Option<ElementId>,
        cause: RejectionCause,
    ) {
        self.sink.record(&TableEvent::EffectRejected {
            effect,
            element,
            cause,
        });
    }

    /// Records one audit line that is not a table event, such as a playground change.
    pub(crate) fn record_line(&self, text: String) {
        self.log().push(text);
    }

    /// Prints new audit lines. Call it with no lock held.
    pub(crate) fn echo_new_lines(&self) {
        if let Some(echo) = &self.echo {
            echo.echo_new(self.log());
        }
    }
}
