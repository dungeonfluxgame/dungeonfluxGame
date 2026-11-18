//! The reply of the client timeline read (`GET /playground/clients/{client}/timeline.json`) and why
//! it or a page's report (`POST /dev/playground/report`) is refused.

use std::fmt;

use axum::http::StatusCode;
use df_playground::timeline::ClientTimeline;
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, QueryError, RouteError};

/// The answer to a timeline read: the page's timeline, or the refusal.
pub type ClientTimelineReply = DevReply<ClientTimeline, ClientsError>;

/// Why a read of a page's timeline, or a page's report, was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum ClientsError {
    /// The query is not `kind=<kind>` and `after=<seq>`.
    InvalidQuery(QueryError),
    /// A report's body is not a report: not its JSON, broken off, or larger than a report may be.
    InvalidBody(BodyError),
    /// The page's name is not a client name.
    InvalidClient { detail: String },
    /// No page of that name is open.
    NoSuchClient { client: String },
}

impl ClientsError {
    /// The closed key an audit line carries.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidQuery(_) => "invalid_query",
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidClient { .. } => "invalid_client",
            Self::NoSuchClient { .. } => "no_such_client",
        }
    }
}

impl fmt::Display for ClientsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuery(error) => write!(formatter, "{error}"),
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidClient { detail } => formatter.write_str(detail),
            Self::NoSuchClient { client } => write!(
                formatter,
                "no page named '{client}' is open (see clients for the open pages)"
            ),
        }
    }
}

impl std::error::Error for ClientsError {}

impl From<BodyError> for ClientsError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<QueryError> for ClientsError {
    fn from(error: QueryError) -> Self {
        Self::InvalidQuery(error)
    }
}

impl RouteError for ClientsError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(error) => error.status(),
            Self::InvalidQuery(_) | Self::InvalidClient { .. } => StatusCode::BAD_REQUEST,
            Self::NoSuchClient { .. } => StatusCode::NOT_FOUND,
        }
    }
}
