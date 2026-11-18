//! Starting the canvas and serving until it is interrupted.
//!
//! It owns startup, the terminal echo and the shutdown: a first interrupt stops accepting and
//! waits a short grace period for open connections, then closes the rest. It never decides what
//! the canvas does.
//!
//! With `--players N` it also starts the player listener (`player`) on every network address and
//! says where the phone page is; the developer API stays on the loopback address. The player
//! listener and its watch stop when serving ends.
//!
//! It also composes the shared diagnostic path when `--telemetry DIR` names a store root: the
//! canvas sink's records then reach the telemetry store too, and `HOST shared_path` says whether
//! they do. The store is drained and stopped when serving ends, a failure to listen included.

use std::fmt;
use std::io::{ErrorKind, Write};
use std::sync::Arc;

use df_table::canvas::Canvas;
use df_table::event::TableEvent;
use df_table::kind::KindRegistry;
use df_table::ports::EventSink;
use tokio::sync::Notify;

use crate::event_log::{EventLog, install_as_audit_sink};
use crate::kind_files::load_kind_files_async;
use crate::limits::SHUTDOWN_GRACE;
use crate::observe::{
    ComposedSharedPath, EventLevel, HostEventSink, SharedPathCause, WallClock, compose_shared_path,
    shared_path_line,
};
use crate::options::ServeOptions;
use crate::playground::PlaygroundFiles;
use crate::seat::scenario::{Scenario, ShellMissing, find_shell};
use crate::server;
use crate::sink::{LineEcho, TERMINAL_FALLBACK_FILE, TerminalWriter};
use crate::state::AppState;

/// Stops a background task when serving ends by any route, including an early return.
struct StopWhenDropped(tokio::task::JoinHandle<()>);

impl Drop for StopWhenDropped {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Why serving stopped with a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeError {
    Listen {
        port: u16,
        kind: ErrorKind,
    },
    /// The player listener could not listen on every network address at `port`.
    ListenPlayers {
        port: u16,
        kind: ErrorKind,
    },
    /// The interrupt handler could not be installed, so the host could not be stopped cleanly.
    Interrupt(ErrorKind),
    /// The listener or a connection failed while serving.
    Serving(ErrorKind),
    /// A scenario is named but no bash to run its runner was found.
    ScenarioShell(ShellMissing),
}

impl fmt::Display for ServeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Listen { port, kind } => {
                write!(formatter, "cannot listen on 127.0.0.1:{port}: {kind}")
            }
            Self::ListenPlayers { port, kind } => {
                write!(
                    formatter,
                    "cannot listen for players on 0.0.0.0:{port}: {kind}"
                )
            }
            Self::Interrupt(kind) => write!(formatter, "cannot watch for Ctrl-C: {kind}"),
            Self::Serving(kind) => write!(formatter, "serving stopped: {kind}"),
            Self::ScenarioShell(missing) => missing.fmt(formatter),
        }
    }
}

impl std::error::Error for ServeError {}

