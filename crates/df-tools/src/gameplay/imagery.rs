//! The runtime image service: server-generated art for the shared display.
//!
//! One typed request ([`ImageRequest`]) serves every caller: a re-framed scene backdrop, a
//! character's changed expression, and a close-up of an object. A request names its reference
//! images (the base backdrop and/or character art from the served asset set), a prompt built
//! only from the server's closed vocabulary (see `prompt`), and the output size. The result is
//! an image the display loads at `GET /gameplay/imagery/<key>`, where the key is the SHA-256 of
//! the kind, the references' hashes, the prompt, the model and the parameters.
//!
//! Like the voice lines, only registered keys are served: a key the server never registered is
//! a 404. Results live in an on-disk cache keyed the same way, so a request never costs twice,
//! not even across runs. There is no spend cap: every provider call is recorded in the hub's
//! spend ledger (a call that was sent is never refunded); at most two calls fly at once; identical requests share one flight. Every
//! failure is typed and means "keep showing what is on screen": nothing in the game waits for
//! an image, and a provider refusal is a failure, never retried with another wording or another
//! model. Generated images are never written under `assets/` and never committed; each is
//! recorded in a run-local manifest (`imagery.jsonl`) with its provenance.
mod artifact;
mod canonical_prompt;
mod cast_sheet;
pub(super) mod narrative_focus;
mod party_sheet;
mod photo_edit;
mod plate_prompt;
mod prompt;
mod references;
mod scene_prompt;
pub(super) use canonical_prompt::{
    canonical_cast_member, canonical_character, canonical_npc, canonical_room,
    canonical_room_content,
};
pub(super) use scene_prompt::{graph_edit, item_picture, model_plate, state_edit, story_beat_edit};

use std::{
    collections::{HashMap, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    extract::{Path as RoutePath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bytes::Bytes;
use df_observe::OperationContext;
use df_providers::{
    HeadLocateConfig, ImageBackground, ImageEdit, ImageEditConfig, ImageEditError, ImageEditModel,
    ImageFormat, ImageModeration, ImageQuality, ImageUsage, LocateTarget, LocatedHead,
    NativeHeadLocator, NativeHttpRefusal, NativeImageEditProvider,
    ReferenceImage as ProviderReference,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, watch};

#[allow(
    unused_imports,
    reason = "the investigation task (a follow-up) is the first caller"
)]
pub(super) use artifact::{ArtifactSubject, SubjectRefusal, close_up_request};
#[allow(
    unused_imports,
    reason = "T02 stage rendering must pass each checked hero identity into these prompts"
)]
pub(super) use prompt::{
    AssembledPrompt, CharacterFacts, CharacterRole, FILM_LOOK, HeroVisualIdentity, LifeStage,
    PlaceFacts, cast_expression, hero_cutout, hero_expression, hero_portrait, scene_refresh,
};
#[allow(
    unused_imports,
    reason = "the investigation task (a follow-up) is the first caller"
)]
pub(super) use prompt::{PromptRefusal, VettedFact};
// PLAY-V09: the scene plate's party sheet and prompts.
pub(super) use cast_sheet::{CastIdentity, cast_sheet};
use party_sheet::SheetError;
pub(super) use party_sheet::{party_sheet, vision_picture};
pub(super) use photo_edit::harbor_photo_edit;
pub(super) use plate_prompt::{PlateReferences, hero_locator_phrase, plate_moment, scene_plate};
#[allow(
    unused_imports,
    reason = "T02 stage rendering must pass each checked hero identity into these prompts"
)]
pub(super) use plate_prompt::{hero_locator_phrase_for_identity, scene_plate_for_heroes};
pub(super) use references::{ReferenceImage, ReferenceStore};

use super::startup_configuration::{
    ContentRating, ImageryConfiguration, ImageryOff, ImagerySettings,
};

const ROUTE: &str = "/gameplay/imagery";
const MANIFEST_NAME: &str = "imagery.jsonl";
/// Most recent registered keys kept; older ones can no longer be requested.
const MAXIMUM_REGISTERED: usize = 512;
const MAXIMUM_IMAGE_BYTES: u64 = 6 * 1024 * 1024;
pub(super) const MAXIMUM_IN_FLIGHT: usize = 2;
const MAXIMUM_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
/// How long one head-location call may take.
const HEAD_LOCATE_DEADLINE: Duration = Duration::from_secs(25);
const CACHE_CONTROL: &str = "private, max-age=86400, immutable";

/// What a result is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageKind {
    CanonicalCharacter,
    CanonicalRoom,
    /// The current backdrop re-framed and re-lit; no people.
    SceneRefresh,
    /// One character's cut-out with another expression; transparent.
    CastExpression,
    /// A close-up photograph of one object in this place.
    ArtifactCloseUp,
    /// PLAY-V09: one composed film still of a story scene, heroes and speaker in the place.
    ScenePlate,
    /// PLAY-V09: an existing plate edited for a lasting change of the speaker's face.
    ScenePlateEdit,
}

impl ImageKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::CanonicalCharacter => "canonical_character",
            Self::CanonicalRoom => "canonical_room",
            Self::SceneRefresh => "scene_refresh",
            Self::CastExpression => "cast_expression",
            Self::ArtifactCloseUp => "artifact_close_up",
            Self::ScenePlate => "scene_plate",
            Self::ScenePlateEdit => "scene_plate_edit",
        }
    }
}

/// Output size and transparency of a result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OutputSpec {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) transparent: bool,
}

impl OutputSpec {
    /// A 16:9 backdrop.
    pub(super) const BACKDROP: Self = Self {
        width: 1536,
        height: 864,
        transparent: false,
    };
    /// A 2:3 transparent cut-out, the aspect of the generated sprites.
    pub(super) const CUTOUT: Self = Self {
        width: 1024,
        height: 1536,
        transparent: true,
    };
    /// PLAY-V09: a 16:9 composed scene plate (heroes are small in it, so it is drawn large).
    pub(super) const PLATE: Self = Self {
        width: 2560,
        height: 1440,
        transparent: false,
    };
    /// A 4:3 photographic close-up.
    pub(super) const CLOSE_UP: Self = Self {
        width: 1024,
        height: 768,
        transparent: false,
    };

    /// A cut-out is a WebP with alpha (a PNG of the same picture is several times larger); a
    /// backdrop or close-up is a JPEG.
    fn format(self) -> ImageFormat {
        if self.transparent {
            ImageFormat::Webp
        } else {
            ImageFormat::Jpeg
        }
    }
}

