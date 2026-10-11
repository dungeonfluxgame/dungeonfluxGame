//! Bounded native ElevenLabs Scribe v2 and Flash v2.5 profiles.
//! Wire sources observed 2026-10-11:
//! <https://elevenlabs.io/docs/api-reference/speech-to-text/convert>
//! <https://elevenlabs.io/docs/api-reference/text-to-speech/stream>
//! Encoded audio and transcripts remain candidates; neither settles supplier spend.

use std::{
    fmt,
    future::Future,
    io::Write,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use df_model::checkpoint::ExecutionMode;
use df_provider_api::{
    CheckedRequest, ProviderEvent, ProviderRequestId as EventId, RequestBasis, RequestError,
    RequestIdentity, RequestLimits, RequestOwnerState, RequestUsage, SttEvent, SttEventKind,
    SttRequest, SttSemantics, TtsEvent, TtsEventKind, TtsRequest, TtsSemantics,
};
use df_types::{LocaleTag, Usage, UsageUnit};
use serde_json::{Map, Value};

use crate::http_contract::encode_path_segment;
use crate::native_http::{NativeHttp, NativeHttpMetadata, NativeResponseProfile};
use crate::text_schema::parse_unique_json;
use crate::{
    CandidateRouteId, HttpRequest, NativeHttpLimits, NativeHttpRefusal, ProviderFailureClass,
    ProviderRequestId, TextSchemaLimits,
};

const ELEVEN_BASE: &str = "https://api.elevenlabs.io";
const MAX_PCM_BYTES: usize = 1_920_000;
const MAX_TRANSCRIPT_BYTES: usize = 65_536;
const MAX_WORDS: usize = 4096;
const JSON_LIMITS: TextSchemaLimits = TextSchemaLimits {
    maximum_schema_bytes: 1_048_576,
    maximum_depth: 16,
    maximum_nodes: 65_536,
};

/// Only raw signed little-endian 16-bit mono PCM at 16 kHz is admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechInputFormat {
    PcmS16Le16Mono,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechTranscriptProfile {
    FinalWords,
    Characters,
}

/// The PCM variant is explicitly unsupported by this encoded-only adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechOutputFormat {
    Mp3At22050Hz32Kbps,
    PcmAt16000Hz,
}

pub type ElevenSttRequest = SttRequest<SpeechInputFormat, SpeechTranscriptProfile>;
pub type ElevenSttBasis = RequestBasis<SttSemantics<SpeechInputFormat, SpeechTranscriptProfile>>;
pub type ElevenSpeechRequest =
    TtsRequest<String, String, SpeechOutputFormat, Option<()>, Option<()>>;
pub type ElevenSpeechBasis =
    RequestBasis<TtsSemantics<String, String, SpeechOutputFormat, Option<()>, Option<()>>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechAdapterReason {
    UnregisteredRoute,
    InvalidCredential,
    InvalidProfile,
    InvalidInput,
    UnsupportedLocale,
    ModeDoesNotDispatch,
    Json,
    Transcript,
    EmptyAudio,
    Owner(RequestError),
    Transport(NativeHttpRefusal),
    Supplier(ProviderFailureClass),
}

/// Explicitly accessed supplier metadata; Debug never prints identifiers or meters.
#[derive(Clone, Default)]
pub struct SpeechResponseMetadata(NativeHttpMetadata);

impl SpeechResponseMetadata {
    pub fn request_id(&self) -> Option<&ProviderRequestId> {
        self.0.request_id.as_ref()
    }
    pub fn transport_request_id(&self) -> Option<&ProviderRequestId> {
        self.0.transport_request_id.as_ref()
    }
    pub fn trace_id(&self) -> Option<&str> {
        self.0.trace_id.as_deref()
    }
    /// Raw supplier-reported character metadata, not verified billing or input count.
    pub fn character_cost(&self) -> Option<&str> {
        self.0.character_cost.as_deref()
    }
}

impl fmt::Debug for SpeechResponseMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SpeechResponseMetadata([REDACTED])")
    }
}

