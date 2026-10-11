//! Spoken scene narration and NPC lines for the shared display.
//!
//! The server only voices text it put into an outgoing display view: each such line is
//! registered under the SHA-256 key of its role and text, and `GET /gameplay/voice/<key>`
//! serves the audio of a registered line. Audio comes from the on-disk cache when present,
//! otherwise from one single-flight ElevenLabs call whose characters are charged to a
//! per-run budget before the call and never refunded. Every refusal is an empty 503 or 404;
//! the browser then simply stays silent.
use std::{
    collections::{HashMap, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use axum::{
    Router,
    extract::{Path as RoutePath, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bytes::Bytes;
use df_model::checkpoint::{Basis, ContentDigest, ExecutionMode, JobId};
use df_observe::OperationContext;
use df_protocol::common as rpc;
use df_provider_api::{
    RequestBinding, RequestIdentity, RequestLimits, RequestModality, RequestOwnerState,
    RequestUsage, TtsEventKind, TtsSemantics,
};
use df_providers::{
    CandidateRouteId, ElevenSpeechBasis, ElevenSpeechRequest, NativeHttpLimits,
    NativeSpeechProvider, SpeechOutputFormat, registered_speech_provider,
};
use df_types::{
    LocaleTag, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage,
    UsageUnit,
};
use tokio::sync::watch;

use super::startup_configuration::{VoiceConfiguration, VoiceOff, VoiceSettings};
use crate::voice_line::VoiceLine;

/// Most recent registered lines kept; older ones can no longer be requested.
const MAXIMUM_LINES: usize = 256;
const MAXIMUM_LINE_CHARACTERS: usize = 600;
const MAXIMUM_ROLE_BYTES: usize = 96;
const MAXIMUM_AUDIO_BYTES: usize = 4 * 1024 * 1024;
const SYNTHESIS_DEADLINE: Duration = Duration::from_secs(30);
const CACHE_CONTROL: &str = "private, max-age=86400";

/// Why a registered line could not be turned into audio. The client never sees the
/// difference: every variant is an empty 503.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Failure {
    BudgetExhausted,
    Request,
    Provider,
    Interrupted,
}

impl Failure {
    fn label(self) -> &'static str {
        match self {
            Self::BudgetExhausted => "budget_exhausted",
            Self::Request => "request_refused",
            Self::Provider => "provider_failed",
            Self::Interrupted => "interrupted",
        }
    }
}

type Outcome = Result<Bytes, Failure>;

/// The result of asking for the audio of one key.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Lookup {
    Audio(Bytes),
    Unregistered,
    Unavailable,
}

#[derive(Default)]
struct Lines {
    order: VecDeque<String>,
    by_key: HashMap<String, VoiceLine>,
}

impl Lines {
    /// Remembers `line`, or refreshes its recency; the oldest line is dropped at capacity.
    fn register(&mut self, line: VoiceLine) {
        if line.text.chars().count() > MAXIMUM_LINE_CHARACTERS
            || line.role.len() > MAXIMUM_ROLE_BYTES
        {
            return;
        }
        let key = line.key();
        if self.by_key.contains_key(&key) {
            self.order.retain(|known| *known != key);
        } else if self.order.len() >= MAXIMUM_LINES
            && let Some(oldest) = self.order.pop_front()
        {
            self.by_key.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.by_key.insert(key, line);
    }
}

struct Budget {
    remaining: u32,
    exhausted: bool,
}

struct Synthesis {
    provider: NativeSpeechProvider,
    narrator_voice: String,
    npc_voice: String,
    cache_dir: PathBuf,
    budget: Mutex<Budget>,
    in_flight: Mutex<HashMap<String, watch::Receiver<Option<Outcome>>>>,
}

enum Mode {
    Off(VoiceOff),
    On {
        synthesis: Arc<Synthesis>,
        summary: String,
    },
}

struct Inner {
    lines: Mutex<Lines>,
    mode: Mode,
}

/// Registry of speakable lines plus, when enabled, the cached ElevenLabs synthesis.
#[derive(Clone)]
pub(super) struct Voice {
    inner: Arc<Inner>,
}

/// The guarded data are plain collections and counters that stay consistent when a holder
/// panics, so a poisoned lock is still safe to use.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Voice {
    /// A voice that registers lines but never synthesizes.
    pub(super) fn disabled(reason: VoiceOff) -> Self {
        Self {
            inner: Arc::new(Inner {
                lines: Mutex::new(Lines::default()),
                mode: Mode::Off(reason),
            }),
        }
    }

    /// Builds the production ElevenLabs flash route from validated settings.
    pub(super) fn start(settings: VoiceSettings) -> Result<Self, io::Error> {
        let configuration = match settings {
            VoiceSettings::Off(reason) => return Ok(Self::disabled(reason)),
            VoiceSettings::On(configuration) => configuration,
        };
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5),
            read_timeout: SYNTHESIS_DEADLINE,
            total_timeout: SYNTHESIS_DEADLINE,
            maximum_request_bytes: 16 * 1024,
            maximum_response_bytes: MAXIMUM_AUDIO_BYTES,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 4096,
        };
        let provider = registered_speech_provider(
            CandidateRouteId::ElevenFlashV25Tts,
            configuration.credential.expose(),
            limits,
        )
        .map_err(|_| io::Error::other("voice provider configuration refused"))?;
        Self::new(configuration, provider)
    }

    /// Uses `provider` for synthesis; tests pass one pointed at a loopback fake.
    fn new(
        configuration: VoiceConfiguration,
        provider: NativeSpeechProvider,
    ) -> Result<Self, io::Error> {
        std::fs::create_dir_all(&configuration.cache_dir)?;
        let summary = format!(
            "VOICE on narrator={} npc={} budget={} cache={}",
            configuration.narrator_voice,
            configuration.npc_voice,
            configuration.character_budget,
            configuration.cache_dir.display()
        );
        let synthesis = Synthesis {
            provider,
            narrator_voice: configuration.narrator_voice,
            npc_voice: configuration.npc_voice,
            cache_dir: configuration.cache_dir,
            budget: Mutex::new(Budget {
                remaining: configuration.character_budget,
                exhausted: false,
            }),
            in_flight: Mutex::new(HashMap::new()),
        };
        Ok(Self {
            inner: Arc::new(Inner {
                lines: Mutex::new(Lines::default()),
                mode: Mode::On {
                    synthesis: Arc::new(synthesis),
                    summary,
                },
            }),
        })
    }

    /// The single startup line; it never contains the credential.
    pub(super) fn startup_line(&self) -> String {
        match &self.inner.mode {
            Mode::Off(reason) => format!("VOICE off ({})", reason.reason()),
            Mode::On { summary, .. } => summary.clone(),
        }
    }

    /// Registers the speakable text of a view the server is sending. Only the display
    /// audience is voiced; player views register nothing.
    pub(super) fn register_view(&self, message: &rpc::ViewMessage) {
        let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
            return;
        };
        let mut lines = locked(&self.inner.lines);
        for line in crate::voice_line::display_lines(display) {
            lines.register(line);
        }
    }

    pub(super) async fn audio(&self, key: &str) -> Lookup {
        let Some(line) = locked(&self.inner.lines).by_key.get(key).cloned() else {
            return Lookup::Unregistered;
        };
        let Mode::On { synthesis, .. } = &self.inner.mode else {
            return Lookup::Unavailable;
        };
        match synthesis.audio(key, line).await {
            Ok(bytes) => Lookup::Audio(bytes),
            Err(_) => Lookup::Unavailable,
        }
    }
}

