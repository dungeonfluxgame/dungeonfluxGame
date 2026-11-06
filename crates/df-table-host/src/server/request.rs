//! What the host's own routes read and answer: the query of `/events.json`, the reply of
//! `GET /dev/host/status`, and why a host route refuses (`HostError`).
//!
//! The events query is typed at the edge: an area is checked once, here, and a reader never sends
//! free text into a filter. The status is a reading of counters the host already keeps; nothing
//! in it is computed for the status alone.

use std::fmt;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::dev_http::{DevReply, RouteError};
use crate::observe::SinkHealth;

/// The answer of a host route: the value, or its refusal.
pub type HostReply<T> = DevReply<T, HostError>;

/// Longest area name an events query may filter by, in bytes. Areas are short uppercase words
/// (`CANVAS`, `PLAYGROUND`, `RESOURCE`), so this only bounds a mistake.
pub const MAXIMUM_AREA_BYTES: usize = 64;

/// The first word of an audit line, as a reader names it to filter the log: uppercase ASCII
/// letters, digits and `_`, at most `MAXIMUM_AREA_BYTES`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LineArea(String);

impl LineArea {
    pub fn new(text: &str) -> Result<Self, LineAreaError> {
        let plain = text.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        });
        if text.is_empty() || text.len() > MAXIMUM_AREA_BYTES || !plain {
            return Err(LineAreaError { length: text.len() });
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for LineArea {
    type Error = LineAreaError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

impl From<LineArea> for String {
    fn from(area: LineArea) -> Self {
        area.0
    }
}

/// Why an area is not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineAreaError {
    /// Bytes of the text given (the text itself is not repeated).
    pub length: usize,
}

impl fmt::Display for LineAreaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "an area is the first word of a line: uppercase letters, digits and '_', 1 to \
             {MAXIMUM_AREA_BYTES} bytes (this one is {} bytes)",
            self.length
        )
    }
}

impl std::error::Error for LineAreaError {}

/// The query of `route::EVENTS`: lines newer than `after`, only those of `area`, at most `limit`
/// (never more than `MAXIMUM_EVENTS_PER_ANSWER`). The field `after` is named like
/// `wire::EVENTS_AFTER_PARAMETER`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsQuery {
    pub after: Option<u64>,
    pub area: Option<LineArea>,
    pub limit: Option<usize>,
}

/// A host route whose query can be refused, for the audit line (`HOST refused route=..`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueriedRoute {
    Events,
    Frame,
}

impl QueriedRoute {
    pub const ALL: [Self; 2] = [Self::Events, Self::Frame];

    pub fn key(self) -> &'static str {
        match self {
            Self::Events => "events",
            Self::Frame => "frame",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|route| route.key() == key)
    }
}

/// Why a host route refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum HostError {
    /// The query is not one the route reads: an unknown parameter, or a value of the wrong type.
    /// `detail` is the reader's sentence, which names the parameter.
    InvalidQuery { detail: String },
}

impl HostError {
    /// The closed word of the refusal, for the audit line.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidQuery { .. } => "invalid_query",
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuery { detail } => write!(formatter, "the query is not valid: {detail}"),
        }
    }
}

impl std::error::Error for HostError {}

impl RouteError for HostError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidQuery { .. } => StatusCode::BAD_REQUEST,
        }
    }
}

/// The audit log and its terminal echo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogStatus {
    /// The sequence of the newest line.
    pub latest: u64,
    /// Lines kept, and their bytes of text.
    pub kept: usize,
    pub bytes: usize,
    /// Lines forgotten to keep the log's bounds since the host started.
    pub forgotten: u64,
    /// Bytes waiting for the terminal, each line with its overhead.
    pub echo_pending: usize,
    /// Lines not echoed and not yet reported by the echo.
    pub echo_dropped: u64,
    /// Whether the echo's writer is gone (lines are then only in the log).
    pub echo_writer_gone: bool,
    /// Library audit lines that found no sink (df-observe's count).
    pub undelivered_audit_lines: u64,
}

/// The host's clock, which moves travelling elements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockStatus {
    /// Ticks that took the canvas lock since the host started.
    pub ticks: u64,
    /// Elements travelling now.
    pub moving: usize,
    /// How often the clock ticks while something moves.
    pub tick_ms: u64,
}

/// What `GET /dev/host/status` answers: the log, the app's event sink, the clock and, when the
/// engine runs, the engine's event sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostStatus {
    pub log: LogStatus,
    pub sink: SinkHealth,
    pub clock: ClockStatus,
    /// The engine owner's event sink. Not filled yet: the owner's sink is not reachable from the
    /// host's state (the broker and sink unit exposes it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_sink: Option<SinkHealth>,
}

/// The method of a request, for the `HTTP` audit lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestMethod {
    Get,
    Post,
    Head,
    /// Any method the routes do not answer (`PUT`, `OPTIONS`, ...).
    Other,
}

impl RequestMethod {
    pub const ALL: [Self; 4] = [Self::Get, Self::Post, Self::Head, Self::Other];

    pub fn of(method: &axum::http::Method) -> Self {
        match *method {
            axum::http::Method::GET => Self::Get,
            axum::http::Method::POST => Self::Post,
            axum::http::Method::HEAD => Self::Head,
            _ => Self::Other,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Head => "head",
            Self::Other => "other",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|method| method.key() == key)
    }

    /// Whether a request with this method only reads.
    pub fn reads(self) -> bool {
        matches!(self, Self::Get | Self::Head)
    }
}