/// Failure retains facts captured before body cancellation, truncation or refusal.
pub struct SpeechAdapterError {
    reason: SpeechAdapterReason,
    metadata: Box<SpeechResponseMetadata>,
    transcription_id: Option<ProviderRequestId>,
    audio_duration_secs: Option<f64>,
    error_request_id: Option<ProviderRequestId>,
}

impl SpeechAdapterError {
    pub fn reason(&self) -> SpeechAdapterReason {
        self.reason
    }
    pub fn metadata(&self) -> &SpeechResponseMetadata {
        &self.metadata
    }
    pub fn transcription_id(&self) -> Option<&ProviderRequestId> {
        self.transcription_id.as_ref()
    }
    pub fn audio_duration_seconds(&self) -> Option<f64> {
        self.audio_duration_secs
    }
    pub fn error_request_id(&self) -> Option<&ProviderRequestId> {
        self.error_request_id.as_ref()
    }
    pub fn failure_class(&self) -> ProviderFailureClass {
        match self.reason {
            SpeechAdapterReason::Owner(RequestError::Cancelled) => ProviderFailureClass::Cancelled,
            SpeechAdapterReason::Owner(RequestError::DeadlineExceeded) => {
                ProviderFailureClass::Deadline
            }
            SpeechAdapterReason::Owner(_)
            | SpeechAdapterReason::Json
            | SpeechAdapterReason::Transcript
            | SpeechAdapterReason::EmptyAudio => ProviderFailureClass::Contract,
            SpeechAdapterReason::Transport(reason) => reason.failure_class(),
            SpeechAdapterReason::Supplier(reason) => reason,
            _ => ProviderFailureClass::InvalidRequest,
        }
    }
}

impl fmt::Debug for SpeechAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpeechAdapterError")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

fn error(reason: SpeechAdapterReason) -> SpeechAdapterError {
    SpeechAdapterError {
        reason,
        metadata: Box::default(),
        transcription_id: None,
        audio_duration_secs: None,
        error_request_id: None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptWordKind {
    Word,
    Spacing,
}

pub struct TranscriptWord {
    pub text: String,
    pub kind: TranscriptWordKind,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub log_probability: f64,
}

impl fmt::Debug for TranscriptWord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TranscriptWord")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// Verbatim transcript and detected language remain distinct from requested locale.
pub struct ValidatedTranscript {
    locale: LocaleTag,
    text: String,
    detected_language: String,
    language_probability: f64,
    words: Vec<TranscriptWord>,
}

impl ValidatedTranscript {
    pub fn locale(&self) -> &LocaleTag {
        &self.locale
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn detected_language(&self) -> &str {
        &self.detected_language
    }
    pub fn language_probability(&self) -> f64 {
        self.language_probability
    }
    pub fn words(&self) -> &[TranscriptWord] {
        &self.words
    }
}

impl fmt::Debug for ValidatedTranscript {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ValidatedTranscript([REDACTED])")
    }
}

/// Exact encoded bytes with the requested profile; no decoder or sample-time claim.
pub struct EncodedSpeechAudio {
    bytes: Vec<u8>,
    locale: LocaleTag,
    voice: String,
    format: SpeechOutputFormat,
}

impl EncodedSpeechAudio {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    pub fn locale(&self) -> &LocaleTag {
        &self.locale
    }
    pub fn voice(&self) -> &str {
        &self.voice
    }
    pub fn requested_format(&self) -> SpeechOutputFormat {
        self.format
    }
}

impl fmt::Debug for EncodedSpeechAudio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EncodedSpeechAudio([REDACTED])")
    }
}

/// Constructed only after the actual native response reaches a validated EOF.
/// Identity and source observations are result facts, never spend/publication grants.
#[derive(Clone)]
pub struct NativeSpeechObservation {
    pub(crate) identity: RequestIdentity,
    pub(crate) route: CandidateRouteId,
    metadata: SpeechResponseMetadata,
    workload: Usage,
    transcription_id: Option<ProviderRequestId>,
    audio_duration_secs: Option<f64>,
    basis: SpeechObservationBasis,
    mode: ExecutionMode,
    deadline: Duration,
    request_payload: Box<[u8]>,
    request_usage: RequestUsage,
    request_limits: RequestLimits,
}

