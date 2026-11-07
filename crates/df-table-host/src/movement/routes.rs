//! The movement routes: each is one model call against a closed schema. `POST /dev/move/interpret`
//! has Jev read one line a player said and answers a `DevReply` of an `Interpreted` reading, or a
//! typed [`InterpretRouteError`] with one `MOVE refused verb=interpret error=<key>` audit line. A
//! reading is one `MOVE interpreted reading=<key> duration_ms=..` line. `POST /dev/move/design`
//! designs the scene a move arrives in, residents included, and answers a `DevReply` of a
//! `SceneDesign` or a [`DesignRouteError`], with `MOVE designed ..` and `MOVE refused verb=design`
//! lines. A line never reaches an audit line: it is data for the model.
//!
//! The question runs under the persona text scope and the default persona model, as the persona
//! questions do (`persona::generate`). The call runs on a task of its own that the handler waits
//! for, so a client that leaves cannot lose an answer already paid for.

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use serde::{Deserialize, Serialize};

use super::design::{
    DESIGN_INSTRUCTION, DesignError, check_move, design_facts, design_schema, parse_design,
};
use super::interpret::{
    InterpretError, JEV_INSTRUCTION, jev_facts, jev_schema, parse_reading, require_text,
};
use super::types::{Interpreted, MoveIntent, PartyMember, SceneDesign};
use crate::blocking::run_owned;
use crate::dev_http::{BodyError, DevReply, RouteError, parse_body, reply};
use crate::persona::generate::{operation_key, text_request};
use crate::state::AppState;
use crate::text::{
    ModelCause, ServiceReason, TextAskError, TextRoute, TextRuntime, TextServiceCause,
};

/// The addresses of the movement dev API.
pub mod path {
    /// Reads one line as a move (POST an `InterpretRequest`). A dev tool; it asks the text service.
    pub const MOVE_INTERPRET: &str = "/dev/move/interpret";
    /// Designs the scene a move arrives in (POST a `DesignRequest`). A dev tool; it asks the text
    /// service.
    pub const MOVE_DESIGN: &str = "/dev/move/design";
}

/// The name of the answer object of a reading.
const OUTPUT_NAME: &str = "movement_reading";
/// The name of the answer object of a design.
const DESIGN_OUTPUT_NAME: &str = "scene_design";

/// The body of `POST /dev/move/interpret`: the line, the speaker's persona id when a character
/// said it, the party that may move, and the model to ask instead of the persona default. The
/// line is not derived for debugging, so it has no `Debug`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpretRequest {
    pub line: String,
    #[serde(default)]
    pub speaker: Option<String>,
    /// What the scene is and who has been talking; Jev reads "we" and "us" against it.
    #[serde(default)]
    pub context: Option<String>,
    pub party: Vec<PartyMember>,
    #[serde(default)]
    pub model: Option<String>,
}

/// The answer of the movement route: its reading, or why it was refused.
pub type InterpretReply = DevReply<Interpreted, InterpretRouteError>;

/// Why a reading was refused. `key()` is the serde tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum InterpretRouteError {
    /// The request body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The line holds no words.
    EmptyLine,
    /// The text service is not set up; `missing` says what to set.
    TextServiceMissing { missing: String },
    /// The model name is not usable.
    InvalidModel { cause: ModelCause },
    /// The text service gave no usable answer.
    TextService {
        cause: TextServiceCause,
        reason: Option<ServiceReason>,
    },
    /// The reader refused the model's answer (`replayed` when it was a replay).
    AnswerRefused {
        replayed: bool,
        cause: InterpretError,
    },
    /// The text-service operation key is not valid.
    InvalidOperationKey,
    /// The blocking task stopped without an answer.
    Stopped,
}

impl InterpretRouteError {
    /// The error's key for the audit line: its serde tag.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::EmptyLine => "empty_line",
            Self::TextServiceMissing { .. } => "text_service_missing",
            Self::InvalidModel { .. } => "invalid_model",
            Self::TextService { .. } => "text_service",
            Self::AnswerRefused { .. } => "answer_refused",
            Self::InvalidOperationKey => "invalid_operation_key",
            Self::Stopped => "stopped",
        }
    }
}

impl std::fmt::Display for InterpretRouteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBody(_) => formatter.write_str("the request body is not the JSON to read"),
            Self::EmptyLine => formatter.write_str("the line is empty"),
            Self::TextServiceMissing { missing } => {
                write!(formatter, "the text service is not set up: {missing}")
            }
            Self::InvalidModel { .. } => formatter.write_str("the model name is not usable"),
            Self::TextService { .. } => formatter.write_str("the text service gave no answer"),
            Self::AnswerRefused { cause, .. } => {
                write!(formatter, "the model's answer was refused: {cause}")
            }
            Self::InvalidOperationKey => formatter.write_str("the operation key is not valid"),
            Self::Stopped => formatter.write_str("the reading stopped without an answer"),
        }
    }
}

impl std::error::Error for InterpretRouteError {}

