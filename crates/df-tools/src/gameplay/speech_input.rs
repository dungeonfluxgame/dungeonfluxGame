//! Speech input: a player speaks, the server transcribes, and the player confirms the words
//! as an ordinary typed line.
//!
//! `POST /gameplay/speech` takes one WAV clip (mono 16-bit PCM at 16 kHz, at most 1 MiB) from
//! one authenticated player of the room and answers with the transcript text and nothing
//! else, to that player only. The clip goes to ElevenLabs speech-to-text through the shared
//! speech provider adapter. Audio and transcript are held in memory for the length of the
//! request: they are never written to disk, cached, logged, or put in a span, which records
//! only sizes, durations and the outcome label. Only the words are used; nothing is derived
//! from the voice itself.
//!
//! The spend is bounded by a per-run budget of audio seconds that is charged before the
//! provider call and never refunded, and a player has at most one clip in flight.
use std::{
    collections::BTreeSet,
    env::VarError,
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
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
    CandidateRouteId, ElevenSttBasis, ElevenSttRequest, NativeHttpLimits, NativeHttpRefusal,
    NativeSpeechProvider, SpeechAdapterReason, SpeechInputFormat, SpeechTranscriptProfile,
    registered_speech_provider,
};
use df_session::inbox::InboxHandle;
use df_types::{
    LocaleTag, MemberId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision, Usage, UsageUnit,
};
use tokio::sync::oneshot;

use super::{actor, model};
use crate::speech_clip::{
    BINDING_HEADER, CLIP_CONTENT_TYPE, ContainerFault, MAXIMUM_ACCEPTED_CLIP, MAXIMUM_CLIP_BYTES,
    MINIMUM_CLIP, OUTCOME_HEADER, SPEECH_PATH, STATUS_PATH, SpeechOutcome, pcm_duration, wav_pcm,
};

const DEFAULT_BUDGET_SECONDS: u32 = 600;
const MAXIMUM_BUDGET_SECONDS: u32 = 86_400;
/// The only model the registered provider route sends. The setting names it so a different
/// model cannot be selected without a matching provider route.
const REGISTERED_MODEL: &str = "scribe_v2";
/// The most the provider may take, end to end; it is also the request deadline.
const PROVIDER_DEADLINE: Duration = Duration::from_secs(12);
const IDENTITY_DEADLINE: Duration = Duration::from_secs(5);
/// A line is no longer than the dialogue path accepts.
const MAXIMUM_TRANSCRIPT_BYTES: usize = 4096;
/// A transcript JSON of a fifteen second clip is far smaller; this bounds a hostile answer.
const MAXIMUM_PROVIDER_RESPONSE_BYTES: usize = 256 * 1024;
const MAXIMUM_REQUEST_BYTES: usize = MAXIMUM_CLIP_BYTES / 2 + 4096;

/// Raw inputs of speech input. Only the process environment supplies the credential.
pub(super) struct SpeechInputs {
    pub(super) credential: Result<String, VarError>,
    pub(super) mode: Result<String, VarError>,
    pub(super) seconds_budget: Result<String, VarError>,
    pub(super) model: Result<String, VarError>,
}

impl SpeechInputs {
    pub(super) fn from_environment() -> Self {
        Self {
            credential: std::env::var("ELEVENLABS_API_KEY"),
            mode: std::env::var("DF_GAMEPLAY_STT"),
            seconds_budget: std::env::var("DF_GAMEPLAY_STT_SECONDS_BUDGET"),
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
    budget_seconds: u32,
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
    let budget_seconds = match optional(inputs.seconds_budget)? {
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|seconds| (1..=MAXIMUM_BUDGET_SECONDS).contains(seconds))
            .ok_or_else(|| refused("DF_GAMEPLAY_STT_SECONDS_BUDGET"))?,
        None => DEFAULT_BUDGET_SECONDS,
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
        budget_seconds,
        model,
    }))
}

struct Active {
    provider: NativeSpeechProvider,
    deadline: Duration,
    model: &'static str,
    budget: Mutex<u32>,
    in_flight: Mutex<BTreeSet<MemberId>>,
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

/// One player's clip in flight; dropping it frees the player's slot.
struct Admission {
    input: SpeechInput,
    member: MemberId,
}

impl Drop for Admission {
    fn drop(&mut self) {
        if let Mode::On(active) = &*self.input.mode {
            locked(&active.in_flight).remove(&self.member);
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
            configuration.budget_seconds,
            PROVIDER_DEADLINE,
        ))
    }

    fn limits() -> NativeHttpLimits {
        NativeHttpLimits {
            connect_timeout: Duration::from_secs(5),
            read_timeout: PROVIDER_DEADLINE,
            total_timeout: PROVIDER_DEADLINE,
            maximum_request_bytes: MAXIMUM_REQUEST_BYTES,
            maximum_response_bytes: MAXIMUM_PROVIDER_RESPONSE_BYTES,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 4096,
        }
    }

