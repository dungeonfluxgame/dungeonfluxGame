//! The `/dev/director/*` routes: the Director's developer API, and its host tick.
//!
//! They sit behind the same guard as every route of the host (loopback host names, JSON for any
//! change, no foreign origin), because they are merged into the one router before its layers. A
//! read is a GET and a verb is a POST of JSON; every answer is a `DirectorReply`, `done` with the
//! value or `rejected` with a typed error and its message, and a refusal that is the caller's
//! mistake is a 400.
//!
//! A decision and the application of its moves are one step. The apply gate (a `tokio` mutex) is
//! held from the moment the Director decides until its last move is applied through
//! `apply_scene_change`, the one path `POST /dev/playground/scene` takes, so the 100 ms tick and
//! a verb can never apply camera moves out of order; the workbench lock inside it is taken for the
//! length of one decision and never across an `.await`. While it holds the gate the host also
//! looks at the playground's camera: a move made by anything but the Director (a scene change, a
//! free camera verb) is told to the Director, and a move that could not be applied is told back
//! (`apply_failed`) so a zoom is never left on the screen while the Director is wide.
//!
//! The host tick (`HOST_TICK_MS`) is a task that wakes the Director while it has a focus held, a
//! request waiting or a pull-back owed, so a hold runs out and a waiting direction starts without
//! anyone asking. It holds the routes weakly and ends when the host drops them.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use df_playground::controls::ControlsReply;
use df_table::director::StageDirection;
use df_table::inspect::InspectAudience;
use serde::Serialize;
use tokio::sync::Mutex as GateMutex;

use super::request::{ActReply, DirectorAct, DirectorFault, DirectorReply};
use super::workbench::{Decided, Workbench, scene_change};
use crate::playground::{SceneApplyError, apply_scene_change, current_stage_ids};
use crate::state::AppState;

/// How often the Director is woken while it has something to decide, in milliseconds.
pub const HOST_TICK_MS: u64 = 100;

/// The addresses of the Director's dev API.
pub mod path {
    pub const ACT: &str = "/dev/director/act";
    pub const SHOW: &str = "/dev/director/show";
}

/// The playground's camera as the Director last left it. A camera that reads differently was
/// moved by someone else.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CameraMark {
    count: u64,
    target: Option<String>,
}

impl CameraMark {
    fn of(reply: &ControlsReply) -> Self {
        Self {
            count: reply.scene.camera_count,
            target: reply
                .scene
                .camera
                .as_ref()
                .and_then(|camera| camera.target.clone()),
        }
    }
}

pub(super) struct DirectorRoutes {
    app: Arc<AppState>,
    workbench: Mutex<Result<Workbench, DirectorFault>>,
    /// The apply gate, with the camera as the Director last left it. Held across a decision and
    /// the application of every move it made.
    gate: GateMutex<CameraMark>,
    #[cfg(test)]
    pub(super) hook: super::apply_hook::ApplyHook,
}

impl DirectorRoutes {
    pub(super) fn new(app: Arc<AppState>, workbench: Result<Workbench, DirectorFault>) -> Self {
        Self {
            app,
            workbench: Mutex::new(workbench),
            gate: GateMutex::new(CameraMark::default()),
            #[cfg(test)]
            hook: super::apply_hook::ApplyHook::default(),
        }
    }

    fn workbench(&self) -> MutexGuard<'_, Result<Workbench, DirectorFault>> {
        self.workbench
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `call` on the workbench, or reports why there is none.
    pub(super) fn with<T>(
        &self,
        call: impl FnOnce(&mut Workbench) -> Result<T, DirectorFault>,
    ) -> Result<T, DirectorFault> {
        match self.workbench().as_mut() {
            Ok(workbench) => call(workbench),
            Err(error) => Err(error.clone()),
        }
    }

    /// Gives the Director the stage's names, and writes the line that says which it refused.
    fn refresh_stage(&self, ids: &[String]) -> Result<(), DirectorFault> {
        if let Some(line) = self.with(|workbench| Ok(workbench.refresh_stage(ids)))? {
            self.app.record_line(line);
        }
        Ok(())
    }

    /// Tells the Director when the camera is no longer where it left it.
    fn notice_outside_moves(&self, seen: &mut CameraMark) -> Result<(), DirectorFault> {
        let current = CameraMark::of(&self.app.playground().snapshot());
        if *seen == current {
            return Ok(());
        }
        let forgot =
            self.with(|workbench| workbench.camera_moved_elsewhere(current.target.as_deref()))?;
        if forgot {
            self.app
                .record_line("CAMERA released reason=moved_elsewhere".to_owned());
        }
        *seen = current;
        Ok(())
    }

    async fn apply_one(
        &self,
        direction: &StageDirection,
    ) -> Result<ControlsReply, SceneApplyError> {
        #[cfg(test)]
        self.hook.before_apply().await?;
        apply_scene_change(&self.app, scene_change(direction)).await
    }

