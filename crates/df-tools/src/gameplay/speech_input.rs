//! Speech input: a player speaks, the server transcribes, and the player confirms the words
//! as an ordinary typed line.
//!
//! `POST /gameplay/speech` takes one WAV clip (mono 16-bit PCM at 16 kHz, at most 1 MiB) from
//! one authenticated player of the room and answers with the transcript text and nothing
//! else, to that player only. The clip goes to ElevenLabs speech-to-text through the shared
//! speech provider adapter. Audio and transcript are held in memory for the length of the
//! request: they are never written to disk, cached, logged, or put in a span, which records
//! only safe actor IDs, sizes, durations and outcome labels. Only the words are used; nothing
//! is derived from the voice itself.
//!
//! The spend is recorded on the generation hub's `SttSecond` ledger account when the provider
//! call is sent and kept whatever the outcome. There is no cap: nothing is refused for cost.
//! A player has at most one clip in flight. A transcription is a call with no store:
//! it keeps no cache entry, only the hub's `RESOURCE` audit lines and its spend.
use std::{
    collections::BTreeMap,
    env::VarError,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use df_model::checkpoint::{Basis, ContentDigest, ExecutionMode, JobId};
use df_observe::{OperationContext, OperationSpan};
use df_provider_api::{
    RequestBinding, RequestIdentity, RequestLimits, RequestModality, RequestOwnerState,
    RequestUsage, SttEventKind, SttSemantics,
};
use df_providers::{
    CandidateRouteId, ELEVEN_STT_MODEL, ElevenSttBasis, ElevenSttRequest, NativeHttpLimits,
    NativeHttpRefusal, NativeSpeechProvider, ProviderLimits, SpeechAdapterReason,
    SpeechInputFormat, SpeechTranscriptProfile, registered_speech_provider,
};
use df_session::inbox::InboxHandle;
use df_types::{
    LocaleTag, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision, Usage, UsageUnit,
};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

use super::model;
use super::{
    actor,
    resource_adapters::{Kit, KitSlot, TRANSCRIPTION, hex},
    resources::Unit,
};
use crate::speech_clip::{
    BINDING_HEADER, CANCEL_PATH, CLIP_CONTENT_TYPE, ContainerFault, MAXIMUM_ACCEPTED_CLIP,
    MAXIMUM_CLIP_BYTES, MINIMUM_CLIP, OUTCOME_HEADER, SPEECH_PATH, STATUS_PATH, SpeechOutcome,
    UPLOAD_ID_HEADER, pcm_duration, wav_pcm,
};

/// The only model the registered provider route sends. The setting names it so a different
/// model cannot be selected without a matching provider route.
const REGISTERED_MODEL: &str = ELEVEN_STT_MODEL;
/// The most the provider may take, end to end; it is also the request deadline.
const PROVIDER_DEADLINE: Duration = Duration::from_secs(12);
const IDENTITY_DEADLINE: Duration = Duration::from_secs(5);
/// An upload cannot occupy a player's speech slot indefinitely before reaching the provider.
const BODY_READ_DEADLINE: Duration = Duration::from_secs(15);
const REFUSAL_BODY_LIMIT: usize = 64 * 1024;
const REFUSAL_BODY_DEADLINE: Duration = Duration::from_secs(1);
/// A line is no longer than the dialogue path accepts.
const MAXIMUM_TRANSCRIPT_BYTES: usize = 4096;
/// A transcript JSON of a fifteen second clip is far smaller; this bounds a hostile answer.
const MAXIMUM_PROVIDER_RESPONSE_BYTES: usize = 256 * 1024;
const MAXIMUM_REQUEST_BYTES: usize = MAXIMUM_CLIP_BYTES / 2 + 4096;

/// Raw inputs of speech input. Only the process environment supplies the credential.
pub(super) struct SpeechInputs {
    pub(super) credential: Result<String, VarError>,
    pub(super) mode: Result<String, VarError>,
    pub(super) model: Result<String, VarError>,
}

impl SpeechInputs {
    pub(super) fn from_environment() -> Self {
        Self {
            credential: std::env::var("ELEVENLABS_API_KEY"),
            mode: crate::gameplay::media_policy::gates()
                .input(crate::gameplay::media_policy::Capability::SpeechInput),
            model: std::env::var("DF_GAMEPLAY_STT_MODEL"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SpeechOff {
    Qualification,
    Disabled,
    NoCredential,
}

impl SpeechOff {
    fn reason(self) -> &'static str {
        match self {
            Self::Qualification => "qualification run",
            Self::Disabled => "DF_GAMEPLAY_STT=off",
            Self::NoCredential => "ELEVENLABS_API_KEY not set",
        }
    }
}

/// The ElevenLabs credential; `Debug` never shows it.
pub(super) struct SpeechCredential(String);

impl std::fmt::Debug for SpeechCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SpeechCredential([REDACTED])")
    }
}

#[derive(Debug)]
pub(super) struct SpeechConfiguration {
    credential: SpeechCredential,
    model: &'static str,
}

#[derive(Debug)]
pub(super) enum SpeechSettings {
    Off(SpeechOff),
    On(SpeechConfiguration),
}

fn refused(name: &str) -> io::Error {
    io::Error::other(format!("{name} invalid"))
}

fn optional(input: Result<String, VarError>) -> Result<Option<String>, io::Error> {
    match input {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(io::Error::other("gameplay setting not unicode")),
    }
}

/// Speech input is on when a credential is present and `DF_GAMEPLAY_STT` is not `off`.
/// Invalid values are refused without echoing them; empty values count as unset.
pub(super) fn speech_settings(inputs: SpeechInputs) -> Result<SpeechSettings, io::Error> {
    let disabled = match optional(inputs.mode)?.as_deref() {
        None | Some("on") => false,
        Some("off") => true,
        Some(_) => return Err(refused("DF_GAMEPLAY_STT")),
    };
    let model = match optional(inputs.model)?.as_deref() {
        None | Some(REGISTERED_MODEL) => REGISTERED_MODEL,
        Some(_) => return Err(refused("DF_GAMEPLAY_STT_MODEL")),
    };
    if disabled {
        return Ok(SpeechSettings::Off(SpeechOff::Disabled));
    }
    let Some(credential) = optional(inputs.credential)? else {
        return Ok(SpeechSettings::Off(SpeechOff::NoCredential));
    };
    Ok(SpeechSettings::On(SpeechConfiguration {
        credential: SpeechCredential(credential),
        model,
    }))
}

struct Active {
    provider: NativeSpeechProvider,
    deadline: Duration,
    model: &'static str,
    kit: KitSlot,
    in_flight: Mutex<BTreeMap<MemberId, Flight>>,
}

enum Mode {
    Off(SpeechOff),
    On(Box<Active>),
}

/// Transcription of players' clips, or the reason there is none.
#[derive(Clone)]
pub(super) struct SpeechInput {
    mode: Arc<Mode>,
}

/// The guarded data are plain collections and counters that stay consistent when a holder
/// panics, so a poisoned lock is still safe to use.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Flight {
    upload_id: [u8; 16],
    cancellation: Option<oneshot::Sender<()>>,
    cancelled: Arc<AtomicBool>,
}

/// One player's clip in flight; dropping it frees the player's slot.
struct Admission {
    input: SpeechInput,
    member: MemberId,
    upload_id: [u8; 16],
    cancellation: Option<oneshot::Receiver<()>>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}

impl Drop for Admission {
    fn drop(&mut self) {
        if let Mode::On(active) = &*self.input.mode {
            let mut flights = locked(&active.in_flight);
            if flights
                .get(&self.member)
                .is_some_and(|flight| flight.upload_id == self.upload_id)
            {
                flights.remove(&self.member);
            }
        }
    }
}

impl SpeechInput {
    pub(super) fn disabled(reason: SpeechOff) -> Self {
        Self {
            mode: Arc::new(Mode::Off(reason)),
        }
    }

    /// Builds the production ElevenLabs Scribe route from validated settings.
    pub(super) fn start(settings: SpeechSettings) -> Result<Self, io::Error> {
        let configuration = match settings {
            SpeechSettings::Off(reason) => return Ok(Self::disabled(reason)),
            SpeechSettings::On(configuration) => configuration,
        };
        let provider = registered_speech_provider(
            CandidateRouteId::ElevenScribeV2Stt,
            &configuration.credential.0,
            Self::limits(),
        )
        .map_err(|_| io::Error::other("speech input provider configuration refused"))?;
        Ok(Self::with_provider(
            provider,
            configuration.model,
            PROVIDER_DEADLINE,
        ))
    }

    fn limits() -> NativeHttpLimits {
        ProviderLimits::new(
            PROVIDER_DEADLINE,
            MAXIMUM_REQUEST_BYTES,
            MAXIMUM_PROVIDER_RESPONSE_BYTES,
        )
        .build()
    }

    fn with_provider(
        provider: NativeSpeechProvider,
        model: &'static str,
        deadline: Duration,
    ) -> Self {
        let active = Active {
            provider,
            deadline,
            model,
            kit: KitSlot::private(),
            in_flight: Mutex::new(BTreeMap::new()),
        };
        Self {
            mode: Arc::new(Mode::On(Box::new(active))),
        }
    }

    /// Puts speech input on the server's generation hub, so its spend is counted on the hub's
    /// `SttSecond` account. A speech input that is off has nothing to attach.
    pub(super) fn attach_kit(&self, kit: &Kit) {
        if let Mode::On(active) = &*self.mode {
            active.kit.attach(kit);
        }
    }

    /// The startup line; it never contains the credential.
    pub(super) fn startup_line(&self) -> String {
        match &*self.mode {
            Mode::Off(reason) => format!("STT off ({})", reason.reason()),
            Mode::On(active) => format!("STT on model={}", active.model),
        }
    }

    /// What the page learns before it shows the control.
    fn status(&self) -> SpeechOutcome {
        match &*self.mode {
            Mode::Off(_) => SpeechOutcome::Unavailable,
            Mode::On(_) => SpeechOutcome::Available,
        }
    }

    fn admit(&self, member: MemberId, upload_id: [u8; 16]) -> Result<Admission, SpeechOutcome> {
        let Mode::On(active) = &*self.mode else {
            return Err(SpeechOutcome::Unavailable);
        };
        let mut flights = locked(&active.in_flight);
        if flights.contains_key(&member) {
            return Err(SpeechOutcome::Busy);
        }
        let (sender, receiver) = oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        flights.insert(
            member,
            Flight {
                upload_id,
                cancellation: Some(sender),
                cancelled: cancelled.clone(),
            },
        );
        Ok(Admission {
            input: self.clone(),
            member,
            upload_id,
            cancellation: Some(receiver),
            cancelled,
            started,
        })
    }

    fn cancel(&self, member: MemberId, upload_id: [u8; 16]) -> bool {
        let Mode::On(active) = &*self.mode else {
            return false;
        };
        let mut flights = locked(&active.in_flight);
        let Some(flight) = flights.get_mut(&member) else {
            return false;
        };
        if flight.upload_id != upload_id || flight.cancelled.load(Ordering::Acquire) {
            return false;
        }
        let Some(sender) = flight.cancellation.take() else {
            return false;
        };
        flight.cancelled.store(true, Ordering::Release);
        if sender.send(()).is_err() {
            flight.cancelled.store(false, Ordering::Release);
            return false;
        }
        true
    }
}

impl Admission {
    /// The words of one clip. The clip and the words live only in this call.
    async fn hear(
        &mut self,
        body: &[u8],
        cancellation: oneshot::Receiver<()>,
        span: &mut OperationSpan,
        provider_elapsed_ms: &mut Option<u128>,
        audio_ms: &mut Option<u128>,
    ) -> Result<String, SpeechOutcome> {
        let Mode::On(active) = &*self.input.mode else {
            return Err(SpeechOutcome::Unavailable);
        };
        let pcm = wav_pcm(body).map_err(|fault| match fault {
            ContainerFault::NotWav => SpeechOutcome::Malformed,
            ContainerFault::WrongFormat => SpeechOutcome::UnsupportedType,
        })?;
        let duration = pcm_duration(pcm.len());
        *audio_ms = Some(duration.as_millis());
        if duration < MINIMUM_CLIP {
            return Err(SpeechOutcome::TooShort);
        }
        if duration > MAXIMUM_ACCEPTED_CLIP {
            return Err(SpeechOutcome::TooLong);
        }
        let seconds = u32::try_from(duration.as_secs() + u64::from(duration.subsec_nanos() > 0))
            .map_err(|_| SpeechOutcome::TooLong)?;
        span.event(
            "gameplay.speech.clip",
            &[
                ("gameplay.speech.clip_bytes", body.len().to_string()),
                ("gameplay.speech.audio_ms", duration.as_millis().to_string()),
                ("gameplay.speech.charged_seconds", seconds.to_string()),
            ],
        );
        let provider_started = Instant::now();
        let text = transcribe(active, pcm, duration, cancellation, u64::from(seconds)).await;
        *provider_elapsed_ms = Some(provider_started.elapsed().as_millis());
        let text = text?;
        span.event(
            "gameplay.speech.transcript",
            &[("gameplay.speech.transcript_bytes", text.len().to_string())],
        );
        if text.is_empty() {
            return Err(SpeechOutcome::NothingHeard);
        }
        Ok(text)
    }
}

/// A request the provider contract refuses is the server's fault, not the player's.
fn failed<E>(_: E) -> SpeechOutcome {
    SpeechOutcome::ProviderFailed
}

/// Identities the provider contract wants. They are random and carry no game, account or
/// audio meaning.
struct RequestIds {
    first: [u8; 16],
    second: [u8; 16],
    digest: [u8; 32],
}

impl RequestIds {
    fn new() -> Result<Self, SpeechOutcome> {
        Ok(Self {
            first: model::random::<16>().map_err(failed)?,
            second: model::random::<16>().map_err(failed)?,
            digest: model::random::<32>().map_err(failed)?,
        })
    }

    fn binding(&self, deadline: Duration) -> Result<RequestBinding<ElevenSttBasis>, SpeechOutcome> {
        let label = |text| RevisionLabel::new(Some(text)).map_err(failed);
        Ok(RequestBinding {
            identity: RequestIdentity {
                basis: Basis {
                    session: SessionId::from_bytes(&self.first).map_err(failed)?,
                    run: RunId::from_bytes(&self.second).map_err(failed)?,
                    revision: SessionRevision::new(RecoveryEpoch::new(1).map_err(failed)?, 1),
                },
                job: JobId::from_bytes(&self.first).map_err(failed)?,
                operation: OperationId::from_bytes(&self.second).map_err(failed)?,
                generation: 1,
            },
            semantic_basis: ElevenSttBasis {
                modality: RequestModality::Stt,
                source: ContentDigest(self.digest),
                rights_revision: label("local-demo-speech-input")?,
                output_format_revision: label("pcm_s16le_16")?,
                semantic: SttSemantics {
                    locale: LocaleTag::parse("en-US").map_err(failed)?,
                    audio_format: SpeechInputFormat::PcmS16Le16Mono,
                    transcript_profile: SpeechTranscriptProfile::FinalWords,
                    keyterms: Vec::new(),
                },
            },
            mode: ExecutionMode::Live,
            deadline,
        })
    }
}

fn owner(binding: &RequestBinding<ElevenSttBasis>) -> RequestOwnerState<'_, ElevenSttBasis> {
    RequestOwnerState {
        current: Some(binding),
        elapsed: Duration::ZERO,
        cancelled: false,
    }
}

