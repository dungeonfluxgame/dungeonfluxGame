//! The bodies and replies of the speech routes, and why they refuse.
//!
//! Every route answers a `SpeechReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`. Nothing here holds a key,
//! a recording or a transcript except [`Transcript`], the one reply that must carry what was said
//! (its `Debug` is redacted).

use std::fmt;

use axum::http::StatusCode;
use df_content::persona::PersonaError as ContentError;
// The page reads these, so the page owns them and the JSON is one.
pub use df_playground::voice_wire::{
    MissingSetting, SpeechErrorKind, SpeechLimits, SpeechStatus, Transcript, VoiceRow,
};
use serde::{Deserialize, Serialize};

use super::limits::{
    MAXIMUM_AUDIO_MS, MAXIMUM_KEPT_CLIPS, MAXIMUM_KEY_TERM_CHARACTERS, MAXIMUM_KEY_TERM_WORDS,
    MAXIMUM_KEY_TERMS, MAXIMUM_TEXT_CHARACTERS, MINIMUM_AUDIO_MS,
};
use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::locate::IoCause;
use crate::playground::PlaygroundChangeError;

/// The answer of a speech route: its value, or why it was refused.
pub type SpeechReply<T> = DevReply<T, SpeechError>;

/// The bounds a caller works within, as this host sets them.
pub(super) fn current_limits() -> SpeechLimits {
    SpeechLimits {
        minimum_audio_ms: MINIMUM_AUDIO_MS,
        maximum_audio_ms: MAXIMUM_AUDIO_MS,
        maximum_text_characters: whole(MAXIMUM_TEXT_CHARACTERS),
        maximum_key_terms: whole(MAXIMUM_KEY_TERMS),
        maximum_kept_clips: whole(MAXIMUM_KEPT_CLIPS),
    }
}

/// A bound as a reply carries it.
fn whole(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// The body of `POST /dev/speech/speak`: text the operator typed, or the host itself produced, to
/// be voiced and played. A dev operator route: nothing a player says is ever sent to it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakRequest {
    pub text: String,
    /// The character whose voice speaks; none for the default voice.
    #[serde(default)]
    pub character: Option<String>,
    /// A voice id to speak in instead, whatever the character's voice is.
    #[serde(default)]
    pub voice: Option<String>,
}

impl fmt::Debug for SpeakRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SpeakRequest([REDACTED])")
    }
}

/// A voiced clip that was cued on the scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipPlayed {
    /// The clip's name in the media library.
    pub clip: String,
    pub bytes: u64,
    /// Which voice and model spoke, without the id.
    pub voice_profile: String,
    /// How long the supplier took to synthesize it.
    pub synthesis_ms: u64,
    /// The revision of the scene change that cued it.
    pub revision: u64,
}

/// What voicing a line came to, for the reply of a character's line. A failed voice never blocks
/// the line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Voiced {
    /// The line was voiced and its clip cued with the line's bubble.
    Played {
        clip: String,
        bytes: u64,
        voice_profile: String,
        synthesis_ms: u64,
    },
    /// The line could not be voiced; its bubble was cued without sound.
    Failed { error: SpeechError },
}

/// The body of `POST /dev/speech/voice`: gives a character a voice, or takes theirs away.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceRequest {
    pub character: String,
    /// The voice id; null clears the character's voice.
    pub voice: Option<String>,
}

/// What `POST /dev/speech/voice` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceChanged {
    pub character: String,
    /// Whether the character has a voice now.
    pub has_voice: bool,
    /// Whether the voices file changed.
    pub changed: bool,
}

/// A voice clip of the host in the media library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipRow {
    pub name: String,
    pub bytes: u64,
}

/// What `POST /dev/speech/clear` removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipsCleared {
    pub removed: u64,
}

/// Why a call to the speech supplier did not give a usable answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechProviderCause {
    /// The key is not a usable credential.
    Credential,
    /// The adapter refused the request before sending it.
    Rejected,
    /// The request could not be built from the host's own fixed inputs (a defect of the host,
    /// not of the caller or the supplier).
    Unbuildable,
    /// The supplier answered with something unusable (no audio, a transcript that does not read).
    Unusable,
    Deadline,
    Cancelled,
    /// The exchange failed on the wire.
    Transport,
    /// The supplier refused the key (an HTTP 401 or 403).
    Denied,
    /// The supplier refused the request as malformed or invalid (an HTTP 400 or 422).
    BadRequest,
    /// The supplier is out of capacity or rate limited.
    Capacity,
    Unavailable,
    /// Any other supplier failure.
    Supplier,
}

