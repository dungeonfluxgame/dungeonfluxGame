//! The `/dev/engine/*` routes: the owner actor's developer API.
//!
//! They sit behind the same guard as every route of the host (loopback host names, JSON for any
//! change, no foreign origin), because they are merged into the one router before its layers.
//! A read is a GET and a change is a POST of JSON; every answer is an `EngineReply`, `done` with
//! the value or `rejected` with a typed error and its message, and a refusal that is the owner's
//! busy or stopped state is a 503 while any other refusal is a 400. The routes hold no state of
//! their own beyond the handle to the owner thread, which they start with the router.

mod observe;
mod play;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::adapters::dev_route;
use crate::adapters::{RulesBackend, RulesCheckKit, story_ports};
use crate::state::AppState;

use super::adapters::{
    Adapters, BrokerSide, ChaosInterpreter, DevClock, DevControls, DevKnowledge, DevNarrator,
};
use super::broker_fields::SinkBrokerAudit;
use super::commit::ScriptedCommitStore;
use super::extras::Extras;
use super::handle::EngineHandle;
use super::limits::COMMAND_MAILBOX_CAPACITY;
use super::report::{EngineError, EngineReply};
use super::request::{
    AudienceSpec, ChaosRequest, ClockControl, DiceControl, ForceRequest, InputRequest, QueueControl,
};
use super::setup::{EngineConfig, EngineStartError, machines_for};

/// The addresses of the dev engine API.
pub mod path {
    pub const INPUT: &str = "/dev/engine/input";
    pub const DUMP: &str = "/dev/engine/dump";
    pub const VIABLE: &str = "/dev/engine/viable";
    pub const FORCE: &str = "/dev/engine/force";
    pub const PROJECTION: &str = "/dev/engine/projection";
    pub const QUEUE: &str = "/dev/engine/queue";
    pub const DESCRIBE: &str = "/dev/engine/describe";
    pub const BROKER_QUEUES: &str = "/dev/engine/broker/queues";
    pub const BROKER_LEDGER: &str = "/dev/engine/broker/ledger";
    pub const BROKER_REQUEST: &str = "/dev/engine/broker/request";
    pub const HEALTH: &str = "/dev/engine/health";
    pub const CLOCK: &str = "/dev/engine/clock";
    pub const DICE: &str = "/dev/engine/dice";
    pub const CHAOS: &str = "/dev/engine/chaos";
    pub const RULES_CHECK: &str = "/dev/engine/rules/check";
    pub const HISTORY: &str = "/dev/engine/history";
    pub const TRACE: &str = "/dev/engine/trace";
    pub const REPLAY: &str = "/dev/engine/replay";
    pub const CONFIG: &str = "/dev/engine/config";
    /// A TV's or a phone's `MachineView` as the dev report of it (GET `?audience=tv|N`).
    pub const WIRE_VIEW: &str = "/dev/engine/wire/view";
    /// A `MachineIntent` for a seat, answered with the dev report of its receipt (POST).
    pub const WIRE_INTENT: &str = "/dev/engine/wire/intent";
}

/// The query parameter of `path::VIABLE`.
pub const SEAT_PARAMETER: &str = "seat";
/// The query parameter of `path::PROJECTION`: `tv`, or a seat number.
pub const AUDIENCE_PARAMETER: &str = "audience";
/// The query parameters of `path::HISTORY`; they are the fields of `HistoryRequest`.
pub const LIMIT_PARAMETER: &str = "limit";
pub const AREA_PARAMETER: &str = "area";
pub const CLASS_PARAMETER: &str = "class";
/// The query parameter of `path::TRACE`: a number or 32 hexadecimal digits.
pub const OPERATION_PARAMETER: &str = "operation";

/// The body of `path::BROKER_REQUEST`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerRequestBody {
    pub purpose: String,
}

#[derive(Clone)]
struct EngineRoutes {
    app: Arc<AppState>,
    engine: Arc<Result<EngineHandle, EngineStartError>>,
}

