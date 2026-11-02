//! A terminal that stops accepting output must not stop the host: the echo only queues lines for
//! its own writer thread.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use df_table::canvas::Canvas;
use df_table::testkit::{CHARACTER_KIND, MARKER_KIND, registry_from};

use crate::event_log::EventLog;
use crate::playground::PlaygroundFiles;
use crate::request::RequestError;
use crate::sink::{HostSink, LineEcho};
use crate::state::AppState;
use crate::wire::EffectRequest;

const PATIENCE: Duration = Duration::from_secs(10);

/// A writer that blocks on its first line until the test lets go, like a console in selection
/// mode, and records what it eventually writes.
struct Terminal {
    release: mpsc::Sender<()>,
    written: Arc<Mutex<Vec<String>>>,
    seen: mpsc::Receiver<String>,
}

fn blocked_terminal() -> (impl FnMut(&str) + Send + 'static, Terminal) {
    let (release, wait) = mpsc::channel::<()>();
    let (tell, seen) = mpsc::channel::<String>();
    let written = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&written);
    let mut blocked_once = false;
    let writer = move |line: &str| {
        if !blocked_once {
            blocked_once = true;
            let _ = wait.recv_timeout(PATIENCE);
        }
        sink.lock().expect("not poisoned").push(line.to_owned());
        let _ = tell.send(line.to_owned());
    };
    (
        writer,
        Terminal {
            release,
            written,
            seen,
        },
    )
}

/// Runs `work` on another thread and fails the test if it does not return in time.
fn returns_promptly<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(work());
    });
    finished
        .recv_timeout(PATIENCE)
        .expect("it returned while the terminal was blocked")
}

#[test]
fn queuing_lines_does_not_wait_for_a_terminal_that_is_blocked() {
    let log = Arc::new(EventLog::default());
    let (writer, terminal) = blocked_terminal();
    let echo = Arc::new(LineEcho::starting_after(&log, writer));
    for number in 0..200 {
        log.push(format!("line {number}"));
    }
    let queued = Arc::clone(&echo);
    let queued_log = Arc::clone(&log);
    returns_promptly(move || queued.echo_new(&queued_log));
    assert!(terminal.written.lock().expect("not poisoned").is_empty());
    terminal.release.send(()).expect("lets go");
    for number in 0..200 {
        let line = terminal.seen.recv_timeout(PATIENCE).expect("a line");
        assert_eq!(line, format!("line {number}"), "in order, none lost");
    }
    assert_eq!(echo.dropped_lines(), 0);
}

#[test]
fn past_the_byte_bound_lines_are_dropped_counted_and_reported_when_the_terminal_moves() {
    let log = Arc::new(EventLog::default());
    let (writer, terminal) = blocked_terminal();
    let echo = Arc::new(LineEcho::bounded(&log, 64, writer));
    for number in 0..20 {
        log.push(format!("line {number:02} {}", "x".repeat(20)));
    }
    returns_promptly({
        let echo = Arc::clone(&echo);
        let log = Arc::clone(&log);
        move || echo.echo_new(&log)
    });
    terminal.release.send(()).expect("lets go");
    let mut heard = Vec::new();
    while let Ok(line) = terminal.seen.recv_timeout(Duration::from_millis(500)) {
        heard.push(line);
    }
    // Every line is either written or counted as lost, and the writer says how many were lost.
    let lost: usize = heard
        .iter()
        .filter_map(|line| {
            line.strip_suffix(" audit lines were not echoed (the terminal fell behind); see events")
        })
        .map(|count| count.parse::<usize>().expect("a count"))
        .sum();
    let written = heard
        .iter()
        .filter(|line| line.starts_with("line "))
        .count();
    assert!(
        lost > 0,
        "lines past the bound were dropped and reported: {heard:?}"
    );
    assert_eq!(
        written + lost + usize::try_from(echo.dropped_lines()).expect("fits"),
        20
    );
}

fn state_with_a_blocked_terminal() -> (Arc<AppState>, Terminal) {
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let log = Arc::new(EventLog::default());
    let (writer, terminal) = blocked_terminal();
    let echo = LineEcho::starting_after(&log, writer);
    let state = Arc::new(AppState::new(
        Canvas::new(registry_from(&[CHARACTER_KIND, MARKER_KIND])),
        HostSink::new(Arc::clone(&log), true),
        Some(echo),
        directory.path().to_path_buf(),
        PlaygroundFiles::new(
            directory.path().join("assets"),
            directory.path().join("pkg"),
        ),
    ));
    (state, terminal)
}

fn apply(state: &AppState, request: EffectRequest) -> Result<(), RequestError> {
    let mut canvas = state.canvas();
    let effect = request.into_effect(&canvas)?;
    canvas
        .apply(effect, state.sink())
        .map_err(RequestError::Canvas)
}

#[test]
fn a_tick_with_an_arrival_returns_while_the_terminal_is_blocked() {
    let (state, terminal) = state_with_a_blocked_terminal();
    let spawn = EffectRequest::Spawn {
        id: "m1".to_owned(),
        kind: "marker".to_owned(),
        x: Some(10.0),
        y: Some(10.0),
        fields: serde_json::Map::new(),
        values: crate::wire::ValueForm::Json,
    };
    apply(&state, spawn).expect("spawns");
    apply(
        &state,
        EffectRequest::Travel {
            id: "m1".to_owned(),
            x: 12.0,
            y: 10.0,
            speed: 100.0,
        },
    )
    .expect("travels");
    let ticking = Arc::clone(&state);
    returns_promptly(move || {
        // The arrival is echoed; with a blocked terminal this used to block the thread.
        ticking.tick(1.0);
        ticking.echo_new_lines();
        ticking.echo_new_lines();
    });
    terminal.release.send(()).expect("lets go");
    let first = terminal.seen.recv_timeout(PATIENCE).expect("a line");
    assert!(first.starts_with("CANVAS"), "{first}");
}

#[tokio::test]
async fn effects_keep_being_applied_over_http_while_the_terminal_is_blocked() {
    let (state, terminal) = state_with_a_blocked_terminal();
    let listener = crate::server::bind(0).await.expect("binds");
    let port = listener.local_addr().expect("has an address").port();
    let served = Arc::clone(&state);
    let server = tokio::spawn(async move {
        let _ = crate::server::serve_listener(listener, served, std::future::pending()).await;
    });
    for index in 0..5 {
        let body = format!(r#"{{"op":"spawn","kind":"marker","id":"m{index}"}}"#);
        let response = tokio::time::timeout(
            PATIENCE,
            crate::client::post_json(port, "/dev/effect", &body),
        )
        .await
        .expect("the host answered while the terminal was blocked")
        .expect("answers");
        assert_eq!(response.status, 200);
    }
    terminal.release.send(()).expect("lets go");
    server.abort();
}
