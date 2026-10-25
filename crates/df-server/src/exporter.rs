//! The internal HTTP listener of a process: `GET /metrics` (Prometheus text) and
//! `GET /healthz/<signal>` for each of the five signals. It speaks the smallest HTTP/1.1 that
//! Prometheus and `healthcheck` need: one request per connection, no body, no keep-alive.
//! Connections are bounded, every read has a deadline, and the accept loop and its connection
//! tasks stop together when the handle drops.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};

use crate::{
    health::{HealthBoard, Signal, SignalState},
    metrics::MetricsRegistry,
};

/// Concurrent connections; a further connection is closed at once.
const MAXIMUM_CONNECTIONS: usize = 16;
/// Largest request head read; anything longer is not a scrape.
const MAXIMUM_REQUEST_HEAD_BYTES: usize = 4096;
const REQUEST_DEADLINE: Duration = Duration::from_secs(2);
/// Pause after a failed accept (descriptor exhaustion repeats at once), so the loop never spins.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: String,
}

/// Owns the accept loop; dropping the handle stops it and every connection it started.
pub struct ExporterHandle {
    local_address: SocketAddr,
    task: JoinHandle<()>,
}

impl ExporterHandle {
    /// Starts serving on an already bound listener.
    pub fn serve(
        listener: TcpListener,
        registry: Arc<MetricsRegistry>,
        board: Arc<HealthBoard>,
    ) -> std::io::Result<Self> {
        let local_address = listener.local_addr()?;
        let task = tokio::spawn(accept_loop(listener, registry, board));
        Ok(Self {
            local_address,
            task,
        })
    }

    pub fn local_address(&self) -> SocketAddr {
        self.local_address
    }
}

impl Drop for ExporterHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept_loop(
    listener: TcpListener,
    registry: Arc<MetricsRegistry>,
    board: Arc<HealthBoard>,
) {
    let mut connections = JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        let Ok((stream, _peer)) = listener.accept().await else {
            sleep(ACCEPT_BACKOFF).await;
            continue;
        };
        if connections.len() >= MAXIMUM_CONNECTIONS {
            drop(stream);
            continue;
        }
        connections.spawn(answer(stream, Arc::clone(&registry), Arc::clone(&board)));
    }
}

async fn answer(mut stream: TcpStream, registry: Arc<MetricsRegistry>, board: Arc<HealthBoard>) {
    let Ok(Some(head)) = timeout(REQUEST_DEADLINE, read_request_head(&mut stream)).await else {
        return;
    };
    let response = route(&head, &registry, &board);
    let message = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        response.status,
        response.content_type,
        response.body.len(),
        response.body
    );
    // The scraper may already be gone; there is nobody to tell.
    let _ignored = timeout(REQUEST_DEADLINE, stream.write_all(message.as_bytes())).await;
    let _ignored = stream.shutdown().await;
}

/// The request head, or `None` when it is too long or the peer closed first.
async fn read_request_head(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut head = Vec::with_capacity(256);
    let mut chunk = [0u8; 256];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 || head.len() + read > MAXIMUM_REQUEST_HEAD_BYTES {
            return None;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    Some(head)
}

fn route(head: &[u8], registry: &MetricsRegistry, board: &HealthBoard) -> Response {
    let line = head.split(|byte| *byte == b'\r').next().unwrap_or_default();
    let mut words = line.split(|byte| *byte == b' ');
    let (method, target) = (words.next(), words.next());
    if method != Some(b"GET") {
        return text("405 Method Not Allowed", "method not allowed\n");
    }
    match target.and_then(|target| std::str::from_utf8(target).ok()) {
        Some("/metrics") => Response {
            status: "200 OK",
            content_type: METRICS_CONTENT_TYPE,
            body: registry.render(),
        },
        Some(path) => match path.strip_prefix("/healthz/").and_then(Signal::from_name) {
            Some(signal) => health_response(board.state(signal)),
            None => text("404 Not Found", "not found\n"),
        },
        None => text("400 Bad Request", "bad request\n"),
    }
}

fn health_response(state: SignalState) -> Response {
    match state {
        SignalState::Healthy => text("200 OK", "ok\n"),
        SignalState::NotApplicable => text("200 OK", "not_applicable\n"),
        SignalState::Unhealthy(reason) => {
            text("503 Service Unavailable", &format!("unhealthy: {reason}\n"))
        }
    }
}

fn text(status: &'static str, body: &str) -> Response {
    Response {
        status,
        content_type: "text/plain; charset=utf-8",
        body: body.to_string(),
    }
}
