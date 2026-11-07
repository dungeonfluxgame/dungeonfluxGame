//! The emotion list as data: `<playground>/persona/emotions.json`, read fresh on every use so a
//! campaign adds or drops an emotion, retunes the decay or moves the rapport thresholds without a
//! restart (`planning/persona-emotion.md`). A missing file is the built-in list; a file that does
//! not read is a typed [`EmotionError`] naming why.
//!
//! [`EmotionError`] is also the error of the state routes (`state_routes`) and of the temperament
//! question: it wraps the persona error where the persona is the cause.

use std::fmt;
use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use df_content::persona::{EmotionConfig, EmotionConfigError};
use serde::{Deserialize, Serialize};

use super::request::PersonaError;
use crate::dev_http::{BodyError, RouteError};
use crate::io_cause::IoCause;
use crate::state::AppState;

/// The folder of the emotion list under the playground's assets.
pub const EMOTIONS_DIRECTORY: &str = "persona";
/// The emotion list's file name.
pub const EMOTIONS_FILE: &str = "emotions.json";

/// Why the emotion list or a persona's live state could not be had or kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum EmotionError {
    /// The request body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The emotion file could not be read.
    EmotionsUnreadable { cause: IoCause },
    /// The emotion file is not the JSON of an emotion list; `cause` says where the reader stopped.
    EmotionsInvalid { cause: BodyError },
    /// The emotion file reads but is not a usable list.
    EmotionsRejected { cause: EmotionConfigError },
    /// The state file of `id` could not be read.
    StateUnreadable { id: String, cause: IoCause },
    /// The state file of `id` is not valid; `cause` says where the reader stopped.
    StateInvalid { id: String, cause: BodyError },
    /// The state file of `id` holds the state of `found`.
    StateIdMismatch { id: String, found: String },
    /// The state file of `id` could not be written or removed.
    StateUnwritable { id: String, cause: IoCause },
    /// The emotion is not on the emotion list.
    UnknownEmotion { emotion: String },
    /// A standing cannot be held toward `toward`: not a character name, the character itself, or
    /// the standings are full.
    InvalidStanding { toward: String },
    /// The persona, its edit or its generation failed.
    Persona { cause: Box<PersonaError> },
    /// The blocking task stopped without an answer.
    Stopped,
}

impl EmotionError {
    /// The error's serde tag, for the audit line; for a wrapped persona error, its key.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::EmotionsUnreadable { .. } => "emotions_unreadable",
            Self::EmotionsInvalid { .. } => "emotions_invalid",
            Self::EmotionsRejected { .. } => "emotions_rejected",
            Self::StateUnreadable { .. } => "state_unreadable",
            Self::StateInvalid { .. } => "state_invalid",
            Self::StateIdMismatch { .. } => "state_id_mismatch",
            Self::StateUnwritable { .. } => "state_unwritable",
            Self::UnknownEmotion { .. } => "unknown_emotion",
            Self::InvalidStanding { .. } => "invalid_standing",
            Self::Persona { cause } => cause.key(),
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for EmotionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::EmotionsUnreadable { cause } => write!(
                formatter,
                "the emotion file {EMOTIONS_DIRECTORY}/{EMOTIONS_FILE} could not be read: {cause}"
            ),
            Self::EmotionsInvalid { cause } => write!(
                formatter,
                "the emotion file {EMOTIONS_DIRECTORY}/{EMOTIONS_FILE} is not valid: {}",
                cause.detail
            ),
            Self::EmotionsRejected { cause } => write!(
                formatter,
                "the emotion file {EMOTIONS_DIRECTORY}/{EMOTIONS_FILE} is not usable: {cause}"
            ),
            Self::StateUnreadable { id, cause } => {
                write!(
                    formatter,
                    "the state file of {id:?} could not be read: {cause}"
                )
            }
            Self::StateInvalid { id, cause } => write!(
                formatter,
                "the state file of {id:?} is not valid: {}",
                cause.detail
            ),
            Self::StateIdMismatch { id, found } => {
                write!(
                    formatter,
                    "the state file of {id:?} holds the state of {found:?}"
                )
            }
            Self::StateUnwritable { id, cause } => {
                write!(
                    formatter,
                    "the state file of {id:?} could not be written: {cause}"
                )
            }
            Self::UnknownEmotion { emotion } => write!(
                formatter,
                "{emotion:?} is not on the emotion list (see {EMOTIONS_DIRECTORY}/{EMOTIONS_FILE})"
            ),
            Self::InvalidStanding { toward } => write!(
                formatter,
                "no standing can be held toward {toward:?}: give another character's name"
            ),
            Self::Persona { cause } => write!(formatter, "{cause}"),
            Self::Stopped => write!(formatter, "the task stopped before it answered"),
        }
    }
}

impl std::error::Error for EmotionError {}

impl From<PersonaError> for EmotionError {
    fn from(cause: PersonaError) -> Self {
        Self::Persona {
            cause: Box::new(cause),
        }
    }
}

impl From<BodyError> for EmotionError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl RouteError for EmotionError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::UnknownEmotion { .. } | Self::InvalidStanding { .. } => {
                StatusCode::BAD_REQUEST
            }
            Self::Persona { cause } => cause.status(),
            Self::EmotionsUnreadable { .. }
            | Self::EmotionsInvalid { .. }
            | Self::EmotionsRejected { .. }
            | Self::StateUnreadable { .. }
            | Self::StateInvalid { .. }
            | Self::StateIdMismatch { .. }
            | Self::StateUnwritable { .. }
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// The emotion file of the host's playground.
pub(super) fn emotions_path(app: &AppState) -> PathBuf {
    app.playground()
        .files()
        .assets()
        .join(EMOTIONS_DIRECTORY)
        .join(EMOTIONS_FILE)
}

/// The emotion list in the file at `path`; the built-in list when there is no file.
pub(super) fn read_emotions(path: &Path) -> Result<EmotionConfig, EmotionError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EmotionConfig::default());
        }
        Err(error) => {
            return Err(EmotionError::EmotionsUnreadable {
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    let config: EmotionConfig =
        serde_json::from_str(&text).map_err(|error| EmotionError::EmotionsInvalid {
            cause: BodyError::from_json(&error),
        })?;
    config
        .check()
        .map_err(|cause| EmotionError::EmotionsRejected { cause })?;
    Ok(config)
}

/// The emotion list now: the file is read on every call.
pub(crate) fn load_emotions(app: &AppState) -> Result<EmotionConfig, EmotionError> {
    read_emotions(&emotions_path(app))
}
