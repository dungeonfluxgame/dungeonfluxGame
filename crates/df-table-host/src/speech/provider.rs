//! The one port to the speech supplier, and its adapters: ElevenLabs (the default) and the
//! owner's local Forge studio (`DF_SPEECH_BACKEND=forge`).
//!
//! [`SpeechPort`] is the whole surface the host asks of a supplier: whether it is set up and with
//! which models, text to one buffered clip (MP3 or WAV), 16 kHz mono PCM to a transcript, and the
//! voices it can speak in (for casting a character's voice; a supplier may keep no list). The
//! host's routes and the persona routes know only the port, so the planned `AudioService`
//! (`df-media`) can replace either adapter without touching them. [`port_from_environment`]
//! picks one. The ElevenLabs adapter builds each request the way the
//! `df-providers` speech contract wants it and calls `NativeSpeechProvider`; it never calls the
//! supplier by hand. A dev host has no session, run or revision to bind a request to, so each
//! call gets a fixed dev basis and a job that counts up; the zero content digest says the request
//! is tied to no stored asset.
//!
//! The recording, the text and the key never leave a call as anything but the supplier's request:
//! errors carry a closed [`SpeechProviderCause`] only.

use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use df_model::checkpoint::{Basis, ContentDigest, ExecutionMode, JobId};
use df_provider_api::{
    ProviderFailureClass, RequestBasis, RequestBinding, RequestIdentity, RequestLimits,
    RequestModality, RequestOwnerState, RequestUsage, SttEventKind, SttSemantics, TtsEventKind,
    TtsSemantics,
};
use df_providers::{
    CandidateRouteId, ELEVEN_STT_LANGUAGE, ELEVEN_STT_MODEL, ELEVEN_TTS_LANGUAGE, ELEVEN_TTS_MODEL,
    ElevenSpeechRequest, ElevenSttRequest, NativeHttpCause, NativeSpeechProvider, ProviderLimits,
    SpeechAdapterCause, SpeechAdapterError, SpeechInputFormat, SpeechOutputFormat,
    SpeechTranscriptProfile, SupplierVoice, TranscriptWordKind, registered_speech_provider,
};
use df_types::{
    LocaleTag, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage,
    UsageUnit,
};

use super::limits::{
    KEY_VARIABLES, MAXIMUM_ADMITTED_AUDIO_BYTES, MAXIMUM_ADMITTED_TEXT_BYTES, MAXIMUM_AUDIO_MS,
    MAXIMUM_PROVIDER_REQUEST_BYTES, MAXIMUM_PROVIDER_RESPONSE_BYTES, MAXIMUM_TEXT_CHARACTERS,
    SPEECH_DEADLINE, VOICE_LIST_DEADLINE,
};
use super::request::{MissingSetting, SpeechError, SpeechProviderCause};
use crate::environment;
use crate::forge::FORGE_BACKEND;

mod forge;

pub use self::forge::ForgeSpeechPort;
use self::forge::UnknownBackendPort;

/// A future the port returns, so the port is usable as a trait object.
pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Text to voice and the voice to voice it in. The text is only what the host produced or the
/// operator typed.
pub struct SynthesisCall {
    pub text: String,
    /// The supplier's voice id.
    pub voice: String,
}

/// The container of a voiced clip, which names its file in the media library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipFormat {
    Mp3,
    Wav,
}

impl ClipFormat {
    /// The file suffix of a clip in this format.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Mp3 => ".mp3",
            Self::Wav => ".wav",
        }
    }
}

/// One buffered clip.
pub struct Synthesis {
    pub audio: Vec<u8>,
    pub format: ClipFormat,
}

/// A recording and the names that bias recognition. The recording is request-lived memory.
pub struct TranscriptionCall {
    /// Signed 16-bit little-endian mono PCM at 16 kHz.
    pub pcm: Vec<u8>,
    pub key_terms: Vec<String>,
}

/// What the supplier heard.
pub struct Transcription {
    pub text: String,
    pub language: String,
    /// Words in the transcript (spacing and sound markers not counted).
    pub words: usize,
}

/// Whether the supplier is set up, and the models it speaks and listens with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortStatus {
    pub configured: bool,
    /// The variable to set when it is not.
    pub missing: Option<MissingSetting>,
    /// The text-to-speech model, for status and clip profiles.
    pub tts_model: &'static str,
    /// The speech-to-text model.
    pub stt_model: &'static str,
}

/// The voices a supplier can speak in, or that it keeps no list a voice can be chosen from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceListing {
    /// The supplier's voices, in its order.
    Listed(Vec<SupplierVoice>),
    /// The supplier keeps no list (Forge maps any voice id to a preset speaker).
    Unlisted,
}

