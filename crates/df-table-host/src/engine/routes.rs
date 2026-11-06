//! The `/dev/engine/*` routes: the owner actor's developer API.
//!
//! They sit behind the same guard as every route of the host (loopback host names, JSON for any
//! change, no foreign origin), because they are merged into the one router before its layers.
//! A read is a GET and a change is a POST of JSON; every answer is an `EngineReply` (the host's
//! one envelope), `done` with the value or `rejected` with a typed `EngineError`, whose `status()`
//! is the answer's status. A body is read with `dev_http::parse_body` and a query with `query`, so
//! a malformed one is a typed `invalid_body` or `invalid_query` refusal and never a bare 400. The
//! routes hold no state of their own beyond the handle to the owner thread, which they start with
//! the router, and they record nothing: the owner audits every call, and the host's middleware
//! echoes its lines.

mod observe;
mod play;

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};

use crate::adapters::dev_route;
use crate::adapters::{RulesBackend, RulesCheckKit, story_ports};
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;

use super::adapters::{
    Adapters, BrokerSide, ChaosInterpreter, DevClock, DevControls, DevKnowledge, DevNarrator,
};
use super::broker_fields::SinkBrokerAudit;
use super::commit::ScriptedCommitStore;
use super::extras::Extras;
use super::handle::EngineHandle;
use super::limits::COMMAND_MAILBOX_CAPACITY;
use super::report::{EngineError, QueryError};
use super::request::{
    AudienceQuery, BrokerRequest, ChaosRequest, ClockControl, DiceControl, ForceRequest,
    InputRequest, QueueControl, ViableQuery,
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

/// The query parameter of `path::VIABLE`: the field of `ViableQuery`.
pub const SEAT_PARAMETER: &str = "seat";
/// The query parameter of `path::PROJECTION` and `path::WIRE_VIEW`: `tv`, or a seat number.
pub const AUDIENCE_PARAMETER: &str = "audience";
/// The query parameters of `path::HISTORY`; they are the fields of `HistoryRequest`.
pub const LIMIT_PARAMETER: &str = "limit";
pub const AREA_PARAMETER: &str = "area";
pub const CLASS_PARAMETER: &str = "class";
/// The query parameter of `path::TRACE`: a number or 32 hexadecimal digits.
pub const OPERATION_PARAMETER: &str = "operation";

#[derive(Clone)]
struct EngineRoutes {
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

/// The owner, or why the routes have none.
fn engine(routes: &EngineRoutes) -> Result<&EngineHandle, EngineError> {
    routes
        .engine
        .as_ref()
        .as_ref()
        .map_err(|error| EngineError::NotStarted {
            cause: error.cause(),
            detail: error.to_string(),
        })
}

/// The query of a GET, or the typed refusal of one that does not read (a missing or an unknown
/// parameter, a value of the wrong kind).
fn query<T>(query: Result<Query<T>, QueryRejection>) -> Result<T, EngineError> {
    query.map(|Query(query)| query).map_err(|rejection| {
        EngineError::from(QueryError {
            detail: rejection.body_text(),
        })
    })
}

async fn input(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: InputRequest = parse_body(&bytes)?;
        engine(&routes)?.submit(request).await
    }
    .await;
    reply(result)
}

async fn dump(State(routes): State<EngineRoutes>) -> Response {
    reply(async { engine(&routes)?.dump().await }.await)
}

async fn viable(
    State(routes): State<EngineRoutes>,
    viable_query: Result<Query<ViableQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let ViableQuery { seat } = query(viable_query)?;
        engine(&routes)?.viable(seat).await
    }
    .await;
    reply(result)
}

async fn force(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: ForceRequest = parse_body(&bytes)?;
        engine(&routes)?.force(request).await
    }
    .await;
    reply(result)
}

async fn projection(
    State(routes): State<EngineRoutes>,
    audience_query: Result<Query<AudienceQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let AudienceQuery { audience } = query(audience_query)?;
        engine(&routes)?.projection(audience.0).await
    }
    .await;
    reply(result)
}

async fn queue_show(State(routes): State<EngineRoutes>) -> Response {
    reply(async { engine(&routes)?.queue_show().await }.await)
}

async fn queue_control(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let control: QueueControl = parse_body(&bytes)?;
        engine(&routes)?.queue_control(control).await
    }
    .await;
    reply(result)
}

async fn describe(State(routes): State<EngineRoutes>) -> Response {
    reply(async { engine(&routes)?.describe().await }.await)
}

async fn health(State(routes): State<EngineRoutes>) -> Response {
    reply(async { engine(&routes)?.health().await }.await)
}

async fn clock(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let control: ClockControl = parse_body(&bytes)?;
        engine(&routes)?.clock(control).await
    }
    .await;
    reply(result)
}

async fn dice(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let control: DiceControl = parse_body(&bytes)?;
        engine(&routes)?.dice(control).await
    }
    .await;
    reply(result)
}

async fn chaos(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: ChaosRequest = parse_body(&bytes)?;
        engine(&routes)?.chaos(request).await
    }
    .await;
    reply(result)
}

async fn broker_queues(State(routes): State<EngineRoutes>) -> Response {
    reply(async { engine(&routes)?.broker_queues().await }.await)
}

async fn broker_ledger(State(routes): State<EngineRoutes>) -> Response {
    reply(async { engine(&routes)?.broker_ledger().await }.await)
}

async fn broker_request(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: BrokerRequest = parse_body(&bytes)?;
        engine(&routes)?.broker_request(request.purpose).await
    }
    .await;
    reply(result)
}

/// The routes of the dev engine, with the owner thread they talk to. The guard and the body limit
/// of the host's router apply to them when this is merged into it.
pub fn router(app: Arc<AppState>) -> Router {
    let started = start_dev_engine(&app);
    let check_kit = started.as_ref().ok().and_then(|(_, kit)| kit.clone());
    let engine = Arc::new(started.map(|(handle, _)| handle));
    let rules_check = dev_route::router(app, check_kit);
    let routes = EngineRoutes { engine };
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
