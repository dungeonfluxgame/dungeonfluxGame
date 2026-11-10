//! The Forge adapter of the speech port: Qwen3-TTS voices a line as a 24 kHz WAV, and Nemotron
//! transcribes a recording, both on the owner's own machine (`df_providers::ForgeClient`).
//!
//! A voice id names a Forge preset speaker when it is one (`speech voice <character> Aiden`);
//! any other id (an ElevenLabs id in `voices.json`) maps to one preset speaker by a stable hash,
//! so a character keeps one voice. Nemotron is English only and has no word boosting, so the key
//! terms are not sent and the language is always English. Each call first makes sure its engine
//! runs (one engine, through Forge's own memory-checked start); Forge loads the model when the
//! queue first needs it.

use df_providers::{
    FORGE_STT_MODEL, FORGE_TTS_MAXIMUM_CHARACTERS, FORGE_TTS_MODEL, ForgeCause, ForgeClient,
    ForgeEngine, ForgeError, ForgeJobKind, ForgeMedia, NativeHttpCause, forge_speaker,
};
use tokio::time::Instant;

use super::{
    ClipFormat, PortFuture, PortStatus, SpeechPort, Synthesis, SynthesisCall, Transcription,
    TranscriptionCall, VoiceListing,
};
use crate::forge::{ForgeSetupCause, ForgeSetupError, client_from_environment};
use crate::speech::limits::SPEECH_DEADLINE;
use crate::speech::request::{MissingSetting, SpeechError, SpeechProviderCause};

/// The language Forge is asked to speak in and reports hearing (Nemotron is English only).
const LANGUAGE: &str = "English";
/// The language tag a transcript carries.
const HEARD_LANGUAGE: &str = "en";
/// The sample rate of the recordings the host transcribes (its input contract).
const RECORDING_SAMPLE_RATE_HZ: u32 = 16_000;

/// The Forge adapter, over the client the environment describes when it was built.
pub struct ForgeSpeechPort {
    client: Result<ForgeClient, ForgeSetupError>,
}

impl ForgeSpeechPort {
    pub fn from_environment() -> Self {
        Self::new(client_from_environment())
    }

    pub fn new(client: Result<ForgeClient, ForgeSetupError>) -> Self {
        Self { client }
    }

    fn ready(&self) -> Result<&ForgeClient, SpeechError> {
        self.client.as_ref().map_err(|error| match error.cause {
            ForgeSetupCause::ProjectMissing => SpeechError::NotConfigured {
                missing: MissingSetting::ForgeProject,
            },
            ForgeSetupCause::InvalidSetting => SpeechError::Provider {
                cause: SpeechProviderCause::Unbuildable,
            },
        })
    }
}

impl SpeechPort for ForgeSpeechPort {
    fn status(&self) -> PortStatus {
        PortStatus {
            configured: self.client.is_ok(),
            missing: self
                .client
                .as_ref()
                .err()
                .map(|_| MissingSetting::ForgeProject),
            tts_model: FORGE_TTS_MODEL,
            stt_model: FORGE_STT_MODEL,
        }
    }

    fn synthesize(&self, call: SynthesisCall) -> PortFuture<'_, Result<Synthesis, SpeechError>> {
        Box::pin(async move { speak(self.ready()?, call).await })
    }

    fn transcribe(
        &self,
        call: TranscriptionCall,
    ) -> PortFuture<'_, Result<Transcription, SpeechError>> {
        Box::pin(async move { listen(self.ready()?, call).await })
    }

    /// Forge keeps no list a voice is chosen from: any voice id maps to a preset speaker.
    fn voices(&self) -> PortFuture<'_, Result<VoiceListing, SpeechError>> {
        Box::pin(std::future::ready(Ok(VoiceListing::Unlisted)))
    }
}