/// One request to the image service.
pub(super) struct ImageRequest {
    pub(super) kind: ImageKind,
    pub(super) references: Vec<ReferenceImage>,
    pub(super) prompt: AssembledPrompt,
    pub(super) output: OutputSpec,
}

/// The identity of a result: lowercase hexadecimal SHA-256, safe as one URL path segment.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ImageKey(String);

impl ImageKey {
    /// `Some` only for exactly 64 lowercase hexadecimal digits.
    pub(super) fn parse(text: &str) -> Option<Self> {
        (text.len() == 64
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| Self(text.to_owned()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }

    /// The path the display loads the image from.
    pub(super) fn path(&self) -> String {
        format!("{ROUTE}/{}", self.0)
    }
}

/// Why a request produced no image. Every variant means "keep what is on screen".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageFailure {
    /// Imagery is off.
    Off,
    /// The request failed local validation; nothing was charged or sent.
    InvalidRequest,
    DeadlineExceeded,
    /// The provider's moderation declined it. It is never reworded or rerouted.
    Refused,
    /// The provider failed, rejected the credential or was rate limited.
    Provider,
    /// The provider answered with an image that is not what was asked for.
    BadOutput,
    /// The flight ended without reporting.
    Interrupted,
}

impl ImageFailure {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::InvalidRequest => "invalid_request",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Refused => "refused",
            Self::Provider => "provider_failed",
            Self::BadOutput => "bad_output",
            Self::Interrupted => "interrupted",
        }
    }

    /// The failure a hub failure label names (a failure the hub remembered is served without
    /// running the request again, so the original is gone and only its label remains).
    fn from_label(label: &str) -> Option<Self> {
        [
            Self::Off,
            Self::InvalidRequest,
            Self::DeadlineExceeded,
            Self::Refused,
            Self::Provider,
            Self::BadOutput,
            Self::Interrupted,
        ]
        .into_iter()
        .find(|failure| failure.label() == label)
    }

    /// A failure that the same request would repeat: the hub remembers it as failed (until it is
    /// invalidated or regenerated) and it is never paid for twice.
    fn is_permanent(self) -> bool {
        matches!(self, Self::Refused | Self::BadOutput | Self::InvalidRequest)
    }
}

type Outcome = Result<(), ImageFailure>;

/// The result of asking for the bytes of one key.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Lookup {
    Image {
        bytes: Bytes,
        format: ImageFormat,
    },
    /// The server never registered this key.
    Unregistered,
    /// Registered, but no image exists (yet).
    NotReady,
}

#[derive(Clone, Copy, Debug)]
struct Registered {
    format: ImageFormat,
}

#[derive(Default)]
struct Registry {
    order: VecDeque<String>,
    by_key: HashMap<String, Registered>,
}

impl Registry {
    fn register(&mut self, key: &ImageKey, registered: Registered) {
        if self.by_key.contains_key(key.as_str()) {
            self.order.retain(|known| known != key.as_str());
        } else if self.order.len() >= MAXIMUM_REGISTERED
            && let Some(oldest) = self.order.pop_front()
        {
            self.by_key.remove(&oldest);
        }
        self.order.push_back(key.0.clone());
        self.by_key.insert(key.0.clone(), registered);
    }
}

/// Image calls are recorded in the hub's spend ledger (`Unit::ImageCall`); nothing caps them.
const IMAGE_UNIT: super::resources::Unit = super::resources::Unit::ImageCall;

/// The resource kinds an image request can be.
const IMAGE_KINDS: [super::resources::ResourceKind; 4] = [
    super::resources::ResourceKind::Depiction,
    super::resources::ResourceKind::ScenePlate,
    super::resources::ResourceKind::PlateEdit,
    super::resources::ResourceKind::ItemCloseUp,
];

/// Counters for tests; they contain no content.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Stats {
    /// Provider calls charged.
    pub(super) calls: u32,
    pub(super) cache_hits: u32,
    /// Cost of the calls that reported usage, in millionths of a US dollar.
    pub(super) micro_dollars: u64,
}

struct Engine {
    provider: ImageRoute,
    model: ImageEditModel,
    quality: ImageQuality,
    rating: ContentRating,
    deadline: Duration,
    cache_dir: PathBuf,
    manifest: Option<PathBuf>,
    ledger: Arc<super::resources::SpendLedger>,
    calls: AtomicU32,
    cache_hits: AtomicU32,
    micro_dollars: AtomicU64,
    permits: Arc<Semaphore>,
    in_flight: Mutex<HashMap<String, watch::Receiver<Option<Outcome>>>>,
    /// PLAY-V09: finds the speaker's head in a finished plate; absent in tests and when the
    /// production provider could not be built.
    locator: std::sync::OnceLock<NativeHeadLocator>,
}

enum Mode {
    Off(ImageryOff),
    On {
        engine: Arc<Engine>,
        summary: String,
    },
}

struct Inner {
    registry: Mutex<Registry>,
    mode: Mode,
    /// The resource generation hub: the one place a generated image is asked for and remembered.
    hub: super::resources::Hub,
}

/// Registry of renderable results plus, when enabled, the cached provider calls.
#[derive(Clone)]
pub(super) struct Imagery {
    inner: Arc<Inner>,
}

/// The guarded data are plain collections and counters that stay consistent when a holder
/// panics, so a poisoned lock is still safe to use.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Serialize, Deserialize)]
struct ManifestLine {
    key: String,
    kind: String,
    model: String,
    prompt: String,
    references: Vec<String>,
    created_unix: u64,
    bytes: u64,
    width: u32,
    height: u32,
    transparent: bool,
    micro_dollars: u64,
    /// PLAY-V09: how long the provider call took, in milliseconds; absent in older manifests.
    #[serde(default)]
    elapsed_ms: u64,
}

/// Where images are made. FLUX (fal) is the default whenever its key is set: the owner judged
/// only FLUX's output a real photograph (2026-10-17); the OpenAI image-edit route remains for runs
/// without a fal key and for the loopback test fakes.
pub(super) enum ImageRoute {
    OpenAi(NativeImageEditProvider),
    Flux(df_providers::NativeFalImageProvider),
}

impl From<NativeImageEditProvider> for ImageRoute {
    fn from(provider: NativeImageEditProvider) -> Self {
        Self::OpenAi(provider)
    }
}