/// One provider call. The transcript is trimmed, its whitespace collapsed, and bounded to a
/// dialogue line; no words at all give an empty string.
async fn transcribe(
    active: &Active,
    pcm: &[u8],
    duration: Duration,
    cancellation: oneshot::Receiver<()>,
    seconds: u64,
) -> Result<String, SpeechOutcome> {
    let ids = RequestIds::new()?;
    let binding = ids.binding(active.deadline)?;
    let milliseconds = duration.as_nanos().div_ceil(1_000_000);
    let request = ElevenSttRequest::new(
        ids.binding(active.deadline)?,
        pcm,
        RequestUsage::new(
            0,
            duration,
            Usage::new(milliseconds, UsageUnit::AudioMillisecond),
        ),
        RequestLimits::new(
            MAXIMUM_CLIP_BYTES,
            0,
            MAXIMUM_ACCEPTED_CLIP,
            Usage::new(
                MAXIMUM_ACCEPTED_CLIP.as_millis(),
                UsageUnit::AudioMillisecond,
            ),
        )
        .map_err(failed)?,
        owner(&binding),
    )
    .map_err(failed)?;
    // The request is formed and checked, so the call is about to be sent: the seconds are spent
    // from here, whatever the outcome. A request that never got this far costs nothing. The
    // metered call is settled below: not billed when the supplier refused the request, of unknown
    // outcome (booked at its maximum) when it timed out, billed once the supplier answered.
    let kit = active.kit.get();
    kit.charge(Unit::SttSecond, seconds);
    let billed = kit
        .hub()
        .ledger()
        .begin_call(super::cost::stt_call(seconds), &[]);
    let call = active
        .provider
        .transcribe(&request, owner(&binding), async move {
            let _cancelled = cancellation.await;
        });
    let prepared = match tokio::time::timeout(active.deadline + Duration::from_secs(1), call).await
    {
        Err(_) => return Err(SpeechOutcome::TimedOut),
        Ok(Err(error)) => {
            let reason = error.reason();
            super::cost::settle_speech_failure(billed, reason);
            return Err(match reason {
                SpeechAdapterReason::Transport(NativeHttpRefusal::Deadline) => {
                    SpeechOutcome::TimedOut
                }
                _ => SpeechOutcome::ProviderFailed,
            });
        }
        Ok(Ok(prepared)) => prepared,
    };
    // The supplier answered: the clip is billed whatever its words turn out to be.
    super::cost::settle_counted(billed, vec![(super::cost::Category::AudioSeconds, seconds)]);
    let events = prepared
        .complete(owner(&binding))
        .map_err(|_| SpeechOutcome::ProviderFailed)?;
    let words = events
        .into_iter()
        .find_map(|event| match event.kind {
            SttEventKind::Final(transcript) => Some(transcript.text().to_owned()),
            _ => None,
        })
        .ok_or(SpeechOutcome::ProviderFailed)?;
    Ok(line_from(&words))
}

