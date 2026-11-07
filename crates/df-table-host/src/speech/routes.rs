//! The speech routes. Each answers a `SpeechReply` and records a refusal as one
//! `SPEECH refused verb=.. error=<key>` line; the lines of the work itself are in `service`.
//!
//! These are dev operator routes. `speak` voices text the operator types and plays it; the
//! player-facing path voices only the line a character's brief produced (`persona ask`/`say`).
//! `transcribe` takes a recording as the raw body and answers a transcript the page puts in an
//! editable draft. Every paid call runs on a task of its own that the handler waits for, so a
//! client that leaves cannot lose an answer already paid for.

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::HeaderMap;
use axum::http::header::CONTENT_TYPE;
use axum::response::Response;
use axum::routing::{get, post};
use serde::Serialize;

use super::clips::{clear_clips, clips};
use super::limits::MAXIMUM_ADMITTED_AUDIO_BYTES;
use super::request::{
    ClipPlayed, ClipRow, ClipsCleared, SpeakRequest, SpeechError, SpeechStatus, Transcript,
    VoiceChanged, VoiceRequest,
};
use super::service::Speech;
use super::voices::{change_voice, voices_path};
use crate::blocking::{run_blocking, run_owned};
use crate::dev_http::{parse_body, reply};

/// The addresses of the speech dev API.
pub mod path {
    // The page reads these two, so the page's addresses are the ones served.
    pub use df_playground::path::{SPEECH_STATUS, SPEECH_TRANSCRIBE};
    /// Voices operator text and plays it (POST a `SpeakRequest`). A dev operator route.
    pub const SPEECH_SPEAK: &str = "/dev/speech/speak";
    /// Gives a character a voice or takes it away (POST a `VoiceRequest`).
    pub const SPEECH_VOICE: &str = "/dev/speech/voice";
    /// The voice clips the host keeps (GET).
    pub const SPEECH_CLIPS: &str = "/dev/speech/clips";
    /// Deletes every voice clip the host made (POST, no body).
    pub const SPEECH_CLEAR: &str = "/dev/speech/clear";
}

/// Answers `result`, recording a refusal first as `SPEECH refused verb=.. error=<key>`.
fn answer<T: Serialize>(speech: &Speech, verb: &str, result: Result<T, SpeechError>) -> Response {
    if let Err(error) = &result {
        speech
            .app()
            .record_line(format!("SPEECH refused verb={verb} error={}", error.key()));
    }
    reply(result)
}

/// That the body is declared as 16 kHz 16-bit PCM: `audio/L16;rate=16000`, optionally with
/// `channels=1`, in any case.
pub(super) fn check_content_type(headers: &HeaderMap) -> Result<(), SpeechError> {
    let declared = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or(SpeechError::UnsupportedContentType)?
        .to_ascii_lowercase();
    let mut parts = declared.split(';').map(str::trim);
    let (mut rate, mut channels_ok) = (false, true);
    if parts.next() != Some("audio/l16") {
        return Err(SpeechError::UnsupportedContentType);
    }
    for part in parts {
        match part.split_once('=') {
            Some(("rate", "16000")) => rate = true,
            Some(("channels", "1")) => {}
            Some(("channels", _)) => channels_ok = false,
            _ => {}
        }
    }
    if rate && channels_ok {
        Ok(())
    } else {
        Err(SpeechError::UnsupportedContentType)
    }
}

/// The key terms of the query: every `terms=` pair, in order. Another key is refused.
fn key_terms(
    query: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Result<Vec<String>, SpeechError> {
    let Query(pairs) = query.map_err(|rejection| SpeechError::InvalidQuery {
        detail: rejection.body_text(),
    })?;
    let mut terms = Vec::new();
    for (key, value) in pairs {
        if key != "terms" {
            return Err(SpeechError::InvalidQuery {
                detail: "the only query key is terms".to_owned(),
            });
        }
        terms.push(value);
    }
    Ok(terms)
}

async fn status(State(speech): State<Speech>) -> Response {
    let result = speech.status().await;
    answer::<SpeechStatus>(&speech, "status", result)
}

async fn transcribe(
    State(speech): State<Speech>,
    headers: HeaderMap,
    query: Result<Query<Vec<(String, String)>>, QueryRejection>,
    body: Bytes,
) -> Response {
    let checked = check_content_type(&headers).and_then(|()| key_terms(query));
    let result = match checked {
        Ok(terms) => {
            let owned = speech.clone();
            run_owned(
                async move { owned.transcribe(Vec::from(body), terms).await },
                SpeechError::Stopped,
            )
            .await
        }
        Err(error) => Err(error),
    };
    answer::<Transcript>(&speech, "transcribe", result)
}

async fn speak(State(speech): State<Speech>, bytes: Bytes) -> Response {
    let result = match parse_body::<SpeakRequest>(&bytes) {
        Ok(request) => {
            let owned = speech.clone();
            run_owned(
                async move { owned.speak(request).await },
                SpeechError::Stopped,
            )
            .await
        }
        Err(error) => Err(error.into()),
    };
    answer::<ClipPlayed>(&speech, "speak", result)
}

async fn voice(State(speech): State<Speech>, bytes: Bytes) -> Response {
    let result = match parse_body::<VoiceRequest>(&bytes) {
        Ok(request) => {
            let path = voices_path(speech.app());
            run_blocking(
                move || change_voice(&path, &request.character, request.voice.as_deref()),
                SpeechError::Stopped,
            )
            .await
        }
        Err(error) => Err(error.into()),
    };
    if let Ok(changed) = &result {
        speech.app().record_line(format!(
            "SPEECH voice_set character={} has_voice={} changed={}",
            changed.character, changed.has_voice, changed.changed
        ));
    }
    answer::<VoiceChanged>(&speech, "voice", result)
}

async fn list_clips(State(speech): State<Speech>) -> Response {
    let result = clips(speech.app()).await;
    answer::<Vec<ClipRow>>(&speech, "clips", result)
}

async fn clear(State(speech): State<Speech>) -> Response {
    let result = clear_clips(speech.app())
        .await
        .map(|removed| ClipsCleared { removed });
    if let Ok(cleared) = &result {
        speech
            .app()
            .record_line(format!("SPEECH cleared removed={}", cleared.removed));
    }
    answer::<ClipsCleared>(&speech, "clear", result)
}

/// The speech routes over `speech`.
pub fn router_with(speech: Speech) -> Router {
    Router::new()
        .route(path::SPEECH_STATUS, get(status))
        .route(
            path::SPEECH_TRANSCRIBE,
            post(transcribe).layer(DefaultBodyLimit::max(MAXIMUM_ADMITTED_AUDIO_BYTES)),
        )
        .route(path::SPEECH_SPEAK, post(speak))
        .route(path::SPEECH_VOICE, post(voice))
        .route(path::SPEECH_CLIPS, get(list_clips))
        .route(path::SPEECH_CLEAR, post(clear))
        .with_state(speech)
}