#[derive(Clone)]
enum SpeechObservationBasis {
    Stt(ElevenSttBasis),
    Speech(ElevenSpeechBasis),
}

impl NativeSpeechObservation {
    pub fn identity(&self) -> RequestIdentity {
        self.identity
    }
    pub fn route(&self) -> CandidateRouteId {
        self.route
    }
    pub fn metadata(&self) -> &SpeechResponseMetadata {
        &self.metadata
    }
    /// Exact admitted input workload, not supplier-metered or verified billed usage.
    pub fn workload(&self) -> Usage {
        self.workload
    }
    pub fn transcription_id(&self) -> Option<&ProviderRequestId> {
        self.transcription_id.as_ref()
    }
    pub fn audio_duration_seconds(&self) -> Option<f64> {
        self.audio_duration_secs
    }
    pub(crate) fn matches_stt(&self, request: &CheckedRequest<ElevenSttBasis>) -> bool {
        let binding = request.binding();
        self.identity == binding.identity
            && self.mode == binding.mode
            && self.deadline == binding.deadline
            && matches!(&self.basis, SpeechObservationBasis::Stt(basis) if basis == &binding.semantic_basis)
            && self.matches_checked(request)
    }
    pub(crate) fn matches_speech(&self, request: &CheckedRequest<ElevenSpeechBasis>) -> bool {
        let binding = request.binding();
        self.identity == binding.identity
            && self.mode == binding.mode
            && self.deadline == binding.deadline
            && matches!(&self.basis, SpeechObservationBasis::Speech(basis) if basis == &binding.semantic_basis)
            && self.matches_checked(request)
    }
    fn matches_checked<Semantic>(&self, request: &CheckedRequest<Semantic>) -> bool {
        self.request_payload.as_ref() == request.payload()
            && self.request_usage == request.usage()
            && self.request_limits == request.limits()
    }
    fn refused(&self, reason: SpeechAdapterReason) -> SpeechAdapterError {
        SpeechAdapterError {
            reason,
            metadata: Box::new(self.metadata.clone()),
            transcription_id: self.transcription_id.clone(),
            audio_duration_secs: self.audio_duration_secs,
            error_request_id: None,
        }
    }
}

impl fmt::Debug for NativeSpeechObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeSpeechObservation([REDACTED])")
    }
}

pub struct PreparedSttResponse<'a> {
    request: &'a ElevenSttRequest,
    transcript: ValidatedTranscript,
    observation: NativeSpeechObservation,
}
pub struct PreparedSpeechResponse<'a> {
    request: &'a ElevenSpeechRequest,
    audio: EncodedSpeechAudio,
    observation: NativeSpeechObservation,
}

impl fmt::Debug for PreparedSttResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PreparedSttResponse([REDACTED])")
    }
}
impl fmt::Debug for PreparedSpeechResponse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PreparedSpeechResponse([REDACTED])")
    }
}

impl PreparedSttResponse<'_> {
    pub fn observation(&self) -> NativeSpeechObservation {
        self.observation.clone()
    }
    /// Final and Completed repeat one nonadditive admitted input workload.
    pub fn complete(
        self,
        owner: RequestOwnerState<'_, ElevenSttBasis>,
    ) -> Result<[SttEvent<Option<ProviderRequestId>, ValidatedTranscript>; 2], SpeechAdapterError>
    {
        self.request
            .checked()
            .validate_current(owner)
            .map_err(|e| self.observation.refused(SpeechAdapterReason::Owner(e)))?;
        let id = self.observation.metadata.0.request_id.clone();
        let usage = self.observation.workload;
        Ok([
            ProviderEvent {
                provider_request_id: EventId(id.clone()),
                usage,
                kind: SttEventKind::Final(self.transcript),
            },
            ProviderEvent {
                provider_request_id: EventId(id),
                usage,
                kind: SttEventKind::Completed {
                    identity: self.observation.identity,
                },
            },
        ])
    }
}

