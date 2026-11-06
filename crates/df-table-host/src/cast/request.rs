//! The bodies and replies of the cast routes, and why they refuse.
//!
//! Every route answers a `CastReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`.

use std::fmt;

use axum::http::StatusCode;
use df_scene_layout::{CastMember, CharacterCue, Shot};
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::locate::LocateError;
use crate::playground::PlaygroundChangeError;

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

/// The named characters of the shown scene's backdrop, left to right.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastList {
    pub picture: String,
    pub members: Vec<CastMember>,
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
    /// The shown scene could not be read, or refused the change.
    Scene { cause: PlaygroundChangeError },
    /// The layout of the shown backdrop could not be read.
    Layout { cause: LocateError },
    /// The shown scene has no backdrop picture.
    NoBackdrop,
    /// The shown backdrop has no current layout.
    NoLayout { picture: String },
    /// `character` is not a named character of `picture`'s layout, whose named ones are `names`.
    UnknownCharacter {
        character: String,
        picture: String,
        names: Vec<String>,
    },
}

impl CastError {
    /// The error's serde tag, for the audit line.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::Scene { .. } => "scene",
            Self::Layout { .. } => "layout",
            Self::NoBackdrop => "no_backdrop",
            Self::NoLayout { .. } => "no_layout",
            Self::UnknownCharacter { .. } => "unknown_character",
        }
    }
}

impl fmt::Display for CastError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::Scene { cause } => write!(formatter, "{cause}"),
            Self::Layout { cause } => write!(formatter, "{cause}"),
            Self::NoBackdrop => formatter.write_str("the shown scene has no backdrop picture"),
            Self::NoLayout { picture } => write!(
                formatter,
                concat!(
                    "the shown scene's backdrop ({0}) has no current layout; ",
                    "scan it with: media locate {0}"
                ),
                picture
            ),
            Self::UnknownCharacter {
                character,
                picture,
                names,
            } => write!(
                formatter,
                "no character is named {character:?} in {picture}'s layout; the named ones are: {}",
                if names.is_empty() {
                    "(none: name them with media locate label or media locate identify)".to_owned()
                } else {
                    names.join(", ")
                }
            ),
        }
    }
}

impl std::error::Error for CastError {}

impl From<BodyError> for CastError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<LocateError> for CastError {
    fn from(cause: LocateError) -> Self {
        Self::Layout { cause }
    }
}

impl From<crate::playground::SceneApplyError> for CastError {
    fn from(error: crate::playground::SceneApplyError) -> Self {
        Self::Scene {
            cause: PlaygroundChangeError::from(error),
        }
    }
}

impl RouteError for CastError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) => StatusCode::BAD_REQUEST,
            Self::Scene { cause } => cause.status(),
            Self::Layout { cause } => cause.status(),
            Self::NoBackdrop | Self::NoLayout { .. } | Self::UnknownCharacter { .. } => {
                StatusCode::NOT_FOUND
            }
        }
    }
}