/// The surface the host asks of a speech supplier.
pub trait SpeechPort: Send + Sync {
    fn status(&self) -> PortStatus;
    fn synthesize(&self, call: SynthesisCall) -> PortFuture<'_, Result<Synthesis, SpeechError>>;
    fn transcribe(
        &self,
        call: TranscriptionCall,
    ) -> PortFuture<'_, Result<Transcription, SpeechError>>;
    /// The voices a character's voice can be cast from.
    fn voices(&self) -> PortFuture<'_, Result<VoiceListing, SpeechError>>;
}

/// The variable that picks the speech supplier: `elevenlabs` (unset) or `forge`.
pub const BACKEND_VARIABLE: &str = MissingSetting::SpeechBackend.variable();
/// The value of `BACKEND_VARIABLE` that picks ElevenLabs, as unset does.
pub const ELEVENLABS_BACKEND: &str = "elevenlabs";
/// The name of ElevenLabs' text-to-speech model, for status and clip profiles.
pub const TTS_MODEL: &str = ELEVEN_TTS_MODEL;
/// The name of ElevenLabs' speech-to-text model.
pub const STT_MODEL: &str = ELEVEN_STT_MODEL;

/// The supplier `DF_SPEECH_BACKEND` names, and the key its audit line carries. A value the host
/// does not know is a port that refuses every call and names the variable: it never falls back to
/// a supplier that costs money.
pub fn port_from_environment() -> (Arc<dyn SpeechPort>, &'static str) {
    match crate::environment::text(BACKEND_VARIABLE).as_deref() {
        None | Some(ELEVENLABS_BACKEND) => (
            Arc::new(ElevenLabsPort::from_environment()),
            ELEVENLABS_BACKEND,
        ),
        Some(FORGE_BACKEND) => (Arc::new(ForgeSpeechPort::from_environment()), FORGE_BACKEND),
        Some(_) => (Arc::new(UnknownBackendPort), "unknown"),
    }
}

/// Nanoseconds of audio in one 16 kHz sample (the input contract).
const NANOSECONDS_PER_SAMPLE: u64 = 62_500;
/// Fixed bytes of the dev basis' session and run identities.
const DEV_SESSION_BYTE: u8 = 1;
const DEV_RUN_BYTE: u8 = 2;
/// The revision label the rights of every dev call carry.
const DEV_RIGHTS_REVISION: &str = "dev-operator";

static NEXT_JOB: AtomicU64 = AtomicU64::new(0);

struct Providers {
    tts: NativeSpeechProvider,
    stt: NativeSpeechProvider,
}

enum PortState {
    /// No key is set.
    Missing,
    /// A key is set and the adapter refused to use it.
    Refused,
    Ready(Box<Providers>),
}

/// The ElevenLabs adapter: Flash v2.5 text to speech and Scribe v2 speech to text through the
/// bounded `df-providers` profiles. Built once from the key in the environment.
pub struct ElevenLabsPort {
    state: PortState,
}

impl ElevenLabsPort {
    /// The adapter over the key `DF_ELEVENLABS_API_KEY` or `ELEVENLABS_API_KEY` holds; without
    /// one it reports itself not configured and refuses every call.
    pub fn from_environment() -> Self {
        let Some(key) = environment::first_text(&KEY_VARIABLES) else {
            return Self {
                state: PortState::Missing,
            };
        };
        let limits = ProviderLimits::new(
            SPEECH_DEADLINE,
            MAXIMUM_PROVIDER_REQUEST_BYTES,
            MAXIMUM_PROVIDER_RESPONSE_BYTES,
        )
        .build();
        let built = registered_speech_provider(CandidateRouteId::ElevenFlashV25Tts, &key, limits)
            .and_then(|tts| {
                registered_speech_provider(CandidateRouteId::ElevenScribeV2Stt, &key, limits)
                    .map(|stt| Providers { tts, stt })
            });
        Self {
            state: match built {
                Ok(providers) => PortState::Ready(Box::new(providers)),
                Err(_) => PortState::Refused,
            },
        }
    }

    fn ready(&self) -> Result<&Providers, SpeechError> {
        match &self.state {
            PortState::Ready(providers) => Ok(providers),
            PortState::Missing => Err(SpeechError::NotConfigured {
                missing: MissingSetting::SpeechKey,
            }),
            PortState::Refused => Err(SpeechError::Provider {
                cause: SpeechProviderCause::Credential,
            }),
        }
    }
}

impl SpeechPort for ElevenLabsPort {
    fn status(&self) -> PortStatus {
        PortStatus {
            configured: matches!(self.state, PortState::Ready(_)),
            missing: match self.state {
                PortState::Ready(_) => None,
                PortState::Missing | PortState::Refused => Some(MissingSetting::SpeechKey),
            },
            tts_model: TTS_MODEL,
            stt_model: STT_MODEL,
        }
    }