/// Starts the engine over the host's log: the first playable path (the lobby, the story scene and
/// the conversation) and the real queue and pipeline, over the dev adapters.
fn start_dev_engine(
    app: &AppState,
) -> Result<(EngineHandle, Option<Arc<RulesCheckKit>>), EngineStartError> {
    let config = EngineConfig::first_playable();
    let log = Arc::clone(app.log());
    let clock = Arc::new(DevClock::starting_now());
    let interpreter = Arc::new(ChaosInterpreter::default());
    let store = Arc::new(ScriptedCommitStore::default());
    let machines = machines_for(&config)?;
    let ports = story_ports(
        RulesBackend::from_environment()?,
        &machines.check,
        machines.urge.as_ref(),
        DEV_DICE_SEED,
    )?;
    let extras = Extras::from_environment(log)?;
    let adapters = Adapters {
        broker: BrokerSide::dev(
            clock.clone(),
            Arc::new(SinkBrokerAudit::new(Arc::clone(&extras.sink))),
        )?,
        rules: ports.rules,
        interpreter: interpreter.clone(),
        narrator: Arc::new(DevNarrator),
        dice: ports.dice,
        knowledge: Arc::new(DevKnowledge),
        controls: DevControls {
            clock: Some(clock.clone()),
            dice: ports.dice_control,
            interpreter: Some(interpreter),
            commit: Some(store.clone()),
        },
        clock,
    };
    let engine =
        EngineHandle::start_with_extras(config, adapters, store, extras, COMMAND_MAILBOX_CAPACITY)?;
    Ok((engine, ports.check_kit))
}

/// The seed of the dev engine's dice, so a run is reproducible.
const DEV_DICE_SEED: u64 = 20_261_102;

fn status_of(error: &EngineError) -> StatusCode {
    match error {
        EngineError::MailboxFull | EngineError::Stopped | EngineError::NotStarted { .. } => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        EngineError::NoAnswer | EngineError::Unwritable { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        EngineError::NotSupportedYet { .. } => StatusCode::NOT_IMPLEMENTED,
        _ => StatusCode::BAD_REQUEST,
    }
}

fn reply<T: Serialize>(routes: &EngineRoutes, result: Result<T, EngineError>) -> Response {
    routes.app.echo_new_lines();
    let status = result.as_ref().err().map_or(StatusCode::OK, status_of);
    (status, Json(EngineReply::from_result(result))).into_response()
}

fn engine(routes: &EngineRoutes) -> Result<&EngineHandle, EngineError> {
    match routes.engine.as_ref() {
        Ok(handle) => Ok(handle),
        Err(error) => Err(EngineError::NotStarted {
            reason: error.to_string(),
        }),
    }
}

fn body<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, EngineError> {
    serde_json::from_slice(bytes).map_err(|error| EngineError::BadRequest {
        reason: format!("the request body is not valid: {error}"),
    })
}

async fn input(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: InputRequest = body(&bytes)?;
        engine(&routes)?.submit(request).await
    }
    .await;
    reply(&routes, result)
}

async fn dump(State(routes): State<EngineRoutes>) -> Response {
    let result = async { engine(&routes)?.dump().await }.await;
    reply(&routes, result)
}

#[derive(Debug, Deserialize)]
struct ViableQuery {
    seat: Option<u64>,
}

async fn viable(State(routes): State<EngineRoutes>, Query(query): Query<ViableQuery>) -> Response {
    let result = async {
        let seat = query.seat.ok_or_else(|| EngineError::BadRequest {
            reason: format!("give ?{SEAT_PARAMETER}=N"),
        })?;
        engine(&routes)?.viable(seat).await
    }
    .await;
    reply(&routes, result)
}

async fn force(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: ForceRequest = body(&bytes)?;
        engine(&routes)?.force(request).await
    }
    .await;
    reply(&routes, result)
}

#[derive(Debug, Deserialize)]
struct ProjectionQuery {
    audience: Option<String>,
}