async fn speak(client: &ForgeClient, call: SynthesisCall) -> Result<Synthesis, SpeechError> {
    let characters = call.text.chars().count();
    if characters > FORGE_TTS_MAXIMUM_CHARACTERS {
        return Err(SpeechError::TextTooLong {
            characters: u64::try_from(characters).unwrap_or(u64::MAX),
            maximum: u64::try_from(FORGE_TTS_MAXIMUM_CHARACTERS).unwrap_or(u64::MAX),
        });
    }
    let until = Instant::now() + SPEECH_DEADLINE;
    client
        .prepare(ForgeEngine::Tts, until)
        .await
        .map_err(provider_error)?;
    let request = serde_json::json!({
        "text": call.text,
        "speaker": forge_speaker(&call.voice),
        "language": LANGUAGE,
    });
    let render = client
        .render(ForgeJobKind::Voices, request, until)
        .await
        .map_err(provider_error)?;
    let audio = client
        .asset(&render.output, ForgeMedia::Wav, until)
        .await
        .map_err(provider_error)?;
    Ok(Synthesis {
        audio,
        format: ClipFormat::Wav,
    })
}

async fn listen(
    client: &ForgeClient,
    call: TranscriptionCall,
) -> Result<Transcription, SpeechError> {
    let until = Instant::now() + SPEECH_DEADLINE;
    client
        .prepare(ForgeEngine::Asr, until)
        .await
        .map_err(provider_error)?;
    let text = client
        .transcribe(&call.pcm, RECORDING_SAMPLE_RATE_HZ, until)
        .await
        .map_err(provider_error)?;
    let words = text.split_whitespace().count();
    Ok(Transcription {
        text,
        language: HEARD_LANGUAGE.to_owned(),
        words,
    })
}

/// The speech cause of a Forge failure.
fn provider_error(error: ForgeError) -> SpeechError {
    let cause = match error.cause {
        ForgeCause::InvalidConfiguration => SpeechProviderCause::Unbuildable,
        ForgeCause::InvalidRequest | ForgeCause::Rejected => SpeechProviderCause::BadRequest,
        ForgeCause::Transport(NativeHttpCause::Deadline) | ForgeCause::Deadline => {
            SpeechProviderCause::Deadline
        }
        ForgeCause::Transport(NativeHttpCause::Cancelled) | ForgeCause::JobCancelled => {
            SpeechProviderCause::Cancelled
        }
        ForgeCause::Transport(_) => SpeechProviderCause::Transport,
        ForgeCause::Malformed => SpeechProviderCause::Unusable,
        ForgeCause::EngineUnavailable
        | ForgeCause::JobInterrupted
        | ForgeCause::JobNeedsAttention
        | ForgeCause::QueueWaitingMemory
        | ForgeCause::QueueInterrupted
        | ForgeCause::QueueNeedsAttention => SpeechProviderCause::Unavailable,
        ForgeCause::Failed | ForgeCause::JobFailed | ForgeCause::Session => {
            SpeechProviderCause::Supplier
        }
    };
    SpeechError::Provider { cause }
}

/// The port of a `DF_SPEECH_BACKEND` value the host does not know: it is not configured and
/// names the variable, so nothing is sent to any supplier.
pub(super) struct UnknownBackendPort;

impl UnknownBackendPort {
    fn refusal() -> SpeechError {
        SpeechError::NotConfigured {
            missing: MissingSetting::SpeechBackend,
        }
    }
}

impl SpeechPort for UnknownBackendPort {
    fn status(&self) -> PortStatus {
        PortStatus {
            configured: false,
            missing: Some(MissingSetting::SpeechBackend),
            tts_model: "-",
            stt_model: "-",
        }
    }

    fn synthesize(&self, _call: SynthesisCall) -> PortFuture<'_, Result<Synthesis, SpeechError>> {
        Box::pin(std::future::ready(Err(Self::refusal())))
    }

    fn transcribe(
        &self,
        _call: TranscriptionCall,
    ) -> PortFuture<'_, Result<Transcription, SpeechError>> {
        Box::pin(std::future::ready(Err(Self::refusal())))
    }

    fn voices(&self) -> PortFuture<'_, Result<VoiceListing, SpeechError>> {
        Box::pin(std::future::ready(Err(Self::refusal())))
    }
}