impl SpeechProviderCause {
    pub const ALL: [Self; 12] = [
        Self::Credential,
        Self::Rejected,
        Self::Unbuildable,
        Self::Unusable,
        Self::Deadline,
        Self::Cancelled,
        Self::Transport,
        Self::Denied,
        Self::BadRequest,
        Self::Capacity,
        Self::Unavailable,
        Self::Supplier,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Credential => "credential",
            Self::Rejected => "rejected",
            Self::Unbuildable => "unbuildable",
            Self::Unusable => "unusable",
            Self::Deadline => "deadline",
            Self::Cancelled => "cancelled",
            Self::Transport => "transport",
            Self::Denied => "denied",
            Self::BadRequest => "bad_request",
            Self::Capacity => "capacity",
            Self::Unavailable => "unavailable",
            Self::Supplier => "supplier",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl fmt::Display for SpeechProviderCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Credential => "the key is not a usable credential",
            Self::Rejected => "the request was refused before it was sent",
            Self::Unbuildable => "the request could not be built",
            Self::Unusable => "the supplier's answer was not usable",
            Self::Deadline => "the call ran out of time",
            Self::Cancelled => "the call was cancelled",
            Self::Transport => "the exchange failed",
            Self::Denied => "the supplier refused the key",
            Self::BadRequest => "the supplier refused the request as invalid",
            Self::Capacity => "the supplier is out of capacity",
            Self::Unavailable => "the supplier is unavailable",
            Self::Supplier => "the supplier failed",
        })
    }
}

/// What is wrong with a voice id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceCause {
    Empty,
    TooLong,
    /// It has a space or a control character.
    InvalidCharacters,
}

impl VoiceCause {
    pub const ALL: [Self; 3] = [Self::Empty, Self::TooLong, Self::InvalidCharacters];

    pub fn key(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TooLong => "too_long",
            Self::InvalidCharacters => "invalid_characters",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl fmt::Display for VoiceCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "it is empty",
            Self::TooLong => "it is too long",
            Self::InvalidCharacters => "it has a space or a control character",
        })
    }
}

/// Why a clip could not be kept in the media library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipCause {
    LibraryFull,
    /// The library refused the file.
    Refused,
    /// The library's folder could not be written.
    Unwritable,
}

impl ClipCause {
    pub const ALL: [Self; 3] = [Self::LibraryFull, Self::Refused, Self::Unwritable];

    pub fn key(self) -> &'static str {
        match self {
            Self::LibraryFull => "library_full",
            Self::Refused => "refused",
            Self::Unwritable => "unwritable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl fmt::Display for ClipCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LibraryFull => "the library is full",
            Self::Refused => "the library refused the file",
            Self::Unwritable => "the library's folder could not be written",
        })
    }
}

/// What is wrong with the query of a transcription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryCause {
    /// The query string could not be read at all.
    Unreadable,
    /// A key other than `terms`.
    UnknownKey,
    TooManyTerms,
    BlankTerm,
    /// A term of more than `MAXIMUM_KEY_TERM_CHARACTERS` characters.
    LongTerm,
    /// A term of more than `MAXIMUM_KEY_TERM_WORDS` words.
    WordyTerm,
    /// A term with a control or bracket character.
    InvalidTermCharacters,
}

impl QueryCause {
    pub const ALL: [Self; 7] = [
        Self::Unreadable,
        Self::UnknownKey,
        Self::TooManyTerms,
        Self::BlankTerm,
        Self::LongTerm,
        Self::WordyTerm,
        Self::InvalidTermCharacters,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Unreadable => "unreadable",
            Self::UnknownKey => "unknown_key",
            Self::TooManyTerms => "too_many_terms",
            Self::BlankTerm => "blank_term",
            Self::LongTerm => "long_term",
            Self::WordyTerm => "wordy_term",
            Self::InvalidTermCharacters => "invalid_term_characters",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl fmt::Display for QueryCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable => formatter.write_str("the query could not be read"),
            Self::UnknownKey => formatter.write_str("the only query key is terms"),
            Self::TooManyTerms => write!(formatter, "at most {MAXIMUM_KEY_TERMS} key terms"),
            Self::BlankTerm => formatter.write_str("a key term is blank"),
            Self::LongTerm => write!(
                formatter,
                "a key term is more than {MAXIMUM_KEY_TERM_CHARACTERS} characters"
            ),
            Self::WordyTerm => write!(
                formatter,
                "a key term is more than {MAXIMUM_KEY_TERM_WORDS} words"
            ),
            Self::InvalidTermCharacters => {
                formatter.write_str("a key term has a control or bracket character")
            }
        }
    }
}

