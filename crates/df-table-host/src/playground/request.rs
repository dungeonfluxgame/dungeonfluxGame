//! The reply of the routes that change what the playground shows (`/dev/playground/controls`,
//! `/dev/playground/controls/batch` and `/dev/playground/scene`), and why they refuse. The page
//! reads the same reply as `df_playground::controls::ControlsChangeReply`.

use std::fmt;

use axum::extract::rejection::BytesRejection;
use axum::http::StatusCode;
use df_playground::controls::ControlsReply;
use serde::{Deserialize, Serialize};

use super::controls_state::ControlsChangeError;
use super::files::FileError;
use super::{FixtureNameError, SceneApplyError};
use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::limits::MAXIMUM_REQUEST_BYTES;

/// The answer to a change of what the playground shows: the controls after it, or its refusal.
pub type ChangeReply = DevReply<ControlsReply, PlaygroundChangeError>;

/// Why the bytes of a request body could not be had at all (before they are read as JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyReadCause {
    /// The body is larger than `MAXIMUM_REQUEST_BYTES`.
    TooLarge,
    /// The body broke off or could not be received.
    Unreadable,
}

impl BodyReadCause {
    pub const ALL: [Self; 2] = [Self::TooLarge, Self::Unreadable];

    /// The cause of axum's refusal to hand over a body.
    pub fn of(rejection: &BytesRejection) -> Self {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            Self::TooLarge
        } else {
            Self::Unreadable
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::Unreadable => "unreadable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

/// Why a change of what the playground shows was refused. `detail` is the sentence naming the value
/// or file, and the reply's `message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum PlaygroundChangeError {
    /// The body is not a change.
    InvalidBody(BodyError),
    /// The body's bytes could not be had: too large, or broken off.
    UnreadableBody { cause: BodyReadCause },
    /// The playground's checks refuse the change: a value out of range, or a fixture, media file
    /// or element that is not there, or no fixture shown.
    InvalidChange { detail: String },
    /// The fixture files or the scene changed while the change was being checked; asking again may
    /// succeed.
    FixtureChanged { detail: String },
    /// A file the change reads is not there.
    FileMissing { detail: String },
    /// A file the change reads is larger than its kind of file may be.
    FileTooLarge { detail: String },
    /// A file the change reads could not be read.
    FileUnreadable { detail: String },
}

impl fmt::Display for PlaygroundChangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::UnreadableBody {
                cause: BodyReadCause::TooLarge,
            } => write!(
                formatter,
                "the request body is larger than {MAXIMUM_REQUEST_BYTES} bytes"
            ),
            Self::UnreadableBody {
                cause: BodyReadCause::Unreadable,
            } => formatter.write_str("the request body could not be read"),
            Self::InvalidChange { detail }
            | Self::FixtureChanged { detail }
            | Self::FileMissing { detail }
            | Self::FileTooLarge { detail }
            | Self::FileUnreadable { detail } => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for PlaygroundChangeError {}

impl RouteError for PlaygroundChangeError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::InvalidChange { .. } => StatusCode::BAD_REQUEST,
            Self::UnreadableBody {
                cause: BodyReadCause::TooLarge,
            } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnreadableBody {
                cause: BodyReadCause::Unreadable,
            } => StatusCode::BAD_REQUEST,
            Self::FixtureChanged { .. } => StatusCode::CONFLICT,
            Self::FileMissing { .. } => StatusCode::NOT_FOUND,
            Self::FileTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::FileUnreadable { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<BodyError> for PlaygroundChangeError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<FixtureNameError> for PlaygroundChangeError {
    fn from(error: FixtureNameError) -> Self {
        Self::InvalidChange {
            detail: error.to_string(),
        }
    }
}

impl From<FileError> for PlaygroundChangeError {
    fn from(error: FileError) -> Self {
        let detail = error.to_string();
        match error {
            FileError::InvalidName | FileError::NotFound => Self::FileMissing { detail },
            FileError::TooLarge { .. } => Self::FileTooLarge { detail },
            FileError::Io(_) | FileError::ModifiedBeforeEpoch | FileError::Stopped => {
                Self::FileUnreadable { detail }
            }
        }
    }
}

impl From<ControlsChangeError> for PlaygroundChangeError {
    fn from(error: ControlsChangeError) -> Self {
        let detail = error.to_string();
        match error {
            ControlsChangeError::FixtureChanged => Self::FixtureChanged { detail },
            ControlsChangeError::NoSuchFixture { .. }
            | ControlsChangeError::EmptyBatch
            | ControlsChangeError::TooManyChanges { .. }
            | ControlsChangeError::Invalid(_) => Self::InvalidChange { detail },
        }
    }
}

/// Transitional: `SceneApplyError` still carries a status and a sentence, because the Director and
/// cast read those, so its status picks the variant. Each variant's status maps back to itself.
impl From<SceneApplyError> for PlaygroundChangeError {
    fn from(error: SceneApplyError) -> Self {
        let detail = error.message;
        match error.status {
            StatusCode::CONFLICT => Self::FixtureChanged { detail },
            StatusCode::NOT_FOUND => Self::FileMissing { detail },
            StatusCode::PAYLOAD_TOO_LARGE => Self::FileTooLarge { detail },
            status if status.is_server_error() => Self::FileUnreadable { detail },
            _ => Self::InvalidChange { detail },
        }
    }
}
