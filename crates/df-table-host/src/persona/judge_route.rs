//! What the judge routes share: the one-call model judgements that answer a closed schema whose
//! every id code checks (who a line is directed at, how much the picture would change). The
//! refusal type is generic over the judgement's own cause, so each judge keeps its typed reasons
//! and all of them answer the same way on the wire: `{"error": "<key>", ...}`.

use std::fmt;
use std::sync::Arc;

use axum::http::StatusCode;
use df_ai::text::TextRequest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::generate::operation_key;
use crate::dev_http::{BodyError, RouteError};
use crate::text::{
    ModelCause, ServiceReason, TextAskError, TextRoute, TextRuntime, TextServiceCause,
};

/// Why a judge route refused, with `C` the judgement's own cause. `key()` is the serde tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum JudgeRouteError<C> {
    /// The request body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The request, or what it is judged against, cannot be used.
    BadRequest { cause: C },
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
    AnswerRefused { replayed: bool, cause: C },
    /// The text-service operation key is not valid.
    InvalidOperationKey,
    /// The blocking task stopped without an answer.
    Stopped,
}

impl<C> JudgeRouteError<C> {
    /// The error's key for the audit line: its serde tag.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::BadRequest { .. } => "bad_request",
            Self::TextServiceMissing { .. } => "text_service_missing",
            Self::InvalidModel { .. } => "invalid_model",
            Self::TextService { .. } => "text_service",
            Self::AnswerRefused { .. } => "answer_refused",
            Self::InvalidOperationKey => "invalid_operation_key",
            Self::Stopped => "stopped",
        }
    }
}

impl<C: fmt::Display> fmt::Display for JudgeRouteError<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(_) => formatter.write_str("the request body is not the JSON to read"),
            Self::BadRequest { cause } => write!(formatter, "the request cannot be used: {cause}"),
            Self::TextServiceMissing { missing } => {
                write!(formatter, "the text service is not set up: {missing}")
            }
            Self::InvalidModel { .. } => formatter.write_str("the model name is not usable"),
            Self::TextService { .. } => formatter.write_str("the text service gave no answer"),
            Self::AnswerRefused { cause, .. } => {
                write!(formatter, "the model's answer was refused: {cause}")
            }
            Self::InvalidOperationKey => formatter.write_str("the operation key is not valid"),
            Self::Stopped => formatter.write_str("the judgement stopped without an answer"),
        }
    }
}

impl<C: fmt::Debug + fmt::Display> std::error::Error for JudgeRouteError<C> {}

impl<C: fmt::Debug + fmt::Display + Send + Sync + 'static> RouteError for JudgeRouteError<C> {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::BadRequest { .. } | Self::InvalidModel { .. } => {
                StatusCode::BAD_REQUEST
            }
            Self::TextServiceMissing { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TextService { .. } | Self::AnswerRefused { .. } => StatusCode::BAD_GATEWAY,
            Self::InvalidOperationKey | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl<C> From<TextAskError<C>> for JudgeRouteError<C> {
    fn from(error: TextAskError<C>) -> Self {
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

/// The route a judgement is asked of: the default persona model, or the one the request names.
pub(crate) fn judge_route<C>(
    text: &TextRuntime,
    model: Option<&str>,
) -> Result<Arc<TextRoute>, JudgeRouteError<C>> {
    if let Some(missing) = text.missing() {
        return Err(JudgeRouteError::TextServiceMissing {
            missing: missing.to_owned(),
        });
    }
    text.persona_route(model)
        .map_err(|cause| JudgeRouteError::InvalidModel { cause })
}

/// Asks `request` once under `operation` and reads the answer with `read`. An answer the reader
/// refuses is [`JudgeRouteError::AnswerRefused`], saying whether it was a replay, so the caller
/// can ask once more with why.
pub(crate) async fn ask_request<T, C: fmt::Display>(
    route: &TextRoute,
    operation: &str,
    request: &TextRequest,
    read: impl Fn(&Value) -> Result<T, C>,
) -> Result<T, JudgeRouteError<C>> {
    let key = operation_key(operation).map_err(|_| JudgeRouteError::InvalidOperationKey)?;
    route
        .ask(&key, request, read)
        .await
        .map(|asked| asked.value)
        .map_err(JudgeRouteError::from)
}
