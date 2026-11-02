//! What every route shares: the canvas under its lock, the event sink and log, the terminal echo
//! and the places the host reads files from.
//!
//! It owns the lock discipline in one place. A route takes the canvas lock briefly and never across
//! an `.await`, and prints new audit lines only after the lock is released. It decides nothing
//! about requests: `server` and `playground` own their routes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use df_table::canvas::Canvas;
use df_table::element::ElementId;
use df_table::event::{EffectKind, RejectionCause, TableEvent};
use df_table::ports::EventSink;

use crate::event_log::EventLog;
use crate::playground::{PlaygroundFiles, PlaygroundState};
use crate::sink::{HostSink, LineEcho};

/// Everything the handlers share.
pub struct AppState {
    canvas: Mutex<Canvas>,
    sink: HostSink,
    echo: Option<LineEcho>,
    kind_directory: PathBuf,
    playground: PlaygroundState,
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
            playground: PlaygroundState::new(playground_files),
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