/// What is wrong with the voices file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoicesCause {
    TooLarge,
    TooManyVoices,
    /// A character id in the file is not valid.
    InvalidCharacterId,
    /// A voice id in the file is not valid.
    InvalidVoiceId,
    /// The default voice id in the file is not valid.
    InvalidDefaultVoice,
    /// The file is not the JSON the host reads.
    Malformed(BodyError),
}

impl fmt::Display for VoicesCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => formatter.write_str("the file is too large"),
            Self::TooManyVoices => formatter.write_str("the file names too many voices"),
            Self::InvalidCharacterId => formatter.write_str("a character id is not valid"),
            Self::InvalidVoiceId => formatter.write_str("a voice id is not valid"),
            Self::InvalidDefaultVoice => formatter.write_str("the default voice id is not valid"),
            Self::Malformed(error) => formatter.write_str(&error.detail),
        }
    }
}

/// Why a speech request was refused. An error never carries a recording, a transcript or the
/// text that was to be voiced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum SpeechError {
    InvalidBody(BodyError),
    /// The query is not one the route reads.
    InvalidQuery {
        cause: QueryCause,
    },
    /// The speech service is not set up; `missing` says what to set.
    NotConfigured {
        missing: MissingSetting,
    },
    /// The recording is not declared as `audio/L16;rate=16000`.
    UnsupportedContentType,
    /// A WAV file that is not 16 kHz mono 16-bit PCM (found by the command line).
    UnsupportedWav,
    /// The recording has no samples.
    AudioEmpty,
    /// The recording has half a sample at its end.
    AudioMisaligned,
    AudioTooShort {
        audio_ms: u64,
        minimum_ms: u64,
    },
    AudioTooLong {
        audio_ms: u64,
        maximum_ms: u64,
    },
    /// The text to voice is blank.
    TextBlank,
    TextTooLong {
        characters: u64,
        maximum: u64,
    },
    /// The character is not a valid character id.
    InvalidCharacter {
        cause: ContentError,
    },
    InvalidVoice {
        cause: VoiceCause,
    },
    /// No voice is set for `character` and the voices file has no default.
    NoVoice {
        character: Option<String>,
    },
    /// The voices file could not be read.
    VoicesUnreadable {
        cause: IoCause,
    },
    /// The voices file is not valid.
    VoicesInvalid {
        cause: VoicesCause,
    },
    VoicesUnwritable {
        cause: IoCause,
    },
    Provider {
        cause: SpeechProviderCause,
    },
    ClipUnstored {
        cause: ClipCause,
    },
    /// The media library could not be listed.
    ClipsUnreadable {
        cause: IoCause,
    },
    /// The clip was made and kept but the scene refused to play it.
    Cue {
        cause: PlaygroundChangeError,
    },
    /// The line may leak private detail, so it was not sent to the voice supplier.
    LineWithheld,
    /// The task stopped without an answer.
    Stopped,
}

impl SpeechError {
    /// The error's key for the audit line: its kind's key, or its cause's for a provider failure.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::Provider { cause } => cause.key(),
            other => other.kind().key(),
        }
    }

    /// Which refusal this is: the `"error"` tag it is written with.
    pub fn kind(&self) -> SpeechErrorKind {
        match self {
            Self::InvalidBody(_) => SpeechErrorKind::InvalidBody,
            Self::InvalidQuery { .. } => SpeechErrorKind::InvalidQuery,
            Self::NotConfigured { .. } => SpeechErrorKind::NotConfigured,
            Self::UnsupportedContentType => SpeechErrorKind::UnsupportedContentType,
            Self::UnsupportedWav => SpeechErrorKind::UnsupportedWav,
            Self::AudioEmpty => SpeechErrorKind::AudioEmpty,
            Self::AudioMisaligned => SpeechErrorKind::AudioMisaligned,
            Self::AudioTooShort { .. } => SpeechErrorKind::AudioTooShort,
            Self::AudioTooLong { .. } => SpeechErrorKind::AudioTooLong,
            Self::TextBlank => SpeechErrorKind::TextBlank,
            Self::TextTooLong { .. } => SpeechErrorKind::TextTooLong,
            Self::InvalidCharacter { .. } => SpeechErrorKind::InvalidCharacter,
            Self::InvalidVoice { .. } => SpeechErrorKind::InvalidVoice,
            Self::NoVoice { .. } => SpeechErrorKind::NoVoice,
            Self::VoicesUnreadable { .. } => SpeechErrorKind::VoicesUnreadable,
            Self::VoicesInvalid { .. } => SpeechErrorKind::VoicesInvalid,
            Self::VoicesUnwritable { .. } => SpeechErrorKind::VoicesUnwritable,
            Self::Provider { .. } => SpeechErrorKind::Provider,
            Self::ClipUnstored { .. } => SpeechErrorKind::ClipUnstored,
            Self::ClipsUnreadable { .. } => SpeechErrorKind::ClipsUnreadable,
            Self::Cue { .. } => SpeechErrorKind::Cue,
            Self::LineWithheld => SpeechErrorKind::LineWithheld,
            Self::Stopped => SpeechErrorKind::Stopped,
        }
    }
}

