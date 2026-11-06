//! The `/dev/engine/scene/*` routes: the scene builder's developer API.
//!
//! They sit behind the same guard as every route of the host (loopback host names, JSON for any
//! change, no foreign origin), because they are merged into the one router before its layers. A
//! read is a GET and a change is a POST of JSON; every answer is a `SceneReply`, `done` with the
//! value or `rejected` with a `SceneBuilderError` and its message, at the error's status: a
//! refusal that is the caller's mistake is a 400, a timing change while plates are being built a
//! 409, a workbench that failed a 500 and one that never started a 503. A refusal writes one
//! `SCENE refused` audit line; the guard's middleware echoes the request's lines.
//!
//! The routes hold the workbench behind one lock that is taken for the length of a call and never
//! across an `.await`. A call runs the pipeline over fake providers in a clock of its own (a
//! bounded number of steps that never waits for time to pass) on the blocking pool, one task per
//! call.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use df_table::plate::timing::PlateTiming;
use serde::Serialize;

use super::lookahead::LookaheadRequest;
use super::request::{BuildRequest, RunRequest, SceneBuilderError, TimingRequest};
use super::workbench::Workbench;
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;

/// The addresses of the scene builder's dev API.
pub mod path {
    pub const BUILD: &str = "/dev/engine/scene/build";
    pub const SHOW: &str = "/dev/engine/scene/show";
    pub const PLATES: &str = "/dev/engine/scene/plates";
    pub const BUFFER: &str = "/dev/engine/scene/buffer";
    pub const LOOKAHEAD: &str = "/dev/engine/scene/lookahead";
    /// POST only: `scene timing set`. The timings are read with `plates`.
    pub const TIMING: &str = "/dev/engine/scene/timing";
}

struct SceneRoutes {
    app: Arc<AppState>,
    /// The workbench, or the refusal every call gets because it did not start.
    workbench: Mutex<Result<Workbench, SceneBuilderError>>,
}

impl SceneRoutes {
    fn workbench(&self) -> MutexGuard<'_, Result<Workbench, SceneBuilderError>> {
        self.workbench
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `call` on the workbench, or reports why there is none.
    fn with<T>(
        &self,
        call: impl FnOnce(&mut Workbench) -> Result<T, SceneBuilderError>,
    ) -> Result<T, SceneBuilderError> {
        match self.workbench().as_mut() {
            Ok(workbench) => call(workbench),
            Err(not_started) => Err(not_started.clone()),
        }
    }
}

/// Runs `call` on the workbench on the blocking pool. A task that stopped without an answer (it
/// panicked, or the runtime is stopping) is `Stopped`.
async fn on_workbench<T, Call>(
    routes: &Arc<SceneRoutes>,
    call: Call,
) -> Result<T, SceneBuilderError>
where
    T: Send + 'static,
    Call: FnOnce(&mut Workbench) -> Result<T, SceneBuilderError> + Send + 'static,
{
    let worker = Arc::clone(routes);
    tokio::task::spawn_blocking(move || worker.with(call))
        .await
        .unwrap_or_else(|_| Err(SceneBuilderError::Stopped))
}

/// Writes the refusal line of `verb` when `result` failed, and answers with it.
fn answer<T: Serialize>(
    routes: &SceneRoutes,
    verb: &str,
    result: Result<T, SceneBuilderError>,
) -> Response {
    if let Err(error) = &result {
        routes
            .app
            .record_line(format!("SCENE refused verb={verb} error={}", error.key()));
    }
    reply(result)
}

async fn build(State(routes): State<Arc<SceneRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<BuildRequest>(&bytes) {
        Ok(request) => on_workbench(&routes, move |bench| bench.build(&request)).await,
        Err(error) => Err(error.into()),
    };
    answer(&routes, "build", result)
}

async fn show(State(routes): State<Arc<SceneRoutes>>) -> Response {
    let result = on_workbench(&routes, |bench| Ok(bench.show())).await;
    answer(&routes, "show", result)
}

async fn plates(State(routes): State<Arc<SceneRoutes>>) -> Response {
    let result = on_workbench(&routes, |bench| Ok(bench.plates())).await;
    answer(&routes, "plates", result)
}

async fn plates_run(State(routes): State<Arc<SceneRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<RunRequest>(&bytes) {
        Ok(request) => on_workbench(&routes, move |bench| bench.run(&request)).await,
        Err(error) => Err(error.into()),
    };
    answer(&routes, "plates_run", result)
}

async fn lookahead(State(routes): State<Arc<SceneRoutes>>) -> Response {
    let result = on_workbench(&routes, |bench| bench.lookahead(None)).await;
    answer(&routes, "lookahead", result)
}

async fn lookahead_change(State(routes): State<Arc<SceneRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<LookaheadRequest>(&bytes) {
        Ok(request) => on_workbench(&routes, move |bench| bench.lookahead(Some(&request))).await,
        Err(error) => Err(error.into()),
    };
    answer(&routes, "lookahead_change", result)
}

async fn timing(State(routes): State<Arc<SceneRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<TimingRequest>(&bytes) {
        Ok(request) => on_workbench(&routes, move |bench| bench.set_timing(&request)).await,
        Err(error) => Err(error.into()),
    };
    answer(&routes, "timing", result)
}

async fn buffer(State(routes): State<Arc<SceneRoutes>>) -> Response {
    let result = on_workbench(&routes, |bench| Ok(bench.buffer_show())).await;
    answer(&routes, "buffer", result)
}

/// The scene builder's routes over a workbench of its own that logs to the host's event log.
pub fn router(app: Arc<AppState>) -> Router {
    let workbench = Workbench::new(PlateTiming::default(), Some(Arc::clone(app.log())))
        .map_err(|error| error.not_started());
    let routes = Arc::new(SceneRoutes {
        app,
        workbench: Mutex::new(workbench),
    });
    Router::new()
        .route(path::BUILD, post(build))
        .route(path::SHOW, get(show))
        .route(path::PLATES, get(plates).post(plates_run))
        .route(path::BUFFER, get(buffer))
        .route(path::LOOKAHEAD, get(lookahead).post(lookahead_change))
        .route(path::TIMING, post(timing))
        .with_state(routes)
}
