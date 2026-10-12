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
//! not even across runs. Spend is charged to a per-run budget before each provider call and is
//! never refunded; at most two calls fly at once; identical requests share one flight. Every
//! failure is typed and means "keep showing what is on screen": nothing in the game waits for
//! an image, and a provider refusal is a failure, never retried with another wording or another
//! model. Generated images are never written under `assets/` and never committed; each is
//! recorded in a run-local manifest (`imagery.jsonl`) with its provenance.
mod artifact;
mod prompt;
mod references;

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
    ImageBackground, ImageEdit, ImageEditConfig, ImageEditError, ImageEditModel, ImageFormat,
    ImageModeration, ImageQuality, ImageUsage, NativeHttpRefusal, NativeImageEditProvider,
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
pub(super) use prompt::{
    AssembledPrompt, CharacterFacts, CharacterRole, LifeStage, PlaceFacts, cast_expression,
    scene_refresh,
};
#[allow(
    unused_imports,
    reason = "the investigation task (a follow-up) is the first caller"
)]
pub(super) use prompt::{PromptRefusal, VettedFact};
pub(super) use references::{ReferenceImage, ReferenceStore};

use super::startup_configuration::{
    ContentRating, ImageryConfiguration, ImageryOff, ImagerySettings,
};

const ROUTE: &str = "/gameplay/imagery";
const MANIFEST_NAME: &str = "imagery.jsonl";
/// Most recent registered keys kept; older ones can no longer be requested.
const MAXIMUM_REGISTERED: usize = 512;
const MAXIMUM_FAILED: usize = 256;
const MAXIMUM_IMAGE_BYTES: u64 = 6 * 1024 * 1024;
const MAXIMUM_IN_FLIGHT: usize = 2;
const MAXIMUM_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const CACHE_CONTROL: &str = "private, max-age=86400, immutable";

/// What a result is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageKind {
    /// The current backdrop re-framed and re-lit; no people.
    SceneRefresh,
    /// One character's cut-out with another expression; transparent.
    CastExpression,
    /// A close-up illustration of one object in this place.
    ArtifactCloseUp,
}

impl ImageKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::SceneRefresh => "scene_refresh",
            Self::CastExpression => "cast_expression",
            Self::ArtifactCloseUp => "artifact_close_up",
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
    /// A 4:3 close-up illustration.
    pub(super) const CLOSE_UP: Self = Self {
        width: 1024,
        height: 768,
        transparent: false,
    };

    fn format(self) -> ImageFormat {
        if self.transparent {
            ImageFormat::Png
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
    /// The per-run budget cannot pay for another call.
    BudgetExhausted,
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
            Self::BudgetExhausted => "budget_exhausted",
            Self::InvalidRequest => "invalid_request",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Refused => "refused",
            Self::Provider => "provider_failed",
            Self::BadOutput => "bad_output",
            Self::Interrupted => "interrupted",
        }
    }

    /// A failure that the same request would repeat, so it is remembered and never paid for twice.
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
    failed: HashMap<String, ImageFailure>,
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

    fn remember_failure(&mut self, key: &ImageKey, failure: ImageFailure) {
        if self.failed.len() >= MAXIMUM_FAILED {
            self.failed.clear();
        }
        self.failed.insert(key.0.clone(), failure);
    }
}

struct Budget {
    remaining: u32,
    exhausted: bool,
}

/// Counters for tests; they contain no content.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Stats {
    /// Provider calls charged.
    pub(super) calls: u32,
    pub(super) cache_hits: u32,
    /// Cost of the calls that reported usage, in millionths of a US dollar.
    pub(super) micro_dollars: u64,
    pub(super) budget_remaining: u32,
}

