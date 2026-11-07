//! The `/dev/director/*` routes: the Director's developer API, and its host tick.
//!
//! They sit behind the same guard as every route of the host (loopback host names, JSON for any
//! change, no foreign origin), because they are merged into the one router before its layers. A
//! read is a GET and a verb is a POST of JSON; every answer is a `DirectorReply`, `done` with the
//! value or `rejected` with a `DirectorError` and its message, at the error's status. A refusal
//! writes one `DIRECTOR refused` audit line; the guard's middleware echoes the request's lines.
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

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use df_playground::controls::ControlsReply;
use df_table::director::StageDirection;
use df_table::director::reason::MINIMUM_ZOOM_PERCENT;
use df_table::inspect::InspectAudience;
use df_table::ports::Clock;
use df_types::saturating_millis;
use tokio::sync::Mutex as GateMutex;
use tokio::time::Instant;

use super::limits::HOST_TICK_MS;
use super::request::{ActReply, ApplyCause, DirectorAct, DirectorError};
use super::workbench::{Decided, Workbench, scene_change};
use crate::dev_http::{parse_body, reply};
use crate::playground::{
    PlaygroundChangeError, SceneApplyError, apply_scene_change, current_stage_ids,
};
use crate::state::AppState;

/// The addresses of the Director's dev API.
pub mod path {
    pub const ACT: &str = "/dev/director/act";
    pub const SHOW: &str = "/dev/director/show";
}

/// Milliseconds since the routes were made, on the runtime's monotonic clock: the edge where the
/// Director's time is read. It is `tokio`'s clock, so a test that pauses the runtime moves it.
struct RuntimeClock {
    started: Instant,
}

impl Clock for RuntimeClock {
    fn now_ms(&self) -> u64 {
        saturating_millis(self.started.elapsed())
    }
}

/// The runtime's clock from now, as the Director reads it.
pub(super) fn runtime_clock() -> Arc<dyn Clock> {
    Arc::new(RuntimeClock {
        started: Instant::now(),
    })
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
            target: reply.scene.camera_target().map(str::to_owned),
        }
    }
}

/// `CAMERA <verb> kind=focus|wide target=<element|->` of a direction.
fn camera_line(verb: &str, direction: &StageDirection) -> String {
    match direction {
        StageDirection::Focus { element, .. } => {
            format!("CAMERA {verb} kind=focus target={element}")
        }
        StageDirection::Wide { .. } => format!("CAMERA {verb} kind=wide target=-"),
    }
}

/// The line of a direction the playground applied: what it drew and how long applying took.
fn applied_line(direction: &StageDirection, duration_ms: u64) -> String {
    let (zoom_percent, over_ms) = match direction {
        StageDirection::Focus {
            zoom_percent,
            over_ms,
            ..
        } => (*zoom_percent, *over_ms),
        StageDirection::Wide { over_ms, .. } => (MINIMUM_ZOOM_PERCENT, *over_ms),
    };
    format!(
        "{} zoom={zoom_percent} over_ms={over_ms} duration_ms={duration_ms}",
        camera_line("applied", direction)
    )
}

/// The line of a host tick the Director refused: the error's key, never its sentence.
pub(super) fn tick_failed_line(error: &DirectorError) -> String {
    format!("CAMERA tick_failed error={}", error.key())
}

pub(super) struct DirectorRoutes {
    app: Arc<AppState>,
    workbench: Mutex<Workbench>,
    /// The apply gate, with the camera as the Director last left it. Held across a decision and
    /// the application of every move it made.
    gate: GateMutex<CameraMark>,
    #[cfg(test)]
    pub(super) hook: super::apply_hook::ApplyHook,
}

impl DirectorRoutes {
    pub(super) fn new(app: Arc<AppState>, workbench: Workbench) -> Self {
        Self {
            app,
            workbench: Mutex::new(workbench),
            gate: GateMutex::new(CameraMark::default()),
            #[cfg(test)]
            hook: super::apply_hook::ApplyHook::default(),
        }
    }

    fn workbench(&self) -> MutexGuard<'_, Workbench> {
        self.workbench
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `call` on the workbench, holding its lock for the call only.
    pub(super) fn with<T>(&self, call: impl FnOnce(&mut Workbench) -> T) -> T {
        call(&mut self.workbench())
    }

    /// Gives the Director the stage's names, and writes the line that says which it refused.
    fn refresh_stage(&self, ids: &[String]) {
        if let Some(line) = self.with(|workbench| workbench.refresh_stage(ids)) {
            self.app.record_line(line);
        }
    }