fn audience_of(text: Option<&str>) -> Result<AudienceSpec, EngineError> {
    let bad = || EngineError::BadRequest {
        reason: format!("give ?{AUDIENCE_PARAMETER}=tv or a seat number"),
    };
    match text.ok_or_else(bad)? {
        "tv" => Ok(AudienceSpec::Tv),
        seat => seat
            .parse::<u64>()
            .map(|seat| AudienceSpec::Seat { seat })
            .map_err(|_| bad()),
    }
}

async fn projection(
    State(routes): State<EngineRoutes>,
    Query(query): Query<ProjectionQuery>,
) -> Response {
    let result = async {
        let audience = audience_of(query.audience.as_deref())?;
        engine(&routes)?.projection(audience).await
    }
    .await;
    reply(&routes, result)
}

async fn queue_show(State(routes): State<EngineRoutes>) -> Response {
    let result = async { engine(&routes)?.queue_show().await }.await;
    reply(&routes, result)
}

async fn queue_control(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let control: QueueControl = body(&bytes)?;
        engine(&routes)?.queue_control(control).await
    }
    .await;
    reply(&routes, result)
}

async fn describe(State(routes): State<EngineRoutes>) -> Response {
    let result = async { engine(&routes)?.describe().await }.await;
    reply(&routes, result)
}

async fn health(State(routes): State<EngineRoutes>) -> Response {
    let result = async { engine(&routes)?.health().await }.await;
    reply(&routes, result)
}

async fn clock(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let control: ClockControl = body(&bytes)?;
        engine(&routes)?.clock(control).await
    }
    .await;
    reply(&routes, result)
}

async fn dice(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let control: DiceControl = body(&bytes)?;
        engine(&routes)?.dice(control).await
    }
    .await;
    reply(&routes, result)
}

async fn chaos(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: ChaosRequest = body(&bytes)?;
        engine(&routes)?.chaos(request).await
    }
    .await;
    reply(&routes, result)
}

async fn broker_queues(State(routes): State<EngineRoutes>) -> Response {
    let result = async { engine(&routes)?.broker_queues().await }.await;
    reply(&routes, result)
}

async fn broker_ledger(State(routes): State<EngineRoutes>) -> Response {
    let result = async { engine(&routes)?.broker_ledger().await }.await;
    reply(&routes, result)
}

async fn broker_request(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: BrokerRequestBody = body(&bytes)?;
        engine(&routes)?.broker_request(request.purpose).await
    }
    .await;
    reply(&routes, result)
}

/// The routes of the dev engine, with the owner thread they talk to. The guard and the body limit
/// of the host's router apply to them when this is merged into it.
pub fn router(app: Arc<AppState>) -> Router {
    let started = start_dev_engine(&app);
    let check_kit = started.as_ref().ok().and_then(|(_, kit)| kit.clone());
    let engine = Arc::new(started.map(|(handle, _)| handle));
    let rules_check = dev_route::router(Arc::clone(&app), check_kit);
    let routes = EngineRoutes { app, engine };
    Router::new()
        .route(path::INPUT, post(input))
        .route(path::DUMP, get(dump))
        .route(path::VIABLE, get(viable))
        .route(path::FORCE, post(force))
        .route(path::PROJECTION, get(projection))
        .route(path::QUEUE, get(queue_show).post(queue_control))
        .route(path::DESCRIBE, get(describe))
        .route(path::BROKER_QUEUES, get(broker_queues))
        .route(path::BROKER_LEDGER, get(broker_ledger))
        .route(path::BROKER_REQUEST, post(broker_request))
        .route(path::HEALTH, get(health))
        .route(path::CLOCK, post(clock))
        .route(path::DICE, post(dice))
        .route(path::CHAOS, post(chaos))
        .route(path::HISTORY, get(observe::history))
        .route(path::TRACE, get(observe::trace))
        .route(path::REPLAY, post(observe::replay))
        .route(
            path::CONFIG,
            get(observe::config_show).post(observe::config_set),
        )
        .route(path::WIRE_VIEW, get(play::wire_view))
        .route(path::WIRE_INTENT, post(play::wire_intent))
        .with_state(routes)
        .merge(rules_check)
}
