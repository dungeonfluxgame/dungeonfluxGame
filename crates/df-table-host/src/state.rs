//! What every route shares: the canvas under its lock, the event sink and log, the terminal echo
//! and the places the host reads files from.
//!
//! It owns the lock discipline in one place. A route takes the canvas lock briefly and never across
//! an `.await`, and queues new audit lines for the terminal only after the lock is released (the
//! echo never waits for the terminal). It also owns what is shared between readers: the frame,
//! built once for each change (the canvas half, off the runtime threads) and handed to every page
//! as the same bytes, and the signal that wakes the host's clock when something starts to move. It
//! decides nothing about requests: `server` and `playground` own their routes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use tokio::sync::{Notify, watch};

use df_table::canvas::Canvas;
use df_table::element::ElementId;
use df_table::event::{EffectKind, RejectionCause, TableEvent};
use df_table::ports::EventSink;
use df_types::saturating_millis;

use crate::event_log::EventLog;
use crate::limits::{TICK_INTERVAL, TURN_SETTLE};
use crate::observe::HostEventSink;
use crate::playground::{PlaygroundFiles, PlaygroundState};
use crate::server::request::{ClockStatus, HostStatus, LogStatus};
use crate::sink::LineEcho;
use crate::view::{FrameCanvas, ViewError};

/// A frame as the pieces of its JSON: the canvas half is built once for a change count and shared
/// by every reader of it; the head (the count) and the tail (what the playground shows, with each
/// move's age as of now) are made for each reader, because a move's age grows while the canvas
/// does not change.
#[derive(Debug, Clone)]
pub(crate) struct BuiltFrame {
    pub(crate) canvas: Arc<[u8]>,
    count: u64,
    head: Vec<u8>,
    tail: Vec<u8>,
}

impl BuiltFrame {
    /// The change count the frame was built at (its `frame` value).
    pub(crate) fn count(&self) -> u64 {
        self.count
    }

    /// The JSON of the whole frame as the pieces in order, without copying the canvas.
    pub(crate) fn chunks(&self) -> [Bytes; 3] {
        [
            Bytes::copy_from_slice(&self.head),
            Bytes::from_owner(Arc::clone(&self.canvas)),
            Bytes::copy_from_slice(&self.tail),
        ]
    }

    /// The JSON of the whole frame in one piece, for a test.
    #[cfg(test)]
    pub(crate) fn to_vec(&self) -> Vec<u8> {
        [self.head.as_slice(), &self.canvas, self.tail.as_slice()].concat()
    }
}

/// The canvas half of the frame for one change count.
#[derive(Debug, Clone)]
struct CachedCanvas {
    count: u64,
    json: Arc<[u8]>,
}

/// Why a frame could not be built.
#[derive(Debug)]
pub(crate) enum FrameError {
    View(ViewError),
    Json(serde_json::Error),
    /// The thread that builds the canvas half did not finish.
    Task(tokio::task::JoinError),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::View(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "the frame could not be written: {error}"),
            Self::Task(error) => write!(formatter, "the frame could not be built: {error}"),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::View(_) => None,
            Self::Json(error) => Some(error),
            Self::Task(error) => Some(error),
        }
    }
}

/// A name for this run of the host: the time it started and the process, so two runs never share
/// one and a page can tell a restarted host from the same one and an older run from a newer. A
/// clock before 1970 reads as zero,
/// and the process number still tells the runs apart.
pub(crate) fn new_boot_id() -> String {
    let nanoseconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    static STARTED_IN_THIS_PROCESS: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let ordinal = STARTED_IN_THIS_PROCESS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Fixed width, so ids sort in the order the runs started and a page can tell an older run.
    format!(
        "{nanoseconds:032x}-{:08x}-{ordinal:08x}",
        std::process::id()
    )
}

