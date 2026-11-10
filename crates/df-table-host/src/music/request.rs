//! What crosses `/dev/playground/scene/music`: the request, the reply and `MusicError`.

use std::fmt;

use axum::http::StatusCode;
use df_playground::limits::MAXIMUM_VOLUME_PERCENT;
use df_playground::scene::Target;
use df_playground::voice_wire::MissingSetting;
use serde::{Deserialize, Serialize};

use super::limits::{MAXIMUM_LENGTH_SECONDS, MAXIMUM_PROMPT_CHARACTERS, MINIMUM_LENGTH_SECONDS};
use crate::dev_http::{BodyError, RouteError};
use crate::imagegen::model::plain_name;
use crate::io_cause::IoCause;
use crate::playground::PlaygroundChangeError;

/// The background music of one narrative scene: the track's name (with no extension), what to
/// compose when the library has no sound of that name, and how to play it. It always loops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusicRequest {
    pub name: String,
    pub prompt: String,
    pub length_seconds: u32,
    pub target: Target,
    pub volume_percent: u8,
}

impl MusicRequest {
    /// That every field is one the route takes; the command line checks the same before sending.
    pub fn check(&self) -> Result<(), MusicError> {
        if !plain_name(&self.name) {
            return Err(MusicError::InvalidName {
                name: self.name.clone(),
            });
        }
        let prompt_characters = self.prompt.chars().count();
        if self.prompt.trim().is_empty() || prompt_characters > MAXIMUM_PROMPT_CHARACTERS {
            return Err(MusicError::InvalidPrompt { prompt_characters });
        }
        if !(MINIMUM_LENGTH_SECONDS..=MAXIMUM_LENGTH_SECONDS).contains(&self.length_seconds) {
            return Err(MusicError::InvalidLength {
                length_seconds: self.length_seconds,
            });
        }
        if self.volume_percent > MAXIMUM_VOLUME_PERCENT {
            return Err(MusicError::InvalidVolume {
                volume_percent: self.volume_percent,
            });
        }
        Ok(())
    }
}

/// Where the played track came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicSource {
    /// The library already held a sound of the name; nothing was composed.
    Library,
    /// It was composed for this request and stored in the library.
    Composed,
}

impl MusicSource {
    pub const ALL: [Self; 2] = [Self::Library, Self::Composed];

    pub fn key(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Composed => "composed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.key() == key)
    }
}

/// The music now playing on the scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusicPlayed {
    /// The library file played, such as `tavern.mp3`.
    pub file: String,
    pub source: MusicSource,
    pub bytes: u64,
    /// The length asked of the composer; `None` for a track from the library.
    pub length_ms: Option<u64>,
    /// How long the composition took; `None` for a track from the library.
    pub compose_ms: Option<u64>,
    pub target: Target,
    pub volume_percent: u8,
    /// The playground revision the music change made.
    pub revision: u64,
}

/// Why a call to the music supplier gave no track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicProviderCause {
    /// The adapter refused the request or the key before sending it.
    Rejected,
    /// The supplier refused the key (an HTTP 401 or 403).
    Denied,
    /// The supplier is rate limited or out of capacity (an HTTP 429).
    Capacity,
    /// The supplier answered another failure.
    Supplier,
    /// The exchange failed on the wire.
    Transport,
    /// The composition took longer than its deadline.
    Deadline,
    /// The supplier answered success with no audio.
    Unusable,
}