fn line_from(words: &str) -> String {
    let mut line = words.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.len() > MAXIMUM_TRANSCRIPT_BYTES {
        let mut end = MAXIMUM_TRANSCRIPT_BYTES;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
    line
}

pub(super) type Identify = Arc<
    dyn Fn([u8; 32]) -> Pin<Box<dyn Future<Output = Result<MemberId, SpeechOutcome>> + Send>>
        + Send
        + Sync,
>;

/// Resolves a room binding to the player it belongs to, through the same check the typed
/// dialogue path makes (a current member holding a player binding).
pub(super) fn actor_identity(actor: InboxHandle<actor::Call>) -> Identify {
    Arc::new(move |credential| {
        let actor = actor.clone();
        Box::pin(async move {
            let (reply, wait) = oneshot::channel();
            actor
                .try_submit(actor::Call::SpeechMember { credential, reply })
                .map_err(|_| SpeechOutcome::Unavailable)?;
            tokio::time::timeout(IDENTITY_DEADLINE, wait)
                .await
                .map_err(|_| SpeechOutcome::TimedOut)?
                .map_err(|_| SpeechOutcome::Unavailable)?
                .map_err(|status| match status.code() {
                    tonic::Code::Unavailable => SpeechOutcome::Unavailable,
                    tonic::Code::PermissionDenied => SpeechOutcome::Refused,
                    _ => SpeechOutcome::Unauthenticated,
                })
        })
    })
}

#[derive(Clone)]
struct RouteState {
    input: SpeechInput,
    identify: Identify,
    allowed_origins: Arc<[String]>,
}

/// `POST /gameplay/speech` and `GET /gameplay/speech/status`.
pub(super) fn router(
    input: SpeechInput,
    identify: Identify,
    allowed_origins: Arc<[String]>,
) -> Router {
    Router::new()
        .route(SPEECH_PATH, post(upload))
        .route(CANCEL_PATH, post(cancel_upload))
        .route(STATUS_PATH, get(status))
        .with_state(RouteState {
            input,
            identify,
            allowed_origins,
        })
}

fn response(outcome: SpeechOutcome, text: String) -> Response {
    let mut response = (
        StatusCode::from_u16(outcome.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        text,
    )
        .into_response();
    response
        .headers_mut()
        .insert(OUTCOME_HEADER, HeaderValue::from_static(outcome.label()));
    response
}

async fn status(State(state): State<RouteState>) -> Response {
    let outcome = state.input.status();
    let mut reply = response(SpeechOutcome::Available, outcome.label().to_owned());
    reply
        .headers_mut()
        .insert(OUTCOME_HEADER, HeaderValue::from_static(outcome.label()));
    reply
}

async fn upload(State(state): State<RouteState>, headers: HeaderMap, body: Body) -> Response {
    let context = OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, "gameplay.speech_input");
    state.handle(&headers, body, &mut span).await
}

async fn cancel_upload(State(state): State<RouteState>, headers: HeaderMap) -> Response {
    let context = OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, "gameplay.speech_cancel");
    match state.cancel(&headers).await {
        Ok(true) => span.finish("signalled", 0),
        Ok(false) => span.finish("not_active", 0),
        Err(outcome) => {
            span.finish(outcome.label(), 0);
            return response(outcome, String::new());
        }
    }
    let mut reply = StatusCode::NO_CONTENT.into_response();
    reply
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    reply
}

fn binding_from(headers: &HeaderMap) -> Result<[u8; 32], SpeechOutcome> {
    let value = headers
        .get(BINDING_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(SpeechOutcome::Unauthenticated)?;
    super::parse_binding(value).ok_or(SpeechOutcome::Unauthenticated)
}

fn upload_id_from(headers: &HeaderMap) -> Result<[u8; 16], SpeechOutcome> {
    let value = headers
        .get(UPLOAD_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() == 32)
        .ok_or(SpeechOutcome::Unauthenticated)?;
    let mut id = [0; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let digits = std::str::from_utf8(pair).map_err(|_| SpeechOutcome::Unauthenticated)?;
        id[index] = u8::from_str_radix(digits, 16).map_err(|_| SpeechOutcome::Unauthenticated)?;
    }
    Ok(id)
}

fn origin_known(headers: &HeaderMap, allowed_origins: &[String]) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| allowed_origins.iter().any(|known| known == origin))
}

fn is_clip_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|kind| {
            let kind = kind.trim();
            [CLIP_CONTENT_TYPE, "audio/x-wav", "audio/wave"]
                .iter()
                .any(|accepted| kind.eq_ignore_ascii_case(accepted))
        })
}

impl RouteState {
    /// Answers one upload and finishes its span, which carries sizes and the outcome only.
    async fn handle(&self, headers: &HeaderMap, body: Body, span: &mut OperationSpan) -> Response {
        let request_started = Instant::now();
        let resource_id = upload_id_from(headers).ok().map(speech_resource_id);
        transcription_line("request", resource_id.as_deref(), Duration::ZERO, "");
        record_speech_resource(
            "gameplay.speech_input.requested",
            resource_id.as_deref(),
            "requested",
            "requested",
            None,
            0,
            None,
            None,
            None,
            None,
            0,
        );
        let mut received = 0;
        let mut cancelled = false;
        let mut close_connection = false;
        let mut provider_elapsed_ms = None;
        let mut audio_ms = None;
        let result = self
            .process(
                headers,
                body,
                span,
                &mut received,
                &mut cancelled,
                &mut close_connection,
                resource_id.as_deref(),
                &mut provider_elapsed_ms,
                &mut audio_ms,
            )
            .await;
        if cancelled {
            transcription_line(
                "failed",
                resource_id.as_deref(),
                request_started.elapsed(),
                "cancelled",
            );
            span.finish("cancelled", received);
            record_speech_resource(
                "gameplay.speech_input.failed",
                resource_id.as_deref(),
                "failed",
                "cancelled",
                Some("cancelled"),
                received,
                audio_ms,
                None,
                provider_elapsed_ms,
                Some(request_started.elapsed().as_millis()),
                SpeechOutcome::ProviderFailed.status(),
            );
            return response(SpeechOutcome::ProviderFailed, String::new());
        }
        let outcome = result
            .as_ref()
            .map_or_else(|outcome| *outcome, |_| SpeechOutcome::Heard);
        let transcript_char_count = result
            .as_ref()
            .ok()
            .map(|transcript| transcript.chars().count());
        span.finish(outcome.label(), received);
        let success = matches!(outcome, SpeechOutcome::Heard | SpeechOutcome::NothingHeard);
        transcription_line(
            if success { "ready" } else { "failed" },
            resource_id.as_deref(),
            request_started.elapsed(),
            if success { "" } else { outcome.label() },
        );
        record_speech_resource(
            if success {
                "gameplay.speech_input.ready"
            } else {
                "gameplay.speech_input.failed"
            },
            resource_id.as_deref(),
            if success { "ready" } else { "failed" },
            outcome.label(),
            (!success).then_some(outcome.label()),
            received,
            audio_ms,
            transcript_char_count,
            provider_elapsed_ms,
            Some(request_started.elapsed().as_millis()),
            outcome.status(),
        );
        let mut reply = response(outcome, result.unwrap_or_default());
        if close_connection {
            reply
                .headers_mut()
                .insert(header::CONNECTION, HeaderValue::from_static("close"));
        }
        reply
    }