impl PreparedSpeechResponse<'_> {
    pub fn observation(&self) -> NativeSpeechObservation {
        self.observation.clone()
    }
    /// One retained encoded chunk follows actual EOF; Completed follows the chunk.
    /// Both events repeat the same nonadditive admitted workload, not billed usage.
    pub fn complete(
        self,
        owner: RequestOwnerState<'_, ElevenSpeechBasis>,
    ) -> Result<[TtsEvent<Option<ProviderRequestId>, EncodedSpeechAudio>; 2], SpeechAdapterError>
    {
        self.request
            .checked()
            .validate_current(owner)
            .map_err(|e| self.observation.refused(SpeechAdapterReason::Owner(e)))?;
        let id = self.observation.metadata.0.request_id.clone();
        let usage = self.observation.workload;
        Ok([
            ProviderEvent {
                provider_request_id: EventId(id.clone()),
                usage,
                kind: TtsEventKind::Chunk {
                    sequence: 0,
                    chunk: self.audio,
                },
            },
            ProviderEvent {
                provider_request_id: EventId(id),
                usage,
                kind: TtsEventKind::Completed {
                    identity: self.observation.identity,
                },
            },
        ])
    }
}

/// One configured route using the existing native HTTP owner; one submit future,
/// no resend, no task spawning, and no ambient credential/model/profile defaults.
pub struct NativeSpeechProvider {
    route: CandidateRouteId,
    credential: String,
    base: String,
    http: NativeHttp,
}

impl fmt::Debug for NativeSpeechProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeSpeechProvider([REDACTED])")
    }
}