impl ImageRoute {
    /// `DF_GAMEPLAY_IMAGERY_PROVIDER` is `flux` (needs `FAL_KEY`), `openai`, or unset: FLUX when
    /// `FAL_KEY` is set, OpenAI otherwise.
    fn configured(configuration: &ImageryConfiguration) -> Result<Self, io::Error> {
        let fal_key = std::env::var("FAL_KEY").ok().filter(|key| !key.is_empty());
        let wanted = std::env::var("DF_GAMEPLAY_IMAGERY_PROVIDER").unwrap_or_default();
        let flux = match (wanted.as_str(), &fal_key) {
            ("openai", _) => None,
            ("flux", None) => {
                return Err(io::Error::other(
                    "DF_GAMEPLAY_IMAGERY_PROVIDER=flux needs FAL_KEY",
                ));
            }
            ("" | "flux", key) => key.clone(),
            _ => return Err(io::Error::other("DF_GAMEPLAY_IMAGERY_PROVIDER refused")),
        };
        match flux {
            Some(key) => df_providers::NativeFalImageProvider::new(
                df_providers::FalImageConfig::live(),
                &key,
                configuration.deadline,
            )
            .map(Self::Flux)
            .map_err(|_| io::Error::other("imagery provider configuration refused")),
            None => NativeImageEditProvider::new(
                ImageEditConfig::live(),
                configuration.credential.expose(),
                configuration.deadline,
            )
            .map(Self::OpenAi)
            .map_err(|_| io::Error::other("imagery provider configuration refused")),
        }
    }

    fn label(&self, model: ImageEditModel) -> &'static str {
        match self {
            Self::OpenAi(_) => model.model_id(),
            Self::Flux(_) => "flux-1.1-ultra-raw+kontext-max",
        }
    }

    async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<df_providers::GeneratedImage, ImageEditError> {
        match self {
            Self::OpenAi(provider) => provider.edit(edit, deadline).await,
            Self::Flux(provider) => provider.edit(edit, deadline).await,
        }
    }
}

impl Imagery {
    /// An image service that registers nothing and never renders.
    pub(super) fn disabled(reason: ImageryOff) -> Self {
        Self {
            inner: Arc::new(Inner {
                registry: Mutex::new(Registry::default()),
                mode: Mode::Off(reason),
                hub: super::resources::Hub::new(),
            }),
        }
    }

    /// Builds the production OpenAI image-edit route from validated settings. The run-local
    /// manifest is written to `manifest_directory` when given.
    pub(super) fn start(
        settings: ImagerySettings,
        manifest_directory: Option<&Path>,
    ) -> Result<Self, io::Error> {
        let configuration = match settings {
            ImagerySettings::Off(reason) => return Ok(Self::disabled(reason)),
            ImagerySettings::On(configuration) => configuration,
        };
        let provider = ImageRoute::configured(&configuration)?;
        let locator = NativeHeadLocator::new(
            HeadLocateConfig::live(),
            configuration.credential.expose(),
            configuration.deadline,
        )
        .ok();
        let imagery = Self::new(configuration, provider, manifest_directory)?;
        if let (Mode::On { engine, .. }, Some(locator)) = (&imagery.inner.mode, locator) {
            // Set once, here; a second call cannot happen.
            let _first = engine.locator.set(locator);
        }
        Ok(imagery)
    }