    async fn process(
        &self,
        headers: &HeaderMap,
        body: Body,
        span: &mut OperationSpan,
        received: &mut usize,
        cancelled: &mut bool,
        close_connection: &mut bool,
        resource_id: Option<&str>,
        provider_elapsed_ms: &mut Option<u128>,
        audio_ms: &mut Option<u128>,
    ) -> Result<String, SpeechOutcome> {
        if !origin_known(headers, &self.allowed_origins) {
            discard_refused_body(headers, body, close_connection).await;
            return Err(SpeechOutcome::Refused);
        }
        if matches!(*self.input.mode, Mode::Off(_)) {
            return Err(SpeechOutcome::Unavailable);
        }
        let credential = binding_from(headers)?;
        let upload_id = upload_id_from(headers)?;
        if !is_clip_type(headers) {
            return Err(SpeechOutcome::UnsupportedType);
        }
        let declared = headers
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());
        if declared.is_some_and(|length| length > MAXIMUM_CLIP_BYTES) {
            return Err(SpeechOutcome::TooLarge);
        }
        let member = (self.identify)(credential).await?;
        let mut admission = match self.input.admit(member, upload_id) {
            Ok(admission) => admission,
            Err(outcome) => {
                discard_refused_body(headers, body, close_connection).await;
                return Err(outcome);
            }
        };
        record_speech_resource(
            "gameplay.speech_input.queued",
            resource_id,
            "queued",
            "queued",
            None,
            0,
            None,
            None,
            None,
            Some(admission.started.elapsed().as_millis()),
            0,
        );
        let mut audit = QueuedAuditGuard::new(resource_id);
        let Some(cancellation) = admission.cancellation.take() else {
            audit.complete();
            return Err(SpeechOutcome::ProviderFailed);
        };
        let mut cancellation = cancellation;
        let result = tokio::select! {
            cancellation_result = &mut cancellation => {
                let _ = cancellation_result;
                *cancelled = true;
                Err(SpeechOutcome::ProviderFailed)
            }
            read_result = tokio::time::timeout(
                BODY_READ_DEADLINE,
                axum::body::to_bytes(body, MAXIMUM_CLIP_BYTES),
            ) => match read_result {
                Err(_) => Err(SpeechOutcome::TimedOut),
                Ok(Err(_)) => Err(SpeechOutcome::TooLarge),
                Ok(Ok(clip)) => {
                    *received = clip.len();
                    let result = admission
                        .hear(&clip, cancellation, span, provider_elapsed_ms, audio_ms)
                        .await;
                    if admission.cancelled.load(Ordering::Acquire) {
                        *cancelled = true;
                        span.event(
                            "STT cancelled",
                            &[
                                ("actor", format!("{member:?}")),
                                ("bytes", clip.len().to_string()),
                                ("ms", admission.started.elapsed().as_millis().to_string()),
                            ],
                        );
                        Err(SpeechOutcome::ProviderFailed)
                    } else {
                        result
                    }
                }
            }
        };
        audit.complete();
        result
    }

    async fn cancel(&self, headers: &HeaderMap) -> Result<bool, SpeechOutcome> {
        if !origin_known(headers, &self.allowed_origins) {
            return Err(SpeechOutcome::Refused);
        }
        if matches!(*self.input.mode, Mode::Off(_)) {
            return Err(SpeechOutcome::Unavailable);
        }
        let credential = binding_from(headers)?;
        let upload_id = upload_id_from(headers)?;
        let member = (self.identify)(credential).await?;
        Ok(self.input.cancel(member, upload_id))
    }
}

async fn discard_refused_body(headers: &HeaderMap, body: Body, close_connection: &mut bool) {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    let chunked_without_length = declared.is_none()
        && headers
            .get(header::TRANSFER_ENCODING)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(',')
                    .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
            });
    if declared.is_some_and(|length| length > REFUSAL_BODY_LIMIT) || chunked_without_length {
        *close_connection = true;
        return;
    }
    let drained = tokio::time::timeout(
        REFUSAL_BODY_DEADLINE,
        axum::body::to_bytes(body, REFUSAL_BODY_LIMIT),
    )
    .await;
    if !matches!(drained, Ok(Ok(_))) {
        *close_connection = true;
    }
}

/// Emits a terminal audit outcome if the upload future is dropped after queue admission.
struct QueuedAuditGuard {
    resource_id: Option<String>,
    started: Instant,
    armed: bool,
}

impl QueuedAuditGuard {
    fn new(resource_id: Option<&str>) -> Self {
        Self {
            resource_id: resource_id.map(str::to_owned),
            started: Instant::now(),
            armed: true,
        }
    }

    fn complete(&mut self) {
        self.armed = false;
    }
}

impl Drop for QueuedAuditGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        transcription_line(
            "failed",
            self.resource_id.as_deref(),
            self.started.elapsed(),
            "client_aborted",
        );
        record_speech_resource(
            "gameplay.speech_input.failed",
            self.resource_id.as_deref(),
            "failed",
            "client_aborted",
            Some("client_aborted"),
            0,
            None,
            None,
            None,
            Some(self.started.elapsed().as_millis()),
            499,
        );
    }
}

/// One `RESOURCE` line of a transcription, in the hub's format. A transcription has no store, so
/// it has no handle; its key is the hash of the clip's opaque resource id, never of the audio or
/// the words.
fn transcription_line(state: &str, resource_id: Option<&str>, took: Duration, error: &str) {
    let id = resource_id.unwrap_or("unidentified");
    let key = hex(&TRANSCRIPTION.digest(id));
    TRANSCRIPTION.line(state, &key, "transcription:clip", took, "", error);
}

fn speech_resource_id(upload_id: [u8; 16]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"df-speech-input-resource-v1\0");
    hasher.update(upload_id);
    format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[allow(clippy::too_many_arguments)]