impl NativeSpeechProvider {
    pub(crate) fn new(
        route: CandidateRouteId,
        credential: &str,
        limits: NativeHttpLimits,
    ) -> Result<Self, SpeechAdapterError> {
        if !matches!(
            route,
            CandidateRouteId::ElevenScribeV2Stt | CandidateRouteId::ElevenFlashV25Tts
        ) {
            return Err(error(SpeechAdapterReason::UnregisteredRoute));
        }
        if credential.is_empty()
            || credential.len() > 8192
            || !credential.bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err(error(SpeechAdapterReason::InvalidCredential));
        }
        let http =
            NativeHttp::new(limits, false).map_err(|e| error(SpeechAdapterReason::Transport(e)))?;
        Ok(Self {
            route,
            credential: credential.to_owned(),
            base: ELEVEN_BASE.to_owned(),
            http,
        })
    }

    /// Separate fixture construction accepts only literal loopback, fixed paths,
    /// and no DNS/production fallback. It does not change production factories.
    pub fn for_loopback_fixture(mut self, address: SocketAddr) -> Result<Self, SpeechAdapterError> {
        if (!matches!(address.ip(), IpAddr::V4(ip) if ip == Ipv4Addr::LOCALHOST)
            && !matches!(address.ip(), IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST))
            || address.port() == 0
        {
            return Err(error(SpeechAdapterReason::Transport(
                NativeHttpRefusal::InvalidEndpoint,
            )));
        }
        self.http = NativeHttp::new(self.http.limits(), true)
            .map_err(|e| error(SpeechAdapterReason::Transport(e)))?;
        self.base = format!("http://{address}");
        Ok(self)
    }

    pub async fn transcribe<'a>(
        &self,
        request: &'a ElevenSttRequest,
        owner: RequestOwnerState<'_, ElevenSttBasis>,
        cancellation: impl Future<Output = ()>,
    ) -> Result<PreparedSttResponse<'a>, SpeechAdapterError> {
        let elapsed = owner.elapsed;
        request
            .checked()
            .validate_current(owner)
            .map_err(|e| error(SpeechAdapterReason::Owner(e)))?;
        if self.route != CandidateRouteId::ElevenScribeV2Stt {
            return Err(error(SpeechAdapterReason::UnregisteredRoute));
        }
        let checked = request.checked();
        let binding = checked.binding();
        if binding.mode != ExecutionMode::Live {
            return Err(error(SpeechAdapterReason::ModeDoesNotDispatch));
        }
        let semantic = &binding.semantic_basis.semantic;
        if semantic.audio_format != SpeechInputFormat::PcmS16Le16Mono
            || semantic.transcript_profile != SpeechTranscriptProfile::FinalWords
        {
            return Err(error(SpeechAdapterReason::InvalidProfile));
        }
        english_locale(&semantic.locale)?;
        let bytes = checked.payload();
        if bytes.len() < 3200 || bytes.len() > MAX_PCM_BYTES || !bytes.len().is_multiple_of(2) {
            return Err(error(SpeechAdapterReason::InvalidInput));
        }
        let duration = Duration::from_nanos((bytes.len() as u64 / 2) * 62_500);
        let milliseconds = duration.as_nanos().div_ceil(1_000_000);
        if checked.usage().tokens() != 0
            || checked.usage().duration() != duration
            || checked.usage().usage() != Usage::new(milliseconds, UsageUnit::AudioMillisecond)
        {
            return Err(error(SpeechAdapterReason::InvalidInput));
        }
        let body = multipart(
            bytes,
            binding.identity,
            self.http.limits().maximum_request_bytes,
        )?;
        let mapped = HttpRequest::elevenlabs_native(
            &format!("{}/v1/speech-to-text", self.base),
            &self.credential,
            body.0,
            body.1,
        );
        let response = self
            .http
            .execute_profile(
                mapped,
                binding.deadline.saturating_sub(elapsed),
                cancellation,
                NativeResponseProfile::ElevenJson,
            )
            .await
            .map_err(|e| SpeechAdapterError {
                reason: SpeechAdapterReason::Transport(e.reason),
                metadata: Box::new(SpeechResponseMetadata(*e.metadata)),
                transcription_id: None,
                audio_duration_secs: None,
                error_request_id: None,
            })?;
        let metadata = SpeechResponseMetadata(response.metadata);
        let json = decode_json(&response.body).map_err(|reason| SpeechAdapterError {
            reason,
            metadata: Box::new(metadata.clone()),
            transcription_id: None,
            audio_duration_secs: None,
            error_request_id: None,
        })?;
        let mut observation = NativeSpeechObservation {
            identity: binding.identity,
            route: self.route,
            metadata,
            workload: checked.usage().usage(),
            transcription_id: None,
            audio_duration_secs: None,
            basis: SpeechObservationBasis::Stt(binding.semantic_basis.clone()),
            mode: binding.mode,
            deadline: binding.deadline,
            request_payload: checked.payload().into(),
            request_usage: checked.usage(),
            request_limits: checked.limits(),
        };
        if response.status != 200 {
            return Err(supplier_error(response.status, &json, &observation));
        }
        observation.transcription_id = optional_id(json.get("transcription_id")).ok().flatten();
        observation.audio_duration_secs = json
            .get("audio_duration_secs")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n >= 0.0);
        let transcript = parse_transcript(
            &json,
            &semantic.locale,
            duration,
            checked.limits().max_duration(),
        )
        .map_err(|reason| observation.refused(reason))?;
        Ok(PreparedSttResponse {
            request,
            transcript,
            observation,
        })
    }

    pub async fn speak<'a>(
        &self,
        request: &'a ElevenSpeechRequest,
        owner: RequestOwnerState<'_, ElevenSpeechBasis>,
        cancellation: impl Future<Output = ()>,
    ) -> Result<PreparedSpeechResponse<'a>, SpeechAdapterError> {
        let elapsed = owner.elapsed;
        request
            .checked()
            .validate_current(owner)
            .map_err(|e| error(SpeechAdapterReason::Owner(e)))?;
        if self.route != CandidateRouteId::ElevenFlashV25Tts {
            return Err(error(SpeechAdapterReason::UnregisteredRoute));
        }
        let checked = request.checked();
        let binding = checked.binding();
        if binding.mode != ExecutionMode::Live {
            return Err(error(SpeechAdapterReason::ModeDoesNotDispatch));
        }
        let semantic = &binding.semantic_basis.semantic;
        if semantic.output_format != SpeechOutputFormat::Mp3At22050Hz32Kbps
            || semantic.pcm_format.is_some()
            || semantic.timebase.is_some()
        {
            return Err(error(SpeechAdapterReason::InvalidProfile));
        }
        english_locale(&semantic.locale)?;
        let text = std::str::from_utf8(checked.payload())
            .map_err(|_| error(SpeechAdapterReason::InvalidInput))?;
        let characters = text.chars().count();
        if text != semantic.text
            || text.trim().is_empty()
            || characters > 4096
            || semantic.voice.trim().is_empty()
            || semantic.voice.len() > 128
            || checked.usage().tokens() != 0
            || !checked.usage().duration().is_zero()
            || checked.usage().usage() != Usage::new(characters as u128, UsageUnit::Character)
        {
            return Err(error(SpeechAdapterReason::InvalidInput));
        }
        let mut body = BoundedBody {
            bytes: Vec::new(),
            limit: self.http.limits().maximum_request_bytes,
        };
        serde_json::to_writer(
            &mut body,
            &serde_json::json!({"text":text,"model_id":"eleven_flash_v2_5","language_code":"en"}),
        )
        .map_err(|_| {
            error(SpeechAdapterReason::Transport(
                NativeHttpRefusal::RequestTooLarge,
            ))
        })?;
        let endpoint = format!(
            "{}/v1/text-to-speech/{}/stream?output_format=mp3_22050_32",
            self.base,
            encode_path_segment(&semantic.voice)
        );
        let mapped = HttpRequest::elevenlabs_native(
            &endpoint,
            &self.credential,
            "application/json".to_owned(),
            body.bytes,
        );
        let response = self
            .http
            .execute_profile(
                mapped,
                binding.deadline.saturating_sub(elapsed),
                cancellation,
                NativeResponseProfile::AudioMpeg,
            )
            .await
            .map_err(|e| SpeechAdapterError {
                reason: SpeechAdapterReason::Transport(e.reason),
                metadata: Box::new(SpeechResponseMetadata(*e.metadata)),
                transcription_id: None,
                audio_duration_secs: None,
                error_request_id: None,
            })?;
        let observation = NativeSpeechObservation {
            identity: binding.identity,
            route: self.route,
            metadata: SpeechResponseMetadata(response.metadata),
            workload: checked.usage().usage(),
            transcription_id: None,
            audio_duration_secs: None,
            basis: SpeechObservationBasis::Speech(binding.semantic_basis.clone()),
            mode: binding.mode,
            deadline: binding.deadline,
            request_payload: checked.payload().into(),
            request_usage: checked.usage(),
            request_limits: checked.limits(),
        };
        if response.status != 200 {
            let json = decode_json(&response.body).map_err(|reason| observation.refused(reason))?;
            return Err(supplier_error(response.status, &json, &observation));
        }
        if response.body.is_empty() {
            return Err(observation.refused(SpeechAdapterReason::EmptyAudio));
        }
        let audio = EncodedSpeechAudio {
            bytes: response.body,
            locale: semantic.locale.clone(),
            voice: semantic.voice.clone(),
            format: semantic.output_format,
        };
        Ok(PreparedSpeechResponse {
            request,
            audio,
            observation,
        })
    }
}

