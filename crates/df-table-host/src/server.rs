//! The developer API and the live viewer, served on the loopback address only.
//!
//! It owns the sockets, the routes and the lock around the canvas. A request locks the canvas
//! briefly and never across an `.await`; the lock stays held from converting a request to
//! applying it, so a reload cannot change the kinds in between. Every request passes the guard
//! first (`guard.rs`), every change goes through `Canvas::apply`, and every rejection, including
//! the ones that happen before a request becomes an effect, is recorded as an event.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use df_table::canvas::Canvas;
use df_table::element::ElementId;
use df_table::event::{EffectKind, RejectionCause, TableEvent};
use df_table::ports::EventSink;
use serde::Deserialize;

use crate::event_log::EventLog;
use crate::guard;
use crate::kind_files::load_kind_files_async;
use crate::render;
use crate::request::RequestError;
use crate::sink::{HostSink, LineEcho};
use crate::view::CanvasView;
use crate::wire::{EffectRequest, EventsReply, Reply, capped, route};

/// Most event lines the viewer shows and one events answer carries.
pub const MAXIMUM_EVENTS_PER_ANSWER: usize = 60;
/// Largest request body accepted, in bytes.
pub const MAXIMUM_REQUEST_BYTES: usize = 64 * 1024;

/// Everything the handlers share.
pub struct AppState {
    canvas: Mutex<Canvas>,
    sink: HostSink,
    echo: Option<LineEcho>,
    kind_directory: PathBuf,
}

impl AppState {
    /// `echo` prints new audit lines to the terminal; `None` keeps the log quiet.
    pub fn new(
        canvas: Canvas,
        sink: HostSink,
        echo: Option<LineEcho>,
        kind_directory: PathBuf,
    ) -> Self {
        Self {
            canvas: Mutex::new(canvas),
            sink,
            echo,
            kind_directory,
        }
    }

    fn canvas(&self) -> MutexGuard<'_, Canvas> {
        self.canvas.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn log(&self) -> &Arc<EventLog> {
        self.sink.log()
    }

    /// Records a rejection that happened before a request became an effect.
    fn record_rejection(
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

    /// Prints new audit lines. Call it with no lock held.
    fn echo_new_lines(&self) {
        if let Some(echo) = &self.echo {
            echo.echo_new(self.log());
        }
    }
}

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

async fn index(State(state): State<Arc<AppState>>) -> Html<String> {
    let view = CanvasView::from_canvas(&state.canvas());
    Html(render::page(
        &view,
        &state.log().newest(MAXIMUM_EVENTS_PER_ANSWER),
    ))
}

async fn canvas_json(State(state): State<Arc<AppState>>) -> Json<CanvasView> {
    Json(CanvasView::from_canvas(&state.canvas()))
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
    match canvas.apply(effect, &state.sink) {
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
    let loaded = load_kind_files_async(state.kind_directory.clone()).await;
    let response = match loaded {
        Ok(registry) => {
            let mut canvas = state.canvas();
            let report = canvas.replace_registry(registry, &state.sink);
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
            state.sink.record(&TableEvent::KindsRejected {
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

/// The routes of the developer API and the viewer.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route(route::CANVAS, get(canvas_json))
        .route(route::EVENTS, get(events_json))
        .route(route::EFFECT, post(apply_effect))
        .route(route::RELOAD, post(reload))
        .layer(DefaultBodyLimit::max(MAXIMUM_REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            guard_requests,
        ))
        .with_state(state)
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