    fn with_provider(
        provider: NativeSpeechProvider,
        model: &'static str,
        budget_seconds: u32,
        deadline: Duration,
    ) -> Self {
        Self {
            mode: Arc::new(Mode::On(Box::new(Active {
                provider,
                deadline,
                model,
                budget: Mutex::new(budget_seconds),
                in_flight: Mutex::new(BTreeSet::new()),
            }))),
        }
    }

    /// The startup line; it never contains the credential.
    pub(super) fn startup_line(&self) -> String {
        match &*self.mode {
            Mode::Off(reason) => format!("STT off ({})", reason.reason()),
            Mode::On(active) => format!(
                "STT on model={} budget={}s",
                active.model,
                *locked(&active.budget)
            ),
        }
    }

    /// What the page learns before it shows the control.
    fn status(&self) -> SpeechOutcome {
        match &*self.mode {
            Mode::Off(_) => SpeechOutcome::Unavailable,
            Mode::On(active) if *locked(&active.budget) == 0 => SpeechOutcome::BudgetSpent,
            Mode::On(_) => SpeechOutcome::Available,
        }
    }

    fn admit(&self, member: MemberId) -> Result<Admission, SpeechOutcome> {
        let Mode::On(active) = &*self.mode else {
            return Err(SpeechOutcome::Unavailable);
        };
        if !locked(&active.in_flight).insert(member) {
            return Err(SpeechOutcome::Busy);
        }
        Ok(Admission {
            input: self.clone(),
            member,
        })
    }

    /// Spend is committed before the call and kept whatever the outcome: a call whose result
    /// is unknown may still have been billed.
    fn charge(active: &Active, seconds: u32) -> Result<(), SpeechOutcome> {
        let mut remaining = locked(&active.budget);
        if seconds > *remaining {
            return Err(SpeechOutcome::BudgetSpent);
        }
        *remaining -= seconds;
        Ok(())
    }
}

impl Admission {
    /// The words of one clip. The clip and the words live only in this call.
    async fn hear(&self, body: &[u8], span: &mut OperationSpan) -> Result<String, SpeechOutcome> {
        let Mode::On(active) = &*self.input.mode else {
            return Err(SpeechOutcome::Unavailable);
        };
        let pcm = wav_pcm(body).map_err(|fault| match fault {
            ContainerFault::NotWav => SpeechOutcome::Malformed,
            ContainerFault::WrongFormat => SpeechOutcome::UnsupportedType,
        })?;
        let duration = pcm_duration(pcm.len());
        if duration < MINIMUM_CLIP {
            return Err(SpeechOutcome::TooShort);
        }
        if duration > MAXIMUM_ACCEPTED_CLIP {
            return Err(SpeechOutcome::TooLong);
        }
        let seconds = u32::try_from(duration.as_secs() + u64::from(duration.subsec_nanos() > 0))
            .map_err(|_| SpeechOutcome::TooLong)?;
        SpeechInput::charge(active, seconds)?;
        span.event(
            "gameplay.speech.clip",
            &[
                ("gameplay.speech.clip_bytes", body.len().to_string()),
                ("gameplay.speech.audio_ms", duration.as_millis().to_string()),
                ("gameplay.speech.charged_seconds", seconds.to_string()),
            ],
        );
        let text = transcribe(active, pcm, duration).await?;
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
    let call = active
        .provider
        .transcribe(&request, owner(&binding), std::future::pending());
    let prepared = tokio::time::timeout(active.deadline + Duration::from_secs(1), call)
        .await
        .map_err(|_| SpeechOutcome::TimedOut)?
        .map_err(|error| match error.reason() {
            SpeechAdapterReason::Transport(NativeHttpRefusal::Deadline) => SpeechOutcome::TimedOut,
            _ => SpeechOutcome::ProviderFailed,
        })?;
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

fn binding_from(headers: &HeaderMap) -> Result<[u8; 32], SpeechOutcome> {
    let value = headers
        .get(BINDING_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(SpeechOutcome::Unauthenticated)?;
    super::parse_binding(value).ok_or(SpeechOutcome::Unauthenticated)
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
        let mut received = 0;
        let result = self.process(headers, body, span, &mut received).await;
        let outcome = result
            .as_ref()
            .map_or_else(|outcome| *outcome, |_| SpeechOutcome::Heard);
        span.finish(outcome.label(), received);
        response(outcome, result.unwrap_or_default())
    }

    async fn process(
        &self,
        headers: &HeaderMap,
        body: Body,
        span: &mut OperationSpan,
        received: &mut usize,
    ) -> Result<String, SpeechOutcome> {
        let origin_known = headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|origin| self.allowed_origins.iter().any(|known| known == origin));
        if !origin_known {
            return Err(SpeechOutcome::Refused);
        }
        if matches!(*self.input.mode, Mode::Off(_)) {
            return Err(SpeechOutcome::Unavailable);
        }
        let credential = binding_from(headers)?;
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
        let admission = self.input.admit(member)?;
        let clip = axum::body::to_bytes(body, MAXIMUM_CLIP_BYTES)
            .await
            .map_err(|_| SpeechOutcome::TooLarge)?;
        *received = clip.len();
        admission.hear(&clip, span).await
    }
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
        stop: Arc<AtomicBool>,
    }