fn english_locale(locale: &LocaleTag) -> Result<(), SpeechAdapterError> {
    if locale
        .as_str()
        .split('-')
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case("en"))
    {
        Ok(())
    } else {
        Err(error(SpeechAdapterReason::UnsupportedLocale))
    }
}

struct BoundedBody {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedBody {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("body bound"))?;
        if n > self.limit {
            return Err(std::io::Error::other("body bound"));
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| std::io::Error::other("body capacity"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn multipart(
    payload: &[u8],
    identity: RequestIdentity,
    limit: usize,
) -> Result<(String, Vec<u8>), SpeechAdapterError> {
    let mut operation = String::new();
    for b in identity.operation.as_bytes() {
        operation.push_str(&format!("{b:02x}"));
    }
    let boundary = (0..8)
        .map(|n| format!("df-scribe-v2-{operation}-{n}"))
        .find(|b| !payload.windows(b.len()).any(|w| w == b.as_bytes()))
        .ok_or_else(|| error(SpeechAdapterReason::InvalidInput))?;
    let mut body = BoundedBody {
        bytes: Vec::new(),
        limit,
    };
    let append_error = |_| {
        error(SpeechAdapterReason::Transport(
            NativeHttpRefusal::RequestTooLarge,
        ))
    };
    for (name, value) in [
        ("model_id", "scribe_v2"),
        ("language_code", "en"),
        ("file_format", "pcm_s16le_16"),
        ("timestamps_granularity", "word"),
        ("diarize", "false"),
        ("tag_audio_events", "false"),
        ("webhook", "false"),
        ("use_multi_channel", "false"),
        ("no_verbatim", "false"),
        ("use_speaker_library", "false"),
        ("detect_speaker_roles", "false"),
    ] {
        body.write_all(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        )
        .map_err(append_error)?;
    }
    body.write_all(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.pcm\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes()).map_err(append_error)?;
    body.write_all(payload).map_err(append_error)?;
    body.write_all(format!("\r\n--{boundary}--\r\n").as_bytes())
        .map_err(append_error)?;
    Ok((
        format!("multipart/form-data; boundary={boundary}"),
        body.bytes,
    ))
}

fn decode_json(bytes: &[u8]) -> Result<Value, SpeechAdapterReason> {
    if bytes.len() > JSON_LIMITS.maximum_schema_bytes {
        return Err(SpeechAdapterReason::Json);
    }
    parse_unique_json(bytes, JSON_LIMITS).map_err(|_| SpeechAdapterReason::Json)
}

fn optional_id(value: Option<&Value>) -> Result<Option<ProviderRequestId>, SpeechAdapterReason> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= 256 && !s.chars().any(char::is_control) => {
            ProviderRequestId::new(s)
                .map(Some)
                .map_err(|_| SpeechAdapterReason::Transcript)
        }
        _ => Err(SpeechAdapterReason::Transcript),
    }
}

fn optional_time(value: Option<&Value>, maximum: f64) -> Result<Option<f64>, SpeechAdapterReason> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0 && *n <= maximum)
            .map(Some)
            .ok_or(SpeechAdapterReason::Transcript),
    }
}