fn record_speech_resource(
    event: &'static str,
    resource_id: Option<&str>,
    stage: &'static str,
    status: &'static str,
    failure_category: Option<&str>,
    byte_length: usize,
    audio_ms: Option<u128>,
    transcript_char_count: Option<usize>,
    provider_ms: Option<u128>,
    elapsed_ms: Option<u128>,
    http_status: u16,
) {
    let resource_id_label = resource_id.unwrap_or("unidentified");
    let line = format!(
        "SPEECH_INPUT {stage} status={status} provider=elevenlabs model=scribe_v2 resource_id={resource_id_label} bytes={byte_length} audio_ms={} transcript_chars={} elapsed_ms={} provider_ms={} http_status={http_status} failure_category={} cache_result=not_cached",
        audio_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        transcript_char_count.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        elapsed_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        provider_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        failure_category.unwrap_or("none"),
    );
    let fields = serde_json::Map::from_iter([
        (
            "resource_type".to_owned(),
            serde_json::json!("speech_input"),
        ),
        ("resource_id".to_owned(), serde_json::json!(resource_id)),
        ("provider".to_owned(), serde_json::json!("elevenlabs")),
        ("model".to_owned(), serde_json::json!(REGISTERED_MODEL)),
        ("stage".to_owned(), serde_json::json!(stage)),
        ("status".to_owned(), serde_json::json!(status)),
        (
            "failure_category".to_owned(),
            serde_json::json!(failure_category),
        ),
        ("byte_length".to_owned(), serde_json::json!(byte_length)),
        ("audio_ms".to_owned(), serde_json::json!(audio_ms)),
        (
            "transcript_char_count".to_owned(),
            serde_json::json!(transcript_char_count),
        ),
        ("provider_ms".to_owned(), serde_json::json!(provider_ms)),
        ("elapsed_ms".to_owned(), serde_json::json!(elapsed_ms)),
        ("http_status".to_owned(), serde_json::json!(http_status)),
        ("cache_result".to_owned(), serde_json::json!("not_cached")),
        (
            "provider_attempted".to_owned(),
            serde_json::json!(provider_ms.is_some()),
        ),
    ]);
    let attributes = [
        ("gameplay.resource.type", "speech_input".to_owned()),
        ("gameplay.resource.id", resource_id_label.to_owned()),
        ("gameplay.speech.provider", "elevenlabs".to_owned()),
        ("gameplay.speech.model", REGISTERED_MODEL.to_owned()),
        ("gameplay.resource.stage", stage.to_owned()),
        ("gameplay.resource.status", status.to_owned()),
        (
            "gameplay.resource.failure_category",
            failure_category.unwrap_or("none").to_owned(),
        ),
        ("gameplay.resource.byte_length", byte_length.to_string()),
        (
            "gameplay.speech.audio_ms",
            audio_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
        (
            "gameplay.speech.transcript_char_count",
            transcript_char_count.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
        (
            "gameplay.speech.provider_ms",
            provider_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
        (
            "gameplay.resource.elapsed_ms",
            elapsed_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
        ("gameplay.media.http_status", http_status.to_string()),
        ("gameplay.media.cache_result", "not_cached".to_owned()),
        (
            "gameplay.speech.provider_attempted",
            provider_ms.is_some().to_string(),
        ),
    ];
    actor::record_audit_diagnostic(event, line, fields, &attributes);
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpListener, TcpStream},
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        thread,
    };

    use axum::http::{Method, Request};
    use df_observe::FixtureTelemetry;
    use tonic::codegen::Service;

    use super::*;
    use crate::speech_clip::encode_wav;

    const ORIGIN: &str = "http://127.0.0.1:63338";
    const SPOKEN: &str = "  Who sent\n you, zebra-crossing-sentinel?  ";
    static NEXT_UPLOAD_ID: AtomicUsize = AtomicUsize::new(1);

    /// What the fake speech-to-text endpoint answers.
    #[derive(Clone, Copy)]
    enum Answer {
        Words,
        NoWords,
        Failure,
    }

    /// A loopback stand-in for the ElevenLabs speech-to-text endpoint.
    struct FakeScribe {
        address: SocketAddr,
        requests: Arc<AtomicUsize>,
        sent_model: Arc<AtomicBool>,
        cancelled: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
    }

    impl FakeScribe {
        fn start(delay: Duration, answer: Answer) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(AtomicUsize::new(0));
            let sent_model = Arc::new(AtomicBool::new(false));
            let cancelled = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let (seen, model, was_cancelled, stopped) = (
                requests.clone(),
                sent_model.clone(),
                cancelled.clone(),
                stop.clone(),
            );
            thread::spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((socket, _)) => {
                            let (seen, model, was_cancelled) =
                                (seen.clone(), model.clone(), was_cancelled.clone());
                            thread::spawn(move || {
                                reply(socket, delay, answer, &seen, &model, &was_cancelled)
                            });
                        }
                        Err(_) => thread::sleep(Duration::from_millis(2)),
                    }
                }
            });
            Self {
                address,
                requests,
                sent_model,
                cancelled,
                stop,
            }
        }

        fn calls(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }

        fn was_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::SeqCst)
        }
    }

    impl Drop for FakeScribe {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    fn reply(
        mut socket: TcpStream,
        delay: Duration,
        answer: Answer,
        requests: &AtomicUsize,
        sent_model: &AtomicBool,
        cancelled: &AtomicBool,
    ) {
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        let length: usize = head
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse().unwrap())
            })
            .unwrap();
        let mut body = vec![0; length];
        socket.read_exact(&mut body).unwrap();
        let text = String::from_utf8_lossy(&body);
        sent_model.store(
            text.contains("name=\"model_id\"\r\n\r\nscribe_v2") && text.contains("name=\"file\""),
            Ordering::SeqCst,
        );
        requests.fetch_add(1, Ordering::SeqCst);
        socket
            .set_read_timeout(Some(Duration::from_millis(10)))
            .unwrap();
        let deadline = std::time::Instant::now() + delay;
        while std::time::Instant::now() < deadline {
            let mut probe = [0];
            match socket.peek(&mut probe) {
                Ok(0) => {
                    cancelled.store(true, Ordering::SeqCst);
                    return;
                }
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => {
                    cancelled.store(true, Ordering::SeqCst);
                    return;
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        let (status, payload) = match answer {
            Answer::Words => ("200 OK", transcript_json(SPOKEN)),
            Answer::NoWords => ("200 OK", transcript_json("")),
            Answer::Failure => (
                "500 Internal Server Error",
                r#"{"detail":{"status":"quota_exceeded"}}"#.to_owned(),
            ),
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        // A caller that gave up has closed its end; nothing is left to answer.
        let _answered = socket
            .write_all(head.as_bytes())
            .and_then(|()| socket.write_all(payload.as_bytes()));
    }

    fn transcript_json(text: &str) -> String {
        let words: Vec<String> = text
            .split_whitespace()
            .enumerate()
            .map(|(index, word)| {
                format!(
                    r#"{{"text":"{word}","type":"word","logprob":-0.1,"start":{0},"end":{1}}}"#,
                    index as f64 * 0.1,
                    index as f64 * 0.1 + 0.05
                )
            })
            .collect();
        format!(
            r#"{{"language_code":"en","language_probability":0.99,"text":"{text}","words":[{}]}}"#,
            words.join(",")
        )
        .replace('\n', " ")
    }

    fn member(byte: u8) -> MemberId {
        MemberId::from_bytes(&[byte; 16]).unwrap()
    }

    /// A stand-in for the room: binding bytes 1 and 2 are two players, 3 is a binding that is
    /// not a player, anything else is unknown.
    fn identity() -> Identify {
        Arc::new(|credential| {
            Box::pin(async move {
                match credential[0] {
                    first @ (1 | 2) if credential[1..].iter().all(|byte| *byte == first) => {
                        Ok(member(first))
                    }
                    3 => Err(SpeechOutcome::Refused),
                    _ => Err(SpeechOutcome::Unauthenticated),
                }
            })
        })
    }

    fn input_with(fake: &FakeScribe, deadline: Duration) -> SpeechInput {
        let provider = registered_speech_provider(
            CandidateRouteId::ElevenScribeV2Stt,
            "test-credential",
            SpeechInput::limits(),
        )
        .unwrap()
        .for_loopback_fixture(fake.address)
        .unwrap();
        let input = SpeechInput::with_provider(provider, REGISTERED_MODEL, deadline);
        // The cost book is open with the Scribe rate, so the metered calls can be read back.
        let prices = crate::gameplay::cost::PriceTable::parse(
            r#"{"schema": "dungeonflux.pricing/1", "currency": "USD", "prices": [
                {"provider": "elevenlabs", "model": "scribe_v2", "category": "audio_seconds",
                 "usd": "0.22", "per": 3600, "as_of": "2026-10-24",
                 "source": "https://elevenlabs.io/pricing/api"}]}"#,
        )
        .unwrap();
        ledger(&input)
            .costs()
            .open(
                "stt",
                None,
                prices,
                crate::gameplay::cost::PricingStatus::NotLoaded,
                1,
            )
            .unwrap();
        input
    }

    fn ledger(input: &SpeechInput) -> Arc<crate::gameplay::resources::SpendLedger> {
        match &*input.mode {
            Mode::On(active) => Arc::clone(active.kit.get().hub().ledger()),
            Mode::Off(_) => panic!("speech input should be on"),
        }
    }

    /// The run's metered calls so far: (events, micro-dollars, estimated events, unknown events).
    fn metered(input: &SpeechInput) -> (u64, u64, u64, u64) {
        let run = &ledger(input).costs().report(2)["run"];
        let number = |name: &str| run[name].as_u64().unwrap();
        (
            number("events"),
            number("micro_usd"),
            number("estimated_events"),
            number("unknown_events"),
        )
    }

    fn app(input: &SpeechInput) -> Router {
        router(
            input.clone(),
            identity(),
            Arc::from(vec![ORIGIN.to_owned()]),
        )
    }

    fn clip(milliseconds: usize) -> Vec<u8> {
        let samples = milliseconds * 16;
        encode_wav(
            &(0..samples)
                .map(|index| (index as f32 / 40.0).sin() * 0.5)
                .collect::<Vec<_>>(),
        )
    }

    fn binding_hex(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    #[test]
    fn speech_resource_ids_hash_the_upload_token() {
        let upload_id = [0x5a; 16];
        let encoded = upload_id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let first = speech_resource_id(upload_id);
        assert_eq!(first, speech_resource_id(upload_id));
        assert!(first.starts_with("sha256:"));
        assert!(!first.contains(&encoded));
        assert_ne!(first, speech_resource_id([0xa5; 16]));
    }

    struct Upload {
        body: Vec<u8>,
        binding: Option<String>,
        content_type: &'static str,
        origin: Option<&'static str>,
        upload_id: String,
    }

    impl Upload {
        fn from(player: u8, body: Vec<u8>) -> Self {
            Self {
                body,
                binding: Some(binding_hex(player)),
                content_type: "audio/wav",
                origin: Some(ORIGIN),
                upload_id: format!(
                    "{:032x}",
                    NEXT_UPLOAD_ID.fetch_add(1, Ordering::SeqCst) as u128
                ),
            }
        }

        fn with_upload_id(mut self, upload_id: &str) -> Self {
            self.upload_id = upload_id.to_owned();
            self
        }

        fn request(self) -> Request<Body> {
            let mut request = Request::builder()
                .method(Method::POST)
                .uri(SPEECH_PATH)
                .header(header::CONTENT_TYPE, self.content_type)
                .header(header::CONTENT_LENGTH, self.body.len())
                .header(UPLOAD_ID_HEADER, &self.upload_id);
            if let Some(binding) = &self.binding {
                request = request.header(BINDING_HEADER, binding);
            }
            if let Some(origin) = self.origin {
                request = request.header(header::ORIGIN, origin);
            }
            request.body(Body::from(self.body)).unwrap()
        }
    }

    async fn send(router: &mut Router, upload: Upload) -> (SpeechOutcome, String) {
        let reply = Service::call(router, upload.request()).await.unwrap();
        let outcome =
            SpeechOutcome::from_label(reply.headers()[OUTCOME_HEADER].to_str().unwrap()).unwrap();
        assert_eq!(reply.status().as_u16(), outcome.status());
        assert_eq!(reply.headers()[header::CACHE_CONTROL], "no-store");
        let text = axum::body::to_bytes(reply.into_body(), 1 << 20)
            .await
            .unwrap();
        (outcome, String::from_utf8(text.to_vec()).unwrap())
    }

    /// The audio seconds charged on the hub's ledger so far.
    fn charged(input: &SpeechInput) -> u64 {
        ledger_spend(input).committed
    }

    /// What the hub's ledger holds for speech input: the seconds reserved and kept.
    fn ledger_spend(input: &SpeechInput) -> crate::gameplay::resources::Spend {
        match &*input.mode {
            Mode::On(active) => active.kit.get().hub().ledger().snapshot(Unit::SttSecond),
            Mode::Off(_) => panic!("speech input should be on"),
        }
    }

    #[tokio::test]
    async fn transcription_spend_goes_through_the_ledger_and_writes_resource_lines() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let id = 0xA11CE_u128;
        let upload = Upload::from(1, clip(2_100)).with_upload_id(&format!("{id:032x}"));
        let resource = speech_resource_id(id.to_be_bytes());

        let (outcome, _) = send(&mut app, upload).await;

        assert_eq!(outcome, SpeechOutcome::Heard);
        let spend = ledger_spend(&input);
        assert_eq!(spend.committed, 3, "2.1 s is charged as 3 whole seconds");
        assert_eq!(charged(&input), 3);
        // The same clip is one metered call of 3 seconds at 0.22 an hour: 183.3 micro-dollars,
        // counted by the server, so estimated.
        assert_eq!(metered(&input), (1, 183, 1, 0));
        let key = hex(&TRANSCRIPTION.digest(&resource));
        let lines = crate::gameplay::actor::audit_lines_containing(&key[..32]);
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("RESOURCE request kind=transcription")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("RESOURCE ready kind=transcription")),
            "{lines:?}"
        );
        assert!(
            lines.iter().all(|line| !line.contains("zebra")),
            "no words reach the audit line"
        );
    }

    #[tokio::test]
    async fn a_clip_that_is_refused_before_it_is_sent_gives_its_seconds_back() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let (outcome, _) = send(&mut app, Upload::from(1, b"RIFF not a clip".to_vec())).await;

        assert_eq!(outcome, SpeechOutcome::Malformed);
        assert_eq!(charged(&input), 0);
        assert_eq!(ledger_spend(&input).committed, 0);
        assert_eq!(fake.calls(), 0);
        assert_eq!(
            metered(&input).0,
            0,
            "a clip that was never sent is not metered"
        );
    }

    #[tokio::test]
    async fn a_players_clip_is_transcribed_and_only_the_trimmed_words_come_back() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let (outcome, text) = send(&mut app, Upload::from(1, clip(1_500))).await;

        assert_eq!(outcome, SpeechOutcome::Heard);
        assert_eq!(text, "Who sent you, zebra-crossing-sentinel?");
        assert_eq!(fake.calls(), 1);
        assert!(fake.sent_model.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancelling_an_active_upload_stops_the_provider_request() {
        const UPLOAD_ID: &str = "00112233445566778899aabbccddeeff";

        let fake = FakeScribe::start(Duration::from_secs(10), Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let mut upload_app = app.clone();
        let upload = Upload::from(1, clip(1_500))
            .with_upload_id(UPLOAD_ID)
            .request();
        let upload_task =
            tokio::spawn(async move { Service::call(&mut upload_app, upload).await.unwrap() });

        tokio::time::timeout(Duration::from_secs(2), async {
            while fake.calls() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let stale_cancel = Request::builder()
            .method(Method::POST)
            .uri(CANCEL_PATH)
            .header(BINDING_HEADER, binding_hex(1))
            .header(UPLOAD_ID_HEADER, "ffffffffffffffffffffffffffffffff")
            .header(header::ORIGIN, ORIGIN)
            .body(Body::empty())
            .unwrap();
        let stale_reply = Service::call(&mut app, stale_cancel).await.unwrap();
        assert_eq!(stale_reply.status(), StatusCode::NO_CONTENT);
        assert!(!fake.was_cancelled());

        let cancel = Request::builder()
            .method(Method::POST)
            .uri(CANCEL_PATH)
            .header(BINDING_HEADER, binding_hex(1))
            .header(UPLOAD_ID_HEADER, UPLOAD_ID)
            .header(header::ORIGIN, ORIGIN)
            .body(Body::empty())
            .unwrap();
        let reply = Service::call(&mut app, cancel).await.unwrap();
        assert_eq!(reply.status(), StatusCode::NO_CONTENT);

        tokio::time::timeout(Duration::from_secs(2), async {
            while !fake.was_cancelled() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let upload_reply = upload_task.await.unwrap();
        assert_eq!(upload_reply.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn a_clip_with_no_words_is_nothing_heard_and_still_charged() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::NoWords);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let (outcome, text) = send(&mut app, Upload::from(1, clip(1_500))).await;

        assert_eq!(outcome, SpeechOutcome::NothingHeard);
        assert_eq!(text, "");
        assert_eq!(charged(&input), 2);
    }

    #[tokio::test]
    async fn a_clip_over_one_mebibyte_is_refused_before_any_provider_call() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let (declared, _) = send(&mut app, Upload::from(1, vec![0; MAXIMUM_CLIP_BYTES + 1])).await;
        // A body that lies about its length is still bounded while it is read.
        let mut lying = Upload::from(1, vec![0; MAXIMUM_CLIP_BYTES + 1]).request();
        lying.headers_mut().remove(header::CONTENT_LENGTH);
        let reply = Service::call(&mut app, lying).await.unwrap();

        assert_eq!(declared, SpeechOutcome::TooLarge);
        assert_eq!(reply.headers()[OUTCOME_HEADER], "too_large");
        assert_eq!(fake.calls(), 0);
        assert_eq!(charged(&input), 0);
    }

    #[tokio::test]
    async fn clips_that_are_the_wrong_type_length_or_container_are_refused_unspent() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let mut ogg = Upload::from(1, clip(1_000));
        ogg.content_type = "audio/ogg";
        let mut stereo = clip(1_000);
        stereo[22] = 2;

        let results = [
            send(&mut app, ogg).await.0,
            send(&mut app, Upload::from(1, b"OggS not a wav".to_vec()))
                .await
                .0,
            send(&mut app, Upload::from(1, stereo)).await.0,
            send(&mut app, Upload::from(1, clip(200))).await.0,
            send(&mut app, Upload::from(1, clip(17_000))).await.0,
        ];

        assert_eq!(
            results,
            [
                SpeechOutcome::UnsupportedType,
                SpeechOutcome::Malformed,
                SpeechOutcome::UnsupportedType,
                SpeechOutcome::TooShort,
                SpeechOutcome::TooLong,
            ]
        );
        assert_eq!(fake.calls(), 0);
        assert_eq!(charged(&input), 0);
    }

    #[tokio::test]
    async fn a_request_that_is_not_an_authenticated_player_of_this_page_is_refused() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let mut anonymous = Upload::from(1, clip(1_000));
        anonymous.binding = None;
        let mut malformed = Upload::from(1, clip(1_000));
        malformed.binding = Some("not hex".to_owned());
        let mut foreign = Upload::from(1, clip(1_000));
        foreign.origin = Some("http://evil.example");
        let mut originless = Upload::from(1, clip(1_000));
        originless.origin = None;

        let results = [
            send(&mut app, anonymous).await.0,
            send(&mut app, malformed).await.0,
            send(&mut app, Upload::from(9, clip(1_000))).await.0,
            send(&mut app, Upload::from(3, clip(1_000))).await.0,
            send(&mut app, foreign).await.0,
            send(&mut app, originless).await.0,
        ];

        assert_eq!(
            results,
            [
                SpeechOutcome::Unauthenticated,
                SpeechOutcome::Unauthenticated,
                SpeechOutcome::Unauthenticated,
                SpeechOutcome::Refused,
                SpeechOutcome::Refused,
                SpeechOutcome::Refused,
            ]
        );
        assert_eq!(fake.calls(), 0);
        assert_eq!(charged(&input), 0);
    }

    #[tokio::test]
    async fn a_foreign_origin_upload_is_consumed_before_the_refusal_is_returned() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let consumed = Arc::new(AtomicBool::new(false));
        let mut request = Upload::from(1, Vec::new()).request();
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("http://evil.example"),
        );
        let body_consumed = consumed.clone();
        *request.body_mut() = Body::from_stream(futures::stream::once(async move {
            body_consumed.store(true, Ordering::Release);
            Ok::<_, io::Error>(axum::body::Bytes::from_static(b"untrusted"))
        }));

        let reply = Service::call(&mut app, request).await.unwrap();
        assert_eq!(reply.status(), StatusCode::FORBIDDEN);
        assert!(consumed.load(Ordering::Acquire));
        assert_eq!(fake.calls(), 0);
    }

    #[tokio::test]
    async fn a_foreign_origin_large_declared_body_is_not_read_and_closes_the_connection() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let consumed = Arc::new(AtomicBool::new(false));
        let mut request = Upload::from(1, Vec::new()).request();
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("http://evil.example"),
        );
        request
            .headers_mut()
            .insert(header::CONTENT_LENGTH, HeaderValue::from_static("65537"));
        let body_consumed = consumed.clone();
        *request.body_mut() = Body::from_stream(futures::stream::once(async move {
            body_consumed.store(true, Ordering::Release);
            Ok::<_, io::Error>(axum::body::Bytes::from_static(b"untrusted"))
        }));

        let reply = Service::call(&mut app, request).await.unwrap();
        assert_eq!(reply.status(), StatusCode::FORBIDDEN);
        assert_eq!(reply.headers()[header::CONNECTION], "close");
        assert!(!consumed.load(Ordering::Acquire));
        assert_eq!(fake.calls(), 0);
    }

    #[tokio::test]
    async fn a_foreign_origin_chunked_body_without_a_length_is_not_read() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let consumed = Arc::new(AtomicBool::new(false));
        let mut request = Upload::from(1, Vec::new()).request();
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("http://evil.example"),
        );
        request.headers_mut().remove(header::CONTENT_LENGTH);
        request.headers_mut().insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        let body_consumed = consumed.clone();
        *request.body_mut() = Body::from_stream(futures::stream::once(async move {
            body_consumed.store(true, Ordering::Release);
            Ok::<_, io::Error>(axum::body::Bytes::from_static(b"untrusted"))
        }));

        let reply = Service::call(&mut app, request).await.unwrap();
        assert_eq!(reply.status(), StatusCode::FORBIDDEN);
        assert_eq!(reply.headers()[header::CONNECTION], "close");
        assert!(!consumed.load(Ordering::Acquire));
        assert_eq!(fake.calls(), 0);
    }

    #[tokio::test]
    async fn a_stalled_small_refusal_drain_closes_after_one_second() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let mut request = Upload::from(1, Vec::new()).request();
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("http://evil.example"),
        );
        request
            .headers_mut()
            .insert(header::CONTENT_LENGTH, HeaderValue::from_static("1"));
        *request.body_mut() = Body::from_stream(futures::stream::pending::<
            Result<axum::body::Bytes, io::Error>,
        >());

        let reply = tokio::time::timeout(
            REFUSAL_BODY_DEADLINE + Duration::from_millis(500),
            Service::call(&mut app, request),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reply.status(), StatusCode::FORBIDDEN);
        assert_eq!(reply.headers()[header::CONNECTION], "close");
        assert_eq!(fake.calls(), 0);
    }

    #[tokio::test]
    async fn a_busy_player_small_body_is_drained_before_the_refusal() {
        let fake = FakeScribe::start(Duration::from_millis(500), Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut first_app = app(&input);
        let first =
            tokio::spawn(async move { send(&mut first_app, Upload::from(1, clip(1_000))).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while fake.calls() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let mut app = app(&input);
        let consumed = Arc::new(AtomicBool::new(false));
        let mut request = Upload::from(1, Vec::new()).request();
        request
            .headers_mut()
            .insert(header::CONTENT_LENGTH, HeaderValue::from_static("10"));
        let body_consumed = consumed.clone();
        *request.body_mut() = Body::from_stream(futures::stream::once(async move {
            body_consumed.store(true, Ordering::Release);
            Ok::<_, io::Error>(axum::body::Bytes::from_static(b"small clip"))
        }));

        let reply = Service::call(&mut app, request).await.unwrap();
        assert_eq!(reply.headers()[OUTCOME_HEADER], "busy");
        assert!(consumed.load(Ordering::Acquire));
        assert_eq!(first.await.unwrap().0, SpeechOutcome::Heard);
    }

    #[tokio::test]
    async fn dropping_a_connected_upload_handler_records_client_aborted() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let id = 0xAB07_u128;
        let resource = speech_resource_id(id.to_be_bytes());
        let resource_key = hex(&TRANSCRIPTION.digest(&resource));
        let mut upload = Upload::from(1, Vec::new())
            .with_upload_id(&format!("{id:032x}"))
            .request();
        let body_read_started = Arc::new(AtomicBool::new(false));
        let body_read_signal = body_read_started.clone();
        *upload.body_mut() = Body::from_stream(futures::stream::poll_fn(move |_| {
            body_read_signal.store(true, Ordering::Release);
            std::task::Poll::<Option<Result<axum::body::Bytes, io::Error>>>::Pending
        }));
        let upload_task = tokio::spawn(async move { Service::call(&mut app, upload).await });

        tokio::time::timeout(Duration::from_secs(2), async {
            while !body_read_started.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        upload_task.abort();
        let _ = upload_task.await;

        let lines = crate::gameplay::actor::audit_lines_containing(&resource_key[..32]);
        assert!(
            lines.iter().any(|line| line.contains("client_aborted")),
            "{lines:?}"
        );
        assert_eq!(fake.calls(), 0);
    }

    #[tokio::test]
    async fn cancelling_during_upload_body_read_releases_the_player_slot() {
        const UPLOAD_ID: &str = "10112233445566778899aabbccddeeff";

        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let mut upload_app = app.clone();
        let mut upload = Upload::from(1, Vec::new())
            .with_upload_id(UPLOAD_ID)
            .request();
        *upload.body_mut() = Body::from_stream(futures::stream::pending::<
            Result<axum::body::Bytes, io::Error>,
        >());
        let upload_task =
            tokio::spawn(async move { Service::call(&mut upload_app, upload).await.unwrap() });

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Mode::On(active) = &*input.mode {
                    if locked(&active.in_flight)
                        .contains_key(&MemberId::from_bytes(&[1; 16]).unwrap())
                    {
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let cancel = Request::builder()
            .method(Method::POST)
            .uri(CANCEL_PATH)
            .header(BINDING_HEADER, binding_hex(1))
            .header(UPLOAD_ID_HEADER, UPLOAD_ID)
            .header(header::ORIGIN, ORIGIN)
            .body(Body::empty())
            .unwrap();
        let reply = Service::call(&mut app, cancel).await.unwrap();
        assert_eq!(reply.status(), StatusCode::NO_CONTENT);
        let upload_reply = tokio::time::timeout(Duration::from_secs(2), upload_task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(upload_reply.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(fake.calls(), 0);
    }

    #[tokio::test]
    async fn a_stalled_upload_body_times_out_and_releases_the_player_slot() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        let mut upload = Upload::from(1, Vec::new()).request();
        *upload.body_mut() = Body::from_stream(futures::stream::pending::<
            Result<axum::body::Bytes, io::Error>,
        >());

        let reply = tokio::time::timeout(
            BODY_READ_DEADLINE + Duration::from_secs(2),
            Service::call(&mut app, upload),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reply.status(), StatusCode::GATEWAY_TIMEOUT);

        let (outcome, _) = send(&mut app, Upload::from(1, clip(1_000))).await;
        assert_eq!(outcome, SpeechOutcome::Heard);
        assert_eq!(fake.calls(), 1);
    }

    #[tokio::test]
    async fn a_second_clip_from_the_same_player_waits_while_the_first_is_in_flight() {
        let fake = FakeScribe::start(Duration::from_millis(400), Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let first = tokio::spawn({
            let mut app = app(&input);
            async move { send(&mut app, Upload::from(1, clip(1_000))).await }
        });
        while fake.calls() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let mut app = app(&input);

        let (same_player, _) = send(&mut app, Upload::from(1, clip(1_000))).await;
        let (other_player, _) = send(&mut app, Upload::from(2, clip(1_000))).await;
        let (first, _) = first.await.unwrap();
        let (afterwards, _) = send(&mut app, Upload::from(1, clip(1_000))).await;

        assert_eq!(same_player, SpeechOutcome::Busy);
        assert_eq!(other_player, SpeechOutcome::Heard);
        assert_eq!(first, SpeechOutcome::Heard);
        assert_eq!(afterwards, SpeechOutcome::Heard);
        assert_eq!(fake.calls(), 3);
    }

    #[tokio::test]
    async fn a_provider_that_is_too_slow_times_out_and_the_player_may_try_again() {
        let fake = FakeScribe::start(Duration::from_millis(1_500), Answer::Words);
        let input = input_with(&fake, Duration::from_millis(300));
        let mut app = app(&input);

        let (outcome, text) = send(&mut app, Upload::from(1, clip(1_000))).await;
        let (again, _) = send(&mut app, Upload::from(1, clip(1_000))).await;

        assert_eq!(outcome, SpeechOutcome::TimedOut);
        assert_eq!(text, "");
        assert_eq!(again, SpeechOutcome::TimedOut);
    }

    #[tokio::test]
    async fn a_provider_error_is_a_provider_failure_and_the_spend_stays_committed() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Failure);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let (outcome, text) = send(&mut app, Upload::from(1, clip(2_100))).await;

        assert_eq!(outcome, SpeechOutcome::ProviderFailed);
        assert_eq!(text, "");
        assert_eq!(charged(&input), 3);
        // The supplier answered 500 with an error it does not classify: the sent clip may have
        // been billed, so the metered call is of unknown outcome, booked at the clip's 3 seconds.
        assert_eq!(metered(&input), (1, 183, 0, 1));
    }

    #[tokio::test]
    async fn clips_are_charged_in_whole_seconds_and_never_refused_for_cost() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);
        assert_eq!(input.status(), SpeechOutcome::Available);

        let first = send(&mut app, Upload::from(1, clip(1_001))).await.0;
        let after_first = charged(&input);
        let second = send(&mut app, Upload::from(1, clip(2_500))).await.0;
        let third = send(&mut app, Upload::from(1, clip(2_000))).await.0;

        assert_eq!(first, SpeechOutcome::Heard);
        assert_eq!(after_first, 2);
        assert_eq!(second, SpeechOutcome::Heard);
        assert_eq!(third, SpeechOutcome::Heard);
        assert_eq!(charged(&input), 2 + 3 + 2);
        assert_eq!(input.status(), SpeechOutcome::Available);
        assert_eq!(fake.calls(), 3);
    }

    #[tokio::test]
    async fn speech_input_that_is_off_says_so_and_never_calls_a_provider() {
        let input = SpeechInput::disabled(SpeechOff::NoCredential);
        let mut app = app(&input);

        let (outcome, _) = send(&mut app, Upload::from(1, clip(1_000))).await;
        let status = Service::call(
            &mut app,
            Request::builder()
                .uri(STATUS_PATH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(outcome, SpeechOutcome::Unavailable);
        assert_eq!(status.status(), StatusCode::OK);
        assert_eq!(status.headers()[OUTCOME_HEADER], "unavailable");
        assert_eq!(input.startup_line(), "STT off (ELEVENLABS_API_KEY not set)");
    }

    #[tokio::test]
    async fn the_status_route_reports_availability_to_anyone_and_the_startup_line_names_the_model()
    {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let status = Service::call(
            &mut app,
            Request::builder()
                .uri(STATUS_PATH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(status.status(), StatusCode::OK);
        assert_eq!(status.headers()[OUTCOME_HEADER], "available");
        assert_eq!(input.startup_line(), "STT on model=scribe_v2");
    }

    #[tokio::test]
    async fn no_audio_or_transcript_text_reaches_an_emitted_span() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, PROVIDER_DEADLINE);
        let state = RouteState {
            input,
            identify: identity(),
            allowed_origins: Arc::from(vec![ORIGIN.to_owned()]),
        };
        let telemetry = FixtureTelemetry::default();
        let context = OperationContext {
            trace_parent: String::new(),
            build: "test-build".to_owned(),
        };
        let clip = clip(1_200);
        let audio_start = format!("{:?}", &clip[44..60]);
        let mut span = telemetry.begin(&context, "gameplay.speech_input");
        let headers = Upload::from(1, Vec::new()).request().headers().clone();
        let body = Body::from(clip);

        let reply = state.handle(&headers, body, &mut span).await;
        let failure_telemetry = FixtureTelemetry::default();
        let mut failed_span = failure_telemetry.begin(&context, "gameplay.speech_input");
        state
            .handle(
                &headers,
                Body::from(b"RIFF not a clip".to_vec()),
                &mut failed_span,
            )
            .await;
        telemetry.shutdown().unwrap();
        failure_telemetry.shutdown().unwrap();

        assert_eq!(reply.headers()[OUTCOME_HEADER], "heard");
        let emitted = format!(
            "{}{}",
            telemetry.snapshot().unwrap(),
            failure_telemetry.snapshot().unwrap()
        );
        assert!(emitted.contains("gameplay.speech.clip_bytes"));
        assert!(emitted.contains("gameplay.speech.audio_ms"));
        assert!(emitted.contains("heard"));
        assert!(emitted.contains("malformed"));
        for forbidden in ["zebra", "Who sent", "sentinel", audio_start.as_str()] {
            assert!(!emitted.contains(forbidden), "span carried {forbidden:?}");
        }
    }

    #[test]
    fn a_transcript_is_collapsed_and_bounded_to_a_dialogue_line_on_a_character_boundary() {
        let long = "é".repeat(MAXIMUM_TRANSCRIPT_BYTES);

        let line = line_from(&long);

        assert_eq!(line_from("  one\n two\t three  "), "one two three");
        assert_eq!(line_from("   "), "");
        assert!(line.len() <= MAXIMUM_TRANSCRIPT_BYTES);
        assert!(line.chars().all(|letter| letter == 'é'));
    }

    fn inputs(values: [Option<&str>; 3]) -> SpeechInputs {
        let read = |value: Option<&str>| value.map(str::to_owned).ok_or(VarError::NotPresent);
        SpeechInputs {
            credential: read(values[0]),
            mode: read(values[1]),
            model: read(values[2]),
        }
    }

    #[test]
    fn settings_default_to_the_registered_model() {
        let SpeechSettings::On(on) = speech_settings(inputs([Some("key"), None, None])).unwrap()
        else {
            panic!("speech input should be on");
        };

        assert_eq!(on.model, "scribe_v2");
        let explicit =
            speech_settings(inputs([Some("key"), Some("on"), Some("scribe_v2")])).unwrap();
        assert!(matches!(explicit, SpeechSettings::On(on) if on.model == "scribe_v2"));
    }

    #[test]
    fn speech_input_is_off_without_a_credential_or_when_switched_off() {
        for (values, reason) in [
            ([None, None, None], SpeechOff::NoCredential),
            ([Some(""), None, None], SpeechOff::NoCredential),
            ([Some("key"), Some("off"), None], SpeechOff::Disabled),
        ] {
            assert!(matches!(
                speech_settings(inputs(values)).unwrap(),
                SpeechSettings::Off(found) if found == reason
            ));
        }
    }

    #[test]
    fn invalid_settings_are_refused_without_echoing_them() {
        for (values, name) in [
            ([Some("key"), Some("maybe-secret"), None], "DF_GAMEPLAY_STT"),
            (
                [Some("key"), None, Some("scribe_v1 secret-model")],
                "DF_GAMEPLAY_STT_MODEL",
            ),
            (
                [Some("key"), None, Some("some_other_model")],
                "DF_GAMEPLAY_STT_MODEL",
            ),
        ] {
            let error = speech_settings(inputs(values)).unwrap_err().to_string();

            assert_eq!(error, format!("{name} invalid"));
        }
        assert!(format!("{:?}", SpeechCredential("hunter2".to_owned())).contains("REDACTED"));
    }
}
