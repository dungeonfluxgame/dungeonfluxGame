//! The bodies and replies of the cast routes, and why they refuse.
//!
//! Every route answers a `CastReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`.

use std::fmt;

use axum::http::StatusCode;
use df_scene_layout::{CastMember, CharacterCue, Shot};
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::persona::PersonaSummary;
use crate::playground::SceneApplyError;
use crate::shown_scene::ShownSceneError;

/// The answer of a cast route: its value, or why it was refused.
pub(crate) type CastReply<T> = DevReply<T, CastError>;

/// Frames a named character of the shown scene with a camera shot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraRequest {
    pub character: String,
    /// `face`, `upper` (the default) or `full`.
    #[serde(default)]
    pub shot: Option<Shot>,
    #[serde(default)]
    pub over_ms: Option<u64>,
}

/// The named characters of the shown scene's backdrop, left to right, and the summary of each that
/// has a stored persona.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastList {
    pub picture: String,
    pub members: Vec<CastMember>,
    pub personas: Vec<PersonaSummary>,
}

/// A cue applied to the live scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cued {
    pub cue: CharacterCue,
    pub revision: u64,
}

/// A camera move on a character, applied to the live scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraMoved {
    pub target: String,
    pub zoom_percent: u16,
    pub duration_ms: u64,
    pub revision: u64,
}

/// Why a cast request was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum CastError {
    /// The request body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The shown scene, its layout or the named character could not be had.
    Shown { cause: ShownSceneError },
}

impl CastError {
    /// The error's key for the audit line: `invalid_body`, or the key of the shown-scene refusal.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::Shown { cause } => cause.key(),
        }
    }
}

impl fmt::Display for CastError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::Shown { cause } => write!(formatter, "{cause}"),
        }
    }
}

impl std::error::Error for CastError {}

impl From<BodyError> for CastError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<ShownSceneError> for CastError {
    fn from(cause: ShownSceneError) -> Self {
        Self::Shown { cause }
    }
}

impl From<SceneApplyError> for CastError {
    fn from(error: SceneApplyError) -> Self {
        Self::Shown {
            cause: ShownSceneError::from(error),
        }
    }
}

impl RouteError for CastError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) => StatusCode::BAD_REQUEST,
            Self::Shown { cause } => cause.status(),
        }
    }
}