/// Everything the handlers share.
pub struct AppState {
    canvas: Mutex<Canvas>,
    sink: HostEventSink,
    echo: Option<LineEcho>,
    kind_directory: PathBuf,
    playground: PlaygroundState,
    /// Counts the host's changes. A reader waiting for news waits on this.
    changes: watch::Sender<u64>,
    /// The canvas half of the frame last built, for the change count it was built at.
    frame: Mutex<Option<CachedCanvas>>,
    /// Held while the canvas half is built, so readers that miss the cache at the same moment wait
    /// for one build instead of each making their own.
    building: tokio::sync::Mutex<()>,
    /// When each element's turn last changed, for as long as the page is still gliding it.
    turns: Mutex<BTreeMap<ElementId, Instant>>,
    /// Signalled when an element starts to travel, so the clock can sleep while nothing moves.
    motion: Notify,
    /// How many ticks took the canvas lock, so a test can see that an idle clock does not.
    ticks: AtomicU64,
    /// The change count of the newest change that was not the clock moving something, so a frame
    /// line is written for what a person changed and not for each step of a travel.
    last_content_change: AtomicU64,
    /// One more than the newest `last_content_change` a `FRAME served` line was written for (0:
    /// none), so each change is logged once however many pages read it.
    frame_logged: AtomicU64,
}

