//! The developer API and the live viewer, served on the loopback address only.
//!
//! It owns the sockets, the routes and the lock around the canvas. A request locks the canvas
//! briefly and never across an `.await`; the lock stays held from converting a request to
//! applying it, so a reload cannot change the kinds in between. Every request passes the guard
//! first (`guard.rs`), every change goes through `Canvas::apply`, and every rejection, including
//! the ones that happen before a request becomes an effect, is recorded as an event. The audit
//! lines a request records are echoed to the terminal once, after it is answered, by the guard
//! middleware, so no route echoes on its own.
//!
//! Every answer of a `/dev/` route that changes something or refuses is one `HTTP served` line
//! with its route template, status and duration (`record_served`); a successful read is not, and
//! neither is a page's report. The clock records when it wakes, idles, stalls and stops.

pub mod request;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, MatchedPath, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use df_table::canvas::{Canvas, CanvasEffect};
use df_table::element::{Degrees, ElementId};
use df_table::event::{EffectKind, RejectionCause, TableEvent};
use df_table::ports::EventSink;
use df_types::saturating_millis;
use serde::Deserialize;

use self::request::{EventsQuery, HostError, QueriedRoute, RequestMethod};
use crate::dev_http;
use crate::guard;
use crate::kind_files::load_kind_files_async;
use crate::limits::{
    FRAME_WAIT, MAXIMUM_EVENTS_PER_ANSWER, MAXIMUM_REQUEST_BYTES, TICK_INTERVAL, TRAVEL_TICK_CUT,
};
use crate::media_library;
use crate::playground;
use crate::render;
use crate::request::RequestError;
use crate::state::AppState;
use crate::view::{CanvasView, ViewError};
use crate::wire::{EffectRequest, EventsReply, FRAME_IDLE_NO_CONTENT, Reply, capped, route};

/// The addresses of the host's own dev routes.
pub mod path {
    /// What the host's log, event sink and clock are doing (GET, a `HostReply<HostStatus>`).
    pub const HOST_STATUS: &str = "/dev/host/status";
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

/// Runs the guard before every route and records what it refuses, then echoes the audit lines
/// the request recorded to the terminal: the one echo of every route.
async fn guard_requests(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let response = match guard::check(request.method(), request.uri().path(), request.headers()) {
        Ok(()) => next.run(request).await,
        Err(error) => {
            state.record_rejection(None, None, RejectionCause::ForbiddenRequest);
            // The guard runs before routing, so there is no route template; the raw path is never
            // written (it carries names a caller chose).
            state.record_line(format!(
                "HTTP refused method={} route=unmatched cause={}",
                RequestMethod::of(request.method()).key(),
                RejectionCause::ForbiddenRequest.key()
            ));
            rejected(
                error.status(),
                Some(RejectionCause::ForbiddenRequest),
                error.message(),
            )
        }
    };
    state.echo_new_lines();
    response
}

/// Whether an answer of `route` gets an `HTTP served` line: a `/dev/` route but a page's report,
/// unless it is a read that succeeded.
fn logs_served(route: &str, method: RequestMethod, status: StatusCode) -> bool {
    let quiet_read = method.reads() && !status.is_client_error() && !status.is_server_error();
    route.starts_with("/dev/") && route != route::DEV_CLIENT_REPORT && !quiet_read
}

/// Records `HTTP served method= route= status= duration_ms=` for a `/dev/` route's answer,
/// including the 400 and 413 answers of the body and query readers. It runs as a route layer, so
/// the route is the matched template, never the raw path.
async fn record_served(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let method = RequestMethod::of(request.method());
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned());
    let started = tokio::time::Instant::now();
    let response = next.run(request).await;
    let status = response.status();
    if let Some(route) = route.filter(|route| logs_served(route, method, status)) {
        state.record_line(format!(
            "HTTP served method={} route={route} status={} duration_ms={}",
            method.key(),
            status.as_u16(),
            saturating_millis(started.elapsed())
        ));
    }
    response
}

