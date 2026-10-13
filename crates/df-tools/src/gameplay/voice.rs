//! Spoken scene narration and NPC lines for the shared display.
//!
//! The server only voices text it put into an outgoing display view: each such line is
//! registered under the SHA-256 key of its role and text, and `GET /gameplay/voice/<key>`
//! serves the audio of a registered line. Audio comes from the on-disk cache when present,
//! otherwise from one single-flight ElevenLabs call whose characters are charged to a
//! per-run budget before the call and never refunded. Every refusal is an empty 503 or 404;
//! the browser then simply stays silent.
//!
//! The public key never includes a voice: the browser does not know voices. Each named
//! speaker is cast to a roster voice at startup, and the on-disk cache name additionally
//! includes the voice id, so a speaker recast to another voice never replays old audio.
mod casting;

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
use casting::{Roster, RosterVoice, cast_speakers};
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
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::startup_configuration::{VoiceConfiguration, VoiceOff, VoiceSettings};
use crate::voice_line::{MAXIMUM_VOICE_CLIP_CHARACTERS, VoiceLine, voice_segments};

/// Most recent registered lines kept; older ones can no longer be requested.
const MAXIMUM_LINES: usize = 256;
const MAXIMUM_LINE_CHARACTERS: usize = MAXIMUM_VOICE_CLIP_CHARACTERS;
const MAXIMUM_ROLE_BYTES: usize = 96;
const MAXIMUM_AUDIO_BYTES: usize = 4 * 1024 * 1024;
const SYNTHESIS_DEADLINE: Duration = Duration::from_secs(30);
const CACHE_CONTROL: &str = "private, max-age=86400";

/// The gender a speaker is voiced with. It is a hard constraint when casting.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Gender {
    He,
    She,
    They,
    It,
}

impl Gender {
    pub(super) fn parse(text: &str) -> Option<Self> {
        match text {
            "he" => Some(Self::He),
            "she" => Some(Self::She),
            "they" => Some(Self::They),
            "it" => Some(Self::It),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::He => "he",
            Self::She => "she",
            Self::They => "they",
            Self::It => "it",
        }
    }

    /// The roster field naming this gender's voices, for errors.
    fn roster_group_field(self) -> &'static str {
        match self {
            Self::He => "npc_voices.he",
            Self::She => "npc_voices.she",
            Self::They => "npc_voices.they",
            Self::It => "npc_voices.it",
        }
    }
}

/// How old a speaker sounds; casting prefers a voice of the same age.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Age {
    Young,
    Adult,
    Elder,
}

impl Age {
    pub(super) fn parse(text: &str) -> Option<Self> {
        match text {
            "young" => Some(Self::Young),
            "adult" => Some(Self::Adult),
            "elder" => Some(Self::Elder),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Young => "young",
            Self::Adult => "adult",
            Self::Elder => "elder",
        }
    }
}

/// One speaking character of the adventure, as the caller knows them at startup.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct SpeakerCasting {
    /// The name the dialogue is attributed to (the display's speaker).
    pub(super) name: String,
    pub(super) gender: Gender,
    pub(super) age: Option<Age>,
    /// Free-text casting phrase, such as "gruff older woman"; only its words are compared.
    pub(super) tone: String,
}

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
    Part { bytes: Bytes, count: usize },
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
        if line.role.len() > MAXIMUM_ROLE_BYTES {
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

/// The voice one line is spoken with.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ChosenVoice {
    voice_id: String,
    /// Short name for diagnostics; never a credential.
    name: String,
    /// The speaker had no casting entry, so the neutral fallback voice was used.
    fallback: bool,
}

impl ChosenVoice {
    fn from_roster(voice: &RosterVoice, fallback: bool) -> Self {
        Self {
            voice_id: voice.voice_id.clone(),
            name: voice.name.clone(),
            fallback,
        }
    }
}

/// Which voice speaks which line.
struct Casting {
    narrator: ChosenVoice,
    /// When set, every NPC line uses it (a testing override).
    forced: Option<ChosenVoice>,
    /// Cast voices by lowercase speaker name.
    by_speaker: HashMap<String, ChosenVoice>,
    fallback: ChosenVoice,
}