impl Synthesis {
    async fn audio(self: &Arc<Self>, key: &str, line: VoiceLine) -> Outcome {
        if let Some(bytes) = self.read_cached(key).await {
            return Ok(bytes);
        }
        let mut receiver = {
            let mut flights = locked(&self.in_flight);
            match flights.get(key) {
                Some(receiver) => receiver.clone(),
                None => {
                    let (sender, receiver) = watch::channel(None);
                    flights.insert(key.to_owned(), receiver.clone());
                    tokio::spawn(Arc::clone(self).fly(key.to_owned(), line, sender));
                    receiver
                }
            }
        };
        let finished = receiver
            .wait_for(Option::is_some)
            .await
            .map_err(|_| Failure::Interrupted)?;
        finished.clone().unwrap_or(Err(Failure::Interrupted))
    }

    /// The one task of a key. Callers that go away do not cancel it: its charge is already
    /// spent, so its audio is finished and cached for the next request.
    async fn fly(
        self: Arc<Self>,
        key: String,
        line: VoiceLine,
        sender: watch::Sender<Option<Outcome>>,
    ) {
        let outcome = self.produce(&key, &line).await;
        locked(&self.in_flight).remove(&key);
        // No receiver left means every requester disconnected; the cache holds the result.
        sender.send_replace(Some(outcome));
    }

