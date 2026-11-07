//! The services report and why its route refuses.
//!
//! The route answers a `ServicesReply<ServicesReport>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>"},"message":".."}`.

use std::fmt;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::dev_http::{DevReply, RouteError};
use crate::text::BackendKind;

/// The answer of the services route: its report, or why it was refused.
pub type ServicesReply<T> = DevReply<T, ServicesError>;

/// How each media and text service is set up and how it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicesReport {
    pub text: TextReport,
    pub imagegen: ImagegenReport,
    pub locate: LocateReport,
}

/// The text surface: its backend and model, whether it can send, and its attempts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextReport {
    pub backend: BackendKind,
    /// The profile requests go under: backend and model.
    pub profile: String,
    /// The model identity questions name; none when Codex uses its own default.
    pub identity_model: Option<String>,
    /// The model persona calls use instead of the default (`DF_TEXT_PERSONA_MODEL`), if any.
    pub persona_model: Option<String>,
    /// The profile persona calls go under when they name no model; none when the persona model
    /// is not a valid model name.
    pub persona_profile: Option<String>,
    pub configured: bool,
    /// The environment variable to set when the backend is not configured.
    pub missing: Option<String>,
    /// Attempts the in-memory store keeps this run.
    pub attempts_kept: u64,
    /// Backend calls under way.
    pub in_flight: u64,
}

/// Whether the picture generator is switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMode {
    /// A real backend makes pictures.
    Live,
    /// The host was not started with the switches that turn generation on.
    Off,
}

impl ImageMode {
    pub const ALL: [Self; 2] = [Self::Live, Self::Off];

    pub fn key(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Off => "off",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }
}

/// The picture generator: its backend, its deadline, where provenance goes and its jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImagegenReport {
    pub backend: String,
    pub mode: ImageMode,
    pub deadline_ms: u64,
    pub provenance_dir_set: bool,
    pub codex_home_set: bool,
    pub running: u64,
    /// How the job that ended last ended: `stored` or its error's key; none before any ended.
    pub last_outcome: Option<String>,
}

/// The picture locator: the segmentation model, the supplier's host, the scan's limit, whether a
/// supplier key is present and how sure an identification must be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocateReport {
    pub model: String,
    pub endpoint_host: String,
    pub timeout_ms: u64,
    pub key_present: bool,
    pub min_confidence_permille: u16,
}

/// Why the services route refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum ServicesError {
    /// The route takes no query.
    InvalidQuery,
}

impl ServicesError {
    /// The error's serde tag, for the audit line.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidQuery => "invalid_query",
        }
    }
}

impl fmt::Display for ServicesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuery => formatter.write_str("the services report takes no query"),
        }
    }
}

impl std::error::Error for ServicesError {}

impl RouteError for ServicesError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidQuery => StatusCode::BAD_REQUEST,
        }
    }
}