    fn synthesize(&self, call: SynthesisCall) -> PortFuture<'_, Result<Synthesis, SpeechError>> {
        Box::pin(async move { speak(&self.ready()?.tts, call).await })
    }

    fn transcribe(
        &self,
        call: TranscriptionCall,
    ) -> PortFuture<'_, Result<Transcription, SpeechError>> {
        Box::pin(async move { listen(&self.ready()?.stt, call).await })
    }

    fn voices(&self) -> PortFuture<'_, Result<VoiceListing, SpeechError>> {
        Box::pin(async move {
            let listed = self
                .ready()?
                .tts
                .voices(VOICE_LIST_DEADLINE, pending())
                .await
                .map_err(adapter_error)?;
            Ok(VoiceListing::Listed(listed))
        })
    }
}

/// The error of a request that could not be built. The builders' error types differ and name
/// nothing a caller could act on (every input is fixed or already checked), so the cause says only
/// that the request could not be built.
fn unbuildable<E>(_: E) -> SpeechError {
    SpeechError::Provider {
        cause: SpeechProviderCause::Unbuildable,
    }
}

/// A count as the request owner's units carry it.
fn units(value: usize) -> u128 {
    u128::try_from(value).unwrap_or(u128::MAX)
}

/// The identity of a new job: the fixed dev basis and a job that counts up.
fn identity() -> Result<RequestIdentity, SpeechError> {
    // Relaxed: the counter only has to hand out distinct numbers; the first is 1, so the bytes
    // are never all zero.
    let ordinal = NEXT_JOB.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    let mut job = [0_u8; 16];
    job[..8].copy_from_slice(&ordinal.to_le_bytes());
    let epoch = RecoveryEpoch::new(1).map_err(unbuildable)?;
    Ok(RequestIdentity {
        basis: Basis {
            session: SessionId::from_bytes(&[DEV_SESSION_BYTE; 16]).map_err(unbuildable)?,
            run: RunId::from_bytes(&[DEV_RUN_BYTE; 16]).map_err(unbuildable)?,
            revision: SessionRevision::new(epoch, 0),
        },
        job: JobId::from_bytes(&job).map_err(unbuildable)?,
        operation: OperationId::from_bytes(&job).map_err(unbuildable)?,
        generation: 1,
    })
}

fn basis<S>(
    modality: RequestModality,
    format_revision: &str,
    semantic: S,
) -> Result<RequestBasis<S>, SpeechError> {
    Ok(RequestBasis {
        modality,
        source: ContentDigest([0; 32]),
        rights_revision: RevisionLabel::new(Some(DEV_RIGHTS_REVISION)).map_err(unbuildable)?,
        output_format_revision: RevisionLabel::new(Some(format_revision)).map_err(unbuildable)?,
        semantic,
    })
}

fn binding<S>(identity: RequestIdentity, basis: S) -> RequestBinding<S> {
    RequestBinding {
        identity,
        semantic_basis: basis,
        mode: ExecutionMode::Live,
        deadline: SPEECH_DEADLINE,
    }
}

/// A copy of `binding`, which the request and the owner each hold.
fn copy_binding<S: Clone>(binding: &RequestBinding<S>) -> RequestBinding<S> {
    RequestBinding {
        identity: binding.identity,
        semantic_basis: binding.semantic_basis.clone(),
        mode: binding.mode,
        deadline: binding.deadline,
    }
}

/// The owner state of a call that is current, not cancelled and has used no time.
fn owner<S>(binding: &RequestBinding<S>) -> RequestOwnerState<'_, S> {
    RequestOwnerState {
        current: Some(binding),
        elapsed: Duration::ZERO,
        cancelled: false,
    }
}

fn locale(tag: &str) -> Result<LocaleTag, SpeechError> {
    LocaleTag::parse(tag).map_err(unbuildable)
}

