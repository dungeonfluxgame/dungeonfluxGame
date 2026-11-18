//! What the player listener sends phones, and the answers a phone already has.
//!
//! A JSON answer to a `GET` that succeeds carries an entity tag of its bytes, `Cache-Control:
//! private, no-cache` and `Vary` on the seat key header, so the phone's browser keeps it and asks
//! with `If-None-Match` next time; an answer whose bytes have not changed is a bodiless 304 that
//! keeps every other header (the signed media links among them). A phone that polls its seat's
//! view, its pages and the controls on a poor connection then costs a short answer a poll until
//! something changes. Nothing on the phone changes: the browser revalidates by itself.
//!
//! It counts every answer that passed the guard: how many, how many were 304, and the body bytes
//! sent (a streamed file counts its `Content-Length`; a stream of unknown length counts none). The
//! listener's watch writes them every `PLAYER_TRAFFIC_WINDOW` as `PLAYER traffic window_s=
//! requests= not_modified= bytes_out=` when there were any.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::{Body, HttpBody, to_bytes};
use axum::extract::{Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, HeaderValue, IF_NONE_MATCH, VARY,
};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use df_playground::limits::{FRAME_REQUEST_TIMEOUT_MS, WORK_REQUEST_TIMEOUT_MS};
use df_playground::path::{PLAY_LOOK, PLAY_SEAT_ACTION, PLAY_SEAT_TRANSCRIBE, SEAT_KEY_HEADER};
use tokio::sync::Semaphore;

use crate::limits::{MAXIMUM_PLAYER_REQUESTS, MAXIMUM_RESPONSE_BYTES};

/// The answers the player listener sent since they were last taken, and the requests it turned
/// away: too many at once (`busy`) or too long (`timed_out`).
#[derive(Debug)]
pub(crate) struct PlayerTraffic {
    requests: AtomicU64,
    not_modified: AtomicU64,
    bytes_out: AtomicU64,
    busy: AtomicU64,
    timed_out: AtomicU64,
    /// The requests the listener may be answering at once (`MAXIMUM_PLAYER_REQUESTS`).
    slots: Arc<Semaphore>,
}

impl Default for PlayerTraffic {
    fn default() -> Self {
        Self {
            requests: AtomicU64::default(),
            not_modified: AtomicU64::default(),
            bytes_out: AtomicU64::default(),
            busy: AtomicU64::default(),
            timed_out: AtomicU64::default(),
            slots: Arc::new(Semaphore::new(MAXIMUM_PLAYER_REQUESTS)),
        }
    }
}

/// One window's counts, taken from `PlayerTraffic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrafficCounts {
    pub(crate) requests: u64,
    pub(crate) not_modified: u64,
    pub(crate) bytes_out: u64,
    pub(crate) busy: u64,
    pub(crate) timed_out: u64,
}

impl PlayerTraffic {
    fn count(&self, not_modified: bool, bytes: u64) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        if not_modified {
            self.not_modified.fetch_add(1, Ordering::Relaxed);
        }
        self.bytes_out.fetch_add(bytes, Ordering::Relaxed);
    }

    /// The counts so far, starting the next window at zero.
    pub(crate) fn take(&self) -> TrafficCounts {
        TrafficCounts {
            requests: self.requests.swap(0, Ordering::Relaxed),
            not_modified: self.not_modified.swap(0, Ordering::Relaxed),
            bytes_out: self.bytes_out.swap(0, Ordering::Relaxed),
            busy: self.busy.swap(0, Ordering::Relaxed),
            timed_out: self.timed_out.swap(0, Ordering::Relaxed),
        }
    }
}

impl TrafficCounts {
    /// The `PLAYER traffic` line of a window `window_s` seconds long, or none when nothing was
    /// sent in it.
    pub(crate) fn line(self, window_s: u64) -> Option<String> {
        (self.requests > 0 || self.busy > 0 || self.timed_out > 0).then(|| {
            format!(
                "PLAYER traffic window_s={window_s} requests={} not_modified={} bytes_out={} \
                 busy={} timed_out={}",
                self.requests, self.not_modified, self.bytes_out, self.busy, self.timed_out
            )
        })
    }
}

