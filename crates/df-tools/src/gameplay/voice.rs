//! Spoken scene narration and NPC lines for the shared display.
//!
//! The server only voices text it put into an outgoing display view: each such line is
//! registered under the SHA-256 key of its role and text, and `GET /gameplay/voice/<key>`
//! serves the audio of a registered line. Audio comes from the on-disk cache when present,
//! otherwise from one single-flight ElevenLabs call whose characters are recorded on the hub's
//! ledger when the call is sent and never given back. There is no cap. Every refusal is an empty
//! 503 or 404;
//! the browser then simply stays silent.
//!
//! The public key never includes a voice: the browser does not know voices. Each named
//! speaker is cast to a roster voice when their saved identity is admitted. The audio itself is a `VoiceLine` resource of
//! the generation hub: its key (and its file name in the cache) is the hash of the voice id, the
//! role, the clip text, the model, the output format, the language, the clip split and the
//! contract version ([`VoiceLineSpec`]), so a speaker recast to another voice, or a provider
//! change of model, format or language, never replays old audio. The hub owns the state, the
//! failure memory and the attempt limit, and the characters spoken are its ledger's
//! `VoiceCharacter` account.
mod casting;

use std::{
    collections::HashMap,
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use super::{
    resource_adapters::{Flights, Kit, KitSlot, Outcome as HubOutcome, Produced, Role},
    resource_adapters::{hex, slug, spec_digest, write_atomic},
    resources::{Chrome, Generated, Limits, ResourceFailure, ResourceKind, ResourceSpec, Unit},
};
use axum::{
    Router,
    extract::{Path as RoutePath, State},
    http::{HeaderMap, StatusCode, Uri, header},
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
    CandidateRouteId, ELEVEN_TTS_LANGUAGE, ELEVEN_TTS_MODEL, ElevenSpeechBasis,
    ElevenSpeechRequest, NativeSpeechProvider, ProviderLimits, SpeechOutputFormat,
    registered_speech_provider,
};
use df_render::art_contract::Neutral;
use df_types::{
    LocaleTag, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage,
    UsageUnit,
};

use super::startup_configuration::{VoiceConfiguration, VoiceOff, VoiceSettings};
use crate::voice_line::{MAXIMUM_VOICE_CLIP_CHARACTERS, VoiceLine, voice_segments};

/// Most recent registered lines kept; older ones can no longer be requested.
const MAXIMUM_LINES: usize = 256;
const MAXIMUM_LINE_CHARACTERS: usize = MAXIMUM_VOICE_CLIP_CHARACTERS;
// Generated actor IDs contain two 32-byte slugs and a separator; names allow 32 Unicode scalars.
const MAXIMUM_ROLE_BYTES: usize = "npc-id:".len() + 65 + 1 + 32 * 4;
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
    pub(super) actor_ids: Vec<String>,
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
    Request,
    Provider,
    TimedOut,
    Interrupted,
}

