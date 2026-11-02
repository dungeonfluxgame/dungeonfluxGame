//! The developer API and the live viewer, served on the loopback address only.
//!
//! It owns the sockets, the routes and the lock around the canvas. A request locks the canvas
//! briefly and never across an `.await`; the lock stays held from converting a request to
//! applying it, so a reload cannot change the kinds in between. Every request passes the guard
//! first (`guard.rs`), every change goes through `Canvas::apply`, and every rejection, including
//! the ones that happen before a request becomes an effect, is recorded as an event.

use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Query, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use df_table::element::ElementId;
use df_table::event::{EffectKind, RejectionCause, TableEvent};
use df_table::ports::EventSink;
use serde::Deserialize;

use crate::guard;
use crate::kind_files::load_kind_files_async;
use crate::limits::{FRAME_WAIT, MAXIMUM_EVENTS_PER_ANSWER, MAXIMUM_REQUEST_BYTES, TICK_INTERVAL};
use crate::media_library;
use crate::playground;
use crate::render;
use crate::request::RequestError;
use crate::state::AppState;
use crate::view::{CanvasView, ViewError};
use crate::wire::{EffectRequest, EventsReply, Reply, capped, route};

fn answer(status: StatusCode, reply: Reply) -> Response {
    (status, Json(reply)).into_response()
}

fn rejected(status: StatusCode, cause: Option<RejectionCause>, message: &str) -> Response {
    answer(
        status,
        Reply::Rejected {
            cause,
            message: message.to_owned(),
            problems: Vec::new(),
        },
    )
}

/// Runs the guard before every route and records what it refuses.
async fn guard_requests(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if let Err(refusal) = guard::check(request.method(), request.headers()) {
        state.record_rejection(None, None, RejectionCause::ForbiddenRequest);
        state.echo_new_lines();
        return rejected(
            refusal.status(),
            Some(RejectionCause::ForbiddenRequest),
            refusal.message(),
        );
    }
    next.run(request).await
}

/// Tells readers waiting for news that a change was made. Every POST is a change attempt, so
/// the announcement is made after the answer is built, whether the change was accepted or not:
/// a reader that looks again and finds nothing new costs one request.
async fn announce_changes(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    // A page's report of what it shows is a reading, not a change, and must not wake pages.
    let is_change =
        request.method() == Method::POST && request.uri().path() != route::DEV_CLIENT_REPORT;
    let response = next.run(request).await;
    if is_change {
        state.announce_change();
    }
    response
}

/// A copy of the canvas for the viewer or `canvas.json`, or a 500 that says why and is logged: a
/// value that cannot be written must not become a quiet `null`.
fn canvas_view(state: &AppState) -> Result<CanvasView, ViewError> {
    CanvasView::from_canvas(&state.canvas())
}

fn view_failed(state: &AppState, error: &dyn std::fmt::Display) -> Response {
    state.record_line(format!("HOST view_failed reason={error}"));
    state.echo_new_lines();
    rejected(StatusCode::INTERNAL_SERVER_ERROR, None, &error.to_string())
}

async fn index(State(state): State<Arc<AppState>>) -> Response {
    match canvas_view(&state) {
        Ok(view) => Html(render::page(
            &view,
            &state.log().newest(MAXIMUM_EVENTS_PER_ANSWER),
        ))
        .into_response(),
        Err(error) => view_failed(&state, &error),
    }
}

async fn canvas_json(State(state): State<Arc<AppState>>) -> Response {
    match canvas_view(&state) {
        Ok(view) => Json(view).into_response(),
        Err(error) => view_failed(&state, &error),
    }
}

/// The query of `route::PLAYGROUND_FRAME`; the field is named like `wire::FRAME_AFTER_PARAMETER`.
#[derive(Debug, Deserialize)]
struct FrameQuery {
    after: Option<u64>,
}

/// The current frame. With `after=N`, a request made while the host is still at frame N is held
/// until the next change (or `FRAME_WAIT`), so a page learns of a change as it happens instead of
/// by asking again and again. A different count, such as after a host restart, answers at once.
/// The frame is built once for each change count and every page is sent the same bytes.
async fn frame_json(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FrameQuery>,
) -> Response {
    if let Some(after) = query.after {
        let mut changes = state.watch_changes();
        // A timeout is not an error: the page asks again with the same count.
        let _ = tokio::time::timeout(FRAME_WAIT, changes.wait_for(|count| *count != after)).await;
    }
    match state.frame() {
        Ok(built) => {
            let mut response = Response::new(axum::body::Body::from(Bytes::from_owner(built.json)));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            response
        }
        Err(error) => view_failed(&state, &error),
    }
}

/// The query of `route::EVENTS`; the field is named like `wire::EVENTS_AFTER_PARAMETER`.
#[derive(Debug, Deserialize)]
struct EventsQuery {
    after: Option<u64>,
}