/// How long the player listener gives a request to be answered: a request that waits on work
/// (a recording written down, a look made, a lobby action that starts the game) the phone's own
/// `WORK_REQUEST_TIMEOUT_MS`; any other the phone's `FRAME_REQUEST_TIMEOUT_MS`, which covers the
/// frame's long wait. The phone gives up at the same bounds, so the two ends agree.
fn deadline_of(method: &Method, path: &str) -> Duration {
    let work = method == Method::POST
        && (path.starts_with(PLAY_LOOK)
            || path == PLAY_SEAT_TRANSCRIBE
            || path == PLAY_SEAT_ACTION);
    Duration::from_millis(if work {
        WORK_REQUEST_TIMEOUT_MS
    } else {
        FRAME_REQUEST_TIMEOUT_MS
    })
}

/// The player listener's bounds: at most `MAXIMUM_PLAYER_REQUESTS` requests are answered at once
/// (one past them is a 503 at once), and a request not answered within its deadline
/// (`deadline_of`) is a 504, its work dropped; a seat action's work runs on a task of its own and
/// finishes, keeping its outcome for the phone's retry (`seat::routes::once`). Both are counted in
/// the `PLAYER traffic` line.
pub(crate) async fn bounded_requests(
    State(traffic): State<Arc<PlayerTraffic>>,
    request: Request,
    next: Next,
) -> Response {
    let Ok(_slot) = Arc::clone(&traffic.slots).try_acquire_owned() else {
        traffic.busy.fetch_add(1, Ordering::Relaxed);
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let deadline = deadline_of(request.method(), request.uri().path());
    match tokio::time::timeout(deadline, next.run(request)).await {
        Ok(response) => response,
        Err(_) => {
            traffic.timed_out.fetch_add(1, Ordering::Relaxed);
            StatusCode::GATEWAY_TIMEOUT.into_response()
        }
    }
}

/// The entity tag of an answer's bytes.
fn tag_of(bytes: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("\"{:x}-{:016x}\"", bytes.len(), hasher.finish())
}

/// The body bytes an answer carries: its `Content-Length`, else its body's exact size, else none
/// counted (a stream of unknown length).
fn declared_length(response: &Response) -> u64 {
    response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .or_else(|| response.body().size_hint().exact())
        .unwrap_or(0)
}

/// The player listener's middleware: tags a successful JSON `GET` answer, answers 304 when the
/// phone already has those bytes, and counts what is sent.
pub(crate) async fn conditional_answers(
    State(traffic): State<Arc<PlayerTraffic>>,
    request: Request,
    next: Next,
) -> Response {
    let is_get = request.method() == Method::GET;
    let held = request
        .headers()
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let response = next.run(request).await;
    let json = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    if !is_get || !json || response.status() != StatusCode::OK {
        let not_modified = response.status() == StatusCode::NOT_MODIFIED;
        traffic.count(not_modified, declared_length(&response));
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAXIMUM_RESPONSE_BYTES).await else {
        traffic.count(false, 0);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let tag = tag_of(&bytes);
    if let Ok(value) = HeaderValue::from_str(&tag) {
        parts.headers.insert(ETAG, value);
    }
    parts
        .headers
        .insert(CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    parts
        .headers
        .append(VARY, HeaderValue::from_static(SEAT_KEY_HEADER));
    if held.as_deref() == Some(tag.as_str()) {
        traffic.count(true, 0);
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(CONTENT_LENGTH);
        parts.headers.remove(CONTENT_TYPE);
        return Response::from_parts(parts, Body::empty());
    }
    traffic.count(false, u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    Response::from_parts(parts, Body::from(bytes))
}

#[cfg(test)]
mod traffic_tests;