async fn speak(
    provider: &NativeSpeechProvider,
    call: SynthesisCall,
) -> Result<Synthesis, SpeechError> {
    let characters = call.text.chars().count();
    let semantic = TtsSemantics {
        locale: locale(ELEVEN_TTS_LANGUAGE)?,
        text: call.text,
        voice: call.voice,
        output_format: SpeechOutputFormat::Mp3At22050Hz32Kbps,
        pcm_format: None,
        timebase: None,
    };
    let basis = basis(
        RequestModality::Tts,
        SpeechOutputFormat::Mp3At22050Hz32Kbps.query_value(),
        semantic,
    )?;
    let binding = binding(identity()?, basis);
    let request = ElevenSpeechRequest::new(
        copy_binding(&binding),
        binding.semantic_basis.semantic.text.as_bytes(),
        RequestUsage::new(
            0,
            Duration::ZERO,
            Usage::new(units(characters), UsageUnit::Character),
        ),
        RequestLimits::new(
            MAXIMUM_ADMITTED_TEXT_BYTES,
            0,
            Duration::ZERO,
            Usage::new(units(MAXIMUM_TEXT_CHARACTERS), UsageUnit::Character),
        )
        .map_err(unbuildable)?,
        owner(&binding),
    )
    .map_err(unbuildable)?;
    let prepared = provider
        .speak(&request, owner(&binding), pending())
        .await
        .map_err(adapter_error)?;
    let [first, _completed] = prepared.complete(owner(&binding)).map_err(adapter_error)?;
    match first.kind {
        TtsEventKind::Chunk { chunk, .. } => Ok(Synthesis {
            audio: chunk.into_bytes(),
            format: ClipFormat::Mp3,
        }),
        _ => Err(SpeechError::Provider {
            cause: SpeechProviderCause::Unusable,
        }),
    }
}

async fn listen(
    provider: &NativeSpeechProvider,
    call: TranscriptionCall,
) -> Result<Transcription, SpeechError> {
    let samples = u64::try_from(call.pcm.len() / 2).map_err(unbuildable)?;
    let audio = Duration::from_nanos(samples.saturating_mul(NANOSECONDS_PER_SAMPLE));
    let semantic = SttSemantics {
        locale: locale(ELEVEN_STT_LANGUAGE)?,
        audio_format: SpeechInputFormat::PcmS16Le16Mono,
        transcript_profile: SpeechTranscriptProfile::FinalWords,
        keyterms: call.key_terms,
    };
    let basis = basis(RequestModality::Stt, "pcm_s16le_16k", semantic)?;
    let binding = binding(identity()?, basis);
    let request = ElevenSttRequest::new(
        copy_binding(&binding),
        &call.pcm,
        RequestUsage::new(
            0,
            audio,
            Usage::new(
                audio.as_nanos().div_ceil(1_000_000),
                UsageUnit::AudioMillisecond,
            ),
        ),
        RequestLimits::new(
            MAXIMUM_ADMITTED_AUDIO_BYTES,
            0,
            Duration::from_millis(MAXIMUM_AUDIO_MS),
            Usage::new(u128::from(MAXIMUM_AUDIO_MS), UsageUnit::AudioMillisecond),
        )
        .map_err(unbuildable)?,
        owner(&binding),
    )
    .map_err(unbuildable)?;
    let prepared = provider
        .transcribe(&request, owner(&binding), pending())
        .await
        .map_err(adapter_error)?;
    let [first, _completed] = prepared.complete(owner(&binding)).map_err(adapter_error)?;
    match first.kind {
        SttEventKind::Final(transcript) => Ok(Transcription {
            text: transcript.text().to_owned(),
            language: transcript.detected_language().to_owned(),
            words: transcript
                .words()
                .iter()
                .filter(|word| word.kind == TranscriptWordKind::Word)
                .count(),
        }),
        _ => Err(SpeechError::Provider {
            cause: SpeechProviderCause::Unusable,
        }),
    }
}

/// The closed cause of the adapter's failure. The adapter's own error is dropped: it can name a
/// supplier request id, which a log of this host has no use for.
fn adapter_error(error: SpeechAdapterError) -> SpeechError {
    let cause = match error.cause() {
        SpeechAdapterCause::InvalidCredential => SpeechProviderCause::Credential,
        SpeechAdapterCause::Json
        | SpeechAdapterCause::Transcript
        | SpeechAdapterCause::EmptyAudio
        | SpeechAdapterCause::VoiceList => SpeechProviderCause::Unusable,
        SpeechAdapterCause::Transport(NativeHttpCause::Deadline) => SpeechProviderCause::Deadline,
        SpeechAdapterCause::Transport(NativeHttpCause::Cancelled) => SpeechProviderCause::Cancelled,
        SpeechAdapterCause::Transport(_) => SpeechProviderCause::Transport,
        SpeechAdapterCause::Supplier(class) => match class {
            ProviderFailureClass::Denied => SpeechProviderCause::Denied,
            ProviderFailureClass::InvalidRequest => SpeechProviderCause::BadRequest,
            ProviderFailureClass::Capacity => SpeechProviderCause::Capacity,
            ProviderFailureClass::Unavailable => SpeechProviderCause::Unavailable,
            ProviderFailureClass::Deadline => SpeechProviderCause::Deadline,
            ProviderFailureClass::Cancelled => SpeechProviderCause::Cancelled,
            _ => SpeechProviderCause::Supplier,
        },
        _ => SpeechProviderCause::Rejected,
    };
    SpeechError::Provider { cause }
}