impl fmt::Display for SpeechError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidQuery { cause } => write!(formatter, "the query is not valid: {cause}"),
            Self::NotConfigured { missing } => write!(
                formatter,
                "the speech service is not set up: set {}",
                missing.variable()
            ),
            Self::UnsupportedContentType => write!(
                formatter,
                "the recording must be sent as {}",
                super::limits::AUDIO_CONTENT_TYPE
            ),
            Self::UnsupportedWav => {
                formatter.write_str("the WAV file must be 16 kHz mono 16-bit PCM")
            }
            Self::AudioEmpty => formatter.write_str("the recording is empty"),
            Self::AudioMisaligned => {
                formatter.write_str("the recording ends in half a 16-bit sample")
            }
            Self::AudioTooShort {
                audio_ms,
                minimum_ms,
            } => write!(
                formatter,
                "the recording is {audio_ms} ms, shorter than the {minimum_ms} ms minimum"
            ),
            Self::AudioTooLong {
                audio_ms,
                maximum_ms,
            } => write!(
                formatter,
                "the recording is {audio_ms} ms, longer than the {maximum_ms} ms maximum"
            ),
            Self::TextBlank => formatter.write_str("the text to voice is blank"),
            Self::TextTooLong {
                characters,
                maximum,
            } => write!(
                formatter,
                "the text is {characters} characters, more than the {maximum} a voice takes"
            ),
            Self::InvalidCharacter { cause } => write!(formatter, "{cause}"),
            Self::InvalidVoice { cause } => {
                write!(formatter, "the voice id is not valid: {cause}")
            }
            Self::NoVoice {
                character: Some(character),
            } => write!(
                formatter,
                "{character:?} has no voice and there is no default (speech voice <character> <id>)"
            ),
            Self::NoVoice { character: None } => formatter.write_str(
                "there is no default voice (set one in voices.json or name a character)",
            ),
            Self::VoicesUnreadable { cause } => {
                write!(formatter, "the voices file could not be read: {cause}")
            }
            Self::VoicesInvalid { cause } => {
                write!(formatter, "the voices file is not valid: {cause}")
            }
            Self::VoicesUnwritable { cause } => {
                write!(formatter, "the voices file could not be written: {cause}")
            }
            Self::Provider { cause } => write!(formatter, "the speech supplier failed: {cause}"),
            Self::ClipUnstored { cause } => write!(
                formatter,
                "the voice clip could not be kept in the media library: {cause}"
            ),
            Self::ClipsUnreadable { cause } => {
                write!(formatter, "the media library could not be listed: {cause}")
            }
            Self::Cue { cause } => write!(formatter, "the clip could not be played: {cause}"),
            Self::LineWithheld => {
                formatter.write_str("the line may leak private detail and was not voiced")
            }
            Self::Stopped => formatter.write_str("the speech task stopped without an answer"),
        }
    }
}

impl std::error::Error for SpeechError {}

impl From<BodyError> for SpeechError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl RouteError for SpeechError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::InvalidQuery { .. }
            | Self::UnsupportedContentType
            | Self::UnsupportedWav
            | Self::AudioEmpty
            | Self::AudioMisaligned
            | Self::AudioTooShort { .. }
            | Self::TextBlank
            | Self::InvalidCharacter { .. }
            | Self::InvalidVoice { .. } => StatusCode::BAD_REQUEST,
            Self::LineWithheld => StatusCode::UNPROCESSABLE_ENTITY,
            Self::AudioTooLong { .. } | Self::TextTooLong { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::NoVoice { .. } => StatusCode::NOT_FOUND,
            Self::NotConfigured { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Provider { .. } => StatusCode::BAD_GATEWAY,
            Self::Cue { cause } => cause.status(),
            Self::VoicesUnreadable { .. }
            | Self::VoicesInvalid { .. }
            | Self::VoicesUnwritable { .. }
            | Self::ClipUnstored { .. }
            | Self::ClipsUnreadable { .. }
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
