//! Starting the canvas and serving until it is interrupted.
//!
//! It owns startup, the terminal echo and the shutdown: a first interrupt stops accepting and
//! waits a short grace period for open connections, then closes the rest. It never decides what
//! the canvas does.

use std::fmt;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use df_table::canvas::Canvas;
use df_table::event::TableEvent;
use df_table::kind::KindRegistry;
use df_table::ports::EventSink;
use tokio::sync::Notify;

use crate::event_log::EventLog;
use crate::kind_files::load_kind_files_async;
use crate::options::ServeOptions;
use crate::server::{self, AppState};
use crate::sink::{HostSink, LineEcho};

/// How long open connections get to finish after an interrupt.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Why serving stopped with a failure.
#[derive(Debug)]
pub enum ServeError {
    Listen {
        port: u16,
        message: String,
    },
    /// The interrupt handler could not be installed, so the host could not be stopped cleanly.
    Interrupt(String),
    Serving(String),
}

impl fmt::Display for ServeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Listen { port, message } => {
                write!(formatter, "cannot listen on 127.0.0.1:{port}: {message}")
            }
            Self::Interrupt(message) => write!(formatter, "cannot watch for Ctrl-C: {message}"),
            Self::Serving(message) => formatter.write_str(message),
        }
    }
}

/// Writes one line to the terminal. A closed terminal drops the line: it must not stop the table.
pub fn write_terminal_line(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

/// Starts the canvas and serves until interrupted. `announce` receives each startup line.
pub async fn run_serve(options: ServeOptions, announce: impl Fn(&str)) -> Result<(), ServeError> {
    let log = Arc::new(EventLog::default());
    let sink = HostSink::new(Arc::clone(&log), options.records_moves);
    let registry = match load_kind_files_async(options.kind_directory.clone()).await {
        Ok(registry) => registry,
        Err(problems) => {
            announce(&format!(
                "the kind files have problems, starting with no kinds:\n{problems}"
            ));
            sink.record(&TableEvent::KindsRejected {
                errors: problems.len(),
            });
            KindRegistry::default()
        }
    };
    let kinds = registry.len();
    let echo = options
        .echoes_lines
        .then(|| LineEcho::starting_after(&log, write_terminal_line));
    let state = Arc::new(AppState::new(
        Canvas::new(registry),
        sink,
        echo,
        options.kind_directory.clone(),
    ));
    let listener = server::bind(options.port)
        .await
        .map_err(|error| ServeError::Listen {
            port: options.port,
            message: error.to_string(),
        })?;
    let address = listener
        .local_addr()
        .map_err(|error| ServeError::Serving(error.to_string()))?;
    announce(&format!(
        "df-table canvas on http://{address}/ with {kinds} kinds from {}",
        options.kind_directory.display()
    ));

    let stop = Arc::new(Notify::new());
    let stopped = Arc::clone(&stop);
    let serving = server::serve_listener(listener, state, async move {
        stopped.notified().await;
    });
    tokio::pin!(serving);
    tokio::select! {
        result = &mut serving => result.map_err(|error| ServeError::Serving(error.to_string())),
        interrupt = tokio::signal::ctrl_c() => {
            interrupt.map_err(|error| ServeError::Interrupt(error.to_string()))?;
            stop.notify_one();
            if let Ok(result) = tokio::time::timeout(SHUTDOWN_GRACE, &mut serving).await {
                result.map_err(|error| ServeError::Serving(error.to_string()))
            } else {
                announce("open connections did not finish in time; closing them");
                Ok(())
            }
        }
    }
}