    /// Tells the Director when the camera is no longer where it left it.
    fn notice_outside_moves(&self, seen: &mut CameraMark) -> Result<(), DirectorError> {
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
            let started = Instant::now();
            match self.apply_one(direction).await {
                Ok(reply) => {
                    *seen = CameraMark::of(&reply);
                    let duration_ms = saturating_millis(started.elapsed());
                    self.app.record_line(applied_line(direction, duration_ms));
                }
                Err(error) => {
                    let cause = ApplyCause::from(&PlaygroundChangeError::from(error.clone()));
                    self.app.record_line(format!(
                        "{} error={}",
                        camera_line("apply_failed", direction),
                        cause.key()
                    ));
                    problems.push(error.message);
                    self.with(|workbench| workbench.apply_failed(direction));
                }
            }
        }
        ActReply {
            lines: decided.lines,
            problems,
        }
    }

    /// One verb: the Director decides and the moves are applied, as one step.
    pub(super) async fn act(&self, act: DirectorAct) -> Result<ActReply, DirectorError> {
        let mut seen = self.gate.lock().await;
        self.notice_outside_moves(&mut seen)?;
        let stage = current_stage_ids(&self.app).await;
        match &stage {
            Ok(ids) => self.refresh_stage(ids),
            Err(error) if matches!(act, DirectorAct::Focus { .. }) => {
                return Err(DirectorError::NoStage {
                    detail: error.message.clone(),
                });
            }
            Err(_) => {}
        }
        let decided = self.with(|workbench| workbench.act(act))?;
        Ok(self.apply(decided, &mut seen).await)
    }

    /// One host tick: the Director decides for the time now, if it has anything to decide, and
    /// the moves are applied, as one step. Nothing answers a tick, so a failure is a line with
    /// the error's key, and no request's middleware echoes the tick's lines: it echoes them
    /// itself.
    pub(super) async fn host_tick(&self) {
        let mut seen = self.gate.lock().await;
        if !self.with(|workbench| workbench.is_busy()) {
            return;
        }
        match self.tick_decision(&mut seen).await {
            Ok(decided) => {
                self.apply(decided, &mut seen).await;
            }
            Err(error) => self.app.record_line(tick_failed_line(&error)),
        }
        self.app.echo_new_lines();
    }

    async fn tick_decision(&self, seen: &mut CameraMark) -> Result<Decided, DirectorError> {
        self.notice_outside_moves(seen)?;
        if let Ok(ids) = current_stage_ids(&self.app).await {
            self.refresh_stage(&ids);
        }
        self.with(Workbench::tick)
    }

    /// Writes the refusal line of a call to `verb` that failed.
    fn record_refusal<T>(&self, verb: &str, result: &Result<T, DirectorError>) {
        if let Err(error) = result {
            self.app.record_line(format!(
                "DIRECTOR refused verb={verb} error={}",
                error.key()
            ));
        }
    }
}

async fn act(State(routes): State<Arc<DirectorRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<DirectorAct>(&bytes) {
        Ok(act) => routes.act(act).await,
        Err(error) => Err(DirectorError::from(error)),
    };
    routes.record_refusal("act", &result);
    reply(result)
}

async fn show(State(routes): State<Arc<DirectorRoutes>>) -> Response {
    if let Ok(ids) = current_stage_ids(&routes.app).await {
        routes.refresh_stage(&ids);
    }
    reply(Ok::<_, DirectorError>(
        routes.with(|workbench| workbench.show()),
    ))
}

/// Starts the host tick, with a `DIRECTOR tick_started` line, and writes `DIRECTOR tick_stopped`
/// when it ends because the routes were dropped.
fn spawn_host_tick(routes: &Arc<DirectorRoutes>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let app = Arc::clone(&routes.app);
    app.record_line_and_echo(format!("DIRECTOR tick_started interval_ms={HOST_TICK_MS}"));
    let weak: Weak<DirectorRoutes> = Arc::downgrade(routes);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(HOST_TICK_MS)).await;
            let Some(routes) = weak.upgrade() else {
                break;
            };
            routes.host_tick().await;
        }
        app.record_line_and_echo("DIRECTOR tick_stopped".to_owned());
    });
}

/// The Director's routes over a workbench of its own, read by the operator: this is a local
/// build's dev API, absent from a hosted build's table.
pub fn router(app: Arc<AppState>) -> Router {
    router_for(app, InspectAudience::Operator)
}

/// The same for `audience`: a shared audience is told nothing of what is hidden. The Director's
/// clock is the runtime's monotonic clock from now.
pub fn router_for(app: Arc<AppState>, audience: InspectAudience) -> Router {
    let workbench = Workbench::new(audience, runtime_clock());
    let routes = Arc::new(DirectorRoutes::new(app, workbench));
    spawn_host_tick(&routes);
    Router::new()
        .route(path::ACT, post(act))
        .route(path::SHOW, get(show))
        .with_state(routes)
}