/// Records and answers a query a host route cannot read.
fn refuse_query(state: &AppState, route: QueriedRoute, rejection: &QueryRejection) -> Response {
    let error = HostError::InvalidQuery {
        detail: rejection.body_text(),
    };
    state.record_line(format!(
        "HOST refused route={} error={}",
        route.key(),
        error.key()
    ));
    dev_http::reply::<(), HostError>(Err(error))
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

/// The query of `route::PLAYGROUND_FRAME`; the fields are named like `wire::FRAME_AFTER_PARAMETER`,
/// `wire::FRAME_BOOT_PARAMETER` and `wire::FRAME_IDLE_PARAMETER`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrameQuery {
    after: Option<u64>,
    /// The run of the host the frame `after` came from. A page that names another run kept a frame
    /// from before a restart, so it is answered at once even when this run's count is the same.
    boot: Option<String>,
    /// `idle=204`: answer a wait that ran out with nothing changed by an empty 204 (the page
    /// asks again with the same count) instead of a frame it would only throw away.
    idle: Option<u16>,
}

/// The current frame. With `after=N`, a request made while the host is still at frame N is held
/// until the next change (or `FRAME_WAIT`), so a page learns of a change as it happens instead of
/// by asking again and again. A different count, or a `boot` that names another run of the host
/// (after a restart), answers at once.
/// The frame is built once for each change count and every page is sent the same bytes.
async fn frame_json(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    query: Result<Query<FrameQuery>, QueryRejection>,
) -> Response {
    let origin = playground::ChangeOrigin::from_headers(&headers);
    match query {
        Ok(Query(query)) => frame_answer(&state, &query, &origin, FRAME_WAIT).await,
        Err(rejection) => refuse_query(&state, QueriedRoute::Frame, &rejection),
    }
}

/// `frame_json` with the wait as a parameter, so a test need not wait twenty-five seconds. The
/// first answer to a page (any reader but the dev command line) of a frame that holds a change
/// not yet logged writes `FRAME served frame= after= bytes= waited_ms= source= client=`, so the
/// line says when a page got the change. The clock's steps of a travel, an idle 204, later
/// answers of the same change and the command line's reads write nothing.
async fn frame_answer(
    state: &Arc<AppState>,
    query: &FrameQuery,
    origin: &playground::ChangeOrigin,
    patience: Duration,
) -> Response {
    let started = tokio::time::Instant::now();
    let same_run = query
        .boot
        .as_deref()
        .is_none_or(|boot| state.playground().is_boot(boot));
    if let Some(after) = query.after.filter(|_| same_run) {
        let mut changes = state.watch_changes();
        // A timeout is not an error: the page asks again with the same count.
        let timed_out = tokio::time::timeout(patience, changes.wait_for(|count| *count != after))
            .await
            .is_err();
        if timed_out && query.idle == Some(FRAME_IDLE_NO_CONTENT) {
            return StatusCode::NO_CONTENT.into_response();
        }
    }
    match state.frame().await {
        Ok(built) => {
            let chunks = built.chunks();
            let length: usize = chunks.iter().map(Bytes::len).sum();
            if !origin.is_cli() && state.first_to_serve_frame(built.count()) {
                let after = query
                    .after
                    .map_or_else(|| "-".to_owned(), |after| after.to_string());
                let [(_, source), (_, client)] = origin.pairs();
                state.record_line(format!(
                    "FRAME served frame={} after={after} bytes={length} waited_ms={} \
                     source={source} client={client}",
                    built.count(),
                    saturating_millis(started.elapsed())
                ));
            }
            let pieces = futures_util::stream::iter(
                chunks
                    .into_iter()
                    .map(Ok::<Bytes, std::convert::Infallible>),
            );
            let mut response = Response::new(axum::body::Body::from_stream(pieces));
            let headers = response.headers_mut();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
            response
        }
        Err(error) => view_failed(state, &error),
    }
}

