//! What the page and the host exchange when someone talks to a character: the speech status, the
//! transcript, the personas, the question and the closed keys of a refusal. The page owns these
//! types and the host (`df_table_host::speech::request` and `persona::request`) re-exports them,
//! so the JSON cannot drift. A transcript and a question are private to the request: the types
//! that hold them print no text. The replies arrive in a `dev_reply::DevReply`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The characters with a voice of their own, and whose voice is which. The voice id is an opaque
/// backend reference, not a permission or an identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceRow {
    pub character: String,
    pub voice: String,
}

/// The bounds a caller works within.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechLimits {
    pub minimum_audio_ms: u64,
    pub maximum_audio_ms: u64,
    pub maximum_text_characters: u64,
    pub maximum_key_terms: u64,
    pub maximum_kept_clips: u64,
}

/// What the host needs set before speech works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingSetting {
    /// The speech supplier's key.
    SpeechKey,
}

impl MissingSetting {
    pub const ALL: [Self; 1] = [Self::SpeechKey];

    pub fn key(self) -> &'static str {
        match self {
            Self::SpeechKey => "speech_key",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|setting| setting.key() == key)
    }

    /// The environment variable that sets it (the first of the variables the host reads).
    pub const fn variable(self) -> &'static str {
        match self {
            Self::SpeechKey => "DF_ELEVENLABS_API_KEY",
        }
    }
}

/// `GET /dev/speech/status`: how the speech service is set up and how it stands. The page enables
/// its microphone when `configured`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechStatus {
    pub configured: bool,
    /// What to set when the service is not configured.
    pub missing: Option<MissingSetting>,
    pub tts_model: String,
    pub stt_model: String,
    /// The voice a character without a voice of their own is spoken in, if one is set.
    pub default_voice: Option<String>,
    pub voices: Vec<VoiceRow>,
    /// Voice clips in the media library now.
    pub clips_kept: u64,
    pub limits: SpeechLimits,
}

/// `POST /dev/speech/transcribe`: what a transcription found, the words, the language and how long
/// it took. The recording and this text exist only for the request; nothing stores or logs them.
/// It prints no text.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    pub language: String,
    /// How long the recording was.
    pub audio_ms: u64,
    /// How long the supplier took.
    pub duration_ms: u64,
}

impl fmt::Debug for Transcript {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Transcript")
            .field("characters", &self.text.chars().count())
            .field("audio_ms", &self.audio_ms)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

/// Which refusal a speech route answered: the `"error"` tag of the host's `SpeechError`, without
/// its fields. The page reads only this, to word what went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechErrorKind {
    InvalidBody,
    InvalidQuery,
    NotConfigured,
    UnsupportedContentType,
    UnsupportedWav,
    AudioEmpty,
    AudioMisaligned,
    AudioTooShort,
    AudioTooLong,
    TextBlank,
    TextTooLong,
    InvalidCharacter,
    InvalidVoice,
    NoVoice,
    VoicesUnreadable,
    VoicesInvalid,
    VoicesUnwritable,
    Provider,
    ClipUnstored,
    ClipsUnreadable,
    Cue,
    Disabled,
    Stopped,
}

impl SpeechErrorKind {
    pub const ALL: [Self; 23] = [
        Self::InvalidBody,
        Self::InvalidQuery,
        Self::NotConfigured,
        Self::UnsupportedContentType,
        Self::UnsupportedWav,
        Self::AudioEmpty,
        Self::AudioMisaligned,
        Self::AudioTooShort,
        Self::AudioTooLong,
        Self::TextBlank,
        Self::TextTooLong,
        Self::InvalidCharacter,
        Self::InvalidVoice,
        Self::NoVoice,
        Self::VoicesUnreadable,
        Self::VoicesInvalid,
        Self::VoicesUnwritable,
        Self::Provider,
        Self::ClipUnstored,
        Self::ClipsUnreadable,
        Self::Cue,
        Self::Disabled,
        Self::Stopped,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::InvalidBody => "invalid_body",
            Self::InvalidQuery => "invalid_query",
            Self::NotConfigured => "not_configured",
            Self::UnsupportedContentType => "unsupported_content_type",
            Self::UnsupportedWav => "unsupported_wav",
            Self::AudioEmpty => "audio_empty",
            Self::AudioMisaligned => "audio_misaligned",
            Self::AudioTooShort => "audio_too_short",
            Self::AudioTooLong => "audio_too_long",
            Self::TextBlank => "text_blank",
            Self::TextTooLong => "text_too_long",
            Self::InvalidCharacter => "invalid_character",
            Self::InvalidVoice => "invalid_voice",
            Self::NoVoice => "no_voice",
            Self::VoicesUnreadable => "voices_unreadable",
            Self::VoicesInvalid => "voices_invalid",
            Self::VoicesUnwritable => "voices_unwritable",
            Self::Provider => "provider",
            Self::ClipUnstored => "clip_unstored",
            Self::ClipsUnreadable => "clips_unreadable",
            Self::Cue => "cue",
            Self::Disabled => "disabled",
            Self::Stopped => "stopped",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// A half of the voice pipeline the playground can turn off (`Controls::tts_on`, `stt_on`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechFeature {
    /// Voicing lines (text to speech).
    Tts,
    /// Taking spoken questions (speech to text).
    Stt,
}

impl SpeechFeature {
    pub const ALL: [Self; 2] = [Self::Tts, Self::Stt];

    pub fn key(self) -> &'static str {
        match self {
            Self::Tts => "tts",
            Self::Stt => "stt",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|feature| feature.key() == key)
    }
}

/// A stored persona in a list: who, and what they do. Nothing private.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaSummary {
    pub id: String,
    pub display_name: String,
    pub job: String,
}

/// The body of `POST /dev/persona/ask` as the page writes it: a spoken question. The host reads a
/// superset (`persona::request::AskRequest`, which adds optional guidance, model and situation).
/// It prints no text.
#[derive(Clone, Serialize)]
pub struct AskRequest<'a> {
    pub character: &'a str,
    pub question: &'a str,
    /// Have the character say the answer aloud.
    pub speak: bool,
    pub coverage: bool,
    pub backfill: bool,
}

impl<'a> AskRequest<'a> {
    /// A spoken question the host answers at once: no coverage check and no backfill first.
    pub fn spoken(character: &'a str, question: &'a str) -> Self {
        Self {
            character,
            question,
            speak: true,
            coverage: false,
            backfill: false,
        }
    }
}

impl fmt::Debug for AskRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AskRequest")
            .field("character", &self.character)
            .field("characters", &self.question.chars().count())
            .finish()
    }
}