fn closed_object<'a>(
    value: &'a Value,
    allowed: &[&str],
) -> Result<&'a Map<String, Value>, SpeechAdapterReason> {
    let object = value.as_object().ok_or(SpeechAdapterReason::Transcript)?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(SpeechAdapterReason::Transcript);
    }
    Ok(object)
}

fn absent_or_empty(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
        || value.is_some_and(|v| v.as_array().is_some_and(Vec::is_empty))
}

fn parse_transcript(
    json: &Value,
    locale: &LocaleTag,
    duration: Duration,
    maximum: Duration,
) -> Result<ValidatedTranscript, SpeechAdapterReason> {
    let o = closed_object(
        json,
        &[
            "language_code",
            "language_probability",
            "text",
            "words",
            "channel_index",
            "additional_formats",
            "transcription_id",
            "entities",
            "audio_duration_secs",
            "edited_transcript",
        ],
    )?;
    if !matches!(o.get("channel_index"), None | Some(Value::Null))
        || !matches!(o.get("edited_transcript"), None | Some(Value::Null))
        || !absent_or_empty(o.get("entities"))
        || !absent_or_empty(o.get("additional_formats"))
    {
        return Err(SpeechAdapterReason::Transcript);
    }
    optional_id(o.get("transcription_id"))?;
    optional_time(o.get("audio_duration_secs"), maximum.as_secs_f64())?;
    let detected = o
        .get("language_code")
        .and_then(Value::as_str)
        .filter(|s| (2..=3).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphabetic()))
        .ok_or(SpeechAdapterReason::Transcript)?;
    let probability = o
        .get("language_probability")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
        .ok_or(SpeechAdapterReason::Transcript)?;
    let text = o
        .get("text")
        .and_then(Value::as_str)
        .filter(|s| s.len() <= MAX_TRANSCRIPT_BYTES)
        .ok_or(SpeechAdapterReason::Transcript)?;
    let values = o
        .get("words")
        .and_then(Value::as_array)
        .filter(|w| w.len() <= MAX_WORDS)
        .ok_or(SpeechAdapterReason::Transcript)?;
    let mut words = Vec::new();
    words
        .try_reserve(values.len())
        .map_err(|_| SpeechAdapterReason::Transcript)?;
    let mut bytes = 0usize;
    let mut last_start = 0.0;
    for value in values {
        let w = closed_object(
            value,
            &[
                "text",
                "type",
                "logprob",
                "start",
                "end",
                "speaker_id",
                "characters",
                "channel_index",
            ],
        )?;
        if !matches!(w.get("channel_index"), None | Some(Value::Null))
            || !matches!(w.get("speaker_id"), None | Some(Value::Null))
            || !absent_or_empty(w.get("characters"))
        {
            return Err(SpeechAdapterReason::Transcript);
        }
        let word = w
            .get("text")
            .and_then(Value::as_str)
            .ok_or(SpeechAdapterReason::Transcript)?;
        bytes = bytes
            .checked_add(word.len())
            .filter(|n| *n <= MAX_TRANSCRIPT_BYTES)
            .ok_or(SpeechAdapterReason::Transcript)?;
        let kind = match w.get("type").and_then(Value::as_str) {
            Some("word") => TranscriptWordKind::Word,
            Some("spacing") => TranscriptWordKind::Spacing,
            _ => return Err(SpeechAdapterReason::Transcript),
        };
        let log_probability = w
            .get("logprob")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n <= 0.0)
            .ok_or(SpeechAdapterReason::Transcript)?;
        let start = optional_time(w.get("start"), duration.as_secs_f64())?;
        let end = optional_time(w.get("end"), duration.as_secs_f64())?;
        if kind == TranscriptWordKind::Word && (start.is_none() || end.is_none()) {
            return Err(SpeechAdapterReason::Transcript);
        }
        if start.zip(end).is_some_and(|(s, e)| s > e) || start.is_some_and(|s| s < last_start) {
            return Err(SpeechAdapterReason::Transcript);
        }
        if let Some(start) = start {
            last_start = start;
        }
        words.push(TranscriptWord {
            text: word.to_owned(),
            kind,
            start_seconds: start,
            end_seconds: end,
            log_probability,
        });
    }
    Ok(ValidatedTranscript {
        locale: locale.clone(),
        text: text.to_owned(),
        detected_language: detected.to_owned(),
        language_probability: probability,
        words,
    })
}

