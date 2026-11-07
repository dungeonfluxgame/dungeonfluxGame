//! What the page sends and reads when it talks to a character: the speech status, the transcript,
//! the personas and the question, named after the host's types (`df_table_host::speech::request`
//! and `persona::request`). The page cannot depend on the host crate, so it reads only what it
//! shows and ignores the other fields. The replies arrive in a `dev_reply::DevReply`. A transcript
//! and a question are private to the request: the types that hold them print no text.

use std::fmt;

use serde::{Deserialize, Serialize};

/// `GET /dev/speech/status`: whether speech to text can be used.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpeechStatus {
    pub configured: bool,
    /// What to set when it is not configured.
    #[serde(default)]
    pub missing: Option<String>,
}

/// `POST /dev/speech/transcribe`: the text of a recording. It prints no text.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Transcript {
    pub text: String,
    #[serde(default)]
    pub duration_ms: u64,
}

impl fmt::Debug for Transcript {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Transcript")
            .field("characters", &self.text.chars().count())
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

/// One persona in `GET /dev/persona`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PersonaSummary {
    pub id: String,
    pub display_name: String,
}

/// The body of `POST /dev/persona/ask`. It prints no text.
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