/// Events newer than `after`, or the newest page when `after` is absent or beyond the newest
/// sequence (which means the host restarted since the reader last looked); only the lines of
/// `area` when it is given, and at most `limit` (never more than `MAXIMUM_EVENTS_PER_ANSWER`). A
/// query that does not read is refused and recorded.
async fn events_json(
    State(state): State<Arc<AppState>>,
    query: Result<Query<EventsQuery>, QueryRejection>,
) -> Response {
    let query = match query {
        Ok(Query(query)) => query,
        Err(rejection) => return refuse_query(&state, QueriedRoute::Events, &rejection),
    };
    let log = state.log();
    let latest = log.latest_sequence();
    let limit = query.limit.map_or(MAXIMUM_EVENTS_PER_ANSWER, |limit| {
        limit.min(MAXIMUM_EVENTS_PER_ANSWER)
    });
    let after = query.after.filter(|after| *after <= latest);
    let events = match (after, &query.area) {
        (Some(after), None) => log.after_sequence(after, limit),
        (Some(after), Some(area)) => log.after_sequence_in(after, limit, area.as_str()),
        (None, None) => log.newest(limit),
        (None, Some(area)) => log.newest_in(limit, area.as_str()),
    };
    Json(EventsReply {
        events,
        latest,
        forgotten: log.forgotten(),
    })
    .into_response()
}

/// What the host's log, event sink and clock are doing.
async fn host_status(State(state): State<Arc<AppState>>) -> Response {
    dev_http::reply::<_, HostError>(Ok(state.host_status()))
}

async fn apply_effect(
    State(state): State<Arc<AppState>>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let state = state.as_ref();
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
    if let Err(error) = pivot_has_settled(state, &canvas, &effect) {
        return reject_conversion(state, effect_kind, element, &error);
    }
    let follow = Follow::of(&canvas, &effect);
    let applied = canvas.apply(effect, state.sink());
    if canvas.is_moving() {
        state.wake_clock();
    }
    let revision = canvas.revision();
    drop(canvas);
    match applied {
        Ok(()) => {
            follow.apply(state);
            answer(
                StatusCode::OK,
                Reply::Done {
                    revision,
                    notices: Vec::new(),
                },
            )
        }
        Err(error) => rejected(
            StatusCode::BAD_REQUEST,
            Some(error.cause()),
            &error.to_string(),
        ),
    }
}

/// Refuses a new pivot for an element whose turn the page is still gliding. The canvas judges the
/// stored rotation, which is 0 the moment `rotate ID 0` is applied, but `transform-origin` does
/// not glide, so a pivot that changes during the glide makes the figure jump.
fn pivot_has_settled(
    state: &AppState,
    canvas: &Canvas,
    effect: &CanvasEffect,
) -> Result<(), RequestError> {
    let CanvasEffect::Transform {
        id,
        pivot: Some(new_pivot),
        ..
    } = effect
    else {
        return Ok(());
    };
    // While the element is still turned the canvas refuses a new pivot itself, with its own
    // message; this is about the turn that has just ended.
    let changes_it = canvas.element(id).is_some_and(|element| {
        element.appearance.pivot != *new_pivot && element.appearance.rotation == Degrees::default()
    });
    match state.turn_settling(id) {
        Some(remaining) if changes_it => Err(RequestError::PivotDuringTurn {
            element: id.to_string(),
            wait_ms: saturating_millis(remaining).saturating_add(1),
        }),
        _ => Ok(()),
    }
}

/// What the playground must follow once an effect has been applied.
enum Follow {
    Nothing,
    /// The element's rotation changed, so the page glides it.
    Turned(ElementId),
    /// These elements are gone (despawned or cleared): a camera on one goes back to the stage.
    Gone(Vec<String>),
}

impl Follow {
    /// Reads what `effect` is about to change, before it is applied.
    fn of(canvas: &Canvas, effect: &CanvasEffect) -> Self {
        match effect {
            CanvasEffect::Transform { id, rotation, .. } => canvas
                .element(id)
                .filter(|element| element.appearance.rotation != *rotation)
                .map_or(Self::Nothing, |_| Self::Turned(id.clone())),
            CanvasEffect::Despawn { id } => Self::Gone(vec![id.to_string()]),
            CanvasEffect::Clear => Self::Gone(
                canvas
                    .elements()
                    .map(|element| element.id.to_string())
                    .collect(),
            ),
            _ => Self::Nothing,
        }
    }