    async fn produce(&self, key: &str, line: &VoiceLine) -> Outcome {
        // A flight that finished between the caller's cache check and this one wrote it.
        if let Some(bytes) = self.read_cached(key).await {
            return Ok(bytes);
        }
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&context, "gameplay.voice_synthesis");
        let outcome = self.synthesize(key, line).await;
        span.finish_unmeasured(match &outcome {
            Ok((_, true)) => "synthesized",
            Ok((_, false)) => "synthesized_uncached",
            Err(failure) => failure.label(),
        });
        outcome.map(|(bytes, _)| bytes)
    }

    /// Charges first, calls the provider once and caches the audio; the boolean tells
    /// whether the cache write succeeded.
    async fn synthesize(&self, key: &str, line: &VoiceLine) -> Result<(Bytes, bool), Failure> {
        let characters = u32::try_from(line.text.chars().count()).map_err(|_| Failure::Request)?;
        self.charge(characters)?;
        let voice = if line.is_npc() {
            &self.npc_voice
        } else {
            &self.narrator_voice
        };
        let audio = self.request_audio(voice, line).await?;
        if audio.is_empty() || audio.len() > MAXIMUM_AUDIO_BYTES {
            return Err(Failure::Provider);
        }
        let audio = Bytes::from(audio);
        let stored = self.store(key, audio.clone()).await.is_ok();
        Ok((audio, stored))
    }

    /// Spend is committed before the call and kept whatever the outcome: a call whose
    /// result is unknown may still have been billed.
    fn charge(&self, characters: u32) -> Result<(), Failure> {
        let mut budget = locked(&self.budget);
        if budget.exhausted || characters > budget.remaining {
            if !budget.exhausted {
                budget.exhausted = true;
                println!("VOICE budget exhausted; no further lines will be synthesized");
            }
            return Err(Failure::BudgetExhausted);
        }
        budget.remaining -= characters;
        Ok(())
    }

    async fn request_audio(&self, voice: &str, line: &VoiceLine) -> Result<Vec<u8>, Failure> {
        let characters = line.text.chars().count();
        let current = speech_binding(voice, line)?;
        let request = ElevenSpeechRequest::new(
            speech_binding(voice, line)?,
            line.text.as_bytes(),
            RequestUsage::new(
                0,
                Duration::ZERO,
                Usage::new(characters as u128, UsageUnit::Character),
            ),
            RequestLimits::new(
                4096,
                0,
                Duration::ZERO,
                Usage::new(MAXIMUM_LINE_CHARACTERS as u128, UsageUnit::Character),
            )
            .map_err(|_| Failure::Request)?,
            owner(&current),
        )
        .map_err(|_| Failure::Request)?;
        let prepared = self
            .provider
            .speak(&request, owner(&current), std::future::pending())
            .await
            .map_err(|_| Failure::Provider)?;
        let events = prepared
            .complete(owner(&current))
            .map_err(|_| Failure::Provider)?;
        events
            .into_iter()
            .find_map(|event| match event.kind {
                TtsEventKind::Chunk { chunk, .. } => Some(chunk.into_bytes()),
                _ => None,
            })
            .ok_or(Failure::Provider)
    }

    async fn read_cached(&self, key: &str) -> Option<Bytes> {
        let path = self.cache_dir.join(key);
        // A failed blocking task is treated as a cache miss.
        tokio::task::spawn_blocking(move || read_cache_file(&path))
            .await
            .ok()
            .flatten()
    }

    /// Writes a temporary file beside the target and renames it into place so a reader
    /// never sees a partial file.
    async fn store(&self, key: &str, audio: Bytes) -> io::Result<()> {
        let target = self.cache_dir.join(key);
        let staging = self
            .cache_dir
            .join(format!(".{key}.{}.tmp", std::process::id()));
        tokio::task::spawn_blocking(move || write_cache_file(&staging, &target, &audio))
            .await
            .map_err(|_| io::Error::other("voice cache write task failed"))?
    }
}