impl MusicProviderCause {
    pub const ALL: [Self; 7] = [
        Self::Rejected,
        Self::Denied,
        Self::Capacity,
        Self::Supplier,
        Self::Transport,
        Self::Deadline,
        Self::Unusable,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Rejected => "rejected",
            Self::Denied => "denied",
            Self::Capacity => "capacity",
            Self::Supplier => "supplier",
            Self::Transport => "transport",
            Self::Deadline => "deadline",
            Self::Unusable => "unusable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

/// Why `scene music generate` played nothing new. The scene is unchanged by every refusal. An
/// error never carries the prompt or audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum MusicError {
    InvalidBody(BodyError),
    /// Not 1 to `imagegen::MAXIMUM_NAME_CHARS` letters, digits, '-' or '_'.
    InvalidName {
        name: String,
    },
    /// Blank, or longer than the supplier takes.
    InvalidPrompt {
        prompt_characters: usize,
    },
    InvalidLength {
        length_seconds: u32,
    },
    InvalidVolume {
        volume_percent: u8,
    },
    /// The music switch is off (`playground set music=true`), so nothing is composed. A track the
    /// library already holds is still found; only the paid call is refused.
    Disabled,
    /// The music supplier is not set up; `missing` says what to set.
    NotConfigured {
        missing: MissingSetting,
    },
    Provider {
        cause: MusicProviderCause,
    },
    /// The media library could not be listed.
    LibraryUnreadable {
        cause: IoCause,
    },
    /// The composed track could not be stored in the library.
    Unstored {
        cause: IoCause,
    },
    /// The scene refused the music change.
    Scene(PlaygroundChangeError),
    /// The task that composed the track stopped without an answer.
    Stopped,
}

impl MusicError {
    /// The error's closed word, as its serde tag writes it.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidName { .. } => "invalid_name",
            Self::InvalidPrompt { .. } => "invalid_prompt",
            Self::InvalidLength { .. } => "invalid_length",
            Self::InvalidVolume { .. } => "invalid_volume",
            Self::Disabled => "disabled",
            Self::NotConfigured { .. } => "not_configured",
            Self::Provider { .. } => "provider",
            Self::LibraryUnreadable { .. } => "library_unreadable",
            Self::Unstored { .. } => "unstored",
            Self::Scene(_) => "scene",
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for MusicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidName { name } => write!(
                formatter,
                "'{name}' is not a track name: use letters, digits, '-' or '_', with no extension"
            ),
            Self::InvalidPrompt { prompt_characters } => write!(
                formatter,
                "the prompt must have words and at most {MAXIMUM_PROMPT_CHARACTERS} characters, \
                 not {prompt_characters}"
            ),
            Self::InvalidLength { length_seconds } => write!(
                formatter,
                "a track lasts {MINIMUM_LENGTH_SECONDS} to {MAXIMUM_LENGTH_SECONDS} seconds, \
                 not {length_seconds}"
            ),
            Self::InvalidVolume { volume_percent } => write!(
                formatter,
                "the volume is 0 to {MAXIMUM_VOLUME_PERCENT}, not {volume_percent}"
            ),
            Self::Disabled => formatter.write_str(concat!(
                "the music is switched off, so no track is composed: use the Music switch or ",
                "`playground set music=true`"
            )),
            Self::NotConfigured { missing } => write!(
                formatter,
                "the music supplier is not set up: set {}",
                missing.variable()
            ),
            Self::Provider { cause } => write!(
                formatter,
                "the music supplier gave no track ({}); the scene's music is unchanged",
                cause.key()
            ),
            Self::LibraryUnreadable { cause } => {
                write!(formatter, "the media library could not be listed: {cause}")
            }
            Self::Unstored { cause } => {
                write!(formatter, "the composed track could not be stored: {cause}")
            }
            Self::Scene(error) => write!(formatter, "the scene refused the music: {error}"),
            Self::Stopped => formatter.write_str("the music task stopped without an answer"),
        }
    }
}

impl std::error::Error for MusicError {}

impl RouteError for MusicError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::InvalidName { .. }
            | Self::InvalidPrompt { .. }
            | Self::InvalidLength { .. }
            | Self::InvalidVolume { .. } => StatusCode::BAD_REQUEST,
            Self::Disabled => StatusCode::CONFLICT,
            Self::NotConfigured { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Provider {
                cause: MusicProviderCause::Deadline,
            } => StatusCode::GATEWAY_TIMEOUT,
            Self::Provider { .. } => StatusCode::BAD_GATEWAY,
            Self::Scene(error) => error.status(),
            Self::LibraryUnreadable { .. } | Self::Unstored { .. } | Self::Stopped => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}