    /// Acts on it after the canvas lock is released.
    fn apply(self, state: &AppState) {
        match self {
            Self::Nothing => {}
            Self::Turned(id) => state.note_turn(id),
            Self::Gone(ids) => {
                state.forget_turns(&ids);
                playground::release_camera_from(state, &ids);
            }
        }
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
    match loaded {
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
    }
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
        .route(path::HOST_STATUS, get(host_status))
        .with_state(state.clone());
    api.merge(playground::router(state.clone()))
        .merge(media_library::router(state.clone()))
        .merge(crate::engine::router(state.clone()))
        .merge(crate::scene_builder::router(state.clone()))
        .merge(crate::director::router(state.clone()))
        // The services router holds the imagegen and locate routes over one text runtime and one
        // picture generator.
        .merge(crate::services::routes::router(state.clone()))
        .merge(crate::cast::routes::router(state.clone()))
        // A route layer sees the matched route, and the answers of the body and query readers.
        .route_layer(middleware::from_fn_with_state(state.clone(), record_served))
        .layer(DefaultBodyLimit::max(MAXIMUM_REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            announce_changes,
        ))
        .layer(middleware::from_fn_with_state(state, guard_requests))
}

/// Writes `HOST clock_stopped` when the clock's task ends (it is aborted, or its runtime stops).
struct ClockStopped(Arc<AppState>);

impl Drop for ClockStopped {
    fn drop(&mut self) {
        self.0.record_line_and_echo("HOST clock_stopped".to_owned());
    }
}

/// Starts the host's clock: every `TICK_INTERVAL` while something travels it advances travelling
/// elements by the time that really passed, which the canvas cuts to a sane length if the host
/// stalled. With nothing travelling it sleeps until an effect wakes it, so an idle host has no
/// timer running. It reads tokio's clock, so a test with paused time drives it. Abort the handle
/// to stop it.
///
/// It records `HOST clock_woken moving=` when it wakes to move something, `HOST clock_idle ticks=
/// ran_ms=` when everything has stopped, `HOST clock_stall gap_ms= cut_to_ms=` for a gap between
/// ticks longer than the canvas travels in one step, and `HOST clock_stopped` when it ends.
pub fn spawn_clock(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let _stopped = ClockStopped(Arc::clone(&state));
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last = tokio::time::Instant::now();
        let mut woken_at = last;
        let mut ticks_since_woken = 0_u64;
        loop {
            if !state.is_moving() {
                if ticks_since_woken > 0 {
                    state.record_line_and_echo(format!(
                        "HOST clock_idle ticks={ticks_since_woken} ran_ms={}",
                        saturating_millis(woken_at.elapsed())
                    ));
                    ticks_since_woken = 0;
                }
                state.clock_woken().await;
                // Time spent asleep is not travel: start counting again from the wake-up.
                interval.reset();
                last = tokio::time::Instant::now();
                woken_at = last;
                // A signal kept from an effect that has already stopped moving wakes the clock
                // for nothing; that is not recorded.
                let moving = state.moving_count();
                if moving > 0 {
                    state.record_line_and_echo(format!("HOST clock_woken moving={moving}"));
                }
                continue;
            }
            interval.tick().await;
            let now = tokio::time::Instant::now();
            let gap = now.duration_since(last);
            if gap > TRAVEL_TICK_CUT {
                state.record_line_and_echo(format!(
                    "HOST clock_stall gap_ms={} cut_to_ms={}",
                    saturating_millis(gap),
                    saturating_millis(TRAVEL_TICK_CUT)
                ));
            }
            last = now;
            state.tick(gap.as_secs_f32());
            ticks_since_woken = ticks_since_woken.saturating_add(1);
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

#[cfg(test)]
impl FrameQuery {
    pub(crate) fn for_test(after: Option<u64>, idle: Option<u16>) -> Self {
        Self {
            after,
            boot: None,
            idle,
        }
    }

    pub(crate) fn with_boot(self, boot: &str) -> Self {
        Self {
            boot: Some(boot.to_owned()),
            ..self
        }
    }
}

#[cfg(test)]
pub(crate) async fn frame_answer_for_test(
    state: &Arc<AppState>,
    query: &FrameQuery,
    patience: Duration,
) -> Response {
    frame_answer(state, query, &playground::ChangeOrigin::default(), patience).await
}