impl AppState {
    /// `echo` prints new audit lines to the terminal; `None` keeps the log quiet.
    pub fn new(
        canvas: Canvas,
        sink: HostEventSink,
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
            building: tokio::sync::Mutex::new(()),
            turns: Mutex::new(BTreeMap::new()),
            motion: Notify::new(),
            ticks: AtomicU64::new(0),
            last_content_change: AtomicU64::new(0),
            frame_logged: AtomicU64::new(0),
        }
    }

    /// Says that something a reader could see has changed. A reader waiting on `changes` wakes.
    pub(crate) fn announce_change(&self) {
        self.changes.send_modify(|count| {
            *count += 1;
            // Stored under the channel's lock, so it never goes back past a later change.
            self.last_content_change.store(*count, Ordering::Relaxed);
        });
    }

    /// Says that the clock moved something: readers wake as for any change, but no frame line is
    /// written for it (a travel steps every tick).
    fn announce_motion(&self) {
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

    /// Says that the turn of `element` just changed, so the page is gliding it for `TURN_SETTLE`.
    /// Elements whose glide is over are forgotten, so this holds only the ones turned lately.
    pub(crate) fn note_turn(&self, element: ElementId) {
        let now = Instant::now();
        let mut turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
        turns.retain(|_, turned| now.saturating_duration_since(*turned) < TURN_SETTLE);
        turns.insert(element, now);
    }

    /// How much longer the page is gliding the turn of `element`, or `None` when it is not.
    pub(crate) fn turn_settling(&self, element: &ElementId) -> Option<Duration> {
        let turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
        let turned = turns.get(element)?;
        TURN_SETTLE.checked_sub(Instant::now().saturating_duration_since(*turned))
    }

    /// Forgets the turns of `elements` that no longer exist.
    pub(crate) fn forget_turns(&self, elements: &[String]) {
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|element, _| !elements.iter().any(|gone| gone == element.as_str()));
    }

    /// Whether any element is travelling.
    pub(crate) fn is_moving(&self) -> bool {
        self.canvas().is_moving()
    }

    /// How many ticks have taken the canvas lock.
    pub fn tick_count(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }

    /// How many elements are travelling. It copies the elements under the canvas lock, so it is
    /// for the clock's wake-up line and the status, not for every tick.
    pub(crate) fn moving_count(&self) -> usize {
        self.canvas()
            .elements()
            .filter(|element| element.motion.is_some())
            .count()
    }

    /// `true` the first time a frame of change count `count` that holds the newest change that was
    /// not the clock's is served, so the caller writes its `FRAME served` line once per such
    /// change. Frames that differ only by the clock's steps of a travel are not logged.
    pub(crate) fn first_to_serve_frame(&self, count: u64) -> bool {
        let change = self.last_content_change.load(Ordering::Relaxed);
        if count < change {
            return false;
        }
        let marker = change.saturating_add(1);
        self.frame_logged.fetch_max(marker, Ordering::Relaxed) < marker
    }

    fn cached_canvas(&self, count: u64) -> Option<Arc<[u8]>> {
        self.frame
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|cached| cached.count == count)
            .map(|cached| Arc::clone(&cached.json))
    }

    /// The canvas half of the frame as JSON: every element is copied once under the canvas lock,
    /// and written after the lock is released. Blocking work: call it off the runtime threads.
    fn build_canvas_json(&self) -> Result<Arc<[u8]>, FrameError> {
        let canvas = FrameCanvas::from_canvas(&self.canvas()).map_err(FrameError::View)?;
        let json = serde_json::to_vec(&canvas).map_err(FrameError::Json)?;
        Ok(json.into())
    }

    /// The frame for the current change count. The canvas half is built once per count, on the
    /// blocking pool, and every reader of that count shares the same bytes; the playground half is
    /// made for each reader, so a move's `elapsed_ms` is its age now and a late reader continues
    /// from where the move is.
    pub(crate) async fn frame(self: &Arc<Self>) -> Result<BuiltFrame, FrameError> {
        let mut count = self.change_count();
        let canvas = match self.cached_canvas(count) {
            Some(canvas) => canvas,
            None => {
                let _building = self.building.lock().await;
                // Another reader may have built it while this one waited.
                count = self.change_count();
                match self.cached_canvas(count) {
                    Some(canvas) => canvas,
                    None => {
                        let builder = Arc::clone(self);
                        let json = tokio::task::spawn_blocking(move || builder.build_canvas_json())
                            .await
                            .map_err(FrameError::Task)??;
                        *self.frame.lock().unwrap_or_else(PoisonError::into_inner) =
                            Some(CachedCanvas {
                                count,
                                json: Arc::clone(&json),
                            });
                        json
                    }
                }
            }
        };
        let controls = serde_json::to_vec(&self.playground.snapshot()).map_err(FrameError::Json)?;
        let head = format!("{{\"frame\":{count},\"canvas\":").into_bytes();
        let tail = [b",\"controls\":".as_slice(), controls.as_slice(), b"}"].concat();
        Ok(BuiltFrame {
            canvas,
            count,
            head,
            tail,
        })
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
            self.announce_motion();
        }
        if !report.arrived.is_empty() {
            self.echo_new_lines();
        }
    }

    pub(crate) fn canvas(&self) -> MutexGuard<'_, Canvas> {
        self.canvas.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn sink(&self) -> &HostEventSink {
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

    /// Records one audit line that is not a table event, such as a playground change. Inside a
    /// request this is all a route does: the `guard_requests` middleware echoes the request's lines
    /// once it has been answered. It takes no lock but the log's, so it may run under the canvas
    /// lock.
    pub(crate) fn record_line(&self, text: String) {
        self.log().push(text);
    }

    /// Records one audit line and queues it for the terminal at once, for work that goes on after
    /// its request was answered (a background job), which no middleware echoes. Call it with no
    /// lock held.
    pub(crate) fn record_line_and_echo(&self, text: String) {
        self.record_line(text);
        self.echo_new_lines();
    }

    /// Waits up to `patience` for the terminal echo to print what is queued, and stops it taking
    /// more. `true` when it did (or there is no echo).
    pub fn finish_echo(&self, patience: Duration) -> bool {
        self.echo.as_ref().is_none_or(|echo| echo.finish(patience))
    }

    /// The log and its echo, as `host status` shows them.
    pub(crate) fn log_status(&self) -> LogStatus {
        let counts = self.log().counts();
        LogStatus {
            latest: counts.latest,
            kept: counts.kept,
            bytes: counts.bytes,
            forgotten: counts.forgotten,
            echo_pending: self.echo.as_ref().map_or(0, LineEcho::pending_bytes),
            echo_dropped: self.echo.as_ref().map_or(0, LineEcho::dropped_lines),
            echo_writer_gone: self.echo.as_ref().is_some_and(LineEcho::writer_gone),
            undelivered_audit_lines: df_observe::undelivered_audit_lines(),
        }
    }

    /// What `GET /dev/host/status` answers.
    pub(crate) fn host_status(&self) -> HostStatus {
        HostStatus {
            log: self.log_status(),
            sink: self.sink.health(),
            clock: ClockStatus {
                ticks: self.tick_count(),
                moving: self.moving_count(),
                tick_ms: saturating_millis(TICK_INTERVAL),
            },
            engine_sink: None,
        }
    }

    /// Prints new audit lines. Call it with no lock held.
    pub(crate) fn echo_new_lines(&self) {
        if let Some(echo) = &self.echo {
            echo.echo_new(self.log());
        }
    }
}