impl RouteError for InterpretRouteError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::EmptyLine | Self::InvalidModel { .. } => {
                StatusCode::BAD_REQUEST
            }
            Self::TextServiceMissing { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TextService { .. } | Self::AnswerRefused { .. } => StatusCode::BAD_GATEWAY,
            Self::InvalidOperationKey | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<TextAskError<InterpretError>> for InterpretRouteError {
    fn from(error: TextAskError<InterpretError>) -> Self {
        match error {
            TextAskError::Service { cause, reason } => Self::TextService { cause, reason },
            TextAskError::AnswerRefused(cause) => Self::AnswerRefused {
                replayed: false,
                cause,
            },
            TextAskError::ReplayRefused(cause) => Self::AnswerRefused {
                replayed: true,
                cause,
            },
        }
    }
}

/// What the movement route works with: the host and the text surface the question goes to.
struct MoveRoutes {
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
}

/// Answers `result`, recording a refusal first as `MOVE refused verb=<verb> error=<key>`.
fn answer<T: Serialize, E: RouteError + Serialize>(
    app: &AppState,
    verb: &str,
    key_of: fn(&E) -> &'static str,
    result: Result<T, E>,
) -> Response {
    if let Err(error) = &result {
        app.record_line(format!("MOVE refused verb={verb} error={}", key_of(error)));
    }
    reply(result)
}

/// Why no route could be made for a question: the text service is not set up, or the model name is
/// not usable. Both route errors carry it.
enum RouteSetup {
    TextServiceMissing { missing: String },
    InvalidModel { cause: ModelCause },
}

impl From<RouteSetup> for InterpretRouteError {
    fn from(setup: RouteSetup) -> Self {
        match setup {
            RouteSetup::TextServiceMissing { missing } => Self::TextServiceMissing { missing },
            RouteSetup::InvalidModel { cause } => Self::InvalidModel { cause },
        }
    }
}

impl From<RouteSetup> for DesignRouteError {
    fn from(setup: RouteSetup) -> Self {
        match setup {
            RouteSetup::TextServiceMissing { missing } => Self::TextServiceMissing { missing },
            RouteSetup::InvalidModel { cause } => Self::InvalidModel { cause },
        }
    }
}

/// The route the question is asked of: the default persona model, or the one the request names.
fn route_for(text: &TextRuntime, model: Option<&str>) -> Result<Arc<TextRoute>, RouteSetup> {
    if let Some(missing) = text.missing() {
        return Err(RouteSetup::TextServiceMissing {
            missing: missing.to_owned(),
        });
    }
    text.persona_route(model)
        .map_err(|cause| RouteSetup::InvalidModel { cause })
}

/// The audit word of a reading.
fn reading_key(reading: &Interpreted) -> &'static str {
    match reading {
        Interpreted::Move { .. } => "move",
        Interpreted::NotAMove => "not_a_move",
        Interpreted::NeedsClarification => "needs_clarification",
    }
}

/// Asks the model for the reading of `request.line` and checks its answer.
async fn interpret_line(
    routes: &MoveRoutes,
    request: &InterpretRequest,
) -> Result<Interpreted, InterpretRouteError> {
    require_text(&request.line).map_err(|_: InterpretError| InterpretRouteError::EmptyLine)?;
    let route = route_for(&routes.text, request.model.as_deref())?;
    let key = operation_key("interpret").map_err(|_| InterpretRouteError::InvalidOperationKey)?;
    let facts = jev_facts(
        &request.line,
        request.speaker.as_deref(),
        request.context.as_deref(),
        &request.party,
    );
    let question = text_request(
        route.profile(),
        JEV_INSTRUCTION,
        facts,
        OUTPUT_NAME,
        jev_schema(),
    );
    let started = Instant::now();
    let reading = route
        .ask(&key, &question, |answer| {
            parse_reading(answer, &request.line, &request.party)
        })
        .await?
        .value;
    routes.app.record_line(format!(
        "MOVE interpreted reading={} duration_ms={}",
        reading_key(&reading),
        started.elapsed().as_millis()
    ));
    Ok(reading)
}

async fn interpret(State(routes): State<Arc<MoveRoutes>>, bytes: Bytes) -> Response {
    let request: InterpretRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return answer::<Interpreted, _>(
                &routes.app,
                "interpret",
                InterpretRouteError::key,
                Err(InterpretRouteError::InvalidBody(error)),
            );
        }
    };
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move { interpret_line(&owned, &request).await },
        InterpretRouteError::Stopped,
    )
    .await;
    answer(&routes.app, "interpret", InterpretRouteError::key, result)
}

/// The body of `POST /dev/move/design`: the move, the party that may move, what the scene was and
/// who has been talking, and the model to ask instead of the persona default. The move holds the
/// player's line, so it has no `Debug`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignRequest {
    pub intent: MoveIntent,
    pub party: Vec<PartyMember>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// The answer of the design route: the scene design, or why it was refused.
pub type DesignReply = DevReply<SceneDesign, DesignRouteError>;