    impl FakeScribe {
        fn start(delay: Duration, answer: Answer) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(AtomicUsize::new(0));
            let sent_model = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let (seen, model, stopped) = (requests.clone(), sent_model.clone(), stop.clone());
            thread::spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((socket, _)) => {
                            let (seen, model) = (seen.clone(), model.clone());
                            thread::spawn(move || reply(socket, delay, answer, &seen, &model));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(2)),
                    }
                }
            });
            Self {
                address,
                requests,
                sent_model,
                stop,
            }
        }

        fn calls(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
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
        thread::sleep(delay);
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

    fn input_with(fake: &FakeScribe, budget: u32, deadline: Duration) -> SpeechInput {
        let provider = registered_speech_provider(
            CandidateRouteId::ElevenScribeV2Stt,
            "test-credential",
            SpeechInput::limits(),
        )
        .unwrap()
        .for_loopback_fixture(fake.address)
        .unwrap();
        SpeechInput::with_provider(provider, REGISTERED_MODEL, budget, deadline)
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

    struct Upload {
        body: Vec<u8>,
        binding: Option<String>,
        content_type: &'static str,
        origin: Option<&'static str>,
    }

    impl Upload {
        fn from(player: u8, body: Vec<u8>) -> Self {
            Self {
                body,
                binding: Some(binding_hex(player)),
                content_type: "audio/wav",
                origin: Some(ORIGIN),
            }
        }

        fn request(self) -> Request<Body> {
            let mut request = Request::builder()
                .method(Method::POST)
                .uri(SPEECH_PATH)
                .header(header::CONTENT_TYPE, self.content_type)
                .header(header::CONTENT_LENGTH, self.body.len());
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

    fn remaining(input: &SpeechInput) -> u32 {
        match &*input.mode {
            Mode::On(active) => *locked(&active.budget),
            Mode::Off(_) => 0,
        }
    }

    #[tokio::test]
    async fn a_players_clip_is_transcribed_and_only_the_trimmed_words_come_back() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, 600, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let (outcome, text) = send(&mut app, Upload::from(1, clip(1_500))).await;

        assert_eq!(outcome, SpeechOutcome::Heard);
        assert_eq!(text, "Who sent you, zebra-crossing-sentinel?");
        assert_eq!(fake.calls(), 1);
        assert!(fake.sent_model.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_clip_with_no_words_is_nothing_heard_and_still_charged() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::NoWords);
        let input = input_with(&fake, 600, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let (outcome, text) = send(&mut app, Upload::from(1, clip(1_500))).await;

        assert_eq!(outcome, SpeechOutcome::NothingHeard);
        assert_eq!(text, "");
        assert_eq!(remaining(&input), 598);
    }

    #[tokio::test]
    async fn a_clip_over_one_mebibyte_is_refused_before_any_provider_call() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, 600, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let (declared, _) = send(&mut app, Upload::from(1, vec![0; MAXIMUM_CLIP_BYTES + 1])).await;
        // A body that lies about its length is still bounded while it is read.
        let mut lying = Upload::from(1, vec![0; MAXIMUM_CLIP_BYTES + 1]).request();
        lying.headers_mut().remove(header::CONTENT_LENGTH);
        let reply = Service::call(&mut app, lying).await.unwrap();

        assert_eq!(declared, SpeechOutcome::TooLarge);
        assert_eq!(reply.headers()[OUTCOME_HEADER], "too_large");
        assert_eq!(fake.calls(), 0);
        assert_eq!(remaining(&input), 600);
    }

    #[tokio::test]
    async fn clips_that_are_the_wrong_type_length_or_container_are_refused_unspent() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, 600, PROVIDER_DEADLINE);
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
        assert_eq!(remaining(&input), 600);
    }

    #[tokio::test]
    async fn a_request_that_is_not_an_authenticated_player_of_this_page_is_refused() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, 600, PROVIDER_DEADLINE);
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
        assert_eq!(remaining(&input), 600);
    }

    #[tokio::test]
    async fn a_second_clip_from_the_same_player_waits_while_the_first_is_in_flight() {
        let fake = FakeScribe::start(Duration::from_millis(400), Answer::Words);
        let input = input_with(&fake, 600, PROVIDER_DEADLINE);
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
        let input = input_with(&fake, 600, Duration::from_millis(300));
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
        let input = input_with(&fake, 600, PROVIDER_DEADLINE);
        let mut app = app(&input);

        let (outcome, text) = send(&mut app, Upload::from(1, clip(2_100))).await;

        assert_eq!(outcome, SpeechOutcome::ProviderFailed);
        assert_eq!(text, "");
        assert_eq!(remaining(&input), 597);
    }

    #[tokio::test]
    async fn the_budget_is_charged_in_whole_seconds_before_the_call_until_it_is_spent() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, 4, PROVIDER_DEADLINE);
        let mut app = app(&input);
        assert_eq!(input.status(), SpeechOutcome::Available);

        let charged_first = send(&mut app, Upload::from(1, clip(1_001))).await.0;
        let remaining_after_first = remaining(&input);
        let too_big_for_what_is_left = send(&mut app, Upload::from(1, clip(2_500))).await.0;
        let exact = send(&mut app, Upload::from(1, clip(2_000))).await.0;
        let spent = send(&mut app, Upload::from(1, clip(1_000))).await.0;

        assert_eq!(charged_first, SpeechOutcome::Heard);
        assert_eq!(remaining_after_first, 2);
        assert_eq!(too_big_for_what_is_left, SpeechOutcome::BudgetSpent);
        assert_eq!(exact, SpeechOutcome::Heard);
        assert_eq!(spent, SpeechOutcome::BudgetSpent);
        assert_eq!(remaining(&input), 0);
        assert_eq!(input.status(), SpeechOutcome::BudgetSpent);
        assert_eq!(fake.calls(), 2);
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
        let input = input_with(&fake, 600, PROVIDER_DEADLINE);
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
        assert_eq!(input.startup_line(), "STT on model=scribe_v2 budget=600s");
    }

    #[tokio::test]
    async fn no_audio_or_transcript_text_reaches_an_emitted_span() {
        let fake = FakeScribe::start(Duration::ZERO, Answer::Words);
        let input = input_with(&fake, 600, PROVIDER_DEADLINE);
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

    fn inputs(values: [Option<&str>; 4]) -> SpeechInputs {
        let read = |value: Option<&str>| value.map(str::to_owned).ok_or(VarError::NotPresent);
        SpeechInputs {
            credential: read(values[0]),
            mode: read(values[1]),
            seconds_budget: read(values[2]),
            model: read(values[3]),
        }
    }

    #[test]
    fn settings_default_to_a_ten_minute_budget_and_the_registered_model() {
        let SpeechSettings::On(on) =
            speech_settings(inputs([Some("key"), None, None, None])).unwrap()
        else {
            panic!("speech input should be on");
        };

        assert_eq!(on.budget_seconds, 600);
        assert_eq!(on.model, "scribe_v2");
        let explicit = speech_settings(inputs([
            Some("key"),
            Some("on"),
            Some("90"),
            Some("scribe_v2"),
        ]))
        .unwrap();
        assert!(matches!(explicit, SpeechSettings::On(on) if on.budget_seconds == 90));
    }

    #[test]
    fn speech_input_is_off_without_a_credential_or_when_switched_off() {
        for (values, reason) in [
            ([None, None, None, None], SpeechOff::NoCredential),
            ([Some(""), None, None, None], SpeechOff::NoCredential),
            ([Some("key"), Some("off"), None, None], SpeechOff::Disabled),
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
            (
                [Some("key"), Some("maybe-secret"), None, None],
                "DF_GAMEPLAY_STT",
            ),
            (
                [Some("key"), None, Some("0"), None],
                "DF_GAMEPLAY_STT_SECONDS_BUDGET",
            ),
            (
                [Some("key"), None, Some("999999999"), None],
                "DF_GAMEPLAY_STT_SECONDS_BUDGET",
            ),
            (
                [Some("key"), None, Some("ten"), None],
                "DF_GAMEPLAY_STT_SECONDS_BUDGET",
            ),
            (
                [Some("key"), None, None, Some("scribe_v1 secret-model")],
                "DF_GAMEPLAY_STT_MODEL",
            ),
            (
                [Some("key"), None, None, Some("some_other_model")],
                "DF_GAMEPLAY_STT_MODEL",
            ),
        ] {
            let error = speech_settings(inputs(values)).unwrap_err().to_string();

            assert_eq!(error, format!("{name} invalid"));
        }
        assert!(format!("{:?}", SpeechCredential("hunter2".to_owned())).contains("REDACTED"));
    }
}