fn read_cache_file(path: &Path) -> Option<Bytes> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let length = usize::try_from(metadata.len()).ok()?;
    if !metadata.is_file() || length == 0 || length > MAXIMUM_AUDIO_BYTES {
        return None;
    }
    std::fs::read(path).ok().map(Bytes::from)
}

fn write_cache_file(staging: &Path, target: &Path, audio: &[u8]) -> io::Result<()> {
    if audio.is_empty() {
        return Err(io::Error::other("empty voice audio is never cached"));
    }
    let written = std::fs::write(staging, audio).and_then(|()| std::fs::rename(staging, target));
    if written.is_err() {
        // Best effort: a leftover staging file is the only residue of a failed write.
        let _removed = std::fs::remove_file(staging);
    }
    written
}

fn owner(current: &RequestBinding<ElevenSpeechBasis>) -> RequestOwnerState<'_, ElevenSpeechBasis> {
    RequestOwnerState {
        current: Some(current),
        elapsed: Duration::ZERO,
        cancelled: false,
    }
}

fn refused<E>(_: E) -> Failure {
    Failure::Request
}

/// The provider contract wants job identities; a spoken line has none of its own, so they
/// are derived from the line's digest and carry no game or account meaning.
fn speech_binding(
    voice: &str,
    line: &VoiceLine,
) -> Result<RequestBinding<ElevenSpeechBasis>, Failure> {
    let digest = line.digest();
    let (first, second) = digest.split_at(16);
    let label = |text| RevisionLabel::new(Some(text)).map_err(refused);
    Ok(RequestBinding {
        identity: RequestIdentity {
            basis: Basis {
                session: SessionId::from_bytes(first).map_err(refused)?,
                run: RunId::from_bytes(second).map_err(refused)?,
                revision: SessionRevision::new(RecoveryEpoch::new(1).map_err(refused)?, 1),
            },
            job: JobId::from_bytes(first).map_err(refused)?,
            operation: OperationId::from_bytes(second).map_err(refused)?,
            generation: 1,
        },
        semantic_basis: ElevenSpeechBasis {
            modality: RequestModality::Tts,
            source: ContentDigest(digest),
            rights_revision: label("local-demo-voice")?,
            output_format_revision: label("mp3_22050_32")?,
            semantic: TtsSemantics {
                locale: LocaleTag::parse("en-US").map_err(refused)?,
                text: line.text.clone(),
                voice: voice.to_owned(),
                output_format: SpeechOutputFormat::Mp3At22050Hz32Kbps,
                pcm_format: None,
                timebase: None,
            },
        },
        mode: ExecutionMode::Live,
        deadline: SYNTHESIS_DEADLINE,
    })
}

/// `GET /gameplay/voice/{key}`.
pub(super) fn router(voice: Voice) -> Router {
    Router::new()
        .route("/gameplay/voice/{key}", get(serve_line))
        .with_state(voice)
}

