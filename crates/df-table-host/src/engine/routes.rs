//! The `/dev/engine/*` routes: the owner actor's developer API.
//!
//! They sit behind the same guard as every route of the host (loopback host names, JSON for any
//! change, no foreign origin), because they are merged into the one router before its layers.
//! A read is a GET and a change is a POST of JSON; every answer is an `EngineReply` (the host's
//! one envelope), `done` with the value or `rejected` with a typed `EngineError`, whose `status()`
//! is the answer's status. A body is read with `dev_http::parse_body` and a query with `query`, so
//! a malformed one is a typed `invalid_body` or `invalid_query` refusal and never a bare 400; a
//! body over the host's limit or one that broke off is a typed `unreadable_body`, not the
//! extractor's own answer. The routes hold no state of their own beyond the handle to the owner
//! thread, which they start with the router.
//!
//! What they record: the owner audits every call it answers, and the routes write one
//! `ENGINE refused verb=<verb> error=<key> duration_ms=<n>` line for every refusal, whoever
//! refused (the body, the query, an owner that never started, the owner itself). Starting the
//! router writes `ENGINE started` or `ENGINE start_failed`. The host's middleware echoes the lines
//! of a request.

mod observe;
mod play;
mod reach;

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::time::Instant;

use crate::adapters::dev_route;
use crate::adapters::{RulesBackend, RulesCheckKit, story_ports};
use crate::dev_http::{parse_body, reply};
use crate::locate::text_runtime;
use crate::state::AppState;

use super::adapters::{
    Adapters, BrokerSide, ChaosInterpreter, DevClock, DevControls, DevKnowledge, DevNarrator,
};
use super::broker_fields::SinkBrokerAudit;
use super::commit::ScriptedCommitStore;
use super::extras::Extras;
use super::handle::EngineHandle;
use super::jev::JevInterpreter;
use super::limits::COMMAND_MAILBOX_CAPACITY;
use super::report::{EngineError, QueryError, UnreadableBodyCause};
use super::request::{
    AudienceQuery, BrokerRequest, ChaosRequest, ClockControl, DiceControl, DumpQuery, ForceRequest,
    InputRequest, QueueControl, ViableQuery,
};
use super::setup::{EngineConfig, EngineStartError, machines_for};

/// The addresses of the dev engine API.
pub mod path {
    pub const INPUT: &str = "/dev/engine/input";
    /// GET; `?machine=table` or `?machine=<name>` dumps that part alone.
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
    /// GET reads the faults in force; POST puts one in force.
    pub const CHAOS: &str = "/dev/engine/chaos";
    pub const RULES_CHECK: &str = "/dev/engine/rules/check";
    pub const RULES_ROLL: &str = "/dev/engine/rules/roll";
    pub const HISTORY: &str = "/dev/engine/history";
    pub const TRACE: &str = "/dev/engine/trace";
    pub const REPLAY: &str = "/dev/engine/replay";
    pub const CONFIG: &str = "/dev/engine/config";
    /// A TV's or a phone's `MachineView` as the dev report of it (GET `?audience=tv|N`).
    pub const WIRE_VIEW: &str = "/dev/engine/wire/view";
    /// A `MachineIntent` for a seat, answered with the dev report of its receipt (POST).
    pub const WIRE_INTENT: &str = "/dev/engine/wire/intent";
    /// The route of one inspect report (GET `?audience=operator|host|seat:N`); `inspect_path`
    /// fills in the report's name.
    pub const INSPECT: &str = "/dev/engine/inspect/{report}";
    /// The level of the host's event sinks: GET reads it, POST `{"level":"always|verbose"}` sets
    /// it.
    pub const EVENTS_LEVEL: &str = "/dev/engine/events/level";

    /// The address of one inspect report.
    pub fn inspect_path(report: &str) -> String {
        INSPECT.replace("{report}", report)
    }
}

/// The query parameter of `path::VIABLE`: the field of `ViableQuery`.
pub const SEAT_PARAMETER: &str = "seat";
/// The query parameter of `path::PROJECTION`, `path::WIRE_VIEW` and `path::INSPECT`: `tv` or a
/// seat number, and for an inspect report `operator`, `host` or `seat:N`.
pub const AUDIENCE_PARAMETER: &str = "audience";
/// The query parameters of `path::HISTORY`; they are the fields of `HistoryRequest`.
pub const LIMIT_PARAMETER: &str = "limit";
pub const AREA_PARAMETER: &str = "area";
pub const CLASS_PARAMETER: &str = "class";
/// The query parameter of `path::TRACE`: a number or 32 hexadecimal digits.
pub const OPERATION_PARAMETER: &str = "operation";
/// The query parameter of `path::DUMP`: `table` or a machine's name.
pub const MACHINE_PARAMETER: &str = "machine";