fn supplier_error(
    status: u16,
    json: &Value,
    observation: &NativeSpeechObservation,
) -> SpeechAdapterError {
    let detail = json.get("detail").and_then(Value::as_object);
    let kind = detail.and_then(|d| d.get("type")).and_then(Value::as_str);
    let code = detail.and_then(|d| d.get("code")).and_then(Value::as_str);
    let class = match (status, kind, code) {
        (400 | 422, Some("validation_error" | "invalid_request"), Some(_)) => {
            ProviderFailureClass::InvalidRequest
        }
        (401, Some("authentication_error"), Some(_))
        | (402, Some("payment_required"), Some(_))
        | (403, Some("authorization_error"), Some(_)) => ProviderFailureClass::Denied,
        (
            429,
            Some("rate_limit_error"),
            Some("rate_limit_exceeded" | "concurrent_limit_exceeded"),
        ) => ProviderFailureClass::Capacity,
        (503, Some("service_unavailable"), Some("service_unavailable" | "maintenance")) => {
            ProviderFailureClass::Unavailable
        }
        _ => ProviderFailureClass::Unknown,
    };
    let mut refused = observation.refused(SpeechAdapterReason::Supplier(class));
    refused.error_request_id = optional_id(detail.and_then(|d| d.get("request_id")))
        .ok()
        .flatten();
    refused
}
