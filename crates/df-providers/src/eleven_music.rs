//! ElevenLabs music and ambience generation for a place's adaptive score.
//!
//! Two bounded calls share the voice route's credential header and the owned native transport:
//! `POST /v1/music` composes an instrumental from a text prompt, and
//! `POST /v1/sound-generation` makes a loopable ambience bed. The adapter never retries, never
//! rewords a prompt after a failure and never reroutes; the caller owns spend, caching and
//! de-duplication. Prompts and result bytes are never logged or placed in errors.
//!
//! Model ids, parameters and the loop flag follow `assets/music/source-manifest.json`, which
//! records the calls that produced the shipped score and ambience beds.

use std::{fmt, net::SocketAddr, time::Duration};

use serde_json::json;

use crate::native_http::{NativeHttp, NativeResponseProfile};
use crate::{HttpRequest, NativeHttpLimits, NativeHttpRefusal};

const LIVE_BASE: &str = "https://api.elevenlabs.io";
const MUSIC_MODEL: &str = "music_v2";
const SOUND_MODEL: &str = "eleven_text_to_sound_v2";
const OUTPUT_FORMAT: &str = "mp3_44100_128";
const MAXIMUM_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAXIMUM_REQUEST_BYTES: usize = 32 * 1024;
/// `/v1/music` accepts prompts up to 4100 characters.
pub const MUSIC_MAXIMUM_PROMPT_CHARACTERS: usize = 4_100;
/// `/v1/music` composes 3 seconds to 10 minutes.
pub const MUSIC_MINIMUM_LENGTH_MILLIS: u32 = 3_000;
pub const MUSIC_MAXIMUM_LENGTH_MILLIS: u32 = 600_000;
/// `/v1/sound-generation` makes at most 30 seconds.
pub const AMBIENCE_MAXIMUM_SECONDS: u32 = 30;
const AMBIENCE_MAXIMUM_CHARACTERS: usize = 1_000;
/// The manifest's value for every ambience bed.
const AMBIENCE_PROMPT_INFLUENCE: f64 = 0.5;

/// Why a call produced no audio. Never carries a prompt, audio or the supplier's text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElevenMusicError {
    /// The request failed local validation and nothing was sent.
    InvalidRequest,
    /// The supplier rejected the credential.
    Unauthorized,
    /// The supplier answered with another failure status, or was rate limited.
    Provider(u16),
    Transport(NativeHttpRefusal),
    /// A success answer with no audio in it.
    Output,
}

/// Where the adapter sends calls.
pub struct ElevenMusicConfig {
    fixture: Option<SocketAddr>,
}

impl ElevenMusicConfig {
    pub fn live() -> Self {
        Self { fixture: None }
    }

    /// A literal loopback socket for tests.
    pub fn loopback_fixture(address: SocketAddr) -> Result<Self, ElevenMusicError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(ElevenMusicError::InvalidRequest);
        }
        Ok(Self {
            fixture: Some(address),
        })
    }
}

/// One MP3 answer, as the supplier sent it.
pub struct GeneratedAudio {
    pub bytes: Vec<u8>,
}

impl fmt::Debug for GeneratedAudio {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeneratedAudio")
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

pub struct NativeElevenMusicProvider {
    base: String,
    credential: String,
    http: NativeHttp,
}

impl fmt::Debug for NativeElevenMusicProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeElevenMusicProvider")
            .finish_non_exhaustive()
    }
}

impl NativeElevenMusicProvider {
    /// `total_timeout` bounds each call, including the response body.
    pub fn new(
        config: ElevenMusicConfig,
        credential: &str,
        total_timeout: Duration,
    ) -> Result<Self, ElevenMusicError> {
        if credential.is_empty()
            || credential.len() > 4096
            || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(ElevenMusicError::InvalidRequest);
        }
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5).min(total_timeout),
            read_timeout: total_timeout,
            total_timeout,
            maximum_request_bytes: MAXIMUM_REQUEST_BYTES,
            maximum_response_bytes: MAXIMUM_RESPONSE_BYTES,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 65_536,
        };
        let http = NativeHttp::new(limits, config.fixture.is_some())
            .map_err(|_| ElevenMusicError::InvalidRequest)?;
        Ok(Self {
            base: config.fixture.map_or_else(
                || LIVE_BASE.to_owned(),
                |address| format!("http://{address}"),
            ),
            credential: credential.to_owned(),
            http,
        })
    }

    /// Composes an instrumental of `length_millis` from `prompt`; one call, never retried.
    /// `deadline` is the time this call may still take.
    pub async fn compose(
        &self,
        prompt: &str,
        length_millis: u32,
        deadline: Duration,
    ) -> Result<GeneratedAudio, ElevenMusicError> {
        if prompt.trim().is_empty()
            || prompt.chars().count() > MUSIC_MAXIMUM_PROMPT_CHARACTERS
            || !(MUSIC_MINIMUM_LENGTH_MILLIS..=MUSIC_MAXIMUM_LENGTH_MILLIS).contains(&length_millis)
        {
            return Err(ElevenMusicError::InvalidRequest);
        }
        let body = json!({
            "prompt": prompt,
            "music_length_ms": length_millis,
            "model_id": MUSIC_MODEL,
            "force_instrumental": true,
        });
        self.post("/v1/music", &body, deadline).await
    }

    /// Makes a loopable ambience bed of `seconds` from `description`; one call, never retried.
    pub async fn ambience(
        &self,
        description: &str,
        seconds: u32,
        deadline: Duration,
    ) -> Result<GeneratedAudio, ElevenMusicError> {
        if description.trim().is_empty()
            || description.chars().count() > AMBIENCE_MAXIMUM_CHARACTERS
            || !(1..=AMBIENCE_MAXIMUM_SECONDS).contains(&seconds)
        {
            return Err(ElevenMusicError::InvalidRequest);
        }
        let body = json!({
            "text": description,
            "duration_seconds": seconds,
            "prompt_influence": AMBIENCE_PROMPT_INFLUENCE,
            "loop": true,
            "model_id": SOUND_MODEL,
        });
        self.post("/v1/sound-generation", &body, deadline).await
    }

    async fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
        deadline: Duration,
    ) -> Result<GeneratedAudio, ElevenMusicError> {
        let body = serde_json::to_vec(body).map_err(|_| ElevenMusicError::InvalidRequest)?;
        let request = HttpRequest::elevenlabs_native(
            &format!("{}{path}?output_format={OUTPUT_FORMAT}", self.base),
            &self.credential,
            "application/json".to_owned(),
            body,
        );
        let response = self
            .http
            .execute_profile(
                request,
                deadline,
                std::future::pending(),
                NativeResponseProfile::AudioMpeg,
            )
            .await
            .map_err(|failure| ElevenMusicError::Transport(failure.reason))?;
        match response.status {
            200 => {}
            401 | 403 => return Err(ElevenMusicError::Unauthorized),
            status => return Err(ElevenMusicError::Provider(status)),
        }
        if response.body.is_empty() {
            return Err(ElevenMusicError::Output);
        }
        Ok(GeneratedAudio {
            bytes: response.body,
        })
    }
}
