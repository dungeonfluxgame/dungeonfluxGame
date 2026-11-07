//! The movement route: one line a player said is read by Jev, one model call against the closed
//! schema. `POST /dev/move/interpret` answers a `DevReply` of an `Interpreted` reading, or a typed
//! [`InterpretRouteError`] with one `MOVE refused verb=interpret error=<key>` audit line. A reading
//! is one `MOVE interpreted reading=<key> duration_ms=..` line. A line never reaches an audit
//! line: it is data for the model.
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

use super::interpret::{
    InterpretError, JEV_INSTRUCTION, jev_facts, jev_schema, parse_reading, require_text,
};
use super::types::{Interpreted, PartyMember};
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
}

/// The name of the answer object of a reading.
const OUTPUT_NAME: &str = "movement_reading";

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

/// Answers `result`, recording a refusal first as `MOVE refused verb=interpret error=<key>`.
fn answer(app: &AppState, result: Result<Interpreted, InterpretRouteError>) -> Response {
    if let Err(error) = &result {
        app.record_line(format!(
            "MOVE refused verb=interpret error={}",
            error.key()
        ));
    }
    reply(result)
}

/// The route the question is asked of: the default persona model, or the one the request names.
fn route_for(text: &TextRuntime, model: Option<&str>) -> Result<Arc<TextRoute>, InterpretRouteError> {
    if let Some(missing) = text.missing() {
        return Err(InterpretRouteError::TextServiceMissing {
            missing: missing.to_owned(),
        });
    }
    text.persona_route(model)
        .map_err(|cause| InterpretRouteError::InvalidModel { cause })
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
            return answer(&routes.app, Err(InterpretRouteError::InvalidBody(error)));
        }
    };
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move { interpret_line(&owned, &request).await },
        InterpretRouteError::Stopped,
    )
    .await;
    answer(&routes.app, result)
}

/// The movement routes, asking the text service of `text`.
// TODO(DEV-12): route tests on a fake text service: a reading, a refused answer, an empty line, a
// missing text service and the audit lines.
pub fn router_with(app: Arc<AppState>, text: Arc<TextRuntime>) -> Router {
    Router::new()
        .route(path::MOVE_INTERPRET, post(interpret))
        .with_state(Arc::new(MoveRoutes { app, text }))
}