/// Events newer than `after`, or the newest page when `after` is absent or beyond the newest
/// sequence (which means the host restarted since the reader last looked).
async fn events_json(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EventsQuery>,
) -> Json<EventsReply> {
    let log = state.log();
    let latest = log.latest_sequence();
    let events = match query.after {
        Some(after) if after <= latest => log.after_sequence(after, MAXIMUM_EVENTS_PER_ANSWER),
        Some(_) | None => log.newest(MAXIMUM_EVENTS_PER_ANSWER),
    };
    Json(EventsReply { events, latest })
}

async fn apply_effect(
    State(state): State<Arc<AppState>>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let response = apply_effect_body(&state, body);
    state.echo_new_lines();
    response
}

fn apply_effect_body(state: &AppState, body: Result<Bytes, BytesRejection>) -> Response {
    let bytes = match body {
        Ok(bytes) => bytes,
        Err(rejection) => {
            state.record_rejection(None, None, RejectionCause::MalformedRequest);
            return rejected(
                rejection.status(),
                Some(RejectionCause::MalformedRequest),
                "the request body could not be read",
            );
        }
    };
    let request: EffectRequest = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(error) => {
            state.record_rejection(None, None, RejectionCause::MalformedRequest);
            return rejected(
                StatusCode::BAD_REQUEST,
                Some(RejectionCause::MalformedRequest),
                &format!("the request is not a valid effect: {error}"),
            );
        }
    };
    let effect_kind = request.effect_kind();
    let element = request.element();
    let mut canvas = state.canvas();
    let effect = match request.into_effect(&canvas) {
        Ok(effect) => effect,
        Err(error) => return reject_conversion(state, effect_kind, element, &error),
    };
    let applied = canvas.apply(effect, state.sink());
    if canvas.is_moving() {
        state.wake_clock();
    }
    match applied {
        Ok(()) => answer(
            StatusCode::OK,
            Reply::Done {
                revision: canvas.revision(),
                notices: Vec::new(),
            },
        ),
        Err(error) => rejected(
            StatusCode::BAD_REQUEST,
            Some(error.cause()),
            &error.to_string(),
        ),
    }
}

fn reject_conversion(
    state: &AppState,
    effect: EffectKind,
    element: Option<ElementId>,
    error: &RequestError,
) -> Response {
    state.record_rejection(Some(effect), element, error.cause());
    rejected(
        StatusCode::BAD_REQUEST,
        Some(error.cause()),
        &error.to_string(),
    )
}

async fn reload(State(state): State<Arc<AppState>>) -> Response {
    let loaded = load_kind_files_async(state.kind_directory().to_path_buf()).await;
    let response = match loaded {
        Ok(registry) => {
            let mut canvas = state.canvas();
            let report = canvas.replace_registry(registry, state.sink());
            let notices = report
                .problems
                .iter()
                .map(|problem| format!("element '{}': {}", problem.element, problem.problem))
                .collect();
            answer(
                StatusCode::OK,
                Reply::Done {
                    revision: canvas.revision(),
                    notices: capped(notices),
                },
            )
        }
        Err(problems) => {
            state.sink().record(&TableEvent::KindsRejected {
                errors: problems.len(),
            });
            answer(
                StatusCode::BAD_REQUEST,
                Reply::Rejected {
                    cause: None,
                    message: "the kind files have problems; the loaded kinds are unchanged"
                        .to_owned(),
                    problems: capped(problems.messages()),
                },
            )
        }
    };
    state.echo_new_lines();
    response
}

/// The routes of the developer API, the viewer and the playground.
pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/", get(index))
        .route(route::CANVAS, get(canvas_json))
        .route(route::EVENTS, get(events_json))
        .route(route::PLAYGROUND_FRAME, get(frame_json))
        .route(route::EFFECT, post(apply_effect))
        .route(route::RELOAD, post(reload))
        .with_state(state.clone());
    api.merge(playground::router(state.clone()))
        .merge(media_library::router(state.clone()))
        .layer(DefaultBodyLimit::max(MAXIMUM_REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            announce_changes,
        ))
        .layer(middleware::from_fn_with_state(state, guard_requests))
}

/// Starts the host's clock: every `TICK_INTERVAL` while something travels it advances travelling
/// elements by the time that really passed, which the canvas cuts to a sane length if the host
/// stalled. With nothing travelling it sleeps until an effect wakes it, so an idle host has no
/// timer running. Abort the handle to stop it.
pub fn spawn_clock(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last = Instant::now();
        loop {
            if !state.is_moving() {
                state.clock_woken().await;
                // Time spent asleep is not travel: start counting again from the wake-up.
                interval.reset();
                last = Instant::now();
                continue;
            }
            interval.tick().await;
            let now = Instant::now();
            let seconds = now.duration_since(last).as_secs_f32();
            last = now;
            state.tick(seconds);
        }
    })
}

/// Binds the loopback address only (port zero picks a free port), so the caller can report the
/// real port.
pub async fn bind(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(("127.0.0.1", port)).await
}

/// Serves on a bound listener until `shutdown` completes.
pub async fn serve_listener(
    listener: tokio::net::TcpListener,
    state: Arc<AppState>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}