#[derive(Clone)]
struct EngineRoutes {
    app: Arc<AppState>,
    engine: Arc<Result<EngineHandle, EngineStartError>>,
}

/// A started engine, with what its start line says about it.
struct Started {
    engine: EngineHandle,
    check_kit: Option<Arc<RulesCheckKit>>,
    machines: usize,
    spooled: bool,
}

/// Starts the engine over the host's log: the first playable path (the lobby, the story scene and
/// the conversation) and the real queue and pipeline, over the dev adapters.
fn start_dev_engine(app: &Arc<AppState>) -> Result<Started, EngineStartError> {
    let config = EngineConfig::first_playable();
    let log = Arc::clone(app.log());
    let clock = Arc::new(DevClock::starting_now());
    let interpreter = Arc::new(ChaosInterpreter::new(Arc::new(JevInterpreter::new(
        text_runtime(app),
    ))));
    let store = Arc::new(ScriptedCommitStore::default());
    let machines = machines_for(&config)?;
    let ports = story_ports(
        RulesBackend::from_environment()?,
        &machines.check,
        machines.urge.as_ref(),
        DEV_DICE_SEED,
    )?;
    let extras = Extras::from_environment(log)?;
    let spooled = extras.spooled;
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
    Ok(Started {
        engine,
        check_kit: ports.check_kit,
        machines: machines.registry.report().machines.len(),
        spooled,
    })
}

/// The seed of the dev engine's dice, so a run is reproducible.
const DEV_DICE_SEED: u64 = 20_261_102;

/// The line that says how the engine started, or why it did not.
fn start_line(started: &Result<Started, EngineStartError>) -> String {
    match started {
        Ok(started) => format!(
            "ENGINE started machines={} spool={} level={}",
            started.machines,
            if started.spooled { "on" } else { "off" },
            started.engine.event_level().key()
        ),
        Err(error) => format!("ENGINE start_failed cause={}", error.cause().key()),
    }
}

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

/// The JSON body of a POST, or the typed refusal of one that could not be read or does not read.
fn body<T: DeserializeOwned>(bytes: Result<Bytes, BytesRejection>) -> Result<T, EngineError> {
    let bytes = bytes.map_err(|rejection| EngineError::UnreadableBody {
        cause: if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            UnreadableBodyCause::TooLarge
        } else {
            UnreadableBodyCause::Interrupted
        },
    })?;
    Ok(parse_body(&bytes)?)
}

/// Answers a call of `verb`, writing its refusal line when it was refused.
fn answer<T: Serialize>(
    routes: &EngineRoutes,
    verb: &str,
    started: Instant,
    result: Result<T, EngineError>,
) -> Response {
    if let Err(error) = &result {
        routes.app.record_line(format!(
            "ENGINE refused verb={verb} error={} duration_ms={}",
            error.key(),
            started.elapsed().as_millis()
        ));
    }
    reply(result)
}

async fn input(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let request: InputRequest = body(bytes)?;
        engine(&routes)?.submit(request).await
    }
    .await;
    answer(&routes, "input", started, result)
}

/// `dump`, or with `?machine=` the table or one machine alone.
async fn dump(
    State(routes): State<EngineRoutes>,
    dump_query: Result<Query<DumpQuery>, QueryRejection>,
) -> Response {
    let started = Instant::now();
    let named = match query(dump_query) {
        Ok(DumpQuery { machine }) => machine,
        Err(error) => return answer::<()>(&routes, "dump", started, Err(error)),
    };
    match named {
        None => {
            let result = async { engine(&routes)?.dump().await }.await;
            answer(&routes, "dump", started, result)
        }
        Some(target) => {
            let result = async { engine(&routes)?.dump_named(target).await }.await;
            answer(&routes, "dump", started, result)
        }
    }
}

async fn viable(
    State(routes): State<EngineRoutes>,
    viable_query: Result<Query<ViableQuery>, QueryRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let ViableQuery { seat } = query(viable_query)?;
        engine(&routes)?.viable(seat).await
    }
    .await;
    answer(&routes, "viable", started, result)
}