    /// PLAY-V09: where the heads of `targets` (`id`, fixed description) are in the finished image
    /// `key`, as fractions of the picture. `face` is a picture of the first target's face (any
    /// image the server holds), which tells the right person from the many in the scene. Costs a
    /// fraction of a cent, never charges the image budget, and every failure is typed: the caller
    /// keeps the layout it predicted.
    pub(super) async fn locate_heads(
        &self,
        key: &ImageKey,
        face: Option<&ReferenceImage>,
        targets: &[(&str, &str)],
    ) -> Result<Vec<LocatedHead>, ImageFailure> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return Err(ImageFailure::Off);
        };
        let Some(locator) = engine.locator.get() else {
            return Err(ImageFailure::Off);
        };
        let registered = locked(&self.inner.registry)
            .by_key
            .get(key.as_str())
            .copied();
        let format = registered.ok_or(ImageFailure::InvalidRequest)?.format;
        let (bytes, stored) = engine
            .read_cached(key.as_str(), format)
            .await
            .ok_or(ImageFailure::InvalidRequest)?;
        let face = face.map(|face| (face.bytes.clone(), face.format));
        let (picture, face) = tokio::task::spawn_blocking(move || {
            let picture = vision_picture(&bytes, stored, 1024)?;
            let face = face
                .map(|(bytes, format)| vision_picture(&bytes, format, 384))
                .transpose()?;
            Ok::<_, SheetError>((picture, face))
        })
        .await
        .map_err(|_| ImageFailure::Interrupted)?
        .map_err(|_| ImageFailure::BadOutput)?;
        let targets: Vec<LocateTarget<'_>> = targets
            .iter()
            .map(|(id, description)| LocateTarget { id, description })
            .collect();
        // Head measurement shares the existing provider concurrency owner with image edits.
        // Its enclosing image job also bounds time spent waiting for this permit.
        let _permit = engine
            .permits
            .acquire()
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        locator
            .locate(&picture, face.as_deref(), &targets, HEAD_LOCATE_DEADLINE)
            .await
            .map_err(|error| match error {
                df_providers::HeadLocateError::InvalidRequest => ImageFailure::InvalidRequest,
                df_providers::HeadLocateError::Output => ImageFailure::BadOutput,
                df_providers::HeadLocateError::Transport(NativeHttpRefusal::Deadline) => {
                    ImageFailure::DeadlineExceeded
                }
                df_providers::HeadLocateError::Unauthorized
                | df_providers::HeadLocateError::Provider(_)
                | df_providers::HeadLocateError::Transport(_) => ImageFailure::Provider,
            })
    }

    /// PLAY-V09: a small note kept beside the cached image `key` (its measured layout), so a
    /// later server run does not ask again. `None` when there is none (or imagery is off).
    pub(super) async fn read_note(&self, key: &ImageKey) -> Option<Vec<u8>> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return None;
        };
        let path = engine
            .cache_dir
            .join(format!("{}.layout.json", key.as_str()));
        tokio::task::spawn_blocking(move || read_cache_file(&path).map(|bytes| bytes.to_vec()))
            .await
            .ok()
            .flatten()
    }

    /// Keeps a note beside the cached image `key`. A failed write only means it is asked again.
    pub(super) async fn write_note(&self, key: &ImageKey, note: Vec<u8>) {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return;
        };
        let target = engine
            .cache_dir
            .join(format!("{}.layout.json", key.as_str()));
        let staging = engine.cache_dir.join(format!(
            ".{}.layout.{}.tmp",
            key.as_str(),
            std::process::id()
        ));
        let _kept =
            tokio::task::spawn_blocking(move || write_cache_file(&staging, &target, &note)).await;
    }

    /// Uses `provider`; tests pass one pointed at a loopback fake.
    pub(super) fn new(
        configuration: ImageryConfiguration,
        provider: impl Into<ImageRoute>,
        manifest_directory: Option<&Path>,
    ) -> Result<Self, io::Error> {
        let provider = provider.into();
        std::fs::create_dir_all(&configuration.cache_dir)?;
        let summary = format!(
            "IMAGERY on model={} interval={}s rating={} quality={}",
            provider.label(configuration.model),
            configuration.interval.as_secs(),
            configuration.rating.label(),
            match configuration.quality {
                ImageQuality::Low => "low",
                ImageQuality::Medium => "medium",
                ImageQuality::High => "high",
            },
        );
        let manifest = manifest_directory.map(|directory| directory.join(MANIFEST_NAME));
        let hub = super::resources::Hub::new();
        let engine = Engine {
            provider,
            model: configuration.model,
            quality: configuration.quality,
            rating: configuration.rating,
            deadline: configuration.deadline,
            cache_dir: configuration.cache_dir,
            manifest,
            ledger: Arc::clone(hub.ledger()),
            calls: AtomicU32::new(0),
            cache_hits: AtomicU32::new(0),
            micro_dollars: AtomicU64::new(0),
            permits: Arc::new(Semaphore::new(MAXIMUM_IN_FLIGHT)),
            in_flight: Mutex::new(HashMap::new()),
            locator: std::sync::OnceLock::new(),
        };
        let mut registry = Registry::default();
        engine.register_from_manifest(&mut registry);
        let engine = Arc::new(engine);
        // The hub's bounds for the image kinds: a request may wait for a free provider slot and
        // then runs under the provider's own deadline, so the hub never fires before it.
        let wait = engine.deadline * (MAXIMUM_IN_FLIGHT as u32 + 1) + Duration::from_secs(10);
        for kind in IMAGE_KINDS {
            hub.set_limits(
                kind,
                super::resources::Limits {
                    timeout: wait,
                    attempts: 2,
                },
            );
            let checked = Arc::clone(&engine);
            hub.set_validator(
                kind,
                Arc::new(move |generated| checked.cached_now(&generated.handle)),
            );
        }
        Ok(Self {
            inner: Arc::new(Inner {
                registry: Mutex::new(registry),
                mode: Mode::On { engine, summary },
                hub,
            }),
        })
    }

    /// The startup line; it never contains the credential.
    pub(super) fn startup_line(&self) -> String {
        match &self.inner.mode {
            Mode::Off(reason) => format!("IMAGERY off ({})", reason.reason()),
            Mode::On { summary, .. } => summary.clone(),
        }
    }

    pub(super) fn is_on(&self) -> bool {
        matches!(self.inner.mode, Mode::On { .. })
    }

    #[cfg(test)]
    pub(super) fn stats(&self) -> Stats {
        match &self.inner.mode {
            Mode::Off(_) => Stats::default(),
            Mode::On { engine, .. } => Stats {
                calls: engine.calls.load(Ordering::SeqCst),
                cache_hits: engine.cache_hits.load(Ordering::SeqCst),
                micro_dollars: engine.micro_dollars.load(Ordering::SeqCst),
            },
        }
    }

    /// The key a request would be stored under.
    pub(super) fn key_for(&self, request: &ImageRequest) -> ImageKey {
        let (model, quality, rating) = match &self.inner.mode {
            Mode::On { engine, .. } => (
                engine.model.model_id(),
                engine.quality_label(),
                engine.rating.label(),
            ),
            Mode::Off(_) => ("off", "off", "off"),
        };
        request_key(request, model, quality, rating)
    }

    /// The resource generation hub that owns every generated image's request, state and audit.
    pub(super) fn hub(&self) -> &super::resources::Hub {
        &self.inner.hub
    }

    /// The one way to get an image made: describes the request as a typed resource, runs it
    /// through the hub (state, timeout, request cap, `RESOURCE` audit lines) and returns the key
    /// to load it by. `subject` names what the picture is about (`character:ant-wolf`,
    /// `place:salt-quay`, `plate:<scene>`, `item:<id>`). Every consumer calls this; the render
    /// below is the hub's own generator and is private to this module.
    pub(super) async fn generate(
        &self,
        subject: &str,
        request: ImageRequest,
    ) -> Result<ImageKey, ImageFailure> {
        use super::resources::{
            DepictionSpec, ItemCloseUpSpec, PlateEditSpec, PlateSpec,
        };
        use df_render::art_contract::Neutral;
        let key = self.key_for(&request);
        let key = key.as_str();
        match request.kind {
            ImageKind::ScenePlate => {
                self.run(PlateSpec::new(subject, key, Neutral::Nothing), request)
                    .await
            }
            ImageKind::ScenePlateEdit => {
                self.run(PlateEditSpec::new(subject, key, Neutral::Nothing), request)
                    .await
            }
            ImageKind::ArtifactCloseUp => {
                self.run(ItemCloseUpSpec::new(subject, key, Neutral::Block), request)
                    .await
            }
            ImageKind::CanonicalCharacter | ImageKind::CastExpression => {
                self.run(
                    DepictionSpec::new(subject, key, Neutral::Silhouette),
                    request,
                )
                .await
            }
            ImageKind::CanonicalRoom | ImageKind::SceneRefresh => {
                self.run(DepictionSpec::new(subject, key, Neutral::Nothing), request)
                    .await
            }
        }
    }

    async fn run<S: super::resources::ResourceSpec>(
        &self,
        spec: S,
        request: ImageRequest,
    ) -> Result<ImageKey, ImageFailure> {
        use super::resources::{Generated, Generation, ResourceFailure};
        let key = self.key_for(&request);
        let original = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&original);
        let imagery = self.clone();
        let generation: Generation = Box::pin(async move {
            match imagery.render(request).await {
                Ok(key) => Ok(Generated::new(key.as_str())),
                Err(failure) => {
                    *locked(&slot) = Some(failure);
                    Err(if failure.is_permanent() {
                        ResourceFailure::Permanent(failure.label())
                    } else {
                        ResourceFailure::Transient(failure.label())
                    })
                }
            }
        });
        match self.hub().render(&spec, generation).await {
            Ok(_) => Ok(key),
            Err(ResourceFailure::TimedOut) => Err(ImageFailure::DeadlineExceeded),
            Err(failure) => Err(locked(&original)
                .take()
                .or_else(|| ImageFailure::from_label(failure.label()))
                .unwrap_or(ImageFailure::Interrupted)),
        }
    }

    /// Renders `request`, or finds its cached result, and returns the key to load it by. The
    /// key is registered before any work so the route can serve it as soon as it exists. Only the
    /// hub (through `generate`) calls this.
    async fn render(&self, request: ImageRequest) -> Result<ImageKey, ImageFailure> {
        let started = std::time::Instant::now();
        let key = self.key_for(&request);
        let kind = request.kind;
        let (line, fields, attributes) = image_request_diagnostics(&key, &request);
        crate::gameplay::actor::record_audit_diagnostic(
            "gameplay.imagery.render_requested",
            line,
            fields,
            &attributes,
        );
        let Mode::On { engine, .. } = &self.inner.mode else {
            record_image_result(kind, &key, started.elapsed(), Err(&ImageFailure::Off));
            return Err(ImageFailure::Off);
        };
        let format = request.output.format();
        locked(&self.inner.registry).register(&key, Registered { format });
        let outcome = engine.image(&key, request).await;
        let result = outcome.map(|()| key.clone());
        record_image_result(
            kind,
            &key,
            started.elapsed(),
            result.as_ref().map_err(|e| e),
        );
        result
    }

    /// The bytes of a registered key, read from the cache.
    pub(super) async fn image(&self, key: &str) -> Lookup {
        let registered = locked(&self.inner.registry).by_key.get(key).copied();
        let Some(registered) = registered else {
            return Lookup::Unregistered;
        };
        let Mode::On { engine, .. } = &self.inner.mode else {
            return Lookup::NotReady;
        };
        match engine.read_cached(key, registered.format).await {
            Some((bytes, format)) => Lookup::Image { bytes, format },
            None => Lookup::NotReady,
        }
    }

    /// The key of `request` if its image is already cached; never calls the provider and never
    /// charges. The only way a fight learns what it may show.
    pub(super) async fn peek(&self, request: &ImageRequest) -> Option<ImageKey> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return None;
        };
        let key = self.key_for(request);
        let format = request.output.format();
        engine.cache_has(&key, format).await?;
        locked(&self.inner.registry).register(&key, Registered { format });
        Some(key)
    }

}