fn speaker_key(name: &str) -> String {
    name.trim().to_lowercase()
}

impl Casting {
    fn voice_for(&self, line: &VoiceLine) -> &ChosenVoice {
        if !line.is_npc() {
            return &self.narrator;
        }
        let speaker = line.speaker().unwrap_or_default();
        self.forced
            .as_ref()
            .or_else(|| self.by_speaker.get(&speaker_key(speaker)))
            .unwrap_or(&self.fallback)
    }
}

/// On-disk cache file name: the voice id is part of the audio's identity, so recasting a
/// speaker never replays audio made with another voice. Voice ids contain no newline.
fn cache_file_name(voice_id: &str, line: &VoiceLine) -> String {
    let mut hasher = Sha256::new();
    for part in [voice_id, &line.role, &line.text] {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct Synthesis {
    provider: NativeSpeechProvider,
    casting: Casting,
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

    /// Builds the production ElevenLabs flash route from validated settings. `speakers` are
    /// cast from the roster file named by the settings; `run_identifier` (the adventure
    /// seed) varies the cast between adventures. An unusable roster refuses startup.
    pub(super) fn start(
        settings: VoiceSettings,
        speakers: &[SpeakerCasting],
        run_identifier: &str,
    ) -> Result<Self, io::Error> {
        let configuration = match settings {
            VoiceSettings::Off(reason) => return Ok(Self::disabled(reason)),
            VoiceSettings::On(configuration) => configuration,
        };
        let roster = Roster::load(&configuration.casting_path)
            .map_err(|error| io::Error::other(error.to_string()))?;
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
        Self::new(configuration, provider, &roster, speakers, run_identifier)
    }

    /// Uses `provider` for synthesis; tests pass one pointed at a loopback fake.
    fn new(
        configuration: VoiceConfiguration,
        provider: NativeSpeechProvider,
        roster: &Roster,
        speakers: &[SpeakerCasting],
        run_identifier: &str,
    ) -> Result<Self, io::Error> {
        std::fs::create_dir_all(&configuration.cache_dir)?;
        let cast = cast_speakers(
            roster,
            &configuration.narrator_voice,
            speakers,
            run_identifier,
        );
        let forced = configuration
            .npc_voice_override
            .map(|voice_id| ChosenVoice {
                name: voice_id.clone(),
                voice_id,
                fallback: false,
            });
        let mut summary = format!(
            "VOICE on narrator={} npc={} budget={} cache={}",
            configuration.narrator_voice,
            match &forced {
                Some(forced) => format!("forced:{}", forced.voice_id),
                None => format!(
                    "cast({} speakers, fallback {})",
                    cast.len(),
                    roster.fallback.name
                ),
            },
            configuration.character_budget,
            configuration.cache_dir.display()
        );
        let mut by_speaker = HashMap::new();
        if forced.is_none() {
            for speaker in speakers {
                let Some(voice) = cast.get(&speaker.name).and_then(|id| roster.voice(id)) else {
                    continue;
                };
                let known = by_speaker
                    .insert(
                        speaker_key(&speaker.name),
                        ChosenVoice::from_roster(voice, false),
                    )
                    .is_some();
                if known {
                    continue;
                }
                let age = speaker
                    .age
                    .map_or_else(String::new, |age| format!("/{}", age.label()));
                summary.push_str(&format!(
                    "\nVOICE cast {} {}{age} -> {}",
                    speaker.name,
                    speaker.gender.label(),
                    voice.name
                ));
            }
        }
        let narrator_name = if configuration.narrator_voice == roster.narrator_voice_id {
            roster.narrator_name.clone()
        } else {
            configuration.narrator_voice.clone()
        };
        let casting = Casting {
            narrator: ChosenVoice {
                voice_id: configuration.narrator_voice,
                name: narrator_name,
                fallback: false,
            },
            forced,
            by_speaker,
            fallback: ChosenVoice::from_roster(&roster.fallback, true),
        };
        let synthesis = Synthesis {
            provider,
            casting,
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

    /// The startup lines (the summary, then one per cast speaker); they never contain the
    /// credential.
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
        self.audio_part(key, 0).await
    }

    pub(super) async fn audio_part(&self, key: &str, index: usize) -> Lookup {
        let Some(line) = locked(&self.inner.lines).by_key.get(key).cloned() else {
            return Lookup::Unregistered;
        };
        let Mode::On { synthesis, .. } = &self.inner.mode else {
            return Lookup::Unavailable;
        };
        let segments = voice_segments(&line.text);
        let Some(text) = segments.get(index) else {
            return Lookup::Unregistered;
        };
        let segment = VoiceLine {
            role: line.role,
            text: text.clone(),
        };
        match synthesis.audio(segment).await {
            Ok(bytes) if segments.len() == 1 => Lookup::Audio(bytes),
            Ok(bytes) => Lookup::Part {
                bytes,
                count: segments.len(),
            },
            Err(_) => Lookup::Unavailable,
        }
    }
}

impl Synthesis {
    async fn audio(self: &Arc<Self>, line: VoiceLine) -> Outcome {
        let voice = self.casting.voice_for(&line).clone();
        let key = cache_file_name(&voice.voice_id, &line);
        if let Some(bytes) = self.read_cached(&key).await {
            return Ok(bytes);
        }
        let mut receiver = {
            let mut flights = locked(&self.in_flight);
            match flights.get(&key) {
                Some(receiver) => receiver.clone(),
                None => {
                    let (sender, receiver) = watch::channel(None);
                    flights.insert(key.clone(), receiver.clone());
                    tokio::spawn(Arc::clone(self).fly(key, voice, line, sender));
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

    /// The one task of a cache key (voice, role and text). Callers that go away do not cancel
    /// it: its charge is already spent, so its audio is finished and cached for the next
    /// request.
    async fn fly(
        self: Arc<Self>,
        key: String,
        voice: ChosenVoice,
        line: VoiceLine,
        sender: watch::Sender<Option<Outcome>>,
    ) {
        let outcome = self.produce(&key, &voice, &line).await;
        locked(&self.in_flight).remove(&key);
        // No receiver left means every requester disconnected; the cache holds the result.
        sender.send_replace(Some(outcome));
    }

    async fn produce(&self, key: &str, voice: &ChosenVoice, line: &VoiceLine) -> Outcome {
        // A flight that finished between the caller's cache check and this one wrote it.
        if let Some(bytes) = self.read_cached(key).await {
            return Ok(bytes);
        }
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&context, "gameplay.voice_synthesis");
        span.event(
            "gameplay.voice.chosen",
            &[
                ("gameplay.voice.name", voice.name.clone()),
                ("gameplay.voice.fallback", voice.fallback.to_string()),
            ],
        );
        let outcome = self.synthesize(key, voice, line).await;
        span.finish_unmeasured(match &outcome {
            Ok((_, true)) => "synthesized",
            Ok((_, false)) => "synthesized_uncached",
            Err(failure) => failure.label(),
        });
        outcome.map(|(bytes, _)| bytes)
    }

    /// Charges first, calls the provider once and caches the audio; the boolean tells
    /// whether the cache write succeeded.
    async fn synthesize(
        &self,
        key: &str,
        voice: &ChosenVoice,
        line: &VoiceLine,
    ) -> Result<(Bytes, bool), Failure> {
        let characters = u32::try_from(line.text.chars().count()).map_err(|_| Failure::Request)?;
        self.charge(characters)?;
        let audio = self.request_audio(&voice.voice_id, line).await?;
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
        .route("/gameplay/voice/{key}/{part}", get(serve_part))
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
        Lookup::Part { bytes, count } => audio_parts_response(bytes, count),
        Lookup::Unregistered => StatusCode::NOT_FOUND.into_response(),
        Lookup::Unavailable => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn serve_part(
    State(voice): State<Voice>,
    RoutePath((key, part)): RoutePath<(String, usize)>,
) -> Response {
    match voice.audio_part(&key, part).await {
        Lookup::Audio(bytes) => audio_parts_response(bytes, 1),
        Lookup::Part { bytes, count } => audio_parts_response(bytes, count),
        Lookup::Unregistered => StatusCode::NOT_FOUND.into_response(),
        Lookup::Unavailable => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

fn audio_parts_response(bytes: Bytes, count: usize) -> Response {
    let mut response = (
        [
            (header::CONTENT_TYPE, "audio/mpeg"),
            (header::CACHE_CONTROL, CACHE_CONTROL),
        ],
        bytes,
    )
        .into_response();
    let Ok(count) = header::HeaderValue::try_from(count.to_string()) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    response
        .headers_mut()
        .insert(header::HeaderName::from_static("x-voice-parts"), count);
    response
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
    use super::casting::fixtures::test_roster;
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
            npc_voice_override: None,
            casting_path: PathBuf::from("voice-casting.json"),
        }
    }

    fn voice_with(fake: &FakeEleven, cache: &Path, budget: u32) -> Voice {
        voice_cast(fake, configuration(cache, budget), &[], "0x1")
    }

    fn voice_cast(
        fake: &FakeEleven,
        configuration: VoiceConfiguration,
        speakers: &[SpeakerCasting],
        run_identifier: &str,
    ) -> Voice {
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
        Voice::new(
            configuration,
            provider,
            &test_roster(),
            speakers,
            run_identifier,
        )
        .unwrap()
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
        let name = cache_file_name("narratorvoice", &narrator("The lanterns sway."));
        assert_eq!(std::fs::read(cache.join(name)).unwrap(), AUDIO);
        let first_target = fake.targets.lock().unwrap()[0].clone();
        assert!(first_target.contains("/v1/text-to-speech/narratorvoice/stream"));
    }

    #[tokio::test]
    async fn long_line_parts_are_individually_addressable_and_cached() {
        let cache = scratch("long-line-parts");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache, 1000);
        let line = narrator(&format!("{}. {}.", "a".repeat(350), "b".repeat(300)));
        let key = register(&voice, &line);

        let first = serve_line(State(voice.clone()), RoutePath(key.clone())).await;
        let second = serve_part(State(voice.clone()), RoutePath((key.clone(), 1))).await;
        let replay = serve_part(State(voice), RoutePath((key, 1))).await;

        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()["x-voice-parts"], "2");
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(second.headers()["x-voice-parts"], "2");
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(fake.calls(), 2);
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
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    }

    fn npc(speaker: &str, text: &str) -> VoiceLine {
        VoiceLine {
            role: format!("npc:{speaker}"),
            text: text.to_owned(),
        }
    }

    fn speaker(name: &str, gender: Gender, age: Option<Age>, tone: &str) -> SpeakerCasting {
        SpeakerCasting {
            name: name.to_owned(),
            gender,
            age,
            tone: tone.to_owned(),
        }
    }

    /// Speaks `line` and returns the voice id the fake provider's latest request targeted.
    async fn voice_id_requested(voice: &Voice, fake: &FakeEleven, line: &VoiceLine) -> String {
        let key = register(voice, line);
        assert_eq!(
            voice.audio(&key).await,
            Lookup::Audio(Bytes::from_static(AUDIO))
        );
        let target = fake.targets.lock().unwrap().last().unwrap().clone();
        let path = target.split_whitespace().nth(1).unwrap().to_owned();
        path.strip_prefix("/v1/text-to-speech/")
            .and_then(|rest| rest.split('/').next())
            .unwrap()
            .to_owned()
    }

    fn roster_voice_gender(voice_id: &str) -> Option<Gender> {
        test_roster().voice(voice_id).map(|voice| voice.gender)
    }

    #[tokio::test]
    async fn a_she_speaker_is_voiced_with_a_she_voice_and_a_he_speaker_with_a_he_voice() {
        let cache = scratch("gender");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let speakers = [
            speaker("Wren", Gender::She, Some(Age::Elder), "warm older woman"),
            speaker("Garrick", Gender::He, Some(Age::Adult), "gruff man"),
        ];
        let voice = voice_cast(&fake, configuration(&cache, 1000), &speakers, "0x7");

        let wren = voice_id_requested(&voice, &fake, &npc("Wren", "Welcome in.")).await;
        let garrick = voice_id_requested(&voice, &fake, &npc("Garrick", "Move along.")).await;

        assert_eq!(roster_voice_gender(&wren), Some(Gender::She));
        assert_eq!(roster_voice_gender(&garrick), Some(Gender::He));
    }

    #[tokio::test]
    async fn they_and_it_speakers_get_their_own_gender_groups() {
        let cache = scratch("they-it");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let speakers = [
            speaker("Marl", Gender::They, None, "calm"),
            speaker("Skrit", Gender::It, None, "raspy"),
        ];
        let voice = voice_cast(&fake, configuration(&cache, 1000), &speakers, "0x7");

        let marl = voice_id_requested(&voice, &fake, &npc("Marl", "Hello.")).await;
        let skrit = voice_id_requested(&voice, &fake, &npc("Skrit", "Hiss.")).await;

        assert_eq!(roster_voice_gender(&marl), Some(Gender::They));
        assert_eq!(skrit, "it1");
    }

    #[tokio::test]
    async fn two_speakers_of_one_adventure_are_voiced_differently() {
        let cache = scratch("distinct");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let speakers = [
            speaker("Wren", Gender::She, Some(Age::Young), "warm"),
            speaker("Brisa", Gender::She, Some(Age::Young), "warm"),
        ];
        let voice = voice_cast(&fake, configuration(&cache, 1000), &speakers, "0x7");

        let wren = voice_id_requested(&voice, &fake, &npc("Wren", "Welcome in.")).await;
        let brisa = voice_id_requested(&voice, &fake, &npc("Brisa", "Welcome in.")).await;

        assert_ne!(wren, brisa);
    }

    #[tokio::test]
    async fn an_unknown_speaker_gets_the_neutral_fallback_never_a_gendered_voice() {
        let cache = scratch("fallback");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let speakers = [speaker("Wren", Gender::She, None, "")];
        let voice = voice_cast(&fake, configuration(&cache, 1000), &speakers, "0x7");

        let stranger = voice_id_requested(&voice, &fake, &npc("Stranger", "Who goes?")).await;

        assert_eq!(stranger, "they1");
        let Mode::On { synthesis, .. } = &voice.inner.mode else {
            panic!("voice should be on");
        };
        assert!(
            synthesis
                .casting
                .voice_for(&npc("Stranger", "Who goes?"))
                .fallback
        );
        assert!(!synthesis.casting.voice_for(&npc("Wren", "Hm.")).fallback);
    }

    #[tokio::test]
    async fn speaker_names_match_regardless_of_case_and_the_narrator_keeps_its_voice() {
        let cache = scratch("case");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let speakers = [speaker("Wren", Gender::She, None, "")];
        let voice = voice_cast(&fake, configuration(&cache, 1000), &speakers, "0x7");

        let loud = voice_id_requested(&voice, &fake, &npc(" WREN ", "Hello.")).await;
        let narration = voice_id_requested(&voice, &fake, &narrator("Rain falls.")).await;

        assert_eq!(roster_voice_gender(&loud), Some(Gender::She));
        assert_eq!(narration, "narratorvoice");
    }

    #[tokio::test]
    async fn the_forced_npc_voice_speaks_every_named_speaker() {
        let cache = scratch("forced");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let mut forced = configuration(&cache, 1000);
        forced.npc_voice_override = Some("npcvoice".to_owned());
        let speakers = [speaker("Wren", Gender::She, None, "")];
        let voice = voice_cast(&fake, forced, &speakers, "0x7");

        let wren = voice_id_requested(&voice, &fake, &npc("Wren", "Hello.")).await;
        let stranger = voice_id_requested(&voice, &fake, &npc("Stranger", "Hello.")).await;

        assert_eq!((wren.as_str(), stranger.as_str()), ("npcvoice", "npcvoice"));
        let line = voice.startup_line();
        assert!(line.contains("npc=forced:npcvoice"));
        assert!(!line.contains("VOICE cast"));
    }

    #[test]
    fn startup_reports_one_line_per_cast_speaker_without_the_credential() {
        let cache = scratch("startup-lines");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let speakers = [
            speaker("Wren", Gender::She, Some(Age::Elder), "warm"),
            speaker("Odo", Gender::He, None, ""),
        ];
        let voice = voice_cast(&fake, configuration(&cache, 1000), &speakers, "0x7");

        let text = voice.startup_line();

        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("VOICE on narrator=narratorvoice npc=cast(2 speakers"));
        assert!(lines[1].starts_with("VOICE cast Wren she/elder -> she"));
        assert!(lines[2].starts_with("VOICE cast Odo he -> he"));
        assert!(!text.contains("test-credential"));
    }

    #[tokio::test]
    async fn recasting_a_speaker_changes_the_cache_file_but_not_the_public_key() {
        let cache = scratch("recast");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let line = npc("Wren", "Welcome in.");
        let first = voice_cast(
            &fake,
            configuration(&cache, 1000),
            &[speaker("Wren", Gender::She, Some(Age::Young), "")],
            "0x7",
        );
        let first_voice = voice_id_requested(&first, &fake, &line).await;
        let public_key = line.key();

        let recast = voice_cast(
            &fake,
            configuration(&cache, 1000),
            &[speaker("Wren", Gender::She, Some(Age::Elder), "")],
            "0x7",
        );
        let second_voice = voice_id_requested(&recast, &fake, &line).await;

        assert_ne!(first_voice, second_voice);
        assert_eq!(line.key(), public_key);
        assert_eq!(
            fake.calls(),
            2,
            "the old voice's cached audio must not be replayed"
        );
        assert!(cache.join(cache_file_name(&first_voice, &line)).is_file());
        assert!(cache.join(cache_file_name(&second_voice, &line)).is_file());
        assert!(!cache.join(&public_key).exists());
    }

    #[test]
    fn cache_names_are_the_sha256_of_voice_role_and_text() {
        let line = npc("Wren", "Welcome in.");

        let name = cache_file_name("abc", &line);

        let mut expected = Sha256::new();
        expected.update(b"abc\nnpc:Wren\nWelcome in.\n");
        let expected: String = expected
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(name, expected);
        assert_ne!(name, line.key());
    }

    #[test]
    fn starting_with_an_unusable_roster_is_refused_with_a_typed_message() {
        let dir = scratch("roster-file");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("voice-casting.json");
        std::fs::write(&path, br#"{"schema":"df-voice-casting/1"}"#).unwrap();
        let mut malformed = configuration(&dir, 1000);
        malformed.casting_path = path;
        let mut missing = configuration(&dir, 1000);
        missing.casting_path = dir.join("absent.json");

        let malformed = Voice::start(VoiceSettings::On(malformed), &[], "0x1")
            .err()
            .unwrap();
        let missing = Voice::start(VoiceSettings::On(missing), &[], "0x1")
            .err()
            .unwrap();

        assert!(
            malformed.to_string().contains("voice roster malformed"),
            "{malformed}"
        );
        assert!(missing.to_string().contains("unreadable"), "{missing}");
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
    fn registry_keeps_the_most_recent_lines_including_long_ones() {
        let mut lines = Lines::default();
        for index in 0..MAXIMUM_LINES + 10 {
            lines.register(narrator(&format!("line {index}")));
        }
        let long_line = narrator(&"x".repeat(MAXIMUM_LINE_CHARACTERS + 1));
        lines.register(long_line.clone());

        assert_eq!(lines.by_key.len(), MAXIMUM_LINES);
        assert_eq!(lines.order.len(), MAXIMUM_LINES);
        assert!(!lines.by_key.contains_key(&narrator("line 0").key()));
        assert!(lines.by_key.contains_key(&narrator("line 265").key()));
        assert!(lines.by_key.contains_key(&long_line.key()));
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
            casting: pick("DF_GAMEPLAY_VOICE_CASTING"),
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
        assert_eq!(configuration.npc_voice_override, None);
        assert_eq!(
            configuration.casting_path,
            PathBuf::from("content/voice-casting.json")
        );
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
            ("DF_GAMEPLAY_VOICE_CASTING", "/roster/voices.json"),
        ];
        let VoiceSettings::On(configuration) = voice_settings(inputs(&overrides)).unwrap() else {
            panic!("voice should be on");
        };
        assert_eq!(configuration.character_budget, 300);
        assert_eq!(configuration.cache_dir, PathBuf::from("/tmp/somewhere"));
        assert_eq!(configuration.narrator_voice, "narr_1");
        assert_eq!(configuration.npc_voice_override.as_deref(), Some("npc-2"));
        assert_eq!(
            configuration.casting_path,
            PathBuf::from("/roster/voices.json")
        );

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