async fn force(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let request: ForceRequest = body(bytes)?;
        engine(&routes)?.force(request).await
    }
    .await;
    answer(&routes, "force", started, result)
}

async fn projection(
    State(routes): State<EngineRoutes>,
    audience_query: Result<Query<AudienceQuery>, QueryRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let AudienceQuery { audience } = query(audience_query)?;
        engine(&routes)?.projection(audience.0).await
    }
    .await;
    answer(&routes, "projection", started, result)
}

async fn queue_show(State(routes): State<EngineRoutes>) -> Response {
    let started = Instant::now();
    let result = async { engine(&routes)?.queue_show().await }.await;
    answer(&routes, "queue", started, result)
}

async fn queue_control(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let control: QueueControl = body(bytes)?;
        engine(&routes)?.queue_control(control).await
    }
    .await;
    answer(&routes, "queue", started, result)
}

async fn describe(State(routes): State<EngineRoutes>) -> Response {
    let started = Instant::now();
    let result = async { engine(&routes)?.describe().await }.await;
    answer(&routes, "describe", started, result)
}

async fn health(State(routes): State<EngineRoutes>) -> Response {
    let started = Instant::now();
    let result = async { engine(&routes)?.health().await }.await;
    answer(&routes, "health", started, result)
}

async fn clock(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let control: ClockControl = body(bytes)?;
        engine(&routes)?.clock(control).await
    }
    .await;
    answer(&routes, "clock", started, result)
}

async fn dice(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let control: DiceControl = body(bytes)?;
        engine(&routes)?.dice(control).await
    }
    .await;
    answer(&routes, "dice", started, result)
}

async fn chaos_show(State(routes): State<EngineRoutes>) -> Response {
    let started = Instant::now();
    let result = async { engine(&routes)?.chaos_show().await }.await;
    answer(&routes, "chaos", started, result)
}

async fn chaos(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let request: ChaosRequest = body(bytes)?;
        engine(&routes)?.chaos(request).await
    }
    .await;
    answer(&routes, "chaos", started, result)
}

async fn broker_queues(State(routes): State<EngineRoutes>) -> Response {
    let started = Instant::now();
    let result = async { engine(&routes)?.broker_queues().await }.await;
    answer(&routes, "broker_queues", started, result)
}

async fn broker_ledger(State(routes): State<EngineRoutes>) -> Response {
    let started = Instant::now();
    let result = async { engine(&routes)?.broker_ledger().await }.await;
    answer(&routes, "broker_ledger", started, result)
}

async fn broker_request(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let request: BrokerRequest = body(bytes)?;
        engine(&routes)?.broker_request(request.purpose).await
    }
    .await;
    answer(&routes, "broker_request", started, result)
}

/// The routes of the dev engine, with the owner thread they talk to. The guard and the body limit
/// of the host's router apply to them when this is merged into it.
pub fn router(app: Arc<AppState>) -> Router {
    let started = start_dev_engine(&app);
    router_over(app, started)
}

/// The routes over an engine that started or did not, writing the line that says which.
fn router_over(app: Arc<AppState>, started: Result<Started, EngineStartError>) -> Router {
    app.record_line_and_echo(start_line(&started));
    let check_kit = started
        .as_ref()
        .ok()
        .and_then(|started| started.check_kit.clone());
    let engine = Arc::new(started.map(|started| started.engine));
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
        .route(path::CHAOS, get(chaos_show).post(chaos))
        .route(path::HISTORY, get(observe::history))
        .route(path::TRACE, get(observe::trace))
        .route(path::REPLAY, post(observe::replay))
        .route(
            path::CONFIG,
            get(observe::config_show).post(observe::config_set),
        )
        .route(path::WIRE_VIEW, get(play::wire_view))
        .route(path::WIRE_INTENT, post(play::wire_intent))
        .route(path::INSPECT, get(reach::inspect))
        .route(
            path::EVENTS_LEVEL,
            get(reach::events_level).post(reach::events_level_set),
        )
        .with_state(routes)
        .merge(rules_check)
}

/// The routes over an engine that did not start, for a test of the refusals that follow.
#[cfg(test)]
pub(super) fn router_not_started(app: Arc<AppState>, error: EngineStartError) -> Router {
    router_over(app, Err(error))
}