impl Engine {
    fn quality_label(&self) -> &'static str {
        match self.quality {
            ImageQuality::Low => "low",
            ImageQuality::Medium => "medium",
            ImageQuality::High => "high",
        }
    }

    fn file_name(key: &ImageKey, format: ImageFormat) -> String {
        let extension = match format {
            ImageFormat::Png => "png",
            ImageFormat::Webp => "webp",
            ImageFormat::Jpeg => "jpg",
        };
        format!("{}.{extension}", key.as_str())
    }

    /// The formats an image of `format` may be found in: a cut-out cached before WebP output
    /// is a PNG of the same request.
    fn stored_formats(format: ImageFormat) -> &'static [ImageFormat] {
        match format {
            ImageFormat::Webp => &[ImageFormat::Webp, ImageFormat::Png],
            ImageFormat::Png => &[ImageFormat::Png],
            ImageFormat::Jpeg => &[ImageFormat::Jpeg],
        }
    }

    async fn image(self: &Arc<Self>, key: &ImageKey, request: ImageRequest) -> Outcome {
        let format = request.output.format();
        if self.cache_has(key, format).await.is_some() {
            self.cache_hits.fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }
        let mut receiver = {
            let mut flights = locked(&self.in_flight);
            match flights.get(key.as_str()) {
                Some(receiver) => receiver.clone(),
                None => {
                    let (sender, receiver) = watch::channel(None);
                    flights.insert(key.0.clone(), receiver.clone());
                    tokio::spawn(Arc::clone(self).fly(key.clone(), request, sender));
                    receiver
                }
            }
        };
        let finished = receiver
            .wait_for(Option::is_some)
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        finished.unwrap_or(Err(ImageFailure::Interrupted))
    }

    /// The one task of a key. Callers that go away do not cancel it: its charge is already
    /// spent, so its image is finished and cached for the next request.
    async fn fly(
        self: Arc<Self>,
        key: ImageKey,
        request: ImageRequest,
        sender: watch::Sender<Option<Outcome>>,
    ) {
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&context, "gameplay.imagery_render");
        span.event(
            "gameplay.imagery.render",
            &span_fields(
                request.kind,
                self.model.model_id(),
                request.references.len(),
            ),
        );
        let outcome = self.produce(&key, request).await;
        span.finish_unmeasured(match outcome {
            Ok(()) => "rendered",
            Err(failure) => failure.label(),
        });
        locked(&self.in_flight).remove(key.as_str());
        // No receiver left means every requester went away; the cache holds the result.
        sender.send_replace(Some(outcome));
    }

    async fn produce(&self, key: &ImageKey, request: ImageRequest) -> Outcome {
        let format = request.output.format();
        // A flight that finished between the caller's cache check and this one wrote it.
        if self.cache_has(key, format).await.is_some() {
            self.cache_hits.fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }
        let references: Vec<ProviderReference<'_>> = request
            .references
            .iter()
            .map(|reference| ProviderReference {
                format: reference.format,
                bytes: &reference.bytes,
            })
            .collect();
        let edit = ImageEdit {
            model: self.model,
            prompt: request.prompt.as_str(),
            references: &references,
            width: request.output.width,
            height: request.output.height,
            quality: self.quality,
            background: if request.output.transparent {
                ImageBackground::Transparent
            } else {
                ImageBackground::Opaque
            },
            output: format,
            moderation: match self.rating {
                ContentRating::Mature => ImageModeration::Low,
                ContentRating::Restrained => ImageModeration::Auto,
            },
        };
        edit.validate().map_err(|_| ImageFailure::InvalidRequest)?;
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        self.charge();
        let started = std::time::Instant::now();
        let generated = self
            .provider
            .edit(&edit, self.deadline)
            .await
            .map_err(failure_of)?;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if generated.bytes.is_empty() || generated.bytes.len() as u64 > MAXIMUM_IMAGE_BYTES {
            return Err(ImageFailure::BadOutput);
        }
        let micro_dollars = usage_cost(generated.usage);
        self.micro_dollars
            .fetch_add(micro_dollars, Ordering::SeqCst);
        // FLUX may answer a WebP request with a PNG; it is kept in a format the request accepts.
        if !Self::stored_formats(format).contains(&generated.format) {
            return Err(ImageFailure::BadOutput);
        }
        let stored_format = generated.format;
        let bytes = Bytes::from(generated.bytes);
        let stored = self.store(key, stored_format, bytes.clone()).await.is_ok();
        if stored {
            self.record(key, &request, bytes.len() as u64, micro_dollars, elapsed_ms)
                .await;
        }
        // A result that could not be cached is still served from memory for this key's first
        // requester only through the cache, so a failed write is a failed render.
        stored.then_some(()).ok_or(ImageFailure::Provider)
    }

    /// Spend is committed before the call and kept whatever the outcome: a call whose result is
    /// unknown may still have been billed.
    fn charge(&self) {
        self.ledger.reserve(IMAGE_UNIT, 1).commit(1);
        self.calls.fetch_add(1, Ordering::SeqCst);
    }

    /// Whether an image of `key` is on disk in any format, without waiting: the hub's
    /// freshness check of a ready entry.
    fn cached_now(&self, key: &str) -> bool {
        let Some(key) = ImageKey::parse(key) else {
            return false;
        };
        [ImageFormat::Webp, ImageFormat::Png, ImageFormat::Jpeg]
            .into_iter()
            .any(|format| {
                std::fs::symlink_metadata(self.cache_dir.join(Self::file_name(&key, format)))
                    .is_ok_and(|metadata| {
                        metadata.is_file() && (1..=MAXIMUM_IMAGE_BYTES).contains(&metadata.len())
                    })
            })
    }

    /// The format an image of `format` is cached in, if it is cached.
    async fn cache_has(&self, key: &ImageKey, format: ImageFormat) -> Option<ImageFormat> {
        let paths: Vec<(ImageFormat, PathBuf)> = Self::stored_formats(format)
            .iter()
            .map(|found| (*found, self.cache_dir.join(Self::file_name(key, *found))))
            .collect();
        tokio::task::spawn_blocking(move || {
            paths.into_iter().find_map(|(found, path)| {
                std::fs::symlink_metadata(&path)
                    .is_ok_and(|metadata| {
                        metadata.is_file() && (1..=MAXIMUM_IMAGE_BYTES).contains(&metadata.len())
                    })
                    .then_some(found)
            })
        })
        .await
        .ok()
        .flatten()
    }

    async fn read_cached(&self, key: &str, format: ImageFormat) -> Option<(Bytes, ImageFormat)> {
        let key = ImageKey::parse(key)?;
        let paths: Vec<(ImageFormat, PathBuf)> = Self::stored_formats(format)
            .iter()
            .map(|found| (*found, self.cache_dir.join(Self::file_name(&key, *found))))
            .collect();
        // A failed blocking task is treated as a cache miss.
        tokio::task::spawn_blocking(move || {
            paths
                .into_iter()
                .find_map(|(found, path)| read_cache_file(&path).map(|bytes| (bytes, found)))
        })
        .await
        .ok()
        .flatten()
    }

    /// Writes a temporary file beside the target and renames it into place so a reader never
    /// sees a partial file.
    async fn store(&self, key: &ImageKey, format: ImageFormat, bytes: Bytes) -> io::Result<()> {
        let name = Self::file_name(key, format);
        let target = self.cache_dir.join(&name);
        let staging = self
            .cache_dir
            .join(format!(".{name}.{}.tmp", std::process::id()));
        tokio::task::spawn_blocking(move || write_cache_file(&staging, &target, &bytes))
            .await
            .map_err(|_| io::Error::other("imagery cache write task failed"))?
    }

    /// Appends the provenance of one image to the run-local manifest. A failed append is
    /// reported to diagnostics and never fails the render.
    async fn record(
        &self,
        key: &ImageKey,
        request: &ImageRequest,
        bytes: u64,
        micro_dollars: u64,
        elapsed_ms: u64,
    ) {
        let Some(path) = self.manifest.clone() else {
            return;
        };
        let line = ManifestLine {
            key: key.0.clone(),
            kind: request.kind.label().to_owned(),
            model: self.model.model_id().to_owned(),
            prompt: request.prompt.as_str().to_owned(),
            references: request
                .references
                .iter()
                .map(|reference| reference.sha256.clone())
                .collect(),
            created_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
            bytes,
            width: request.output.width,
            height: request.output.height,
            transparent: request.output.transparent,
            micro_dollars,
            elapsed_ms,
        };
        let appended = tokio::task::spawn_blocking(move || append_manifest(&path, &line)).await;
        if !matches!(appended, Ok(Ok(()))) {
            let context = OperationContext {
                trace_parent: String::new(),
                build: crate::BUILD_ID.to_owned(),
            };
            let mut span = df_observe::begin(&context, "gameplay.imagery_manifest");
            span.finish_unmeasured("append_failed");
        }
    }

    /// Re-registers the keys an earlier run of this adventure generated and cached, so a display
    /// that reconnects after a restart can still load the art it was shown.
    fn register_from_manifest(&self, registry: &mut Registry) {
        let Some(path) = &self.manifest else {
            return;
        };
        let Some(text) = read_bounded_text(path) else {
            return;
        };
        for line in text.lines() {
            let Ok(entry) = serde_json::from_str::<ManifestLine>(line) else {
                continue;
            };
            let Some(key) = ImageKey::parse(&entry.key) else {
                continue;
            };
            let format = if entry.transparent {
                ImageFormat::Webp
            } else {
                ImageFormat::Jpeg
            };
            let present = Self::stored_formats(format).iter().any(|found| {
                std::fs::symlink_metadata(self.cache_dir.join(Self::file_name(&key, *found)))
                    .is_ok_and(|metadata| metadata.is_file())
            });
            if present {
                registry.register(&key, Registered { format });
            }
        }
    }
}

