//! The `/dev/engine/scene/*` routes: the scene builder's developer API.
//!
//! They sit behind the same guard as every route of the host (loopback host names, JSON for any
//! change, no foreign origin), because they are merged into the one router before its layers. A
//! read is a GET and a change is a POST of JSON; every answer is a `SceneReply`, `done` with the
//! value or `rejected` with a typed error and its message, and a refusal that is the caller's
//! mistake is a 400 while a workbench that failed is a 500 and one that never started a 503. The
//! routes hold the workbench behind one lock that is taken for the length of a call and never across
//! an `.await`: a call runs the pipeline over fake providers in a clock of its own, which is a
//! bounded number of cheap steps, so it never waits for time to pass.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use df_table::plate_timing::PlateTiming;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::lookahead::LookaheadRequest;
use super::request::{BuildRequest, RunRequest, SceneError, SceneReply};
use super::workbench::Workbench;
use crate::state::AppState;

/// The addresses of the scene builder's dev API.
pub mod path {
    pub const BUILD: &str = "/dev/engine/scene/build";
    pub const SHOW: &str = "/dev/engine/scene/show";
    pub const PLATES: &str = "/dev/engine/scene/plates";
    pub const BUFFER: &str = "/dev/engine/scene/buffer";
    pub const LOOKAHEAD: &str = "/dev/engine/scene/lookahead";
}

struct SceneRoutes {
    app: Arc<AppState>,
    workbench: Mutex<Result<Workbench, SceneError>>,
}

impl SceneRoutes {
    fn workbench(&self) -> MutexGuard<'_, Result<Workbench, SceneError>> {
        self.workbench
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `call` on the workbench, or reports why there is none.
    fn with<T>(
        &self,
        call: impl FnOnce(&mut Workbench) -> Result<T, SceneError>,
    ) -> Result<T, SceneError> {
        match self.workbench().as_mut() {
            Ok(workbench) => call(workbench),
            Err(error) => Err(SceneError::NotStarted {
                reason: error.to_string(),
            }),
        }
    }
}

fn status_of(error: &SceneError) -> StatusCode {
    match error {
        SceneError::InvalidName { .. }
        | SceneError::NothingBuilt
        | SceneError::BadRequest { .. } => StatusCode::BAD_REQUEST,
        SceneError::Workbench { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        SceneError::NotStarted { .. } => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn reply<T: Serialize>(routes: &SceneRoutes, result: Result<T, SceneError>) -> Response {
    routes.app.echo_new_lines();
    let status = result.as_ref().err().map_or(StatusCode::OK, status_of);
    (status, Json(SceneReply::from_result(result))).into_response()
}

fn body<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, SceneError> {
    serde_json::from_slice(bytes).map_err(|error| SceneError::BadRequest {
        reason: format!("the request body is not valid: {error}"),
    })
}

type Shared = axum::extract::State<Arc<SceneRoutes>>;

async fn build(axum::extract::State(routes): Shared, bytes: Bytes) -> Response {
    let result =
        body::<BuildRequest>(&bytes).and_then(|request| routes.with(|bench| bench.build(&request)));
    reply(&routes, result)
}

async fn show(axum::extract::State(routes): Shared) -> Response {
    let result = routes.with(|bench| Ok(bench.show()));
    reply(&routes, result)
}

async fn plates(axum::extract::State(routes): Shared) -> Response {
    let result = routes.with(|bench| Ok(bench.plates()));
    reply(&routes, result)
}

async fn plates_run(axum::extract::State(routes): Shared, bytes: Bytes) -> Response {
    let result =
        body::<RunRequest>(&bytes).and_then(|request| routes.with(|bench| bench.run(&request)));
    reply(&routes, result)
}

async fn lookahead(axum::extract::State(routes): Shared) -> Response {
    let result = routes.with(|bench| bench.lookahead(None));
    reply(&routes, result)
}

async fn lookahead_change(axum::extract::State(routes): Shared, bytes: Bytes) -> Response {
    let result = body::<LookaheadRequest>(&bytes)
        .and_then(|request| routes.with(|bench| bench.lookahead(Some(&request))));
    reply(&routes, result)
}

async fn buffer(axum::extract::State(routes): Shared) -> Response {
    let result = routes.with(|bench| Ok(bench.buffer_show()));
    reply(&routes, result)
}

/// The scene builder's routes over a workbench of its own that logs to the host's event log.
pub fn router(app: Arc<AppState>) -> Router {
    let workbench = Workbench::new(PlateTiming::defaults(), Some(Arc::clone(app.log())));
    let routes = Arc::new(SceneRoutes {
        app,
        workbench: Mutex::new(workbench),
    });
    Router::new()
        .route(path::BUILD, axum::routing::post(build))
        .route(path::SHOW, get(show))
        .route(path::PLATES, get(plates).post(plates_run))
        .route(path::BUFFER, get(buffer))
        .route(path::LOOKAHEAD, get(lookahead).post(lookahead_change))
        .with_state(routes)
}