    /// Writes the audit lines of a decision and applies each of its moves to the playground. A
    /// move that fails is told to the Director.
    async fn apply(&self, decided: Decided, seen: &mut CameraMark) -> ActReply {
        for line in &decided.lines {
            self.app.record_line(line.clone());
        }
        let mut problems = Vec::new();
        for direction in &decided.directions {
            match self.apply_one(direction).await {
                Ok(reply) => *seen = CameraMark::of(&reply),
                Err(error) => {
                    self.app.record_line(format!(
                        "CAMERA apply_failed status={}",
                        error.status.as_u16()
                    ));
                    problems.push(error.message);
                    if let Err(fault) = self.with(|workbench| {
                        workbench.apply_failed(direction);
                        Ok(())
                    }) {
                        problems.push(fault.to_string());
                    }
                }
            }
        }
        self.app.echo_new_lines();
        ActReply {
            lines: decided.lines,
            problems,
        }
    }

    /// One verb: the Director decides and the moves are applied, as one step.
    pub(super) async fn act(&self, act: DirectorAct) -> Result<ActReply, DirectorFault> {
        let mut seen = self.gate.lock().await;
        self.notice_outside_moves(&mut seen)?;
        let stage = current_stage_ids(&self.app).await;
        if let Ok(ids) = &stage {
            self.refresh_stage(ids)?;
        }
        let decided = self.with(|workbench| {
            if let Err(error) = &stage
                && matches!(act, DirectorAct::Focus { .. })
            {
                return Err(DirectorFault::NoStage {
                    reason: error.message.clone(),
                });
            }
            workbench.act(act)
        })?;
        Ok(self.apply(decided, &mut seen).await)
    }

    /// One host tick: the Director decides for the time now, if it has anything to decide, and
    /// the moves are applied, as one step.
    pub(super) async fn host_tick(&self) {
        let mut seen = self.gate.lock().await;
        if !matches!(self.with(|workbench| Ok(workbench.is_busy())), Ok(true)) {
            return;
        }
        let decided = match self.tick_decision(&mut seen).await {
            Ok(decided) => decided,
            Err(error) => {
                self.app
                    .record_line(format!("CAMERA tick_failed reason={error}"));
                return;
            }
        };
        self.apply(decided, &mut seen).await;
    }

    async fn tick_decision(&self, seen: &mut CameraMark) -> Result<Decided, DirectorFault> {
        self.notice_outside_moves(seen)?;
        if let Ok(ids) = current_stage_ids(&self.app).await {
            self.refresh_stage(&ids)?;
        }
        self.with(Workbench::tick)
    }
}

type Shared = axum::extract::State<Arc<DirectorRoutes>>;

fn status_of(error: &DirectorFault) -> StatusCode {
    match error {
        DirectorFault::Refused { .. }
        | DirectorFault::BadElement { .. }
        | DirectorFault::UnknownReason { .. }
        | DirectorFault::BadRequest { .. }
        | DirectorFault::NoStage { .. } => StatusCode::BAD_REQUEST,
    }
}

fn reply<T: Serialize>(result: Result<T, DirectorFault>) -> Response {
    let status = result.as_ref().err().map_or(StatusCode::OK, status_of);
    (status, Json(DirectorReply::from_result(result))).into_response()
}

async fn run_act(routes: &DirectorRoutes, bytes: &Bytes) -> Result<ActReply, DirectorFault> {
    let act: DirectorAct =
        serde_json::from_slice(bytes).map_err(|error| DirectorFault::BadRequest {
            reason: format!("the request body is not valid: {error}"),
        })?;
    routes.act(act).await
}

async fn act(axum::extract::State(routes): Shared, bytes: Bytes) -> Response {
    let result = run_act(&routes, &bytes).await;
    reply(result)
}

async fn show(axum::extract::State(routes): Shared) -> Response {
    if let Ok(ids) = current_stage_ids(&routes.app).await
        && let Err(fault) = routes.refresh_stage(&ids)
    {
        return reply::<()>(Err(fault));
    }
    reply(routes.with(|workbench| Ok(workbench.show())))
}

fn spawn_host_tick(routes: &Arc<DirectorRoutes>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let weak: Weak<DirectorRoutes> = Arc::downgrade(routes);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(HOST_TICK_MS)).await;
            let Some(routes) = weak.upgrade() else {
                break;
            };
            routes.host_tick().await;
        }
    });
}

/// The Director's routes over a workbench of its own, read by the operator: this is a local
/// build's dev API, absent from a hosted build's table.
pub fn router(app: Arc<AppState>) -> Router {
    router_for(app, InspectAudience::Operator)
}

/// The same for `audience`: a shared audience is told nothing of what is hidden.
pub fn router_for(app: Arc<AppState>, audience: InspectAudience) -> Router {
    let routes = Arc::new(DirectorRoutes::new(app, Workbench::new(audience)));
    spawn_host_tick(&routes);
    Router::new()
        .route(path::ACT, axum::routing::post(act))
        .route(path::SHOW, get(show))
        .with_state(routes)
}