fn failure_of(error: ImageEditError) -> ImageFailure {
    match error {
        ImageEditError::InvalidRequest => ImageFailure::InvalidRequest,
        ImageEditError::Refused => ImageFailure::Refused,
        ImageEditError::Unauthorized | ImageEditError::Provider(_) => ImageFailure::Provider,
        ImageEditError::Transport(NativeHttpRefusal::Deadline) => ImageFailure::DeadlineExceeded,
        ImageEditError::Transport(_) => ImageFailure::Provider,
        ImageEditError::Output => ImageFailure::BadOutput,
    }
}

fn usage_cost(usage: ImageUsage) -> u64 {
    usage.micro_dollars()
}

/// The only fields a span carries: the kind, the model and a count. Never a prompt or an image.
fn span_fields(
    kind: ImageKind,
    model: &'static str,
    references: usize,
) -> [(&'static str, String); 3] {
    [
        ("gameplay.imagery.kind", kind.label().to_owned()),
        ("gameplay.imagery.model", model.to_owned()),
        ("gameplay.imagery.references", references.to_string()),
    ]
}

fn image_request_diagnostics(
    key: &ImageKey,
    request: &ImageRequest,
) -> (
    String,
    serde_json::Map<String, serde_json::Value>,
    Vec<(&'static str, String)>,
) {
    let kind = request.kind.label();
    let key = key.as_str();
    let prompt_char_count = request.prompt.character_count();
    let reference_byte_lengths: Vec<usize> = request
        .references
        .iter()
        .map(|reference| reference.bytes.len())
        .collect();
    let reference_sha256: Vec<&str> = request
        .references
        .iter()
        .map(|reference| reference.sha256.as_str())
        .collect();
    let prompt_sha256 = format!("{:x}", Sha256::digest(request.prompt.as_str().as_bytes()));
    let reference_bytes_total = reference_byte_lengths.iter().sum::<usize>();
    let reference_count = reference_byte_lengths.len();
    let line = format!(
        "IMAGERY request kind={kind} id={key} prompt_chars={prompt_char_count} references={reference_count} reference_bytes={reference_bytes_total}"
    );
    let fields = serde_json::Map::from_iter([
        (
            "request_schema".to_owned(),
            serde_json::json!(REQUEST_SCHEMA),
        ),
        ("prompt_sha256".to_owned(), serde_json::json!(prompt_sha256)),
        (
            "reference_sha256".to_owned(),
            serde_json::json!(reference_sha256),
        ),
        ("kind".to_owned(), serde_json::json!(kind)),
        ("image_key".to_owned(), serde_json::json!(key)),
        (
            "prompt_char_count".to_owned(),
            serde_json::json!(prompt_char_count),
        ),
        (
            "reference_count".to_owned(),
            serde_json::json!(reference_count),
        ),
        (
            "reference_byte_lengths".to_owned(),
            serde_json::json!(reference_byte_lengths),
        ),
        (
            "reference_bytes_total".to_owned(),
            serde_json::json!(reference_bytes_total),
        ),
    ]);
    let attributes = vec![
        ("gameplay.imagery.request_schema", REQUEST_SCHEMA.to_owned()),
        ("gameplay.imagery.prompt_sha256", prompt_sha256),
        (
            "gameplay.imagery.reference_sha256",
            reference_sha256.join(","),
        ),
        ("gameplay.imagery.kind", kind.to_owned()),
        ("gameplay.imagery.image_key", key.to_owned()),
        (
            "gameplay.imagery.prompt_char_count",
            prompt_char_count.to_string(),
        ),
        (
            "gameplay.imagery.reference_count",
            reference_count.to_string(),
        ),
        (
            "gameplay.imagery.reference_byte_lengths",
            reference_byte_lengths
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(","),
        ),
        (
            "gameplay.imagery.reference_bytes_total",
            reference_bytes_total.to_string(),
        ),
    ];
    (line, fields, attributes)
}

fn record_image_result(
    kind: ImageKind,
    key: &ImageKey,
    elapsed: Duration,
    result: Result<&ImageKey, &ImageFailure>,
) {
    let (status, failure_category) = match result {
        Ok(_) => ("ready", None),
        Err(failure) => ("failed", Some(failure.label())),
    };
    let elapsed_ms = elapsed.as_millis();
    let line = format!(
        "IMAGERY {status} kind={} id={} ms={elapsed_ms} failure_category={}",
        kind.label(),
        key.as_str(),
        failure_category.unwrap_or("none"),
    );
    let fields = serde_json::Map::from_iter([
        ("kind".to_owned(), serde_json::json!(kind.label())),
        ("image_key".to_owned(), serde_json::json!(key.as_str())),
        ("status".to_owned(), serde_json::json!(status)),
        (
            "failure_category".to_owned(),
            serde_json::json!(failure_category),
        ),
        ("elapsed_ms".to_owned(), serde_json::json!(elapsed_ms)),
    ]);
    let attributes = [
        ("gameplay.imagery.kind", kind.label().to_owned()),
        ("gameplay.imagery.image_key", key.as_str().to_owned()),
        ("gameplay.imagery.status", status.to_owned()),
        (
            "gameplay.imagery.failure_category",
            failure_category.unwrap_or("none").to_owned(),
        ),
        ("gameplay.imagery.elapsed_ms", elapsed_ms.to_string()),
    ];
    let event = match status {
        "ready" => "gameplay.imagery.render_ready",
        _ => "gameplay.imagery.render_failed",
    };
    crate::gameplay::actor::record_audit_diagnostic(event, line, fields, &attributes);
}

const REQUEST_SCHEMA: &str = "df-imagery/1";

fn request_key(request: &ImageRequest, model: &str, quality: &str, rating: &str) -> ImageKey {
    let mut hasher = Sha256::new();
    let mut part = |text: &str| {
        hasher.update(text.as_bytes());
        hasher.update(b"\n");
    };
    part(REQUEST_SCHEMA);
    part(request.kind.label());
    part(model);
    part(quality);
    part(rating);
    part(&format!(
        "{}x{} transparent={}",
        request.output.width, request.output.height, request.output.transparent
    ));
    for reference in &request.references {
        part(&reference.sha256);
    }
    part(request.prompt.as_str());
    ImageKey(
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn read_cache_file(path: &Path) -> Option<Bytes> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAXIMUM_IMAGE_BYTES {
        return None;
    }
    std::fs::read(path).ok().map(Bytes::from)
}

fn write_cache_file(staging: &Path, target: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() {
        return Err(io::Error::other("an empty image is never cached"));
    }
    let written = std::fs::write(staging, bytes).and_then(|()| std::fs::rename(staging, target));
    if written.is_err() {
        // Best effort: a leftover staging file is the only residue of a failed write.
        let _removed = std::fs::remove_file(staging);
    }
    written
}

fn append_manifest(path: &Path, line: &ManifestLine) -> io::Result<()> {
    use std::io::Write;
    let mut text = serde_json::to_string(line).map_err(io::Error::other)?;
    text.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(text.as_bytes())
}

fn read_bounded_text(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAXIMUM_MANIFEST_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// `GET /gameplay/imagery/{key}`.
pub(super) fn router(imagery: Imagery) -> Router {
    Router::new()
        .route("/gameplay/imagery/{key}", get(serve_image))
        .with_state(imagery)
}

async fn serve_image(
    State(imagery): State<Imagery>,
    RoutePath(key): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    let started = std::time::Instant::now();
    let resource_id = safe_image_resource_id(&key);
    let browser_asset_id =
        df_observe::presentation::local_identity(&format!("/gameplay/imagery/{key}"));
    let (response, status_category, cache_result, byte_length, failure_category) =
        match imagery.image(&key).await {
            Lookup::Image { bytes, format } => {
                let etag = format!("\"{key}\"");
                let content_type = match format {
                    ImageFormat::Png => "image/png",
                    ImageFormat::Webp => "image/webp",
                    ImageFormat::Jpeg => "image/jpeg",
                };
                if headers
                    .get(header::IF_NONE_MATCH)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value == etag)
                {
                    (
                        (
                            StatusCode::NOT_MODIFIED,
                            [
                                (header::ETAG, etag),
                                (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
                            ],
                        )
                            .into_response(),
                        "not_modified",
                        "client_revalidated",
                        0,
                        None,
                    )
                } else {
                    let byte_length = bytes.len();
                    (
                        (
                            [
                                (header::CONTENT_TYPE, content_type.to_owned()),
                                (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
                                (header::ETAG, etag),
                            ],
                            bytes,
                        )
                            .into_response(),
                        "ready",
                        "server_hit",
                        byte_length,
                        None,
                    )
                }
            }
            Lookup::Unregistered => (
                StatusCode::NOT_FOUND.into_response(),
                "unregistered",
                "not_applicable",
                0,
                Some("unregistered"),
            ),
            Lookup::NotReady => (
                StatusCode::SERVICE_UNAVAILABLE.into_response(),
                "not_ready",
                "miss",
                0,
                Some("not_ready"),
            ),
        };
    record_media_delivery(
        "image",
        &resource_id,
        browser_asset_id,
        status_category,
        failure_category,
        response.status().as_u16(),
        byte_length,
        cache_result,
        started.elapsed().as_millis(),
    );
    response
}

fn safe_image_resource_id(key: &str) -> String {
    if key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        key.to_owned()
    } else {
        df_observe::presentation::local_identity(key).to_string()
    }
}

#[allow(clippy::too_many_arguments)]
fn record_media_delivery(
    resource_type: &'static str,
    resource_id: &str,
    browser_asset_id: u64,
    status: &'static str,
    failure_category: Option<&'static str>,
    http_status: u16,
    byte_length: usize,
    cache_result: &'static str,
    elapsed_ms: u128,
) {
    let line = format!(
        "MEDIA_DELIVERY type={resource_type} resource_id={resource_id} browser_asset_id={browser_asset_id} status={status} http_status={http_status} bytes={byte_length} cache_result={cache_result} elapsed_ms={elapsed_ms} failure_category={}",
        failure_category.unwrap_or("none"),
    );
    let fields = serde_json::Map::from_iter([
        ("resource_type".to_owned(), serde_json::json!(resource_type)),
        ("resource_id".to_owned(), serde_json::json!(resource_id)),
        (
            "browser_asset_id".to_owned(),
            serde_json::json!(browser_asset_id),
        ),
        (
            "stage".to_owned(),
            serde_json::json!(if failure_category.is_some() {
                "failed"
            } else {
                "delivered"
            }),
        ),
        (
            "status".to_owned(),
            serde_json::json!(if failure_category.is_some() {
                "failed"
            } else {
                "delivered"
            }),
        ),
        ("status_category".to_owned(), serde_json::json!(status)),
        (
            "failure_category".to_owned(),
            serde_json::json!(failure_category),
        ),
        ("provider".to_owned(), serde_json::json!("image_cache")),
        ("http_status".to_owned(), serde_json::json!(http_status)),
        ("byte_length".to_owned(), serde_json::json!(byte_length)),
        ("cache_result".to_owned(), serde_json::json!(cache_result)),
        ("elapsed_ms".to_owned(), serde_json::json!(elapsed_ms)),
    ]);
    let attributes = [
        ("gameplay.resource.type", resource_type.to_owned()),
        ("gameplay.resource.id", resource_id.to_owned()),
        (
            "gameplay.media.browser_asset_id",
            browser_asset_id.to_string(),
        ),
        (
            "gameplay.resource.stage",
            if failure_category.is_some() {
                "failed".to_owned()
            } else {
                "delivered".to_owned()
            },
        ),
        (
            "gameplay.resource.status",
            if failure_category.is_some() {
                "failed".to_owned()
            } else {
                "delivered".to_owned()
            },
        ),
        ("gameplay.media.status_category", status.to_owned()),
        (
            "gameplay.resource.failure_category",
            failure_category.unwrap_or("none").to_owned(),
        ),
        ("gameplay.media.provider", "image_cache".to_owned()),
        ("gameplay.media.http_status", http_status.to_string()),
        ("gameplay.resource.byte_length", byte_length.to_string()),
        ("gameplay.media.cache_result", cache_result.to_owned()),
        ("gameplay.resource.elapsed_ms", elapsed_ms.to_string()),
    ];
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.media.delivery",
        line,
        fields,
        &attributes,
    );
}

#[cfg(test)]
#[path = "imagery_tests.rs"]
pub(super) mod tests;

#[cfg(test)]
#[path = "imagery_live_tests.rs"]
mod live_tests;
