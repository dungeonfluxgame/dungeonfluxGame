use std::sync::Arc;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::{
    exporter::ExporterHandle,
    health::{HealthBoard, Signal, SignalState},
    metrics::MetricsRegistry,
};

const OK: &str = "HTTP/1.1 200 OK";
const UNAVAILABLE: &str = "HTTP/1.1 503 Service Unavailable";

fn run<T>(test: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(test)
}

async fn started() -> (ExporterHandle, Arc<MetricsRegistry>, Arc<HealthBoard>) {
    let registry = Arc::new(MetricsRegistry::new());
    let board = Arc::new(HealthBoard::starting(&registry).expect("board"));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let handle =
        ExporterHandle::serve(listener, Arc::clone(&registry), Arc::clone(&board)).expect("serve");
    (handle, registry, board)
}

/// The status line and body of one request.
async fn request(handle: &ExporterHandle, request: &str) -> (String, String) {
    let mut stream = TcpStream::connect(handle.local_address())
        .await
        .expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).await.expect("read");
    let (head, body) = response.split_once("\r\n\r\n").expect("head and body");
    (
        head.lines().next().expect("status").to_string(),
        body.to_string(),
    )
}

async fn get(handle: &ExporterHandle, path: &str) -> (String, String) {
    request(
        handle,
        &format!("GET {path} HTTP/1.1\r\nHost: test\r\n\r\n"),
    )
    .await
}

#[test]
fn the_exporter_serves_the_registry_as_prometheus_text() {
    run(async {
        let (handle, registry, _board) = started().await;
        registry
            .counter("df_things_total", "Things.")
            .expect("counter")
            .add(7);
        let (status, body) = get(&handle, "/metrics").await;
        assert_eq!(status, OK);
        assert!(body.contains("df_things_total 7\n"));
        assert!(body.contains("df_health_signal{signal=\"live\"} 1\n"));
    });
}

#[test]
fn the_exporter_reports_five_separate_health_signals() {
    run(async {
        let (handle, _registry, board) = started().await;
        board.set(Signal::Ready, SignalState::Healthy);
        board.set(
            Signal::Database,
            SignalState::Unhealthy("round_trip_failed"),
        );
        board.set(
            Signal::Providers,
            SignalState::Unhealthy("narration_circuit_open"),
        );
        board.set(Signal::Progress, SignalState::Unhealthy("reduce_stuck"));
        let expectations = [
            ("live", OK, "ok\n"),
            ("ready", OK, "ok\n"),
            ("database", UNAVAILABLE, "unhealthy: round_trip_failed\n"),
            (
                "providers",
                UNAVAILABLE,
                "unhealthy: narration_circuit_open\n",
            ),
            ("progress", UNAVAILABLE, "unhealthy: reduce_stuck\n"),
        ];
        for (name, status, body) in expectations {
            assert_eq!(
                get(&handle, &format!("/healthz/{name}")).await,
                (status.to_string(), body.to_string()),
                "{name}"
            );
        }
        board.set(Signal::Database, SignalState::Healthy);
        assert_eq!(get(&handle, "/healthz/database").await.0, OK);
        assert_eq!(
            get(&handle, "/healthz/progress").await.0,
            UNAVAILABLE,
            "one signal never moves another"
        );
    });
}

#[test]
fn a_starting_process_is_live_but_not_ready_and_says_which_signals_do_not_apply() {
    run(async {
        let (handle, _registry, _board) = started().await;
        assert_eq!(get(&handle, "/healthz/live").await.0, OK);
        assert_eq!(
            get(&handle, "/healthz/ready").await,
            (UNAVAILABLE.to_string(), "unhealthy: starting\n".to_string())
        );
        assert_eq!(
            get(&handle, "/healthz/database").await.1,
            "unhealthy: not_checked\n"
        );
        assert_eq!(
            get(&handle, "/healthz/providers").await,
            (OK.to_string(), "not_applicable\n".to_string())
        );
        let (_, metrics) = get(&handle, "/metrics").await;
        assert!(
            !metrics.contains("signal=\"providers\""),
            "a signal that does not apply reports no value"
        );
    });
}

#[test]
fn unknown_paths_and_methods_are_refused() {
    run(async {
        let (handle, _registry, _board) = started().await;
        assert_eq!(
            get(&handle, "/healthz/other").await.0,
            "HTTP/1.1 404 Not Found"
        );
        assert_eq!(get(&handle, "/").await.0, "HTTP/1.1 404 Not Found");
        assert_eq!(
            request(&handle, "POST /metrics HTTP/1.1\r\n\r\n").await.0,
            "HTTP/1.1 405 Method Not Allowed"
        );
    });
}

#[test]
fn a_request_head_over_the_bound_gets_no_answer() {
    run(async {
        let (handle, _registry, _board) = started().await;
        let mut stream = TcpStream::connect(handle.local_address())
            .await
            .expect("connect");
        let oversized = format!(
            "GET /metrics HTTP/1.1\r\nX-Padding: {}\r\n\r\n",
            "a".repeat(8192)
        );
        // The peer closes while the oversized head is still being written.
        let _written = stream.write_all(oversized.as_bytes()).await;
        let mut response = String::new();
        let _read = stream.read_to_string(&mut response).await;
        assert!(response.is_empty());
    });
}

#[test]
fn dropping_the_handle_stops_the_listener() {
    run(async {
        let (handle, _registry, _board) = started().await;
        let address = handle.local_address();
        drop(handle);
        // The aborted accept loop drops its listener when the runtime next schedules it.
        let mut closed = false;
        for _ in 0..200 {
            if TcpStream::connect(address).await.is_err() {
                closed = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(closed, "the listener must close once its handle is dropped");
    });
}