impl Failure {
    fn label(self) -> &'static str {
        match self {
            Self::Request => "request_refused",
            Self::Provider => "provider_failed",
            Self::TimedOut => "timed_out",
            Self::Interrupted => "interrupted",
        }
    }

    /// How the hub remembers it: a refused request would fail again, a provider failure may
    /// pass.
    fn to_hub(self) -> ResourceFailure {
        match self {
            Self::Request => ResourceFailure::Permanent("request_refused"),
            Self::Provider => ResourceFailure::Transient("provider_failed"),
            Self::TimedOut => ResourceFailure::TimedOut,
            Self::Interrupted => ResourceFailure::Interrupted,
        }
    }

    fn from_hub(failure: &ResourceFailure) -> Self {
        match failure {
            ResourceFailure::Permanent(_) => Self::Request,
            ResourceFailure::Transient(_) => Self::Provider,
            ResourceFailure::TimedOut => Self::TimedOut,
            ResourceFailure::Interrupted => Self::Interrupted,
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
            super::actor::record_audit_line(
                "gameplay.voice.cast",
                format!(
                    "VOICE unregistered reason=role_capacity bytes={} limit={MAXIMUM_ROLE_BYTES}",
                    line.role.len()
                ),
            );
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
    by_actor: HashMap<String, ChosenVoice>,
    fallback: ChosenVoice,
    roster: Roster,
    run_identifier: String,
    generated: Mutex<HashMap<String, GeneratedCasting>>,
}

struct GeneratedCasting {
    speaker: SpeakerCasting,
    voice: ChosenVoice,
    reason: String,
}

fn speaker_key(name: &str) -> String {
    name.trim().to_lowercase()
}

impl Casting {
    fn voice_for(&self, line: &VoiceLine) -> ChosenVoice {
        if !line.is_npc() {
            return self.narrator.clone();
        }
        if let Some(forced) = &self.forced {
            return forced.clone();
        }
        let generated = locked(&self.generated);
        let selected = match line.actor_id() {
            Some(id) => self
                .by_actor
                .get(id)
                .or_else(|| generated.get(id).map(|cast| &cast.voice)),
            None => {
                let key = speaker_key(line.speaker().unwrap_or_default());
                self.by_speaker.get(&key).or_else(|| {
                    generated
                        .values()
                        .find(|cast| speaker_key(&cast.speaker.name) == key)
                        .map(|cast| &cast.voice)
                })
            }
        };
        selected.unwrap_or(&self.fallback).clone()
    }

    fn register_place(
        &self,
        place: &super::place::PlaceRecord,
    ) -> Result<(), super::place::PlaceRefusal> {
        let mut cast = place.expand()?.cast;
        cast.sort_by(|left, right| left.id.cmp(&right.id));
        let mut generated = locked(&self.generated);
        let mut uses = HashMap::<String, usize>::new();
        for person in &cast {
            if let Some(voice) = self
                .by_actor
                .get(&person.id)
                .or_else(|| generated.get(&person.id).map(|cast| &cast.voice))
            {
                *uses.entry(voice.voice_id.clone()).or_default() += 1;
            }
        }
        for person in cast {
            if self.by_actor.contains_key(&person.id) || generated.contains_key(&person.id) {
                continue;
            }
            let gender = match person.gender {
                Some(super::pack::Gender::He) => Gender::He,
                Some(super::pack::Gender::She) => Gender::She,
                Some(super::pack::Gender::They) => Gender::They,
                Some(super::pack::Gender::It) => Gender::It,
                None => continue,
            };
            let age = match person.age {
                Some(super::pack::Age::Young) => Age::Young,
                Some(super::pack::Age::Elder) => Age::Elder,
                _ => Age::Adult,
            };
            let selected = self
                .roster
                .voices
                .iter()
                .filter(|voice| voice.gender == gender && voice.voice_id != self.narrator.voice_id)
                .min_by_key(|voice| {
                    use sha2::{Digest, Sha256};
                    let tie = Sha256::digest(format!(
                        "{}\0{}\0{}",
                        self.run_identifier, person.id, voice.voice_id
                    ));
                    (
                        (age as i8 - voice.age as i8).unsigned_abs(),
                        uses.get(&voice.voice_id).copied().unwrap_or(0),
                        tie.to_vec(),
                    )
                });
            let voice = selected.map_or_else(
                || self.fallback.clone(),
                |voice| ChosenVoice::from_roster(voice, false),
            );
            let reason = match selected {
                Some(selected) if selected.age == age => {
                    "presentation_and_age; scene_voice_diversity"
                }
                Some(_) => "presentation_and_nearest_available_age; scene_voice_diversity",
                None => "fallback: no_matching_presentation_in_roster",
            }
            .to_owned();
            super::actor::record_audit_line(
                "gameplay.voice.cast",
                format!(
                    "VOICE cast {} {}/{} -> {}{} reason={reason}",
                    person.name,
                    gender.label(),
                    age.label(),
                    voice.name,
                    if voice.fallback { " fallback" } else { "" }
                ),
            );
            *uses.entry(voice.voice_id.clone()).or_default() += 1;
            generated.insert(
                person.id.clone(),
                GeneratedCasting {
                    speaker: SpeakerCasting {
                        actor_ids: vec![person.id],
                        name: person.name,
                        gender,
                        age: Some(age),
                        tone: String::new(),
                    },
                    voice,
                    reason,
                },
            );
        }
        Ok(())
    }
}

/// One clip of speech as a `VoiceLine` resource: everything that decides what the audio sounds
/// like. The hub key is the hash of these with the contract version, and the cache file is named
/// by the whole digest of the same text, so changing any of them is a different clip. Voice ids
/// contain no newline.
#[derive(Clone, Debug)]
pub(super) struct VoiceLineSpec {
    voice_id: String,
    role: String,
    /// The clip's text: one segment of a long line (see `voice_segments`).
    text: String,
    model: String,
    format: String,
    language: String,
    /// The most characters a clip holds, which decides where a long line is split.
    clip_limit: usize,
}

impl VoiceLineSpec {
    /// The clip `line` is spoken as by `voice_id`, with the provider's own model, format and
    /// language.
    pub(super) fn new(voice_id: &str, line: &VoiceLine) -> Self {
        Self {
            voice_id: voice_id.to_owned(),
            role: line.role.clone(),
            text: line.text.clone(),
            model: ELEVEN_TTS_MODEL.to_owned(),
            format: SpeechOutputFormat::Mp3At22050Hz32Kbps
                .query_value()
                .to_owned(),
            language: ELEVEN_TTS_LANGUAGE.to_owned(),
            clip_limit: MAXIMUM_VOICE_CLIP_CHARACTERS,
        }
    }

    /// The cache file name and the key the clip is stored under.
    pub(super) fn file_name(&self) -> String {
        hex(&spec_digest(self))
    }
}

impl ResourceSpec for VoiceLineSpec {
    const KIND: ResourceKind = ResourceKind::VoiceLine;
    const CONTRACT_VERSION: u32 = 1;

    fn canonical(&self) -> String {
        serde_json::json!({
            "voice": self.voice_id,
            "role": self.role,
            "text": self.text,
            "model": self.model,
            "format": self.format,
            "language": self.language,
            "clip_limit": self.clip_limit,
        })
        .to_string()
    }

    fn subject(&self) -> String {
        format!("voice:{}", slug(&self.role, 48))
    }

    fn chrome(&self) -> Chrome {
        Neutral::Nothing
    }
}

/// What a hub run of a clip hands the waiters: the audio, shared by every caller of one key.
type Clip = HubOutcome<Bytes>;

struct Synthesis {
    provider: NativeSpeechProvider,
    casting: Casting,
    cache_dir: PathBuf,
    kit: KitSlot,
    /// Identical concurrent asks for one clip share one provider call.
    flights: Flights<Clip>,
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
    people_inputs: Mutex<Option<(df_model::checkpoint::GameState, super::pack::CacheIdentity)>>,
    /// Bounded prewarming waiters, aborted on drop. Dispatched paid hub flights retain their
    /// existing deadline and finish/store policy even when their waiters disappear.
    prefetch: Mutex<tokio::task::JoinSet<()>>,
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
                people_inputs: Mutex::new(None),
                prefetch: Mutex::new(tokio::task::JoinSet::new()),
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
        let limits = ProviderLimits::new(SYNTHESIS_DEADLINE, 16 * 1024, MAXIMUM_AUDIO_BYTES);
        let provider = registered_speech_provider(
            CandidateRouteId::ElevenFlashV25Tts,
            configuration.credential.expose(),
            limits.build(),
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
            "VOICE on narrator={} npc={} cache={}",
            configuration.narrator_voice,
            match &forced {
                Some(forced) => format!("forced:{}", forced.voice_id),
                None => format!(
                    "cast({} speakers, fallback {})",
                    cast.len(),
                    roster.fallback.name
                ),
            },
            configuration.cache_dir.display()
        );
        let mut by_speaker = HashMap::new();
        let mut by_actor = HashMap::new();
        if forced.is_none() {
            for speaker in speakers {
                let Some(voice) = cast.get(&speaker.name).and_then(|id| roster.voice(id)) else {
                    continue;
                };
                for id in &speaker.actor_ids {
                    by_actor.insert(id.clone(), ChosenVoice::from_roster(voice, false));
                }
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
            by_actor,
            fallback: ChosenVoice::from_roster(&roster.fallback, true),
            roster: roster.clone(),
            run_identifier: run_identifier.to_owned(),
            generated: Mutex::new(HashMap::new()),
        };
        let synthesis = Synthesis {
            provider,
            casting,
            cache_dir: configuration.cache_dir,
            kit: KitSlot::private(),
            flights: Flights::default(),
        };
        synthesis.configure();
        Ok(Self {
            inner: Arc::new(Inner {
                lines: Mutex::new(Lines::default()),
                people_inputs: Mutex::new(None),
                prefetch: Mutex::new(tokio::task::JoinSet::new()),
                mode: Mode::On {
                    synthesis: Arc::new(synthesis),
                    summary,
                },
            }),
        })
    }

    /// Puts the voice on the server's generation hub, so its clips, failures and spend are listed
    /// and invalidated with every other resource. A disabled voice has nothing to attach.
    pub(super) fn attach_kit(&self, kit: &Kit) {
        if let Mode::On { synthesis, .. } = &self.inner.mode {
            synthesis.kit.attach(kit);
            synthesis.configure();
        }
    }

    /// The startup lines (the summary, then one per cast speaker); they never contain the
    /// credential.
    pub(super) fn startup_line(&self) -> String {
        match &self.inner.mode {
            Mode::Off(reason) => format!("VOICE off ({})", reason.reason()),
            Mode::On { summary, synthesis } => {
                let generated = locked(&synthesis.casting.generated);
                let mut cast: Vec<_> = generated.values().collect();
                cast.sort_by_key(|cast| &cast.speaker.name);
                let mut summary = summary.clone();
                for cast in cast {
                    summary.push_str(&format!(
                        "\nVOICE cast {} {}/{} -> {} reason={}",
                        cast.speaker.name,
                        cast.speaker.gender.label(),
                        cast.speaker.age.map_or("unknown", Age::label),
                        cast.voice.name,
                        cast.reason
                    ));
                }
                summary
            }
        }
    }

    /// Replays committed person admission in journal order. The free-roam gate predates that
    /// journal; local spots may reuse its keepers, so it is always admitted first.
    pub(super) fn register_people(
        &self,
        state: &df_model::checkpoint::GameState,
    ) -> Result<(), df_session::submission::RepositoryError> {
        let Mode::On { synthesis, .. } = &self.inner.mode else {
            return Ok(());
        };
        let source = super::pack::cache_identity();
        let mut inputs = locked(&self.inner.people_inputs);
        if inputs
            .as_ref()
            .is_some_and(|(known, pack)| *pack == source && known == state)
        {
            return Ok(());
        }
        if let Some(world) = super::adventure::free_roam::world() {
            synthesis
                .casting
                .register_place(&world.gate)
                .map_err(|_| df_session::submission::RepositoryError::InvalidCandidate)?;
        }
        for (_, entry) in super::place::entries(state)? {
            if let Some(place) = entry.change.created {
                synthesis
                    .casting
                    .register_place(&place)
                    .map_err(|_| df_session::submission::RepositoryError::InvalidCandidate)?;
            }
        }
        *inputs = Some((state.clone(), source));
        Ok(())
    }

    /// Registers the speakable text of a view the server is sending. Only the display
    /// audience is voiced; player views register nothing.
    pub(super) fn register_view(&self, message: &rpc::ViewMessage) {
        let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
            return;
        };
        let fresh = {
            let mut lines = locked(&self.inner.lines);
            crate::voice_line::display_lines(display)
                .into_iter()
                .filter_map(|line| {
                    let key = line.key();
                    let fresh = !lines.by_key.contains_key(&key);
                    lines.register(line.clone());
                    (fresh && lines.by_key.contains_key(&key)).then_some(line)
                })
                .collect::<Vec<_>>()
        };
        let (Mode::On { synthesis, .. }, Ok(runtime)) =
            (&self.inner.mode, tokio::runtime::Handle::try_current())
        else {
            return;
        };
        let mut prefetch = locked(&self.inner.prefetch);
        while let Some(result) = prefetch.try_join_next() {
            if result.is_err() {
                super::actor::record_audit_line(
                    "gameplay.voice.prefetch",
                    "VOICE prefetch status=interrupted".into(),
                );
            }
        }
        for line in fresh {
            for text in voice_segments(&line.text) {
                if prefetch.len() >= 8 {
                    return;
                }
                let synthesis = Arc::clone(synthesis);
                let segment = VoiceLine {
                    role: line.role.clone(),
                    text,
                };
                prefetch.spawn_on(
                    async move {
                        // The cache/flight and shared ledger are identical to the later clip request.
                        if synthesis.audio(segment).await.0.is_err() {
                            super::actor::record_audit_line(
                                "gameplay.voice.prefetch",
                                "VOICE prefetch status=unavailable".into(),
                            );
                        }
                    },
                    &runtime,
                );
            }
        }
    }

    #[cfg(test)]
    pub(super) async fn audio(&self, key: &str) -> Lookup {
        self.audio_part(key, 0, None).await
    }

    pub(super) async fn audio_part(
        &self,
        key: &str,
        index: usize,
        presentation_id: Option<u64>,
    ) -> Lookup {
        let started = std::time::Instant::now();
        let resource_id = safe_voice_resource_id(key);
        let Some(line) = locked(&self.inner.lines).by_key.get(key).cloned() else {
            record_voice_resource(
                "gameplay.voice.delivery_failed",
                &resource_id,
                presentation_id,
                index,
                0,
                "failed",
                "unregistered",
                Some("unregistered"),
                "not_applicable",
                None,
                None,
                started.elapsed().as_millis(),
                404,
            );
            return Lookup::Unregistered;
        };
        if crate::fake_profile::is_enabled() {
            let Some(bytes) = crate::fake_profile::silent_part(&line.text, index) else {
                return Lookup::Unregistered;
            };
            super::actor::record_audit_line(
                "gameplay.voice.fake",
                format!("RESOURCE ready kind=voice_line key={resource_id} provider=fake"),
            );
            return Lookup::Audio(bytes.into());
        }
        let Mode::On { synthesis, .. } = &self.inner.mode else {
            record_voice_resource(
                "gameplay.voice.delivery_failed",
                &resource_id,
                presentation_id,
                index,
                0,
                "failed",
                "voice_off",
                Some("voice_off"),
                "not_applicable",
                None,
                None,
                started.elapsed().as_millis(),
                503,
            );
            return Lookup::Unavailable;
        };
        let segments = voice_segments(&line.text);
        let Some(text) = segments.get(index) else {
            record_voice_resource(
                "gameplay.voice.delivery_failed",
                &resource_id,
                presentation_id,
                index,
                segments.len(),
                "failed",
                "part_unavailable",
                Some("part_unavailable"),
                "not_applicable",
                Some(line.text.chars().count()),
                None,
                started.elapsed().as_millis(),
                404,
            );
            return Lookup::Unregistered;
        };
        let segment = VoiceLine {
            role: line.role,
            text: text.clone(),
        };
        record_voice_resource(
            "gameplay.voice.requested",
            &resource_id,
            presentation_id,
            index,
            segments.len(),
            "requested",
            "requested",
            None,
            "unknown",
            Some(text.chars().count()),
            None,
            0,
            0,
        );
        record_voice_resource(
            "gameplay.voice.queued",
            &resource_id,
            presentation_id,
            index,
            segments.len(),
            "queued",
            "queued",
            None,
            "pending",
            Some(text.chars().count()),
            None,
            0,
            0,
        );
        let (outcome, cache_result) = synthesis.audio(segment).await;
        let (status, failure, bytes) = match &outcome {
            Ok(bytes) => ("delivered", "none", Some(bytes.len())),
            Err(failure) => ("failed", failure.label(), None),
        };
        let http_status = match (&outcome, index < segments.len()) {
            (Ok(_), true) => 200,
            (Err(_), true) => 503,
            (_, false) => 404,
        };
        record_voice_resource(
            if outcome.is_ok() {
                "gameplay.voice.delivery"
            } else {
                "gameplay.voice.delivery_failed"
            },
            &resource_id,
            presentation_id,
            index,
            segments.len(),
            if outcome.is_ok() {
                "delivered"
            } else {
                "failed"
            },
            status,
            if outcome.is_ok() { None } else { Some(failure) },
            cache_result,
            Some(text.chars().count()),
            bytes,
            started.elapsed().as_millis(),
            http_status,
        );
        match outcome {
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
    /// Tunes the clip kind on the current kit: the kind's bounds, how a ready entry's file is
    /// checked and how it is erased.
    fn configure(&self) {
        let kit = self.kit.get();
        let checked = self.cache_dir.clone();
        let erased = self.cache_dir.clone();
        kit.tune(
            ResourceKind::VoiceLine,
            Limits::for_kind(ResourceKind::VoiceLine),
            Arc::new(move |generated: &Generated| is_stored(&checked.join(&generated.handle))),
            Arc::new(move |generated: &Generated| {
                // Best effort: a clip that is already gone is what was asked for.
                let _removed = std::fs::remove_file(erased.join(&generated.handle));
            }),
        );
    }

    /// The audio of one clip, and where it came from (`hit`, `in_flight` or `miss`).
    async fn audio(self: &Arc<Self>, line: VoiceLine) -> (Outcome, &'static str) {
        let voice = self.casting.voice_for(&line);
        let kit = self.kit.get();
        // A ready entry whose file vanished between the hub's check and the read is asked for
        // again once.
        let mut result = (Err(Failure::Interrupted), "miss");
        for _ in 0..2 {
            let spec = VoiceLineSpec::new(&voice.voice_id, &line);
            let name = spec.file_name();
            let work = Arc::clone(self).generate(name, voice.clone(), line.clone());
            let (outcome, role) = kit.produce(&self.flights, spec, work).await;
            let produced = match outcome {
                Ok(produced) => produced,
                Err(failure) => return (Err(Failure::from_hub(&failure)), "miss"),
            };
            let source = match (role, produced.from_store) {
                (Role::Joined, _) => "in_flight",
                (Role::Leader, true) => "hit",
                (Role::Leader, false) => "miss",
            };
            let bytes = match produced.payload {
                Some(bytes) => Some(bytes),
                None => self.read_cached(&produced.generated.handle).await,
            };
            match bytes {
                Some(bytes) => return (Ok(bytes), source),
                None => result = (Err(Failure::Provider), source),
            }
        }
        result
    }

    /// The one run of a clip: the stored audio when the store has it, otherwise one paid
    /// synthesis. Callers that go away do not cancel it: its charge is already spent, so its
    /// audio is finished and stored for the next request.
    async fn generate(self: Arc<Self>, name: String, voice: ChosenVoice, line: VoiceLine) -> Clip {
        // A run that finished before this one started stored it.
        if let Some(bytes) = self.read_cached(&name).await {
            return Ok(Produced {
                generated: Generated::new(name),
                payload: Some(bytes),
                from_store: true,
            });
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
        let outcome = self.synthesize(&name, &voice, &line).await;
        span.finish_unmeasured(match &outcome {
            Ok((_, true)) => "synthesized",
            Ok((_, false)) => "synthesized_uncached",
            Err(failure) => failure.label(),
        });
        match outcome {
            Ok((bytes, stored)) => Ok(Produced {
                generated: Generated {
                    handle: name,
                    note: if stored {
                        String::new()
                    } else {
                        "uncached".to_owned()
                    },
                },
                payload: Some(bytes),
                from_store: false,
            }),
            Err(failure) => Err(failure.to_hub()),
        }
    }

    /// Builds the request, charges its characters, calls the provider once and stores the audio;
    /// the boolean tells whether the store write succeeded.
    async fn synthesize(
        &self,
        key: &str,
        voice: &ChosenVoice,
        line: &VoiceLine,
    ) -> Result<(Bytes, bool), Failure> {
        let characters = u64::try_from(line.text.chars().count()).map_err(|_| Failure::Request)?;
        // A request the contract refuses is found before anything is charged.
        let (request, current) = speech_request(&voice.voice_id, line)?;
        // The call is about to be sent: spend is kept whatever the outcome, since a call whose
        // result is unknown may still have been billed.
        self.kit.get().charge(Unit::VoiceCharacter, characters);
        let counted = [(super::cost::Category::Characters, characters)];
        let billed = self
            .kit
            .get()
            .hub()
            .ledger()
            .begin_call(super::cost::voice_call(ELEVEN_TTS_MODEL, line), &counted);
        let audio = self.send(&request, &current, billed, &counted).await?;
        if audio.is_empty() || audio.len() > MAXIMUM_AUDIO_BYTES {
            return Err(Failure::Provider);
        }
        let audio = Bytes::from(audio);
        let stored = self.store(key, audio.clone()).await.is_ok();
        Ok((audio, stored))
    }

    /// Sends one synthesis and settles its cost: a request refused before any billable work is
    /// not billed, any other failure is of unknown outcome, and an answer bills the characters.
    async fn send(
        &self,
        request: &ElevenSpeechRequest,
        current: &RequestBinding<ElevenSpeechBasis>,
        billed: super::cost::BilledCall,
        counted: &[(super::cost::Category, u64)],
    ) -> Result<Vec<u8>, Failure> {
        let prepared = match self
            .provider
            .speak(request, owner(current), std::future::pending())
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                super::cost::settle_speech_failure(billed, error.reason());
                return Err(Failure::Provider);
            }
        };
        billed.settle(super::cost::Usage::estimated(counted.to_vec()));
        let events = prepared
            .complete(owner(current))
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

    /// Writes the audio through the shared atomic write so a reader never sees a partial file.
    async fn store(&self, key: &str, audio: Bytes) -> io::Result<()> {
        let directory = self.cache_dir.clone();
        let name = key.to_owned();
        tokio::task::spawn_blocking(move || write_cache_file(&directory, &name, &audio))
            .await
            .map_err(|_| io::Error::other("voice cache write task failed"))?
    }
}

/// The provider request for one clip, checked against the contract, and the binding it was
/// checked against.
fn speech_request(
    voice: &str,
    line: &VoiceLine,
) -> Result<(ElevenSpeechRequest, RequestBinding<ElevenSpeechBasis>), Failure> {
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
    Ok((request, current))
}

/// Whether a stored clip is a bounded, non-empty regular file.
fn is_stored(path: &Path) -> bool {
    stored_length(path).is_some()
}

fn stored_length(path: &Path) -> Option<usize> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let length = usize::try_from(metadata.len()).ok()?;
    (metadata.is_file() && length != 0 && length <= MAXIMUM_AUDIO_BYTES).then_some(length)
}

fn read_cache_file(path: &Path) -> Option<Bytes> {
    stored_length(path)?;
    std::fs::read(path).ok().map(Bytes::from)
}

fn write_cache_file(directory: &Path, name: &str, audio: &[u8]) -> io::Result<()> {
    if audio.is_empty() {
        return Err(io::Error::other("empty voice audio is never cached"));
    }
    write_atomic(directory, name, audio)
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
            output_format_revision: label(SpeechOutputFormat::Mp3At22050Hz32Kbps.query_value())?,
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

pub(super) fn attach_urls(message: &mut rpc::ViewMessage) {
    let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
        return;
    };
    let voiced_lines = crate::voice_line::display_lines(display);
    let check_voice = crate::voice_line::check_voice_line(display);
    let Some(journey) = display.journey.as_mut() else {
        return;
    };
    for line in &mut journey.table_log {
        if let Some(check) = check_voice
            .as_ref()
            .filter(|check| check.result_sequence == line.sequence)
        {
            attach_voice_url(line, &check.voice);
            continue;
        }
        let role = match rpc::TableRole::try_from(line.role) {
            Ok(rpc::TableRole::Narrator) => "narrator".to_owned(),
            Ok(rpc::TableRole::Npc) if !line.actor_id.is_empty() => {
                format!("npc-id:{}|{}", line.actor_id, line.speaker)
            }
            Ok(rpc::TableRole::Npc) if !line.speaker.is_empty() => {
                format!("npc:{}", line.speaker)
            }
            _ => continue,
        };
        let Some(voice_line) = voiced_lines.iter().find(|voice_line| {
            voice_line.role == role && voice_line.text.trim() == line.text.trim()
        }) else {
            continue;
        };
        attach_voice_url(line, voice_line);
    }
    if let Some(combat) = journey.combat.as_mut() {
        for outcome in combat
            .outcomes
            .iter_mut()
            .chain(&mut combat.move_outcomes)
            .chain(&mut combat.death_save_outcomes)
        {
            for line in &mut outcome.lines {
                if let Some(voice_line) = crate::voice_line::fight_voice_line(line) {
                    attach_fight_url(line, &voice_line);
                }
            }
        }
        for line in &mut combat.closing_lines {
            if let Some(voice_line) = crate::voice_line::fight_voice_line(line) {
                attach_fight_url(line, &voice_line);
            }
        }
    }
}

fn attach_voice_url(line: &mut rpc::TableLine, voice_line: &VoiceLine) {
    let key = voice_line.key();
    line.audio_url = format!("/gameplay/voice/{key}");
    let segment_count = voice_segments(&voice_line.text).len();
    if segment_count > 1 {
        line.audio_part_urls = (1..segment_count)
            .map(|index| format!("/gameplay/voice/{key}/{index}"))
            .collect();
    }
}

fn attach_fight_url(line: &mut rpc::FightLine, voice_line: &VoiceLine) {
    line.audio_url = format!("/gameplay/voice/{}", voice_line.key());
    let segment_count = voice_segments(&voice_line.text).len();
    if segment_count > 1 {
        line.audio_part_urls = (1..segment_count)
            .map(|index| format!("/gameplay/voice/{}/{index}", voice_line.key()))
            .collect();
    }
}

async fn serve_line(
    State(voice): State<Voice>,
    RoutePath(key): RoutePath<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let Some(grant) = super::media_grant::verify_request(
        &headers,
        &uri,
        &key,
        super::media_grant::Audience::Voice,
        &[
            super::media_grant::PrincipalAudience::Seat,
            super::media_grant::PrincipalAudience::Display,
        ],
    ) else {
        return super::media_grant::concealed_not_found();
    };
    let response = match voice.audio_part(&key, 0, presentation_id(&headers)).await {
        Lookup::Audio(bytes) => (
            [
                (header::CONTENT_TYPE, audio_content_type()),
                (header::CACHE_CONTROL, CACHE_CONTROL),
            ],
            bytes,
        )
            .into_response(),
        Lookup::Part { bytes, count } => audio_parts_response(bytes, count),
        Lookup::Unregistered => StatusCode::NOT_FOUND.into_response(),
        Lookup::Unavailable => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    super::media_grant::secure_media_response(response, grant.remaining)
}

async fn serve_part(
    State(voice): State<Voice>,
    RoutePath((key, part)): RoutePath<(String, usize)>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let Some(grant) = super::media_grant::verify_request(
        &headers,
        &uri,
        &key,
        super::media_grant::Audience::Voice,
        &[
            super::media_grant::PrincipalAudience::Seat,
            super::media_grant::PrincipalAudience::Display,
        ],
    ) else {
        return super::media_grant::concealed_not_found();
    };
    let response = match voice
        .audio_part(&key, part, presentation_id(&headers))
        .await
    {
        Lookup::Audio(bytes) => audio_parts_response(bytes, 1),
        Lookup::Part { bytes, count } => audio_parts_response(bytes, count),
        Lookup::Unregistered => StatusCode::NOT_FOUND.into_response(),
        Lookup::Unavailable => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    super::media_grant::secure_media_response(response, grant.remaining)
}

fn presentation_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("x-gameplay-presentation-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value <= i64::MAX as u64)
}

fn safe_voice_resource_id(key: &str) -> String {
    if key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        key.to_ascii_lowercase()
    } else {
        df_observe::presentation::local_identity(key).to_string()
    }
}

#[allow(clippy::too_many_arguments)]
fn record_voice_resource(
    event: &'static str,
    resource_id: &str,
    presentation_id: Option<u64>,
    part: usize,
    part_count: usize,
    stage: &'static str,
    status: &'static str,
    failure_category: Option<&'static str>,
    cache_result: &'static str,
    char_count: Option<usize>,
    byte_length: Option<usize>,
    elapsed_ms: u128,
    http_status: u16,
) {
    let presentation = presentation_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "none".to_owned());
    let line = format!(
        "VOICE {stage} status={status} provider=elevenlabs resource_id={resource_id} presentation_id={presentation} part={part} part_count={part_count} chars={} bytes={} elapsed_ms={elapsed_ms} http_status={http_status} cache_result={cache_result}",
        char_count.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        byte_length.map_or_else(|| "none".to_owned(), |value| value.to_string()),
    );
    let fields = serde_json::Map::from_iter([
        ("resource_type".to_owned(), serde_json::json!("voice")),
        ("resource_id".to_owned(), serde_json::json!(resource_id)),
        (
            "presentation_id".to_owned(),
            serde_json::json!(presentation_id),
        ),
        ("provider".to_owned(), serde_json::json!("elevenlabs")),
        ("stage".to_owned(), serde_json::json!(stage)),
        ("status".to_owned(), serde_json::json!(status)),
        (
            "failure_category".to_owned(),
            serde_json::json!(failure_category),
        ),
        ("part".to_owned(), serde_json::json!(part)),
        ("part_count".to_owned(), serde_json::json!(part_count)),
        ("char_count".to_owned(), serde_json::json!(char_count)),
        ("byte_length".to_owned(), serde_json::json!(byte_length)),
        ("elapsed_ms".to_owned(), serde_json::json!(elapsed_ms)),
        ("http_status".to_owned(), serde_json::json!(http_status)),
        ("cache_result".to_owned(), serde_json::json!(cache_result)),
    ]);
    let attributes = [
        ("gameplay.resource.type", "voice".to_owned()),
        ("gameplay.resource.id", resource_id.to_owned()),
        ("gameplay.voice.presentation_id", presentation),
        ("gameplay.voice.provider", "elevenlabs".to_owned()),
        ("gameplay.resource.stage", stage.to_owned()),
        ("gameplay.resource.status", status.to_owned()),
        (
            "gameplay.resource.failure_category",
            failure_category.unwrap_or("none").to_owned(),
        ),
        ("gameplay.voice.part", part.to_string()),
        ("gameplay.voice.part_count", part_count.to_string()),
        (
            "gameplay.voice.char_count",
            char_count.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
        (
            "gameplay.resource.byte_length",
            byte_length.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
        ("gameplay.resource.elapsed_ms", elapsed_ms.to_string()),
        ("gameplay.media.http_status", http_status.to_string()),
        ("gameplay.media.cache_result", cache_result.to_owned()),
    ];
    super::actor::record_audit_diagnostic(event, line, fields, &attributes);
}

fn audio_content_type() -> &'static str {
    if crate::fake_profile::is_enabled() {
        "audio/wav"
    } else {
        "audio/mpeg"
    }
}

fn audio_parts_response(bytes: Bytes, count: usize) -> Response {
    let mut response = (
        [
            (header::CONTENT_TYPE, audio_content_type()),
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
    use crate::gameplay::resources::{Invalidate, ResourceKey};

    const AUDIO: &[u8] = b"ID3 fake mpeg audio";

    fn granted_uri(key: &str) -> Uri {
        let expires_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60;
        let token = super::super::media_grant::sign(
            key,
            super::super::media_grant::Audience::Voice,
            super::super::media_grant::PrincipalAudience::Display,
            &[1; 32],
            expires_at,
        )
        .unwrap();
        format!("/?g={token}").parse().unwrap()
    }

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

    fn configuration(cache: &Path) -> VoiceConfiguration {
        VoiceConfiguration {
            credential: VoiceCredential::for_test("test-credential"),
            cache_dir: cache.to_path_buf(),
            narrator_voice: "narratorvoice".to_owned(),
            npc_voice_override: None,
            casting_path: PathBuf::from("voice-casting.json"),
        }
    }

    fn voice_with(fake: &FakeEleven, cache: &Path) -> Voice {
        voice_cast(fake, configuration(cache), &[], "0x1")
    }

    fn fixture_provider(fake: &FakeEleven) -> NativeSpeechProvider {
        let limits = ProviderLimits::new(Duration::from_secs(5), 16_384, 65_536)
            .connect_timeout(Duration::from_secs(1))
            .header_bytes(4096)
            .build();
        registered_speech_provider(
            CandidateRouteId::ElevenFlashV25Tts,
            "test-credential",
            limits,
        )
        .unwrap()
        .for_loopback_fixture(fake.address)
        .unwrap()
    }

    fn voice_cast(
        fake: &FakeEleven,
        configuration: VoiceConfiguration,
        speakers: &[SpeakerCasting],
        run_identifier: &str,
    ) -> Voice {
        let provider = fixture_provider(fake);
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

    /// The characters charged on the hub's ledger so far.
    fn charged(voice: &Voice) -> u64 {
        match &voice.inner.mode {
            Mode::On { synthesis, .. } => {
                synthesis
                    .kit
                    .get()
                    .hub()
                    .ledger()
                    .snapshot(Unit::VoiceCharacter)
                    .committed
            }
            Mode::Off(_) => 0,
        }
    }

    fn file_name(voice_id: &str, line: &VoiceLine) -> String {
        VoiceLineSpec::new(voice_id, line).file_name()
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
    async fn hs54_registered_display_voice_prewarms_before_audio_request_and_is_not_paid_twice() {
        let cache = scratch("hs54-prefetch");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_cast(&fake, configuration(&cache), &[], "hs54");
        let message = display_message("A quiet cart crosses the square.");
        voice.register_view(&message);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if fake.calls() > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        voice.register_view(&message);
        assert_eq!(
            voice
                .audio(&narrator("A quiet cart crosses the square.").key())
                .await,
            Lookup::Audio(Bytes::from_static(AUDIO))
        );
        assert_eq!(fake.calls(), 1);
        assert_eq!(
            charged(&voice),
            "A quiet cart crosses the square.".chars().count() as u64
        );
        assert!(locked(&voice.inner.prefetch).len() <= 8);
    }

    #[tokio::test]
    async fn hs54_dispatched_voice_prewarm_finishes_in_the_shared_cache_after_waiter_drop() {
        let cache = scratch("hs54-prefetch-drop");
        let fake = FakeEleven::start(Duration::from_millis(100), true);
        let line = narrator("The distant bell rings once.");
        let voice = voice_cast(&fake, configuration(&cache), &[], "hs54");
        let Mode::On { synthesis, .. } = &voice.inner.mode else {
            panic!("voice on");
        };
        let file = cache.join(file_name(
            &synthesis.casting.voice_for(&line).voice_id,
            &line,
        ));
        let message = display_message(&line.text);
        voice.register_view(&message);
        tokio::time::timeout(Duration::from_secs(3), async {
            while fake.calls() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(voice);
        tokio::time::timeout(Duration::from_secs(3), async {
            while !file.exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let resumed = voice_cast(&fake, configuration(&cache), &[], "hs54");
        register(&resumed, &line);
        assert_eq!(
            resumed.audio(&line.key()).await,
            Lookup::Audio(Bytes::from_static(AUDIO))
        );
        assert_eq!(
            fake.calls(),
            1,
            "the dispatched paid flight finishes once and its cache survives the waiter"
        );
    }

    #[tokio::test]
    async fn unregistered_key_is_not_found() {
        let cache = scratch("unregistered");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache);

        let key = "0".repeat(64);
        let response = serve_line(
            State(voice),
            RoutePath(key.clone()),
            HeaderMap::new(),
            granted_uri(&key),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(fake.calls(), 0);
    }

    #[tokio::test]
    async fn registered_line_is_synthesized_once_then_served_from_cache() {
        let cache = scratch("once");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache);
        let key = register(&voice, &narrator("The lanterns sway."));

        let first = serve_line(
            State(voice.clone()),
            RoutePath(key.clone()),
            HeaderMap::new(),
            granted_uri(&key),
        )
        .await;
        let second = serve_line(
            State(voice.clone()),
            RoutePath(key.clone()),
            HeaderMap::new(),
            granted_uri(&key),
        )
        .await;

        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[header::CONTENT_TYPE], "audio/mpeg");
        assert_eq!(first.headers()[header::REFERRER_POLICY], "no-referrer");
        let cache_control = first.headers()[header::CACHE_CONTROL].to_str().unwrap();
        let maximum_age = cache_control
            .strip_prefix("private, max-age=")
            .unwrap()
            .parse::<u64>()
            .unwrap();
        assert!((1..=60).contains(&maximum_age));
        let body = axum::body::to_bytes(first.into_body(), 1 << 20)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), AUDIO);
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(fake.calls(), 1);
        let name = file_name("narratorvoice", &narrator("The lanterns sway."));
        assert_eq!(std::fs::read(cache.join(name)).unwrap(), AUDIO);
        let first_target = fake.targets.lock().unwrap()[0].clone();
        assert!(first_target.contains("/v1/text-to-speech/narratorvoice/stream"));
    }

    #[tokio::test]
    async fn long_line_parts_are_individually_addressable_and_cached() {
        let cache = scratch("long-line-parts");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache);
        let line = narrator(&format!("{}. {}.", "a".repeat(350), "b".repeat(300)));
        let key = register(&voice, &line);

        let first = serve_line(
            State(voice.clone()),
            RoutePath(key.clone()),
            HeaderMap::new(),
            granted_uri(&key),
        )
        .await;
        let second = serve_part(
            State(voice.clone()),
            RoutePath((key.clone(), 1)),
            HeaderMap::new(),
            granted_uri(&key),
        )
        .await;
        let replay = serve_part(
            State(voice),
            RoutePath((key.clone(), 1)),
            HeaderMap::new(),
            granted_uri(&key),
        )
        .await;

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
        let first_run = voice_with(&fake, &cache);
        let key = register(&first_run, &line);
        assert_eq!(
            first_run.audio(&key).await,
            Lookup::Audio(Bytes::from_static(AUDIO))
        );

        let second_run = voice_with(&fake, &cache);
        register(&second_run, &line);
        let replay = second_run.audio(&key).await;

        assert_eq!(replay, Lookup::Audio(Bytes::from_static(AUDIO)));
        assert_eq!(fake.calls(), 1);
        assert_eq!(charged(&second_run), 0, "a stored line costs nothing");
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_provider_call() {
        let cache = scratch("flight");
        let fake = FakeEleven::start(Duration::from_millis(250), true);
        let voice = voice_with(&fake, &cache);
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
        assert_eq!(charged(&voice), text.chars().count() as u64);
    }

    #[tokio::test]
    async fn characters_are_charged_when_the_call_is_sent_and_kept_when_the_provider_fails() {
        let cache = scratch("charge");
        let fake = FakeEleven::start(Duration::ZERO, false);
        let voice = voice_with(&fake, &cache);
        let text = "Thirty characters of narration";
        assert_eq!(text.chars().count(), 30);
        let key = register(&voice, &narrator(text));

        let failed = voice.audio(&key).await;

        assert_eq!(failed, Lookup::Unavailable);
        assert_eq!(fake.calls(), 1);
        assert_eq!(charged(&voice), 30);
    }

    #[tokio::test]
    async fn no_line_is_refused_for_cost_however_much_has_been_spoken() {
        let cache = scratch("no-cap");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache);
        let first = register(&voice, &narrator("Exactly twenty chars"));
        let second = register(&voice, &narrator("Another line"));

        let spoken = voice.audio(&first).await;
        let also_spoken = voice.audio(&second).await;

        assert_eq!(spoken, Lookup::Audio(Bytes::from_static(AUDIO)));
        assert_eq!(also_spoken, Lookup::Audio(Bytes::from_static(AUDIO)));
        assert_eq!(fake.calls(), 2);
        assert_eq!(charged(&voice), 20 + 12);
        let replay = voice.audio(&first).await;
        assert_eq!(replay, Lookup::Audio(Bytes::from_static(AUDIO)));
        assert_eq!(fake.calls(), 2, "a stored line is replayed for nothing");
        assert_eq!(charged(&voice), 32);
    }

    #[tokio::test]
    async fn provider_failure_writes_no_cache_file() {
        let cache = scratch("failure");
        let fake = FakeEleven::start(Duration::ZERO, false);
        let voice = voice_with(&fake, &cache);
        let key = register(&voice, &narrator("The seal cracks."));

        let response = serve_line(
            State(voice),
            RoutePath(key.clone()),
            HeaderMap::new(),
            granted_uri(&key),
        )
        .await;

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

    #[test]
    fn canonical_actor_id_selects_voice_before_legacy_public_name() {
        let voice = |id: &str| ChosenVoice {
            voice_id: id.into(),
            name: id.into(),
            fallback: false,
        };
        let casting = Casting {
            narrator: voice("narrator"),
            forced: None,
            by_speaker: HashMap::from([("courier".into(), voice("courier-voice"))]),
            by_actor: HashMap::from([("bandit".into(), voice("bandit-voice"))]),
            fallback: voice("neutral"),
            roster: test_roster(),
            run_identifier: "test".into(),
            generated: Mutex::new(HashMap::new()),
        };
        let line = VoiceLine {
            role: "npc-id:bandit|Courier".into(),
            text: "Who sent you?".into(),
        };
        assert_eq!(casting.voice_for(&line).voice_id, "bandit-voice");
        let stale = VoiceLine {
            role: "npc-id:departed|Courier".into(),
            text: line.text,
        };
        assert_eq!(casting.voice_for(&stale).voice_id, "neutral");
    }

    #[tokio::test]
    async fn scripted_courier_view_registers_canonical_voice_and_reuses_it_after_case_change() {
        let cache = scratch("scripted-courier");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let mut courier = speaker("Courier", Gender::She, Some(Age::Adult), "warm");
        courier.actor_ids = vec!["courier".into()];
        let voice = voice_cast(&fake, configuration(&cache), &[courier], "0x7");
        let mut display = rpc::DisplayGameplayView {
            journey: Some(rpc::JourneyView {
                speaker: "courier".into(),
                dialogue: "The crest belongs to a ship that never appears on the ledger.".into(),
                ..Default::default()
            }),
            scene: Some(rpc::GameplayScene {
                cast: vec![rpc::SceneActor {
                    id: "courier".into(),
                    name: "Courier".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let line = crate::voice_line::display_lines(&display).pop().unwrap();
        for speaker in ["courier", "COURIER", "Courier"] {
            display.journey.as_mut().unwrap().speaker = speaker.into();
            voice.register_view(&rpc::ViewMessage {
                audience: Some(rpc::view_message::Audience::Display(display.clone())),
                ..Default::default()
            });
            assert_eq!(
                crate::voice_line::display_lines(&display)
                    .pop()
                    .unwrap()
                    .key(),
                line.key()
            );
            assert_eq!(
                voice.audio(&line.key()).await,
                Lookup::Audio(Bytes::from_static(AUDIO))
            );
        }
        assert_eq!(fake.calls(), 1);
        let Mode::On { synthesis, .. } = &voice.inner.mode else {
            panic!("voice should be on");
        };
        let chosen = synthesis.casting.voice_for(&line);
        assert!(!chosen.fallback);
        assert_eq!(roster_voice_gender(&chosen.voice_id), Some(Gender::She));
        assert!(
            fake.targets.lock().unwrap()[0]
                .contains(&format!("/v1/text-to-speech/{}/stream", chosen.voice_id))
        );
    }

    fn speaker(name: &str, gender: Gender, age: Option<Age>, tone: &str) -> SpeakerCasting {
        SpeakerCasting {
            actor_ids: Vec::new(),
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
        let voice = voice_cast(&fake, configuration(&cache), &speakers, "0x7");

        let wren = voice_id_requested(&voice, &fake, &npc("Wren", "Welcome in.")).await;
        let garrick = voice_id_requested(&voice, &fake, &npc("Garrick", "Move along.")).await;

        assert_eq!(roster_voice_gender(&wren), Some(Gender::She));
        assert_eq!(roster_voice_gender(&garrick), Some(Gender::He));
    }

    fn generated_place() -> super::super::place::PlaceRecord {
        use super::super::place_creation::{build, tests};
        let mut place = tests::place_slot();
        // This fixture tests person admission independently of market generation and its byte budget.
        for field in ["wares", "services", "stalls", "mentioned_stalls"] {
            place[field] = serde_json::json!([]);
        }
        place["market"] = serde_json::json!(false);
        build(
            &tests::request(),
            &place.to_string(),
            &tests::people_slot().to_string(),
            &tests::things_slot().to_string(),
        )
        .unwrap()
    }

    fn generated_line(place: &super::super::place::PlaceRecord, index: usize) -> VoiceLine {
        let actor = place.expand().unwrap().cast.remove(index);
        VoiceLine {
            role: format!("npc-id:{}|{}", actor.id, actor.name),
            text: "Good day.".into(),
        }
    }

    #[tokio::test]
    async fn generated_man_and_woman_use_matching_distinct_voices_without_fallback() {
        let cache = scratch("generated-presentation");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache);
        let place = generated_place();
        let Mode::On { synthesis, .. } = &voice.inner.mode else {
            panic!("voice on");
        };
        synthesis.casting.register_place(&place).unwrap();
        let man = voice_id_requested(&voice, &fake, &generated_line(&place, 0)).await;
        let woman = voice_id_requested(&voice, &fake, &generated_line(&place, 1)).await;
        assert_eq!(roster_voice_gender(&man), Some(Gender::He));
        assert_eq!(roster_voice_gender(&woman), Some(Gender::She));
        assert_ne!(man, woman);
        for index in 0..place.people.len() {
            let selected = synthesis.casting.voice_for(&generated_line(&place, index));
            assert!(!selected.fallback);
            assert_eq!(
                synthesis
                    .casting
                    .roster
                    .voice(&selected.voice_id)
                    .unwrap()
                    .age,
                Age::Adult
            );
        }
    }

    #[test]
    fn four_generated_vendors_are_distinct_and_keep_their_voices_after_restart() {
        let cache = scratch("generated-restart");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let mut roster = test_roster();
        for voice in &mut roster.voices {
            voice.age = Age::Adult;
        }
        let start = || {
            Voice::new(
                configuration(&cache),
                fixture_provider(&fake),
                &roster,
                &[],
                "same-run",
            )
            .unwrap()
        };
        let first = start();
        let mut market = generated_place();
        market.people[0].gender = super::super::pack::Gender::She;
        for person in &mut market.people {
            person.role = "vendor".into();
        }
        let mut fourth = market.people[1].clone();
        fourth.name = "Fourth Vendor".into();
        market.people.push(fourth);
        let Mode::On { synthesis, .. } = &first.inner.mode else {
            panic!("voice on");
        };
        synthesis.casting.register_place(&market).unwrap();
        let chosen = |casting: &Casting| {
            (0..market.people.len())
                .map(|index| casting.voice_for(&generated_line(&market, index)).voice_id)
                .collect::<Vec<_>>()
        };
        let before = chosen(&synthesis.casting);
        assert_eq!(
            before
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
        let mut later = market.clone();
        later.place_id = "place-another".into();
        for person in &mut later.people {
            person.name = format!("Earlier {}", person.name);
        }
        synthesis.casting.register_place(&later).unwrap();
        assert_eq!(chosen(&synthesis.casting), before);
        let stored: super::super::place::PlaceRecord =
            serde_json::from_str(&serde_json::to_string(&market).unwrap()).unwrap();
        let restarted = start();
        let Mode::On {
            synthesis: restored,
            ..
        } = &restarted.inner.mode
        else {
            panic!("voice on");
        };
        restored.casting.register_place(&stored).unwrap();
        restored.casting.register_place(&later).unwrap();
        assert_eq!(chosen(&restored.casting), before);
        let mut spot = market.clone();
        spot.place_id = "place-stall".into();
        spot.origin.id = market.place_id.clone();
        spot.local_spot = Some("the vendor stall".into());
        spot.local_keeper = Some(market.people[0].name.clone());
        spot.people.truncate(1);
        restored.casting.register_place(&spot).unwrap();
        assert_eq!(
            restored
                .casting
                .voice_for(&generated_line(&spot, 0))
                .voice_id,
            before[0]
        );
        assert!(
            locked(&restored.casting.generated)
                .values()
                .all(|cast| !cast.voice.fallback)
        );
    }

    #[tokio::test]
    async fn longest_generated_actor_identity_registers_and_speaks_with_its_cast_voice() {
        let cache = scratch("generated-long-identity");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache);
        let mut place = generated_place();
        place.place_id = format!("place-{}", "p".repeat(32));
        place.people[0].name = "Long Generated Merchant Identity".into();
        let Mode::On { synthesis, .. } = &voice.inner.mode else {
            panic!("voice on");
        };
        synthesis.casting.register_place(&place).unwrap();
        let actor = place.expand().unwrap().cast.remove(0);
        let display = rpc::DisplayGameplayView {
            journey: Some(rpc::JourneyView {
                speaker: actor.name.clone(),
                dialogue: "Welcome to my stall.".into(),
                ..Default::default()
            }),
            scene: Some(rpc::GameplayScene {
                cast: vec![rpc::SceneActor {
                    id: actor.id,
                    name: actor.name,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let line = crate::voice_line::display_lines(&display).pop().unwrap();
        assert!(line.role.len() > 96);
        voice.register_view(&rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(display)),
            ..Default::default()
        });
        assert_eq!(
            voice.audio(&line.key()).await,
            Lookup::Audio(Bytes::from_static(AUDIO))
        );
        assert!(!synthesis.casting.voice_for(&line).fallback);
    }

    #[test]
    fn committed_people_admission_replays_journal_and_does_not_recast() {
        use super::super::{
            adventure::{RulesMode, record, shop},
            beat_script_variety_tests as native, pack, place,
            startup_configuration::ContentRating,
        };
        let adventure = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
        let (loaded, _) = record::reload(&adventure.documents(true).unwrap()).unwrap();
        pack::with_test_pack(loaded, || {
            let cache = scratch("generated-journal");
            let fake = FakeEleven::start(Duration::ZERO, true);
            let voice = voice_with(&fake, &cache);
            let first = generated_place();
            let mut second = first.clone();
            second.place_id = "place-next-market".into();
            second.name = "Next Market".into();
            for person in &mut second.people {
                person.name = format!("New {}", person.name);
            }
            second.speaker = format!("New {}", second.speaker);
            second.origin.id = first.place_id.clone();
            second.origin.name = first.name.clone();
            first.validate().unwrap();
            second.validate().unwrap();
            let mut checkpoint =
                native::act(&native::prepared(), native::first(), 5, "begin-story");
            for (number, destination) in [(81, &first), (82, &second)] {
                let scene = super::super::beat_script_runtime::scene(checkpoint.state()).unwrap();
                let from = place::home_id(&scene).unwrap();
                checkpoint = match place::plan(
                    &checkpoint,
                    "Brynn",
                    OperationId::from_bytes(&[number; 16]).unwrap(),
                    &from,
                    place::JOURNEY_EXIT,
                    &destination.place_id,
                    Some(destination),
                ) {
                    Ok(planned) => planned.candidate,
                    Err(place::PlaceFailure::Refused(reason)) => panic!(
                        "fixture place admission refused: {reason:?}; step={:?}",
                        super::super::refusal_detail::take()
                    ),
                    Err(place::PlaceFailure::State(error)) => panic!(
                        "fixture place state failed: {error:?}; step={:?}",
                        super::super::refusal_detail::take()
                    ),
                };
                voice.register_people(checkpoint.state()).unwrap();
            }
            let Mode::On { synthesis, .. } = &voice.inner.mode else {
                panic!("voice on");
            };
            let selected = synthesis.casting.voice_for(&generated_line(&first, 0));
            let count = locked(&synthesis.casting.generated).len();
            voice.register_people(checkpoint.state()).unwrap();
            assert_eq!(locked(&synthesis.casting.generated).len(), count);
            let restored = voice_with(&fake, &cache);
            restored.register_people(checkpoint.state()).unwrap();
            let Mode::On {
                synthesis: replay, ..
            } = &restored.inner.mode
            else {
                panic!("voice on");
            };
            assert_eq!(
                replay.casting.voice_for(&generated_line(&first, 0)),
                selected
            );
            assert_eq!(locked(&replay.casting.generated).len(), count);
            assert_eq!(count, first.people.len() + second.people.len());
        });
    }

    #[test]
    fn generated_age_match_wins_over_unused_wrong_age_and_missing_match_is_explicit_fallback() {
        let cache = scratch("generated-age");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let mut roster = test_roster();
        roster.voices.retain(|voice| voice.gender != Gender::He);
        let voice = Voice::new(
            configuration(&cache),
            fixture_provider(&fake),
            &roster,
            &[],
            "age-run",
        )
        .unwrap();
        let mut place = generated_place();
        place.people[1].age = Some(super::super::pack::Age::Young);
        place.people[2].age = Some(super::super::pack::Age::Young);
        let Mode::On { synthesis, .. } = &voice.inner.mode else {
            panic!("voice on");
        };
        synthesis.casting.register_place(&place).unwrap();
        for index in [1, 2] {
            let selected = synthesis.casting.voice_for(&generated_line(&place, index));
            assert!(!selected.fallback);
            assert_eq!(roster.voice(&selected.voice_id).unwrap().age, Age::Young);
        }
        let generated = locked(&synthesis.casting.generated);
        let missing = generated
            .values()
            .find(|cast| cast.speaker.gender == Gender::He)
            .unwrap();
        assert!(missing.voice.fallback);
        assert!(missing.reason.starts_with("fallback:"));
    }

    #[tokio::test]
    async fn they_and_it_speakers_get_their_own_gender_groups() {
        let cache = scratch("they-it");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let speakers = [
            speaker("Marl", Gender::They, None, "calm"),
            speaker("Skrit", Gender::It, None, "raspy"),
        ];
        let voice = voice_cast(&fake, configuration(&cache), &speakers, "0x7");

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
        let voice = voice_cast(&fake, configuration(&cache), &speakers, "0x7");

        let wren = voice_id_requested(&voice, &fake, &npc("Wren", "Welcome in.")).await;
        let brisa = voice_id_requested(&voice, &fake, &npc("Brisa", "Welcome in.")).await;

        assert_ne!(wren, brisa);
    }

    #[tokio::test]
    async fn an_unknown_speaker_gets_the_neutral_fallback_never_a_gendered_voice() {
        let cache = scratch("fallback");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let speakers = [speaker("Wren", Gender::She, None, "")];
        let voice = voice_cast(&fake, configuration(&cache), &speakers, "0x7");

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
        let voice = voice_cast(&fake, configuration(&cache), &speakers, "0x7");

        let loud = voice_id_requested(&voice, &fake, &npc(" WREN ", "Hello.")).await;
        let narration = voice_id_requested(&voice, &fake, &narrator("Rain falls.")).await;

        assert_eq!(roster_voice_gender(&loud), Some(Gender::She));
        assert_eq!(narration, "narratorvoice");
    }

    #[tokio::test]
    async fn the_forced_npc_voice_speaks_every_named_speaker() {
        let cache = scratch("forced");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let mut forced = configuration(&cache);
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
        let voice = voice_cast(&fake, configuration(&cache), &speakers, "0x7");

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
            configuration(&cache),
            &[speaker("Wren", Gender::She, Some(Age::Young), "")],
            "0x7",
        );
        let first_voice = voice_id_requested(&first, &fake, &line).await;
        let public_key = line.key();

        let recast = voice_cast(
            &fake,
            configuration(&cache),
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
        assert!(cache.join(file_name(&first_voice, &line)).is_file());
        assert!(cache.join(file_name(&second_voice, &line)).is_file());
        assert!(!cache.join(&public_key).exists());
    }

    #[test]
    fn the_voice_key_changes_with_the_voice_role_text_model_format_language_or_clip_split() {
        let line = npc("Wren", "Welcome in.");
        let base = VoiceLineSpec::new("abc", &line);
        let key = |spec: &VoiceLineSpec| (spec.file_name(), ResourceKey::of(spec));
        let (base_name, base_key) = key(&base);
        assert_eq!(base_name.len(), 64);
        assert!(base_name.starts_with(base_key.as_str()));
        assert_ne!(
            base_name,
            line.key(),
            "the cache name is not the public key"
        );
        assert_eq!(key(&VoiceLineSpec::new("abc", &line)).0, base_name);

        let changed = [
            ("voice", VoiceLineSpec::new("abd", &line)),
            (
                "role",
                VoiceLineSpec::new("abc", &npc("Garrick", "Welcome in.")),
            ),
            (
                "text",
                VoiceLineSpec::new("abc", &npc("Wren", "Welcome back.")),
            ),
            (
                "model",
                VoiceLineSpec {
                    model: "eleven_multilingual_v2".to_owned(),
                    ..base.clone()
                },
            ),
            (
                "format",
                VoiceLineSpec {
                    format: "mp3_44100_128".to_owned(),
                    ..base.clone()
                },
            ),
            (
                "language",
                VoiceLineSpec {
                    language: "fr".to_owned(),
                    ..base.clone()
                },
            ),
            (
                "clip split",
                VoiceLineSpec {
                    clip_limit: base.clip_limit + 100,
                    ..base.clone()
                },
            ),
        ];
        for (what, spec) in changed {
            let (name, hub_key) = key(&spec);
            assert_ne!(name, base_name, "a changed {what} must be another file");
            assert_ne!(
                hub_key, base_key,
                "a changed {what} must be another hub entry"
            );
        }
        // The spec reads the provider's own model, language and format, so a change there changes
        // the key without anyone remembering to bump a version.
        assert_eq!(base.model, ELEVEN_TTS_MODEL);
        assert_eq!(base.language, ELEVEN_TTS_LANGUAGE);
        assert_eq!(base.format, "mp3_22050_32");
    }

    #[tokio::test]
    async fn a_failed_line_is_not_retried_beyond_its_attempts_and_is_listed_by_the_hub() {
        let cache = scratch("attempts");
        let fake = FakeEleven::start(Duration::ZERO, false);
        let voice = voice_with(&fake, &cache);
        let key = register(&voice, &narrator("The seal cracks twice."));

        let mut lookups = Vec::new();
        for _ in 0..4 {
            lookups.push(voice.audio(&key).await);
        }

        assert!(lookups.iter().all(|lookup| *lookup == Lookup::Unavailable));
        assert_eq!(fake.calls(), 2, "two attempts, then the failure stands");
        let Mode::On { synthesis, .. } = &voice.inner.mode else {
            panic!("voice should be on");
        };
        let rows = synthesis
            .kit
            .get()
            .hub()
            .list(Some(ResourceKind::VoiceLine));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, "failed");
        assert_eq!(rows[0].last_error, "provider_failed");
        assert_eq!(rows[0].subject, "voice:narrator");
        assert_eq!(
            charged(&voice),
            2 * "The seal cracks twice.".chars().count() as u64,
            "each paid attempt is charged and none is refunded"
        );
    }

    #[tokio::test]
    async fn a_clip_is_a_voice_line_resource_with_resource_audit_lines_and_a_ledger_charge() {
        let cache = scratch("resource-lines");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache);
        let line = narrator("A bell rings once over the quay.");
        let key = register(&voice, &line);

        assert_eq!(
            voice.audio(&key).await,
            Lookup::Audio(Bytes::from_static(AUDIO))
        );

        let hub_key = ResourceKey::of(&VoiceLineSpec::new("narratorvoice", &line));
        let lines = crate::gameplay::actor::audit_lines_containing(hub_key.as_str());
        assert!(
            lines
                .iter()
                .any(|text| text.starts_with("RESOURCE request kind=voice_line")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|text| text.starts_with("RESOURCE ready kind=voice_line")
                    && text.contains(&VoiceLineSpec::new("narratorvoice", &line).file_name())),
            "{lines:?}"
        );
        assert_eq!(charged(&voice), line.text.chars().count() as u64);
    }

    #[tokio::test]
    async fn invalidating_a_clip_erases_its_file_so_the_next_ask_speaks_it_again() {
        let cache = scratch("invalidate");
        let fake = FakeEleven::start(Duration::ZERO, true);
        let voice = voice_with(&fake, &cache);
        let line = narrator("The tide turns.");
        let key = register(&voice, &line);
        voice.audio(&key).await;
        let Mode::On { synthesis, .. } = &voice.inner.mode else {
            panic!("voice should be on");
        };
        let kit = synthesis.kit.get();
        let name = VoiceLineSpec::new("narratorvoice", &line).file_name();
        assert!(cache.join(&name).is_file());

        let dropped = kit.invalidate(Invalidate::Subject("voice:narrator"));
        let again = voice.audio(&key).await;

        assert_eq!(dropped, 1);
        assert_eq!(again, Lookup::Audio(Bytes::from_static(AUDIO)));
        assert_eq!(
            fake.calls(),
            2,
            "an invalidated clip is made again, not replayed"
        );
    }

    #[test]
    fn starting_with_an_unusable_roster_is_refused_with_a_typed_message() {
        let dir = scratch("roster-file");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("voice-casting.json");
        std::fs::write(&path, br#"{"schema":"df-voice-casting/1"}"#).unwrap();
        let mut malformed = configuration(&dir);
        malformed.casting_path = path;
        let mut missing = configuration(&dir);
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

        let known = serve_line(
            State(voice.clone()),
            RoutePath(key.clone()),
            HeaderMap::new(),
            granted_uri(&key),
        )
        .await;
        let unknown_key = "1".repeat(64);
        let unknown = serve_line(
            State(voice.clone()),
            RoutePath(unknown_key.clone()),
            HeaderMap::new(),
            granted_uri(&unknown_key),
        )
        .await;

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
    fn register_view_registers_every_public_npc_reply_without_a_cast_figure() {
        let voice = Voice::disabled(VoiceOff::Disabled);
        let row = |sequence: u64, text: &str| rpc::TableLine {
            audio_url: String::new(),
            audio_part_urls: Vec::new(),
            sequence,
            actor_id: "bandit".into(),
            speaker: "Dain Rusk".into(),
            text: text.into(),
            role: rpc::TableRole::Npc as i32,
            kind: rpc::TableKind::Speech as i32,
            ..Default::default()
        };
        let display = rpc::DisplayGameplayView {
            scene: Some(rpc::GameplayScene {
                title: "Salt Quay".into(),
                cast: vec![rpc::SceneActor {
                    id: "courier".into(),
                    name: "Courier".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            journey: Some(rpc::JourneyView {
                table_log: vec![
                    row(1, "The packet stays with me."),
                    row(2, "Ask the harbourmaster."),
                    row(3, "The packet stays with me."),
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let expected = crate::voice_line::display_lines(&display);
        assert_eq!(expected.len(), 2);
        voice.register_view(&rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(display)),
            ..Default::default()
        });
        let lines = locked(&voice.inner.lines);
        assert_eq!(lines.by_key.len(), expected.len());
        for line in &expected {
            assert_eq!(line.actor_id(), Some("bandit"));
            assert_eq!(line.speaker(), Some("Dain Rusk"));
            assert!(lines.by_key.contains_key(&line.key()));
        }
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
            ("DF_GAMEPLAY_VOICE_CACHE", "/tmp/somewhere"),
            ("DF_GAMEPLAY_VOICE_NARRATOR", "narr_1"),
            ("DF_GAMEPLAY_VOICE_NPC", "npc-2"),
            ("DF_GAMEPLAY_VOICE_CASTING", "/roster/voices.json"),
        ];
        let VoiceSettings::On(configuration) = voice_settings(inputs(&overrides)).unwrap() else {
            panic!("voice should be on");
        };
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

#[cfg(test)]
mod fake_profile_voice_tests {
    use super::*;

    #[tokio::test]
    async fn fake_profile_keeps_authored_narration_bound_to_the_registered_voice_slot() {
        let text = "The heroes wait for the courier.";
        let mut message = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    narration: text.to_owned(),
                    journey: Some(rpc::JourneyView {
                        table_log: vec![rpc::TableLine {
                            sequence: 7,
                            role: rpc::TableRole::Narrator as i32,
                            kind: rpc::TableKind::Speech as i32,
                            text: text.to_owned(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        crate::fake_profile::FakeProfile.project(&mut message);
        let voice = Voice::disabled(VoiceOff::Disabled);
        voice.register_view(&message);
        attach_urls(&mut message);
        let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
            panic!("display lost");
        };
        let caption = &display.journey.as_ref().unwrap().table_log[0];
        assert_eq!(display.narration, text);
        assert_eq!(caption.text, text);
        let key = caption
            .audio_url
            .strip_prefix("/gameplay/voice/")
            .expect("registered narrative URL");
        assert!(
            crate::voice_line::display_lines(display)
                .iter()
                .any(|slot| slot.key() == key && slot.text == caption.text)
        );
        // Off mode cannot synthesize, but the real lookup distinguishes a registered slot
        // from an unregistered key without accessing any provider.
        assert_eq!(voice.audio(key).await, Lookup::Unavailable);
    }
}