struct Engine {
    provider: NativeImageEditProvider,
    model: ImageEditModel,
    quality: ImageQuality,
    rating: ContentRating,
    deadline: Duration,
    cache_dir: PathBuf,
    manifest: Option<PathBuf>,
    budget: Mutex<Budget>,
    calls: AtomicU32,
    cache_hits: AtomicU32,
    micro_dollars: AtomicU64,
    permits: Arc<Semaphore>,
    in_flight: Mutex<HashMap<String, watch::Receiver<Option<Outcome>>>>,
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
}

impl Imagery {
    /// An image service that registers nothing and never renders.
    pub(super) fn disabled(reason: ImageryOff) -> Self {
        Self {
            inner: Arc::new(Inner {
                registry: Mutex::new(Registry::default()),
                mode: Mode::Off(reason),
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
        let provider = NativeImageEditProvider::new(
            ImageEditConfig::live(),
            configuration.credential.expose(),
            configuration.deadline,
        )
        .map_err(|_| io::Error::other("imagery provider configuration refused"))?;
        Self::new(configuration, provider, manifest_directory)
    }

    /// Uses `provider`; tests pass one pointed at a loopback fake.
    pub(super) fn new(
        configuration: ImageryConfiguration,
        provider: NativeImageEditProvider,
        manifest_directory: Option<&Path>,
    ) -> Result<Self, io::Error> {
        std::fs::create_dir_all(&configuration.cache_dir)?;
        let summary = format!(
            "IMAGERY on model={} budget={} images interval={}s rating={} quality={}",
            configuration.model.model_id(),
            configuration.budget,
            configuration.interval.as_secs(),
            configuration.rating.label(),
            match configuration.quality {
                ImageQuality::Low => "low",
                ImageQuality::Medium => "medium",
                ImageQuality::High => "high",
            },
        );
        let manifest = manifest_directory.map(|directory| directory.join(MANIFEST_NAME));
        let engine = Engine {
            provider,
            model: configuration.model,
            quality: configuration.quality,
            rating: configuration.rating,
            deadline: configuration.deadline,
            cache_dir: configuration.cache_dir,
            manifest,
            budget: Mutex::new(Budget {
                remaining: configuration.budget,
                exhausted: false,
            }),
            calls: AtomicU32::new(0),
            cache_hits: AtomicU32::new(0),
            micro_dollars: AtomicU64::new(0),
            permits: Arc::new(Semaphore::new(MAXIMUM_IN_FLIGHT)),
            in_flight: Mutex::new(HashMap::new()),
        };
        let mut registry = Registry::default();
        engine.register_from_manifest(&mut registry);
        Ok(Self {
            inner: Arc::new(Inner {
                registry: Mutex::new(registry),
                mode: Mode::On {
                    engine: Arc::new(engine),
                    summary,
                },
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
                budget_remaining: locked(&engine.budget).remaining,
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

    /// Renders `request`, or finds its cached result, and returns the key to load it by. The
    /// key is registered before any work so the route can serve it as soon as it exists.
    pub(super) async fn render(&self, request: ImageRequest) -> Result<ImageKey, ImageFailure> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return Err(ImageFailure::Off);
        };
        let key = self.key_for(&request);
        let format = request.output.format();
        {
            let mut registry = locked(&self.inner.registry);
            if let Some(failure) = registry.failed.get(key.as_str()) {
                return Err(*failure);
            }
            registry.register(&key, Registered { format });
        }
        let outcome = engine.image(&key, request).await;
        if let Err(failure) = outcome
            && failure.is_permanent()
        {
            locked(&self.inner.registry).remember_failure(&key, failure);
        }
        outcome.map(|()| key)
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
            Some(bytes) => Lookup::Image {
                bytes,
                format: registered.format,
            },
            None => Lookup::NotReady,
        }
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
        let extension = if format == ImageFormat::Png {
            "png"
        } else {
            "jpg"
        };
        format!("{}.{extension}", key.as_str())
    }

    async fn image(self: &Arc<Self>, key: &ImageKey, request: ImageRequest) -> Outcome {
        let format = request.output.format();
        if self.cache_has(key, format).await {
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
        if self.cache_has(key, format).await {
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
        self.charge()?;
        let generated = self
            .provider
            .edit(&edit, self.deadline)
            .await
            .map_err(failure_of)?;
        if generated.bytes.is_empty() || generated.bytes.len() as u64 > MAXIMUM_IMAGE_BYTES {
            return Err(ImageFailure::BadOutput);
        }
        let micro_dollars = usage_cost(generated.usage);
        self.micro_dollars
            .fetch_add(micro_dollars, Ordering::SeqCst);
        let bytes = Bytes::from(generated.bytes);
        let stored = self.store(key, format, bytes.clone()).await.is_ok();
        if stored {
            self.record(key, &request, bytes.len() as u64, micro_dollars)
                .await;
        }
        // A result that could not be cached is still served from memory for this key's first
        // requester only through the cache, so a failed write is a failed render.
        stored.then_some(()).ok_or(ImageFailure::Provider)
    }

    /// Spend is committed before the call and kept whatever the outcome: a call whose result is
    /// unknown may still have been billed.
    fn charge(&self) -> Result<(), ImageFailure> {
        let mut budget = locked(&self.budget);
        if budget.remaining == 0 {
            if !budget.exhausted {
                budget.exhausted = true;
                println!(
                    "IMAGERY budget exhausted after {} calls ({} cache hits, about ${:.2}); no \
                     further images will be generated",
                    self.calls.load(Ordering::SeqCst),
                    self.cache_hits.load(Ordering::SeqCst),
                    self.micro_dollars.load(Ordering::SeqCst) as f64 / 1_000_000.0,
                );
            }
            return Err(ImageFailure::BudgetExhausted);
        }
        budget.remaining -= 1;
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn cache_has(&self, key: &ImageKey, format: ImageFormat) -> bool {
        let path = self.cache_dir.join(Self::file_name(key, format));
        tokio::task::spawn_blocking(move || {
            std::fs::symlink_metadata(&path)
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
        })
        .await
        .unwrap_or(false)
    }

    async fn read_cached(&self, key: &str, format: ImageFormat) -> Option<Bytes> {
        let key = ImageKey::parse(key)?;
        let path = self.cache_dir.join(Self::file_name(&key, format));
        // A failed blocking task is treated as a cache miss.
        tokio::task::spawn_blocking(move || read_cache_file(&path))
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
    async fn record(&self, key: &ImageKey, request: &ImageRequest, bytes: u64, micro_dollars: u64) {
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
                ImageFormat::Png
            } else {
                ImageFormat::Jpeg
            };
            let present =
                std::fs::symlink_metadata(self.cache_dir.join(Self::file_name(&key, format)))
                    .is_ok_and(|metadata| metadata.is_file());
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

fn request_key(request: &ImageRequest, model: &str, quality: &str, rating: &str) -> ImageKey {
    let mut hasher = Sha256::new();
    let mut part = |text: &str| {
        hasher.update(text.as_bytes());
        hasher.update(b"\n");
    };
    part("df-imagery/1");
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
    match imagery.image(&key).await {
        Lookup::Image { bytes, format } => {
            let etag = format!("\"{key}\"");
            let content_type = if format == ImageFormat::Png {
                "image/png"
            } else {
                "image/jpeg"
            };
            if headers
                .get(header::IF_NONE_MATCH)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == etag)
            {
                return (
                    StatusCode::NOT_MODIFIED,
                    [
                        (header::ETAG, etag),
                        (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
                    ],
                )
                    .into_response();
            }
            (
                [
                    (header::CONTENT_TYPE, content_type.to_owned()),
                    (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
                    (header::ETAG, etag),
                ],
                bytes,
            )
                .into_response()
        }
        Lookup::Unregistered => StatusCode::NOT_FOUND.into_response(),
        Lookup::NotReady => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[cfg(test)]
#[path = "imagery_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "imagery_live_tests.rs"]
mod live_tests;