/// Why a design was refused. `key()` is the serde tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum DesignRouteError {
    /// The request body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The move names nobody, a non-party mover or no destination.
    BadMove { cause: DesignError },
    /// The text service is not set up; `missing` says what to set.
    TextServiceMissing { missing: String },
    /// The model name is not usable.
    InvalidModel { cause: ModelCause },
    /// The text service gave no usable answer.
    TextService {
        cause: TextServiceCause,
        reason: Option<ServiceReason>,
    },
    /// The design check refused the model's answer (`replayed` when it was a replay).
    AnswerRefused { replayed: bool, cause: DesignError },
    /// The text-service operation key is not valid.
    InvalidOperationKey,
    /// The blocking task stopped without an answer.
    Stopped,
}

impl DesignRouteError {
    /// The error's key for the audit line: its serde tag.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::BadMove { .. } => "bad_move",
            Self::TextServiceMissing { .. } => "text_service_missing",
            Self::InvalidModel { .. } => "invalid_model",
            Self::TextService { .. } => "text_service",
            Self::AnswerRefused { .. } => "answer_refused",
            Self::InvalidOperationKey => "invalid_operation_key",
            Self::Stopped => "stopped",
        }
    }
}

impl std::fmt::Display for DesignRouteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBody(_) => formatter.write_str("the request body is not the JSON to read"),
            Self::BadMove { cause } => write!(formatter, "the move cannot be designed: {cause}"),
            Self::TextServiceMissing { missing } => {
                write!(formatter, "the text service is not set up: {missing}")
            }
            Self::InvalidModel { .. } => formatter.write_str("the model name is not usable"),
            Self::TextService { .. } => formatter.write_str("the text service gave no answer"),
            Self::AnswerRefused { cause, .. } => {
                write!(formatter, "the model's design was refused: {cause}")
            }
            Self::InvalidOperationKey => formatter.write_str("the operation key is not valid"),
            Self::Stopped => formatter.write_str("the design stopped without an answer"),
        }
    }
}

impl std::error::Error for DesignRouteError {}

impl RouteError for DesignRouteError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::BadMove { .. } | Self::InvalidModel { .. } => {
                StatusCode::BAD_REQUEST
            }
            Self::TextServiceMissing { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TextService { .. } | Self::AnswerRefused { .. } => StatusCode::BAD_GATEWAY,
            Self::InvalidOperationKey | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<TextAskError<DesignError>> for DesignRouteError {
    fn from(error: TextAskError<DesignError>) -> Self {
        match error {
            TextAskError::Service { cause, reason } => Self::TextService { cause, reason },
            TextAskError::AnswerRefused(cause) => Self::AnswerRefused {
                replayed: false,
                cause,
            },
            TextAskError::ReplayRefused(cause) => Self::AnswerRefused {
                replayed: true,
                cause,
            },
        }
    }
}

/// Asks the model to design the scene `request.intent` arrives in and checks its answer.
async fn design_scene(
    routes: &MoveRoutes,
    request: &DesignRequest,
) -> Result<SceneDesign, DesignRouteError> {
    check_move(&request.intent, &request.party)
        .map_err(|cause| DesignRouteError::BadMove { cause })?;
    let route = route_for(&routes.text, request.model.as_deref())?;
    let key = operation_key("design").map_err(|_| DesignRouteError::InvalidOperationKey)?;
    let facts = design_facts(&request.intent, &request.party, request.context.as_deref());
    let question = text_request(
        route.profile(),
        DESIGN_INSTRUCTION,
        facts,
        DESIGN_OUTPUT_NAME,
        design_schema(),
    );
    let started = Instant::now();
    let design = route
        .ask(&key, &question, |answer| {
            parse_design(answer, &request.intent, &request.party)
        })
        .await?
        .value;
    routes.app.record_line(format!(
        "MOVE designed placements={} residents={} duration_ms={}",
        design.placements.len(),
        design.residents.len(),
        started.elapsed().as_millis()
    ));
    Ok(design)
}

async fn design(State(routes): State<Arc<MoveRoutes>>, bytes: Bytes) -> Response {
    let request: DesignRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return answer::<SceneDesign, _>(
                &routes.app,
                "design",
                DesignRouteError::key,
                Err(DesignRouteError::InvalidBody(error)),
            );
        }
    };
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move { design_scene(&owned, &request).await },
        DesignRouteError::Stopped,
    )
    .await;
    answer(&routes.app, "design", DesignRouteError::key, result)
}

/// The movement routes, asking the text service of `text`.
// TODO(DEV-12): route tests on a fake text service: a reading, a design with a resident, a refused
// answer, an empty line, a bad move, a missing text service and the audit lines.
pub fn router_with(app: Arc<AppState>, text: Arc<TextRuntime>) -> Router {
    Router::new()
        .route(path::MOVE_INTERPRET, post(interpret))
        .route(path::MOVE_DESIGN, post(design))
        .with_state(Arc::new(MoveRoutes { app, text }))
}