async fn serve_line(State(voice): State<Voice>, RoutePath(key): RoutePath<String>) -> Response {
    match voice.audio(&key).await {
        Lookup::Audio(bytes) => (
            [
                (header::CONTENT_TYPE, "audio/mpeg"),
                (header::CACHE_CONTROL, CACHE_CONTROL),
            ],
            bytes,
        )
            .into_response(),
        Lookup::Unregistered => StatusCode::NOT_FOUND.into_response(),
        Lookup::Unavailable => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpListener, TcpStream},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use super::super::startup_configuration::{
        VoiceCredential, VoiceInputs, VoiceOff, voice_settings,
    };
    use super::*;

    const AUDIO: &[u8] = b"ID3 fake mpeg audio";

    /// A loopback stand-in for the ElevenLabs streaming endpoint.
    struct FakeEleven {
        address: SocketAddr,
        requests: Arc<AtomicUsize>,
        targets: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
    }

    impl FakeEleven {
        fn start(delay: Duration, healthy: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(AtomicUsize::new(0));
            let targets = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let (seen, targets_seen, stopped) = (requests.clone(), targets.clone(), stop.clone());
            thread::spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((socket, _)) => {
                            let (seen, targets_seen) = (seen.clone(), targets_seen.clone());
                            thread::spawn(move || {
                                answer(socket, delay, healthy, &seen, &targets_seen);
                            });
                        }
                        Err(_) => thread::sleep(Duration::from_millis(2)),
                    }
                }
            });
            Self {
                address,
                requests,
                targets,
                stop,
            }
        }

        fn calls(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }
    }

    impl Drop for FakeEleven {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    fn answer(
        mut socket: TcpStream,
        delay: Duration,
        healthy: bool,
        requests: &AtomicUsize,
        targets: &Mutex<Vec<String>>,
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
        requests.fetch_add(1, Ordering::SeqCst);
        targets
            .lock()
            .unwrap()
            .push(head.lines().next().unwrap().to_owned());
        thread::sleep(delay);
        let (status, mime, payload): (&str, &str, &[u8]) = if healthy {
            ("200 OK", "audio/mpeg", AUDIO)
        } else {
            (
                "500 Internal Server Error",
                "application/json",
                br#"{"detail":{"status":"quota_exceeded"}}"#,
            )
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        socket.write_all(head.as_bytes()).unwrap();
        socket.write_all(payload).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "df-voice-test-{name}-{}-{}",
            std::process::id(),
            line_counter()
        ));
        let _removed = std::fs::remove_dir_all(&dir);
        dir
    }

    fn line_counter() -> usize {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        NEXT.fetch_add(1, Ordering::SeqCst)
    }

    fn configuration(cache: &Path, budget: u32) -> VoiceConfiguration {
        VoiceConfiguration {
            credential: VoiceCredential::for_test("test-credential"),
            character_budget: budget,
            cache_dir: cache.to_path_buf(),
            narrator_voice: "narratorvoice".to_owned(),
            npc_voice: "npcvoice".to_owned(),
        }
    }

    fn voice_with(fake: &FakeEleven, cache: &Path, budget: u32) -> Voice {
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(1),
            read_timeout: Duration::from_secs(5),
            total_timeout: Duration::from_secs(5),
            maximum_request_bytes: 16_384,
            maximum_response_bytes: 65_536,
            maximum_header_bytes: 4096,
            maximum_chunks: 4096,
        };
        let provider = registered_speech_provider(
            CandidateRouteId::ElevenFlashV25Tts,
            "test-credential",
            limits,
        )
        .unwrap()
        .for_loopback_fixture(fake.address)
        .unwrap();
        Voice::new(configuration(cache, budget), provider).unwrap()
    }

    fn narrator(text: &str) -> VoiceLine {
        VoiceLine {
            role: "narrator".to_owned(),
            text: text.to_owned(),
        }
    }

    fn register(voice: &Voice, line: &VoiceLine) -> String {
        locked(&voice.inner.lines).register(line.clone());
        line.key()
    }

    fn remaining(voice: &Voice) -> u32 {
        match &voice.inner.mode {
            Mode::On { synthesis, .. } => locked(&synthesis.budget).remaining,
            Mode::Off(_) => 0,
        }
    }

    fn display_message(narration: &str) -> rpc::ViewMessage {
        rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    narration: narration.to_owned(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn unregistered_key_is_not_found() {
        let cache = scratch("unregistered");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache, 1000);

        let response = serve_line(State(voice), RoutePath("0".repeat(64))).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(fake.calls(), 0);
    }

    #[tokio::test]
    async fn registered_line_is_synthesized_once_then_served_from_cache() {
        let cache = scratch("once");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache, 1000);
        let key = register(&voice, &narrator("The lanterns sway."));

        let first = serve_line(State(voice.clone()), RoutePath(key.clone())).await;
        let second = serve_line(State(voice.clone()), RoutePath(key.clone())).await;

        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[header::CONTENT_TYPE], "audio/mpeg");
        assert_eq!(first.headers()[header::CACHE_CONTROL], CACHE_CONTROL);
        let body = axum::body::to_bytes(first.into_body(), 1 << 20)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), AUDIO);
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(fake.calls(), 1);
        assert_eq!(std::fs::read(cache.join(&key)).unwrap(), AUDIO);
        let first_target = fake.targets.lock().unwrap()[0].clone();
        assert!(first_target.contains("/v1/text-to-speech/narratorvoice/stream"));
    }

    #[tokio::test]
    async fn cache_survives_a_new_server_run() {
        let cache = scratch("restart");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let line = narrator("Rain on the slates.");
        let first_run = voice_with(&fake, &cache, 1000);
        let key = register(&first_run, &line);
        assert_eq!(
            first_run.audio(&key).await,
            Lookup::Audio(Bytes::from_static(AUDIO))
        );

        let second_run = voice_with(&fake, &cache, 1000);
        register(&second_run, &line);
        let replay = second_run.audio(&key).await;

        assert_eq!(replay, Lookup::Audio(Bytes::from_static(AUDIO)));
        assert_eq!(fake.calls(), 1);
        assert_eq!(remaining(&second_run), 1000);
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_provider_call() {
        let cache = scratch("flight");
        let fake = FakeEleven::start(Duration::from_millis(250), true);
        let voice = voice_with(&fake, &cache, 1000);
        let text = "A bell tolls twice.";
        let key = register(&voice, &narrator(text));

        let (a, b, c, d) = tokio::join!(
            voice.audio(&key),
            voice.audio(&key),
            voice.audio(&key),
            voice.audio(&key)
        );

        for lookup in [a, b, c, d] {
            assert_eq!(lookup, Lookup::Audio(Bytes::from_static(AUDIO)));
        }
        assert_eq!(fake.calls(), 1);
        assert_eq!(remaining(&voice), 1000 - text.chars().count() as u32);
    }

    #[tokio::test]
    async fn budget_is_charged_before_the_call_and_kept_when_the_provider_fails() {
        let cache = scratch("charge");
        let fake = FakeEleven::start(Duration::ZERO, false);
        let voice = voice_with(&fake, &cache, 40);
        let text = "Thirty characters of narration";
        assert_eq!(text.chars().count(), 30);
        let key = register(&voice, &narrator(text));

        let failed = voice.audio(&key).await;

        assert_eq!(failed, Lookup::Unavailable);
        assert_eq!(fake.calls(), 1);
        assert_eq!(remaining(&voice), 10);
    }

    #[tokio::test]
    async fn exhausted_budget_is_unavailable_without_a_provider_call() {
        let cache = scratch("exhausted");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache, 20);
        let first = register(&voice, &narrator("Exactly twenty chars"));
        let second = register(&voice, &narrator("Another line"));

        let spent = voice.audio(&first).await;
        let refused = voice.audio(&second).await;

        assert_eq!(spent, Lookup::Audio(Bytes::from_static(AUDIO)));
        assert_eq!(refused, Lookup::Unavailable);
        assert_eq!(fake.calls(), 1);
        assert_eq!(remaining(&voice), 0);
        let response = serve_line(State(voice), RoutePath(second)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            axum::body::to_bytes(response.into_body(), 16)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn cached_line_is_served_after_the_budget_is_exhausted() {
        let cache = scratch("cached-after-exhausted");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache, 20);
        let key = register(&voice, &narrator("Exactly twenty chars"));
        let other = register(&voice, &narrator("Different"));
        assert_eq!(
            voice.audio(&key).await,
            Lookup::Audio(Bytes::from_static(AUDIO))
        );
        assert_eq!(voice.audio(&other).await, Lookup::Unavailable);

        let replay = voice.audio(&key).await;

        assert_eq!(replay, Lookup::Audio(Bytes::from_static(AUDIO)));
        assert_eq!(fake.calls(), 1);
    }

    #[tokio::test]
    async fn provider_failure_writes_no_cache_file() {
        let cache = scratch("failure");
        let fake = FakeEleven::start(Duration::ZERO, false);
        let voice = voice_with(&fake, &cache, 1000);
        let key = register(&voice, &narrator("The seal cracks."));

        let response = serve_line(State(voice), RoutePath(key.clone())).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(fake.calls(), 1);
        assert!(!cache.join(&key).exists());
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn named_speakers_use_the_npc_voice() {
        let cache = scratch("npc");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache, 1000);
        let npc = VoiceLine {
            role: "npc:Courier".to_owned(),
            text: "You are late.".to_owned(),
        };
        let key = register(&voice, &npc);

        let lookup = voice.audio(&key).await;

        assert_eq!(lookup, Lookup::Audio(Bytes::from_static(AUDIO)));
        let target = fake.targets.lock().unwrap()[0].clone();
        assert!(target.contains("/v1/text-to-speech/npcvoice/stream"));
    }

    #[tokio::test]
    async fn disabled_voice_registers_lines_but_never_synthesizes() {
        let voice = Voice::disabled(VoiceOff::Disabled);
        voice.register_view(&display_message("The wharf is quiet."));
        let key = narrator("The wharf is quiet.").key();

        let known = serve_line(State(voice.clone()), RoutePath(key)).await;
        let unknown = serve_line(State(voice.clone()), RoutePath("1".repeat(64))).await;

        assert_eq!(known.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        assert_eq!(voice.startup_line(), "VOICE off (DF_GAMEPLAY_VOICE=off)");
    }

    #[test]
    fn only_display_views_register_speakable_lines() {
        let voice = Voice::disabled(VoiceOff::Disabled);
        let player = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    narration: "A private whisper.".to_owned(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };

        voice.register_view(&player);
        voice.register_view(&display_message("Public narration."));

        let lines = locked(&voice.inner.lines);
        assert_eq!(lines.by_key.len(), 1);
        assert!(
            lines
                .by_key
                .contains_key(&narrator("Public narration.").key())
        );
    }

    #[test]
    fn registry_keeps_the_most_recent_lines_and_skips_long_ones() {
        let mut lines = Lines::default();
        for index in 0..MAXIMUM_LINES + 10 {
            lines.register(narrator(&format!("line {index}")));
        }
        lines.register(narrator(&"x".repeat(MAXIMUM_LINE_CHARACTERS + 1)));

        assert_eq!(lines.by_key.len(), MAXIMUM_LINES);
        assert_eq!(lines.order.len(), MAXIMUM_LINES);
        assert!(!lines.by_key.contains_key(&narrator("line 0").key()));
        assert!(lines.by_key.contains_key(&narrator("line 265").key()));
        assert!(
            !lines
                .by_key
                .contains_key(&narrator(&"x".repeat(MAXIMUM_LINE_CHARACTERS + 1)).key())
        );
    }

    #[test]
    fn registering_a_known_line_again_refreshes_its_recency() {
        let mut lines = Lines::default();
        lines.register(narrator("keep me"));
        for index in 0..MAXIMUM_LINES - 1 {
            lines.register(narrator(&format!("filler {index}")));
        }
        lines.register(narrator("keep me"));
        lines.register(narrator("one more"));

        assert!(lines.by_key.contains_key(&narrator("keep me").key()));
        assert!(!lines.by_key.contains_key(&narrator("filler 0").key()));
    }

    fn inputs(pairs: &[(&str, &str)]) -> VoiceInputs {
        let pick = |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
                .ok_or(std::env::VarError::NotPresent)
        };
        VoiceInputs {
            credential: pick("ELEVENLABS_API_KEY"),
            mode: pick("DF_GAMEPLAY_VOICE"),
            character_budget: pick("DF_GAMEPLAY_VOICE_CHAR_BUDGET"),
            cache_dir: pick("DF_GAMEPLAY_VOICE_CACHE"),
            narrator_voice: pick("DF_GAMEPLAY_VOICE_NARRATOR"),
            npc_voice: pick("DF_GAMEPLAY_VOICE_NPC"),
        }
    }

    #[test]
    fn settings_default_to_off_without_a_credential() {
        let settings = voice_settings(inputs(&[])).unwrap();

        assert!(matches!(
            settings,
            VoiceSettings::Off(VoiceOff::NoCredential)
        ));
    }

    #[test]
    fn settings_turn_on_with_a_credential_and_use_documented_defaults() {
        let settings = voice_settings(inputs(&[("ELEVENLABS_API_KEY", "k")])).unwrap();

        let VoiceSettings::On(configuration) = settings else {
            panic!("voice should be on");
        };
        assert_eq!(configuration.character_budget, 12_000);
        assert_eq!(
            configuration.cache_dir,
            PathBuf::from("artifacts/tmp/play/voice-cache")
        );
        assert_eq!(configuration.narrator_voice, "JBFqnCBsd6RMkjVDRZzb");
        assert_eq!(configuration.npc_voice, "N2lVS1w4EtoT3dr4eOWO");
        assert!(format!("{configuration:?}").contains("[REDACTED]"));
    }

    #[test]
    fn settings_read_every_override_and_off_wins_over_the_credential() {
        let overrides = [
            ("ELEVENLABS_API_KEY", "k"),
            ("DF_GAMEPLAY_VOICE_CHAR_BUDGET", "300"),
            ("DF_GAMEPLAY_VOICE_CACHE", "/tmp/somewhere"),
            ("DF_GAMEPLAY_VOICE_NARRATOR", "narr_1"),
            ("DF_GAMEPLAY_VOICE_NPC", "npc-2"),
        ];
        let VoiceSettings::On(configuration) = voice_settings(inputs(&overrides)).unwrap() else {
            panic!("voice should be on");
        };
        assert_eq!(configuration.character_budget, 300);
        assert_eq!(configuration.cache_dir, PathBuf::from("/tmp/somewhere"));
        assert_eq!(configuration.narrator_voice, "narr_1");
        assert_eq!(configuration.npc_voice, "npc-2");

        let off = voice_settings(inputs(&[
            ("ELEVENLABS_API_KEY", "k"),
            ("DF_GAMEPLAY_VOICE", "off"),
        ]))
        .unwrap();
        assert!(matches!(off, VoiceSettings::Off(VoiceOff::Disabled)));
    }

    #[test]
    fn settings_refuse_invalid_values_without_echoing_them() {
        for bad in [
            ("DF_GAMEPLAY_VOICE", "maybe"),
            ("DF_GAMEPLAY_VOICE_CHAR_BUDGET", "lots"),
            ("DF_GAMEPLAY_VOICE_CHAR_BUDGET", "1000001"),
            ("DF_GAMEPLAY_VOICE_NARRATOR", "../etc"),
            ("DF_GAMEPLAY_VOICE_NPC", "a b"),
        ] {
            let error = voice_settings(inputs(&[("ELEVENLABS_API_KEY", "k"), bad])).unwrap_err();

            assert!(!error.to_string().contains(bad.1), "{bad:?}");
        }
    }

    #[test]
    fn credential_never_appears_in_debug_output() {
        let credential = VoiceCredential::for_test("sentinel-secret-value");

        assert!(!format!("{credential:?}").contains("sentinel"));
    }
}
