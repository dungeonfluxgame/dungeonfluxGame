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

use crate::native_http::{
    NativeHttp, NativeResponseProfile, is_loopback_socket, is_valid_credential,
};
use crate::{HttpRequest, NativeHttpCause, ProviderLimits};

const LIVE_BASE: &str = "https://api.elevenlabs.io";
/// The model `/v1/music` is asked for.
pub const MUSIC_MODEL: &str = "music_v2";
/// The model `/v1/sound-generation` is asked for.
pub const SOUND_MODEL: &str = "eleven_text_to_sound_v2";
const OUTPUT_FORMAT: &str = "mp3_44100_128";
/// The largest audio answer kept, in bytes. The longest composition (10 minutes at the 128 kbit/s
/// of `OUTPUT_FORMAT`) is 9.6 MB, so this leaves ample room; the old 8 MiB refused every
/// composition longer than about 8.7 minutes.
const MAXIMUM_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
/// The largest request body sent, in bytes (a prompt is at most a few KiB of JSON).
const MAXIMUM_REQUEST_BYTES: usize = 1024 * 1024;
/// `/v1/music` accepts prompts up to 4100 characters.
pub const MUSIC_MAXIMUM_PROMPT_CHARACTERS: usize = 4_100;
/// `/v1/music` composes 3 seconds to 10 minutes; the shortest, in milliseconds.
pub const MUSIC_MINIMUM_LENGTH_MS: u32 = 3_000;
/// The longest composition `/v1/music` makes, in milliseconds.
pub const MUSIC_MAXIMUM_LENGTH_MS: u32 = 600_000;
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
    /// The transport refused or failed the exchange.
    Transport(NativeHttpCause),
    /// A success answer with no audio in it.
    Output,
}

impl fmt::Display for ElevenMusicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest => formatter.write_str("the music request is invalid"),
            Self::Unauthorized => formatter.write_str("the music credential was rejected"),
            Self::Provider(status) => write!(formatter, "the music service answered {status}"),
            Self::Transport(cause) => write!(formatter, "the music call failed: {cause}"),
            Self::Output => formatter.write_str("the music answer carried no audio"),
        }
    }
}

impl std::error::Error for ElevenMusicError {}

/// Where the adapter sends calls.
pub struct ElevenMusicConfig {
    fixture: Option<SocketAddr>,
}

impl ElevenMusicConfig {
    /// Calls go to the live ElevenLabs API.
    pub fn live() -> Self {
        Self { fixture: None }
    }

    /// A literal loopback socket for tests.
    pub fn loopback_fixture(address: SocketAddr) -> Result<Self, ElevenMusicError> {
        if !is_loopback_socket(address) {
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

/// The music and ambience client; it never retries and holds no cache.
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
        if !is_valid_credential(credential) {
            return Err(ElevenMusicError::InvalidRequest);
        }
        let limits =
            ProviderLimits::new(total_timeout, MAXIMUM_REQUEST_BYTES, MAXIMUM_RESPONSE_BYTES)
                .build();
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

    /// Composes an instrumental of `length_ms` from `prompt`; one call, never retried.
    /// `deadline` is the time this call may still take.
    pub async fn compose(
        &self,
        prompt: &str,
        length_ms: u32,
        deadline: Duration,
    ) -> Result<GeneratedAudio, ElevenMusicError> {
        if prompt.trim().is_empty()
            || prompt.chars().count() > MUSIC_MAXIMUM_PROMPT_CHARACTERS
            || !(MUSIC_MINIMUM_LENGTH_MS..=MUSIC_MAXIMUM_LENGTH_MS).contains(&length_ms)
        {
            return Err(ElevenMusicError::InvalidRequest);
        }
        let body = json!({
            "prompt": prompt,
            "music_length_ms": length_ms,
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
            .map_err(|failure| ElevenMusicError::Transport(failure.cause))?;
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