/// Writes one line to the terminal. A closed terminal drops the line: it must not stop the table.
/// The echo of audit lines does not use this: its `TerminalWriter` records a failed write.
pub fn write_terminal_line(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

/// Starts the canvas and serves until interrupted. `announce` receives each startup line.
pub async fn run_serve(options: ServeOptions, announce: impl Fn(&str)) -> Result<(), ServeError> {
    // A scenario that could not run is refused before anything starts.
    let scenario_shell = match &options.scenario {
        Some(_) => Some(
            find_shell(options.scenario_shell.as_deref())
                .await
                .map_err(ServeError::ScenarioShell)?,
        ),
        None => None,
    };
    let log = Arc::new(EventLog::default());
    if !install_as_audit_sink(&log) {
        announce("another log already takes the libraries' audit lines in this process");
    }
    // Moves are verbose events: left out unless asked for, and then counted as filtered.
    let level = if options.records_moves {
        EventLevel::Verbose
    } else {
        EventLevel::Always
    };
    // Opening the telemetry store is disk work, so it runs off the async workers.
    let root = options.telemetry_root.clone();
    let root_set = root.is_some();
    let (shared_path, shared_path_audit) =
        tokio::task::spawn_blocking(move || compose_shared_path(root.as_deref()))
            .await
            // A blocking task that panicked or was cancelled opened nothing the host can use.
            .unwrap_or_else(|_| {
                let cause = Some(SharedPathCause::Unavailable);
                (None, shared_path_line(root_set, cause))
            });
    if shared_path.is_none() && root_set {
        announce("the telemetry store could not be opened; records stay in this process");
    }
    // Nothing queries the canvas's history (its lines are read), so it keeps none.
    let sink =
        HostEventSink::without_history(Arc::clone(&log), Arc::new(WallClock)).with_level(level);
    let sink = match &shared_path {
        Some(path) => sink.with_outlet(path.outlet()),
        None => sink,
    };
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
    let echo = options.echoes_lines.then(|| {
        let mut terminal = TerminalWriter::new(
            Arc::clone(&log),
            options.playground_directory.join(TERMINAL_FALLBACK_FILE),
        );
        LineEcho::starting_after(&log, move |line| terminal.write_line(line))
    });
    let state = Arc::new(AppState::new(
        Canvas::new(registry),
        sink,
        echo,
        options.kind_directory.clone(),
        PlaygroundFiles::new(
            options.playground_directory.clone(),
            options.package_directory.clone(),
        ),
    ));
    state.record_line(shared_path_audit);
    state.record_line(state.playground().restore_line());
    // Show a fixture from the first request: the host reconciles what is shown with the files at
    // startup and when a fixture is written or removed, never when a page reads.
    if let Err(problem) = crate::playground::reconcile_fixtures(&state) {
        announce(&format!(
            "the playground's fixtures could not be read: {}",
            problem.message()
        ));
    }
    state.echo_new_lines();
    let listening = match listen(options.port).await {
        Ok(bound) => listen_for_players(options.players_port)
            .await
            .map(|players| (bound, players)),
        Err(error) => Err(error),
    };
    let ((listener, address), players) = match listening {
        Ok(listening) => listening,
        Err(error) => {
            // The records written before the failure (the shared path line, rejected kinds) still
            // reach the store: it is drained and stopped as at any other end of serving.
            stop_shared_path(shared_path, &announce).await;
            return Err(error);
        }
    };
    announce(&format!(
        "df-table canvas on http://{address}/ with {kinds} kinds from {}\n\
         playground on http://{address}/playground",
        options.kind_directory.display()
    ));
    let first_ask = state.seats().talk().first_ask();
    state.record_line(format!("SEAT asks_seeded first={first_ask} reason=boot"));
    if let (Some(beats), Some(shell)) = (&options.scenario, scenario_shell) {
        announce(&format!(
            "scenario for the table: {} (shell {})",
            beats.display(),
            shell.display()
        ));
        state.record_line(format!(
            "SCENARIO configured beats={} shell={}",
            beats.display(),
            shell.display()
        ));
        state
            .seats()
            .set_scenario(Scenario::new(beats.clone(), shell, address.port()));
        // The table starts in the lobby: no seat plays a day left from an earlier host.
        crate::seat::scenario::clear_bindings(&state).await;
    }
    let _players = players.map(|players| {
        for link in players.links() {
            announce(&format!("phone page: {link}"));
        }
        announce(&format!(
            "phone page link and QR code: http://{address}{}",
            crate::player::path::PHONE_LINK
        ));
        let tasks = crate::player::start(players, &state);
        (StopWhenDropped(tasks.serving), StopWhenDropped(tasks.watch))
    });
    // The table's lobby opens once the server serves (the table's engine starts with it): its room
    // code, join link and QR code go on the TV.
    let opening = Arc::clone(&state);
    let _lobby = StopWhenDropped(tokio::spawn(async move {
        crate::seat::room::open_lobby(&opening).await;
    }));

    let stop = Arc::new(Notify::new());
    let stopped = Arc::clone(&stop);
    let _clock = StopWhenDropped(server::spawn_clock(Arc::clone(&state)));
    let _files = StopWhenDropped(crate::playground::spawn_files_watch(Arc::clone(&state)));
    let echoing = Arc::clone(&state);
    let serving = server::serve_listener(listener, state, async move {
        stopped.notified().await;
    });
    tokio::pin!(serving);
    let result = tokio::select! {
        result = &mut serving => result.map_err(|error| ServeError::Serving(error.kind())),
        interrupt = tokio::signal::ctrl_c() => match interrupt {
            Err(error) => Err(ServeError::Interrupt(error.kind())),
            Ok(()) => {
                stop.notify_one();
                if let Ok(result) = tokio::time::timeout(SHUTDOWN_GRACE, &mut serving).await {
                    result.map_err(|error| ServeError::Serving(error.kind()))
                } else {
                    announce("open connections did not finish in time; closing them");
                    Ok(())
                }
            }
        },
    };
    // A day the table was playing stops with the host: its runner is the host's child.
    if let Some(scenario) = echoing.seats().scenario() {
        scenario.stop(&echoing).await;
    }
    // The last audit lines before the stop (a final arrival, a reject) reach the terminal.
    echoing.echo_new_lines();
    if !echoing.finish_echo(SHUTDOWN_GRACE) {
        announce("the terminal did not take the last audit lines in time");
    }
    stop_shared_path(shared_path, &announce).await;
    result
}

/// The listener on `port` and the address it is bound to.
async fn listen(port: u16) -> Result<(tokio::net::TcpListener, std::net::SocketAddr), ServeError> {
    let listener = server::bind(port)
        .await
        .map_err(|error| ServeError::Listen {
            port,
            kind: error.kind(),
        })?;
    let address = listener
        .local_addr()
        .map_err(|error| ServeError::Serving(error.kind()))?;
    Ok((listener, address))
}

/// The player listener on `port`, when one is asked for.
async fn listen_for_players(
    port: Option<u16>,
) -> Result<Option<crate::player::PlayerListener>, ServeError> {
    let Some(port) = port else {
        return Ok(None);
    };
    crate::player::listen(port)
        .await
        .map(Some)
        .map_err(|error| ServeError::ListenPlayers {
            port,
            kind: error.kind(),
        })
}

/// Drains the shared path's receipts and stops its store, when one was composed. It blocks, so it
/// runs off the async workers.
async fn stop_shared_path(shared_path: Option<ComposedSharedPath>, announce: &impl Fn(&str)) {
    if let Some(path) = shared_path {
        let stopped = tokio::task::spawn_blocking(move || path.shutdown(SHUTDOWN_GRACE)).await;
        if !matches!(stopped, Ok(Ok(()))) {
            announce("the telemetry store did not confirm every record before it stopped");
        }
    }
}
