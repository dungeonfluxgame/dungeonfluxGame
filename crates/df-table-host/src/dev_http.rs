//! The plumbing every dev route module shares: the reply envelope, answering with JSON, and reading
//! a JSON body or a query into the one body error (`BodyError`) and the one query error
//! (`QueryError`) every route error wraps. Each route module keeps its own typed error and status
//! rules; this module owns only the steps that are the same for all.
//!
//! Audit lines echo to the terminal once per request, in the `guard_requests` middleware
//! (`server.rs`): a route records with `AppState::record_line` and does not echo. Work that keeps
//! recording after its request has been answered (a background job) uses
//! `AppState::record_line_and_echo`.

use axum::Json;
use axum::extract::Query;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
/// The answer of every dev route. The page owns the type, so the envelope is one.
pub use df_playground::dev_reply::DevReply;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A dev route module's typed error: the `error` of a `DevReply::Rejected`.
///
/// It is an enum tagged `"error"` (`#[serde(tag = "error", rename_all = "snake_case")]`) whose
/// `Display` is a lowercase sentence, and it answers the HTTP status of its refusal.
pub trait RouteError: std::error::Error + Send + Sync + 'static {
    /// The status a refusal with this error is answered with.
    fn status(&self) -> StatusCode;
}

/// `body` as JSON with `status`.
pub(crate) fn json(status: StatusCode, body: &impl Serialize) -> Response {
    (status, Json(body)).into_response()
}

/// The answer to a dev route call: `200` with `Done`, or the error's status with `Rejected`.
pub(crate) fn reply<T: Serialize, E: RouteError + Serialize>(result: Result<T, E>) -> Response {
    let status = result
        .as_ref()
        .err()
        .map_or(StatusCode::OK, RouteError::status);
    json(status, &DevReply::from_result(result))
}

/// What kind of mistake made a request body unreadable: the one cause of every body reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyCause {
    /// The bytes are not JSON.
    Syntax,
    /// The JSON is not the shape the route reads (a missing or unknown field, a wrong type).
    Data,
    /// The body ended in the middle of a value (an empty body included), or broke off before it
    /// was all received.
    Truncated,
    /// The body is larger than the route reads (the listener's limit or the route's own).
    TooLarge,
}

impl BodyCause {
    pub const ALL: [Self; 4] = [Self::Syntax, Self::Data, Self::Truncated, Self::TooLarge];

    pub fn key(self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::Data => "data",
            Self::Truncated => "truncated",
            Self::TooLarge => "too_large",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

/// A request body (or stored file) that is not the JSON the reader reads. It is a wire value, so a
/// route error can carry it whole (`InvalidBody(BodyError)`) and the CLI can match its `cause`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyError {
    pub cause: BodyCause,
    /// Where the reader stopped, counted from 1.
    pub line: usize,
    pub column: usize,
    /// The reader's own sentence, naming the field or the token it stopped at.
    pub detail: String,
}

impl BodyError {
    /// A body over `maximum_bytes`, refused before it is read.
    pub(crate) fn too_large(maximum_bytes: usize) -> Self {
        Self {
            cause: BodyCause::TooLarge,
            line: 0,
            column: 0,
            detail: format!("the body is larger than {maximum_bytes} bytes"),
        }
    }

    /// The status a route answers this body with: `413` for one too large, else `400`.
    pub fn status(&self) -> StatusCode {
        match self.cause {
            BodyCause::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            BodyCause::Syntax | BodyCause::Data | BodyCause::Truncated => StatusCode::BAD_REQUEST,
        }
    }

    pub(crate) fn from_json(error: &serde_json::Error) -> Self {
        let cause = match error.classify() {
            serde_json::error::Category::Eof => BodyCause::Truncated,
            serde_json::error::Category::Data => BodyCause::Data,
            serde_json::error::Category::Syntax | serde_json::error::Category::Io => {
                BodyCause::Syntax
            }
        };
        Self {
            cause,
            line: error.line(),
            column: error.column(),
            detail: error.to_string(),
        }
    }
}

impl std::fmt::Display for BodyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the request body is not valid: {}", self.detail)
    }
}

impl std::error::Error for BodyError {}

/// Reads a route's JSON request body.
pub(crate) fn parse_body<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, BodyError> {
    serde_json::from_slice(bytes).map_err(|error| BodyError::from_json(&error))
}

/// Reads a route's JSON request body as axum read it: a body over the listener's limit is refused
/// as too large, and one that broke off as truncated, with what axum said.
pub(crate) fn read_body<T: DeserializeOwned>(
    bytes: Result<axum::body::Bytes, BytesRejection>,
) -> Result<T, BodyError> {
    let bytes = bytes.map_err(|rejection| BodyError {
        cause: if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            BodyCause::TooLarge
        } else {
            BodyCause::Truncated
        },
        line: 0,
        column: 0,
        detail: rejection.body_text(),
    })?;
    parse_body(&bytes)
}

/// A query of a GET route that is missing a parameter or has one that does not read. It is a wire
/// value, so a route error carries it whole (`InvalidQuery(QueryError)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryError {
    /// The reader's own sentence, naming the parameter it stopped at.
    pub detail: String,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the query is not valid: {}", self.detail)
    }
}

impl std::error::Error for QueryError {}

/// Reads a route's query as axum read it, or the query error naming what does not read.
pub(crate) fn read_query<T>(query: Result<Query<T>, QueryRejection>) -> Result<T, QueryError> {
    query
        .map(|Query(query)| query)
        .map_err(|rejection| QueryError {
            detail: rejection.body_text(),
        })
}
