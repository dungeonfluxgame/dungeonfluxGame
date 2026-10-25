//! PLAY-V10: the home screen's looping video, made just in time from its still.
//!
//! When the lobby's home painting is first shown and no loop is cached for it, one image-to-video
//! request goes to fal (`fal-ai/kling-video/o1/image-to-video`) with the still as both the start
//! and the end frame, five seconds, and an ambient-motion prompt. The result is validated by the
//! provider adapter, written to an on-disk cache keyed by the SHA-256 of the still, the model, the
//! prompt, the duration and the recipe, and served at `/gameplay/home-video/<key>` with byte
//! ranges. A cached loop is served with no provider call. Nothing waits on the video: until it is
//! ready the display shows the still, and every failure leaves the still in place.
//!
//! Both kinds of clip, the home loop and the scene loop, are one resource kind of the generation
//! hub (`ResourceKind::VideoClip`) with two specs ([`clip_spec::HomeLoopSpec`] and
//! [`clip_spec::SceneLoopSpec`]). The spec's key is the hash of the still, the prompt, the model,
//! the duration, the size and the recipe version, and it is also the clip's file name, so a change
//! to any of them is a new clip and an old loop is never served for it. The hub owns the state,
//! the failure memory and the attempt limit; this module keeps only the index from a painting or a
//! plate to its clip, the disk store and the provider call.
//!
//! Spend is one clip per provider call, recorded on the hub's ledger when the call is about to be
//! sent and kept whatever the outcome, for a home loop and a scene loop alike. There is no cap:
//! the ledger counts and the audit lines say what was spent, and nothing is refused. The live
//! price is read before the first submit of a run and logged.
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError},
    time::{Duration, Instant},
};
use tokio::sync::watch;

use axum::{
    Router,
    body::Body,
    extract::Path as RoutePath,
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use df_model::checkpoint::{AssetKind, AssetReference, Basis, ContentDigest, ExecutionMode, JobId};
use df_observe::OperationContext;
use df_provider_api::{
    RequestBasis, RequestBinding, RequestIdentity, RequestLimits, RequestModality,
    RequestOwnerState, RequestUsage, VideoEventKind, VideoSemantics,
};
use df_providers::{
    EncodedMediaFacts, FalCdnPin, FalMediaConfig, FalMediaError, FalMediaFamily,
    FalQueueObservation, FalReferenceFrame, FalVideoBasis, FalVideoProfile, FalVideoRequest,
    NativeFalMediaProvider, NativeHttpLimits, ProviderLimits, registered_fal_media_provider,
};
use df_types::{
    OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage, UsageUnit,
};
use sha2::{Digest, Sha256};

use super::cost;
use super::home_theme;
use super::living_scenes::LivingScenes;
use super::resource_adapters::{Flights, Kit, KitSlot, Produced, write_atomic};
use super::resources::{
    Generated, Limits, ResourceFailure, ResourceKey, ResourceKind, ResourceSpec, ResourceStatus,
    Unit,
};
use super::rpc;
use crate::BUILD_ID;

/// The endpoint the server requests. Only endpoints the adapter has a body and checks for are
/// accepted as the setting.
pub(crate) const MODEL: &str = df_providers::KLING_O1_IMAGE_TO_VIDEO;
/// The look of a landing ambience loop: a documentary photograph of a place, with no cast. The
/// shared `FILM_LOOK` is written for people (costumes, skin pores) and invites them into an empty
/// room, so a loop of a place does not use it.
const AMBIENCE_LOOK: &str = "A quiet establishing shot from a live-action fantasy television drama set in a lived-in Dungeons & Dragons world. Documentary photograph of a place, shot on a real camera with a prime lens in natural practical light, believable textures of weathered wood, stone, water and cloth, natural depth of field, fine film grain. No people and no creatures anywhere in the frame.";
/// The gentle-ambience wording of the home loop, in one place: a locked-off camera and only small
/// ambient motion. The still is both the first and the last frame, which is what makes it loop.
const HOME_MOTION: &str = "Locked-off camera: the frame does not move, zoom or drift. Only small ambient motion: lamplight and candle flicker, drifting smoke or dust, cloth and banners stirring, water shimmer. Nothing enters or leaves the frame, and no new objects, text or logos appear. The last frame matches the first so the motion loops seamlessly, the composition exactly as in the reference image.";
const DURATION_SECONDS: u32 = 5;
/// Changing how a clip is requested or cached changes this, so old loops are not reused for it.
const RECIPE: &str = "df-home-video/2";
const SCENE_RECIPE: &str = "df-scene-video/2";
const KILL_RECIPE: &str = "df-kill-shot/1";
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
/// The still is sent inline as the first and the last frame, so its size is the request's size twice;
/// 85 keeps a 1080p frame near a quarter of a megabyte. Uploading it once to fal storage and sending
/// the URL twice is the follow-up, and needs a pinned storage host (see the report).
const STILL_QUALITY: u8 = 85;
const DEFAULT_CACHE: &str = "artifacts/tmp/play/home-video-cache";
/// The fal media hosts the download is pinned to. The adapter refuses any other host.
const CDN_HOSTS: [&str; 2] = ["v3.fal.media", "v3b.fal.media"];
const MAXIMUM_CLIP_BYTES: usize = 16 * 1024 * 1024;
const MINIMUM_CLIP_MILLISECONDS: u128 = 4_500;
const MAXIMUM_CLIP_MILLISECONDS: u128 = 5_500;
const POLL_INTERVAL: Duration = Duration::from_secs(4);
const DEFAULT_DEADLINE: Duration = Duration::from_secs(15 * 60);
const REQUEST_DEADLINE: Duration = Duration::from_secs(170);
const ROUTE: &str = "/gameplay/home-video";
/// A paid clip is made once per key. A failure stays failed until the dev tools invalidate the
/// key: a clip that failed in an unknown way may already have been billed, so nothing retries it.
const CLIP_ATTEMPTS: u8 = 1;
/// What the hub's timeout allows beyond the engine's own deadline: the still and the cache write,
/// so the engine's labelled `Timeout` fires first.
const HUB_SLACK: Duration = Duration::from_secs(60);

#[path = "scene_video_prompt.rs"]
mod scene_video_prompt;
pub(super) use scene_video_prompt::model_staging;
#[path = "video_clip_spec.rs"]
pub(super) mod clip_spec;
#[path = "kill_shot.rs"]
pub(in crate::gameplay) mod kill_shot;
#[cfg(test)]
#[path = "kill_shot_tests.rs"]
mod kill_shot_tests;
use clip_spec::{ClipInput, ClipRecipe, HomeLoopSpec, KillShotSpec, SceneLoopSpec};

fn home_video_prompt() -> String {
    format!("{AMBIENCE_LOOK} {HOME_MOTION}")
}

/// The raw `DF_GAMEPLAY_HOME_VIDEO*`, `DF_GAMEPLAY_SCENE_VIDEO` and `FAL_KEY` inputs.
pub(crate) struct HomeVideoInputs {
    pub(crate) mode: Result<String, std::env::VarError>,
    pub(crate) scene_mode: Result<String, std::env::VarError>,
    pub(crate) model: Result<String, std::env::VarError>,
    pub(crate) cache: Result<String, std::env::VarError>,
    pub(crate) credential: Result<String, std::env::VarError>,
}

impl HomeVideoInputs {
    pub(crate) fn from_environment() -> Self {
        Self {
            mode: crate::gameplay::media_policy::gates()
                .input(crate::gameplay::media_policy::Capability::HomeVideo),
            scene_mode: crate::gameplay::media_policy::gates()
                .input(crate::gameplay::media_policy::Capability::SceneVideo),
            model: std::env::var("DF_GAMEPLAY_HOME_VIDEO_MODEL"),
            cache: std::env::var("DF_GAMEPLAY_HOME_VIDEO_CACHE"),
            credential: std::env::var("FAL_KEY"),
        }
    }
}

/// The fal credential. It never appears in `Debug` output.
pub(crate) struct FalCredential(String);

impl std::fmt::Debug for FalCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FalCredential([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HomeVideoOff {
    Qualification,
    Disabled,
    NoCredential,
    /// None of the pinned fal media hosts resolved at startup, so no clip could be downloaded.
    NoHost,
}

impl HomeVideoOff {
    fn reason(self) -> &'static str {
        match self {
            Self::Qualification => "qualification run",
            Self::Disabled => "DF_GAMEPLAY_HOME_VIDEO=off",
            Self::NoCredential => "no FAL_KEY",
            Self::NoHost => "fal media hosts do not resolve",
        }
    }
}

#[derive(Debug)]
pub(crate) struct HomeVideoConfiguration {
    pub(crate) model: &'static str,
}

#[derive(Debug)]
struct SceneVideoConfiguration {
    enabled: bool,
    off_reason: &'static str,
}

#[derive(Debug)]
pub(crate) struct HomeVideoSettings {
    home: Result<HomeVideoConfiguration, HomeVideoOff>,
    scene: SceneVideoConfiguration,
    credential: Option<FalCredential>,
    cache_dir: PathBuf,
}

/// A setting that is present and invalid is refused by name, never echoed.
fn refused(name: &str) -> io::Error {
    io::Error::other(format!("{name} invalid"))
}

/// Home and scene/kill video generation default off, even with a credential.
/// Explicit `on` opts in; enabling scene clips without a key reports unavailable without submitting.
pub(crate) fn home_video_settings(
    inputs: HomeVideoInputs,
    current: &Path,
    qualification: bool,
) -> Result<HomeVideoSettings, io::Error> {
    if qualification {
        return Ok(HomeVideoSettings {
            home: Err(HomeVideoOff::Qualification),
            scene: SceneVideoConfiguration {
                enabled: false,
                off_reason: "qualification run",
            },
            credential: None,
            cache_dir: current.join(DEFAULT_CACHE),
        });
    }
    let home_requested = super::media_policy::enabled(inputs.mode, "DF_GAMEPLAY_HOME_VIDEO")?;
    let scene_enabled = super::media_policy::enabled(inputs.scene_mode, "DF_GAMEPLAY_SCENE_VIDEO")?;
    let credential = match inputs.credential {
        Ok(value) if !value.is_empty() => Some(value),
        Err(std::env::VarError::NotPresent) | Ok(_) => None,
        Err(_) if home_requested || scene_enabled => return Err(refused("FAL_KEY")),
        Err(_) => None,
    };
    let scene_off_reason = if scene_enabled {
        "no FAL_KEY"
    } else {
        "DF_GAMEPLAY_SCENE_VIDEO=off"
    };
    let home_enabled = home_requested && credential.is_some();
    if (home_enabled || scene_enabled)
        && credential.as_ref().is_some_and(|value| {
            value.len() > 8192 || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        })
    {
        return Err(refused("FAL_KEY"));
    }
    let home = if !home_requested {
        Err(HomeVideoOff::Disabled)
    } else if credential.is_none() {
        Err(HomeVideoOff::NoCredential)
    } else {
        let model = match inputs.model {
            Ok(value) if value.is_empty() => MODEL,
            Ok(value) if value == MODEL => MODEL,
            Ok(_) => return Err(refused("DF_GAMEPLAY_HOME_VIDEO_MODEL")),
            Err(std::env::VarError::NotPresent) => MODEL,
            Err(_) => return Err(refused("DF_GAMEPLAY_HOME_VIDEO_MODEL")),
        };
        Ok(HomeVideoConfiguration { model })
    };
    let cache_dir = if home_enabled || scene_enabled {
        match inputs.cache {
            Ok(value) if !value.is_empty() => PathBuf::from(value),
            Ok(_) | Err(std::env::VarError::NotPresent) => current.join(DEFAULT_CACHE),
            Err(_) => return Err(refused("DF_GAMEPLAY_HOME_VIDEO_CACHE")),
        }
    } else {
        current.join(DEFAULT_CACHE)
    };
    Ok(HomeVideoSettings {
        home,
        scene: SceneVideoConfiguration {
            enabled: scene_enabled,
            off_reason: scene_off_reason,
        },
        credential: credential.map(FalCredential),
        cache_dir,
    })
}

/// The one startup line for the feature, printed with the other services.
pub(crate) fn startup_line(settings: &HomeVideoSettings) -> String {
    let home = match &settings.home {
        Ok(configuration) => format!(
            "HOME VIDEO on model={} duration={}s",
            configuration.model, DURATION_SECONDS
        ),
        Err(reason) => format!("HOME VIDEO off ({})", reason.reason()),
    };
    let scene = if settings.scene.enabled {
        let unavailable = settings
            .credential
            .is_none()
            .then_some(" (no FAL_KEY; request logging only)")
            .unwrap_or_default();
        format!("SCENE VIDEO on model={MODEL} duration={DURATION_SECONDS}s{unavailable}")
    } else {
        format!("SCENE VIDEO off ({})", settings.scene.off_reason)
    };
    format!("{home}; {scene}")
}

/// What the view says about a lobby's loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LoopView {
    /// No loop is requested or the feature is off: the still stays.
    None,
    Pending,
    Ready {
        path: String,
    },
    Failed,
}

impl LoopView {
    /// The state the view carries; a missing feature or a missing loop is `NONE`.
    pub(crate) fn proto_state(&self) -> rpc::HomeVideoState {
        match self {
            Self::None => rpc::HomeVideoState::None,
            Self::Pending => rpc::HomeVideoState::Pending,
            Self::Ready { .. } => rpc::HomeVideoState::Ready,
            Self::Failed => rpc::HomeVideoState::Failed,
        }
    }

    /// The served path, empty unless the loop is ready.
    pub(crate) fn path(&self) -> String {
        match self {
            Self::Ready { path } => path.clone(),
            _ => String::new(),
        }
    }
}

/// The scene plate's optional ambient loop. The still remains the presentation fallback in every
/// state except `Ready`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SceneClipView {
    None,
    Pending,
    Ready { path: String },
    Failed,
}

impl SceneClipView {
    pub(crate) fn proto_state(&self) -> rpc::HomeVideoState {
        match self {
            Self::None => rpc::HomeVideoState::None,
            Self::Pending => rpc::HomeVideoState::Pending,
            Self::Ready { .. } => rpc::HomeVideoState::Ready,
            Self::Failed => rpc::HomeVideoState::Failed,
        }
    }

    pub(crate) fn path(&self) -> String {
        match self {
            Self::Ready { path } => path.clone(),
            _ => String::new(),
        }
    }
}

/// Why a loop was not made. The label is the diagnostic; the still stays on screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    Still(&'static str),
    Provider(String),
    Timeout,
    Clip(&'static str),
}

impl Failure {
    fn label(&self) -> String {
        match self {
            Self::Still(reason) => format!("still: {reason}"),
            Self::Provider(reason) => format!("provider: {reason}"),
            Self::Timeout => "timeout".to_owned(),
            Self::Clip(reason) => format!("clip: {reason}"),
        }
    }
}

/// Which loop a clip is, and so which spec, recipe and log lines it uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Variant {
    Home,
    Scene,
    KillShot,
}

/// The clip a painting or a plate currently points at: the hub's key for it and its file name in
/// the store (the whole digest of the same spec, so the two are one identity).
#[derive(Clone, Debug)]
struct Lane {
    key: ResourceKey,
    file: String,
}

/// One kill shot's slot: its request, its clip lane and its still's served path.
#[derive(Clone, Debug)]
struct KillSlot {
    request: kill_shot::KillShotRequest,
    lane: Lane,
    still_path: String,
    started: Instant,
}

/// A painting's slot: its still is being prepared (the clip's key is not known yet), or its clip
/// is known to the hub.
#[derive(Clone, Debug)]
enum HomeSlot {
    Preparing,
    Keyed(Lane),
}

/// The still a clip is made from, before the provider sees it.
enum StillBytes {
    /// Decoded and re-encoded to the provider's frame.
    Prepared(Vec<u8>),
    /// A scene plate as the imagery service made it; prepared only if the clip must be made.
    Plate(Vec<u8>),
}

/// Everything the one generation of a clip needs, owned so the task can outlive its caller.
struct ClipJob {
    variant: Variant,
    /// The hub's subject for the clip (`video:home:...`), for the `RESOURCE` lines.
    resource_subject: String,
    file: String,
    input: ClipInput,
    /// Why the clip cannot be made, when `input` is [`ClipInput::Unavailable`].
    unavailable: Option<Failure>,
    still: StillBytes,
    /// Why the clip was asked for, for the log lines.
    reason: String,
    /// Killer and victim names for a kill shot's `KILL_SHOT` audit lines.
    kill_names: Option<(String, String)>,
}

/// Where the clip requests go: the live fal endpoints, or a loopback fixture in tests.
#[derive(Clone, Debug)]
pub(crate) enum Endpoints {
    Live {
        pins: Vec<FalCdnPin>,
    },
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the loopback fixture is constructed by the native tests only"
        )
    )]
    Fixture {
        api: SocketAddr,
        media: SocketAddr,
    },
}

pub(crate) struct Engine {
    root: PathBuf,
    cache_dir: PathBuf,
    credential: Option<String>,
    endpoints: Endpoints,
    deadline: Duration,
    home_enabled: bool,
    scene_enabled: bool,
    kit: KitSlot,
    /// Identical concurrent asks for one clip share one run.
    flights: Flights<Result<Produced<()>, ResourceFailure>>,
    /// The home painting to its clip. The hub holds the clip's state; this is only the index.
    home: Mutex<BTreeMap<String, HomeSlot>>,
    /// The scene plate key to the clip of its latest request.
    scenes: Mutex<BTreeMap<String, Lane>>,
    /// Kill shots by their blow identity, plus the latest one's id.
    kill_shots: Mutex<BTreeMap<String, KillSlot>>,
    latest_kill: Mutex<Option<String>>,
    kill_shown_audited: Mutex<BTreeSet<(String, u64)>>,
    /// Clips whose run has been started and has not ended. This is not a failure memory: it only
    /// tells a view that a run is on the way before the hub has heard of it.
    launching: Mutex<BTreeSet<ResourceKey>>,
    living: Option<LivingScenes>,
}

/// The guarded data are plain collections that stay consistent when a holder panics.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HomeVideoEngine")
            .field("cache_dir", &self.cache_dir)
            .finish_non_exhaustive()
    }
}

static ENGINE: Mutex<Option<Arc<Engine>>> = Mutex::new(None);

pub(super) fn retire() -> Result<(), io::Error> {
    ENGINE
        .lock()
        .map_err(|_| io::Error::other("home video lock poisoned"))?
        .take();
    Ok(())
}

/// Counts the loops that have landed. A display stream subscribes to it when the living scenes
/// have no wake of their own, so a landed loop reaches the display whatever else is running.
static WAKE: OnceLock<watch::Sender<u64>> = OnceLock::new();

/// A subscription to the loops' wake, for a display stream.
pub(crate) fn wake_subscribe() -> watch::Receiver<u64> {
    WAKE.get_or_init(|| watch::channel(0).0).subscribe()
}

/// Starts the feature for this process: resolves the fal media hosts it is pinned to, and records
/// the engine that the view and the route use. A host that does not resolve is left out; if none
/// resolves the feature is off, with the reason in the startup line.
pub(crate) fn install(
    settings: HomeVideoSettings,
    asset_root: &Path,
    living: LivingScenes,
) -> Result<String, io::Error> {
    let line = startup_line(&settings);
    let home_enabled = settings.home.is_ok();
    let scene_enabled = settings.scene.enabled;
    if !home_enabled && !scene_enabled {
        return Ok(line);
    }
    let credential = settings.credential.map(|credential| credential.0);
    let pins = if credential.is_some() {
        resolve_pins()
    } else {
        Vec::new()
    };
    let home_configuration = settings.home.ok();
    let home_enabled = home_enabled && !pins.is_empty();
    let scene_enabled = settings.scene.enabled && (credential.is_none() || !pins.is_empty());
    let effective_line = if credential.is_some() && pins.is_empty() {
        let home_line = if home_configuration.is_some() {
            format!("HOME VIDEO off ({})", HomeVideoOff::NoHost.reason())
        } else {
            line.split_once("; ")
                .map(|(home, _)| home.to_owned())
                .unwrap_or_else(|| "HOME VIDEO off".to_owned())
        };
        let scene_line = if settings.scene.enabled {
            format!("SCENE VIDEO off ({})", HomeVideoOff::NoHost.reason())
        } else {
            line.split_once("; ")
                .map(|(_, scene)| scene.to_owned())
                .unwrap_or_else(|| "SCENE VIDEO off".to_owned())
        };
        format!("{home_line}; {scene_line}")
    } else {
        line
    };
    if !home_enabled && !scene_enabled {
        return Ok(effective_line);
    }
    let engine = Engine::new(EngineSetup {
        root: asset_root.to_path_buf(),
        cache_dir: settings.cache_dir,
        credential,
        endpoints: Endpoints::Live { pins },
        deadline: DEFAULT_DEADLINE,
        home_enabled,
        scene_enabled,
        living: Some(living),
    });
    *ENGINE
        .lock()
        .map_err(|_| io::Error::other("home video lock poisoned"))? = Some(Arc::new(engine));
    Ok(effective_line)
}

/// Puts the installed engine on the server's generation hub, so its clips, failures and spend are
/// listed and invalidated with every other resource. A server with no clip engine has nothing to
/// attach.
pub(crate) fn attach_kit(kit: &Kit) -> Result<(), io::Error> {
    let engine = ENGINE
        .lock()
        .map_err(|_| io::Error::other("home video lock poisoned"))?
        .clone();
    if let Some(engine) = engine {
        engine.kit.attach(kit);
        engine.configure();
    }
    Ok(())
}

/// Resolves each pinned fal media host once, at startup. Only public addresses are kept.
pub(super) fn resolve_pins() -> Vec<FalCdnPin> {
    CDN_HOSTS
        .iter()
        .filter_map(|host| {
            let addresses: Vec<std::net::IpAddr> = (*host, 443)
                .to_socket_addrs()
                .ok()?
                .map(|address| address.ip())
                .collect();
            FalCdnPin::new(host, &addresses).ok()
        })
        .collect()
}

/// The loop the view carries for `painting`. The first call for a painting starts its one flight;
/// later calls, from any number of displays, see the same state.
pub(crate) fn observe(painting: &str) -> LoopView {
    match ENGINE.lock().ok().and_then(|engine| engine.clone()) {
        Some(engine) => engine.observe(painting),
        None => LoopView::None,
    }
}

/// Starts one scene clip after the owning plate is current on the display. The caller suppresses
/// combat and rolls; repeats for the same plate key return without another provider request.
pub(crate) fn request_scene_clip(
    plate_key: &str,
    still: Vec<u8>,
    description: &str,
    reason: &str,
    blocked_by_combat_or_roll: bool,
) {
    if blocked_by_combat_or_roll {
        return;
    }
    let Some(engine) = ENGINE.lock().ok().and_then(|engine| engine.clone()) else {
        return;
    };
    engine.request_scene_clip(plate_key, still, description, reason);
}

/// A plate's staging without player dialogue, for the motion prompt.
pub(in crate::gameplay) fn scene_description(spec: &super::shot::ShotSpec) -> String {
    scene_video_prompt::staging(spec)
}

/// The clip state for a plate image key, for the current display view projection.
pub(crate) fn scene_clip_view(plate_key: &str) -> SceneClipView {
    ENGINE
        .lock()
        .ok()
        .and_then(|engine| engine.clone())
        .map(|engine| engine.scene_clip_view(plate_key))
        .unwrap_or(SceneClipView::None)
}

/// Requests one kill shot for a committed killing blow. Called only after the
/// blow is committed and its still is ready; the first call for a blow starts
/// its one clip flight and a replayed call reuses it. A still that failed its
/// photo check never reaches here, so no clip is made from it.
pub(crate) fn request_kill_shot(
    request: kill_shot::KillShotRequest,
    still: Vec<u8>,
    still_path: String,
) {
    let Some(engine) = ENGINE.lock().ok().and_then(|engine| engine.clone()) else {
        return;
    };
    engine.request_kill_shot(request, still, still_path);
}

/// Dispatches still work to the existing living-scenes owner, after commit only.
pub(in crate::gameplay) fn request_kill_shot_still(
    request: kill_shot::KillShotRequest,
    checkpoint: &df_model::checkpoint::Checkpoint,
    outcome: &rpc::CombatOutcome,
) {
    let living = locked(&ENGINE)
        .as_ref()
        .filter(|engine| engine.scene_enabled)
        .and_then(|engine| engine.living.clone());
    match living {
        Some(living) => living.request_kill_shot(request, checkpoint, outcome),
        None => audit_kill("failed", &request.killer, &request.victim, 0, "video_off"),
    }
}

fn audit_kill(status: &str, killer: &str, victim: &str, elapsed_ms: u128, reason: &str) {
    super::actor::record_audit_line(
        "gameplay.kill_shot",
        kill_shot::audit_line(status, killer, victim, elapsed_ms, reason),
    );
}

/// The still a lobby view will show: the scene's own asset, which is what the display
/// draws, with the view's asset as its fallback. The home loop is paired with this still
/// and no other (HOME-01).
fn shown_still(display: &rpc::DisplayGameplayView) -> String {
    display
        .scene
        .as_ref()
        .map(|scene| scene.scene_asset.clone())
        .filter(|asset| !asset.is_empty())
        .unwrap_or_else(|| display.scene_asset.clone())
}

/// Sets the home loop on a view: the state and the served path, for the display's lobby only. It
/// runs on every view the service hands out, so a loop that lands is seen by the next view, and
/// the journey projection cached by revision does not hold a stale state.
///
/// HOME-01: a loop belongs to the one still it was made from. The view carries a loop path
/// only for the still it actually shows, and only once that still's own loop is ready;
/// while the shown still has no ready loop of its own — including right after the still
/// changes to the adventure's own opening picture — the view carries no path, the still
/// shows alone, and `observe` has already asked for that still's loop through the home
/// video path. A loop made from another picture is never sent over this still.
pub(crate) fn apply(message: &mut rpc::ViewMessage) {
    let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
        return;
    };
    let lobby = display
        .journey
        .as_ref()
        .is_some_and(|journey| journey.phase == rpc::JourneyPhase::Room as i32);
    let painting = if lobby {
        shown_still(display)
    } else {
        String::new()
    };
    let Some(journey) = display.journey.as_mut() else {
        return;
    };
    let loop_view = if lobby && !painting.is_empty() {
        observe(&painting)
    } else {
        LoopView::None
    };
    journey.home_video_state = loop_view.proto_state() as i32;
    journey.home_video_path = loop_view.path();
    // K1: the latest kill shot rides on the display view while pending/ready;
    // a pending clip carries no path and the client shows nothing for it.
    if let Some(engine) = ENGINE.lock().ok().and_then(|engine| engine.clone()) {
        journey.kill_shot = engine.kill_shot_for_view();
    }
}

/// Serves the cached loops as `GET /gameplay/home-video/<key>`.
pub(crate) fn router() -> Router {
    Router::new().route("/gameplay/home-video/{key}", get(serve_clip))
}

/// What an engine is made of, apart from its runtime state.
struct EngineSetup {
    root: PathBuf,
    cache_dir: PathBuf,
    credential: Option<String>,
    endpoints: Endpoints,
    deadline: Duration,
    home_enabled: bool,
    scene_enabled: bool,
    living: Option<LivingScenes>,
}

/// What a view is told about a clip, read from the hub.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaneState {
    Ready,
    Pending,
    Failed,
    /// Nothing is running and the hub holds no usable clip: never asked, invalidated, or its
    /// stored file is gone. The next ask starts a run.
    Forgotten,
}

/// What a finished run produced.
struct ClipDone {
    cached: bool,
    bytes: u64,
}

/// A provider call that was made: its price per second of video in micro-dollars, from the cost
/// book (`None` when the price file has no rate), and how long it took.
struct PaidCall {
    micro_per_second: Option<u64>,
    elapsed: Duration,
}

/// The way clips of one variant are requested and cached, for the spec's key.
fn clip_recipe(variant: Variant) -> ClipRecipe {
    ClipRecipe {
        model: MODEL.to_owned(),
        duration_seconds: DURATION_SECONDS,
        width: WIDTH,
        height: HEIGHT,
        recipe: match variant {
            Variant::Home => RECIPE,
            Variant::Scene => SCENE_RECIPE,
            Variant::KillShot => KILL_RECIPE,
        }
        .to_owned(),
        finish: LoopFinish::CURRENT.label(),
    }
}

/// What is done to the provider's clip between download and store: the seam for a post-processing
/// step. Today the clip is stored as delivered, since the still is both its first and last frame.
/// If the provider's loop turns out not to close cleanly, a tail crossfade (blend the last frames
/// into the first) or a ping-pong (play it forward then reversed) is added as a variant here and
/// made `CURRENT`; its label is in the spec's key, so every clip made without it is superseded and
/// none is served as if it had it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoopFinish {
    AsDelivered,
}

impl LoopFinish {
    const CURRENT: Self = Self::AsDelivered;

    const fn label(self) -> &'static str {
        match self {
            Self::AsDelivered => "as_delivered",
        }
    }

    /// The bytes to store for the provider's `clip`. A step that cannot finish a clip fails it
    /// rather than storing a half-finished loop.
    fn apply(self, clip: Vec<u8>) -> Result<Vec<u8>, Failure> {
        match self {
            Self::AsDelivered => Ok(clip),
        }
    }
}

impl Engine {
    fn new(setup: EngineSetup) -> Self {
        let engine = Self {
            root: setup.root,
            cache_dir: setup.cache_dir,
            credential: setup.credential,
            endpoints: setup.endpoints,
            deadline: setup.deadline,
            home_enabled: setup.home_enabled,
            scene_enabled: setup.scene_enabled,
            kit: KitSlot::private(),
            flights: Flights::default(),
            home: Mutex::new(BTreeMap::new()),
            scenes: Mutex::new(BTreeMap::new()),
            kill_shots: Mutex::new(BTreeMap::new()),
            latest_kill: Mutex::new(None),
            kill_shown_audited: Mutex::new(BTreeSet::new()),
            launching: Mutex::new(BTreeSet::new()),
            living: setup.living,
        };
        engine.configure();
        engine
    }

    /// Tunes the clip kind on the engine's current kit: how long a run may take, how a ready
    /// entry's file is checked and how it is erased.
    fn configure(&self) {
        let kit = self.kit.get();
        let checked = self.cache_dir.clone();
        let erased = self.cache_dir.clone();
        kit.tune(
            ResourceKind::VideoClip,
            Limits {
                timeout: self.deadline + HUB_SLACK,
                attempts: CLIP_ATTEMPTS,
            },
            Arc::new(move |generated: &Generated| {
                cached_clip(&checked, &generated.handle).is_some()
            }),
            Arc::new(move |generated: &Generated| {
                // Best effort: a clip that is already gone is what was asked for.
                let _removed = std::fs::remove_file(clip_path(&erased, &generated.handle));
            }),
        );
    }

    /// What the hub says about a clip, with a run that has been started but not yet reached the
    /// hub counted as pending.
    fn lane_state(&self, lane: &Lane) -> LaneState {
        if locked(&self.launching).contains(&lane.key) {
            return LaneState::Pending;
        }
        match self.kit.get().hub().status(&lane.key) {
            ResourceStatus::Ready(_) => LaneState::Ready,
            ResourceStatus::Failed(..) => LaneState::Failed,
            ResourceStatus::Pending(_) | ResourceStatus::Fallback(_) => LaneState::Forgotten,
        }
    }

    /// Marks a run of `key` as started; `false` when one already is.
    fn claim_launch(&self, key: &ResourceKey) -> bool {
        locked(&self.launching).insert(key.clone())
    }

    fn observe(self: &Arc<Self>, painting: &str) -> LoopView {
        if !self.home_enabled {
            return LoopView::None;
        }
        let lane = {
            let mut home = locked(&self.home);
            match home.get(painting) {
                None => {
                    home.insert(painting.to_owned(), HomeSlot::Preparing);
                    drop(home);
                    self.start_home(painting);
                    return LoopView::Pending;
                }
                Some(HomeSlot::Preparing) => return LoopView::Pending,
                Some(HomeSlot::Keyed(lane)) => lane.clone(),
            }
        };
        match self.lane_state(&lane) {
            LaneState::Ready => LoopView::Ready {
                path: format!("{ROUTE}/{}", lane.file),
            },
            LaneState::Pending => LoopView::Pending,
            LaneState::Failed => LoopView::Failed,
            LaneState::Forgotten => {
                locked(&self.home).insert(painting.to_owned(), HomeSlot::Preparing);
                self.start_home(painting);
                LoopView::Pending
            }
        }
    }

    fn start_home(self: &Arc<Self>, painting: &str) {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let engine = Arc::clone(self);
            let painting = painting.to_owned();
            handle.spawn(async move { engine.fly_home(painting).await });
        }
    }

    fn request_scene_clip(
        self: &Arc<Self>,
        plate_key: &str,
        still: Vec<u8>,
        description: &str,
        reason: &str,
    ) {
        if !self.scene_enabled || !valid_plate_key(plate_key) {
            return;
        }
        let (input, unavailable) = match scene_video_prompt::ambient(description) {
            Ok(prompt) => (ClipInput::ready(&still, &prompt), None),
            Err(refusal) => {
                let label = match refusal {
                    scene_video_prompt::Refusal::EmptyDescription => "scene description missing",
                    scene_video_prompt::Refusal::InvalidDescription => "scene description invalid",
                    scene_video_prompt::Refusal::PromptTooLong => "prompt too long",
                };
                (
                    ClipInput::refused(Some(&still), label),
                    Some(Failure::Clip(label)),
                )
            }
        };
        let spec = SceneLoopSpec::new(plate_key, input.clone(), clip_recipe(Variant::Scene));
        let lane = Lane {
            key: ResourceKey::of(&spec),
            file: clip_spec::file_key(&spec),
        };
        locked(&self.scenes).insert(plate_key.to_owned(), lane.clone());
        // A clip that is running, ready or remembered as failed is not asked for again.
        if self.lane_state(&lane) != LaneState::Forgotten || !self.claim_launch(&lane.key) {
            return;
        }
        let job = ClipJob {
            variant: Variant::Scene,
            resource_subject: spec.subject(),
            file: lane.file.clone(),
            input,
            unavailable,
            still: StillBytes::Plate(still),
            reason: reason.to_owned(),
            kill_names: None,
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            locked(&self.launching).remove(&lane.key);
            log_failed(
                &job,
                &Failure::Clip("runtime unavailable"),
                None,
                Duration::ZERO,
            );
            return;
        };
        let engine = Arc::clone(self);
        handle.spawn(async move { engine.fly_clip(spec, lane.key, job).await });
    }

    fn request_kill_shot(
        self: &Arc<Self>,
        request: kill_shot::KillShotRequest,
        still: Vec<u8>,
        still_path: String,
    ) {
        if !self.scene_enabled {
            audit_kill("failed", &request.killer, &request.victim, 0, "video_off");
            return;
        }
        let prompt = kill_shot::motion_prompt(&request);
        let input = ClipInput::ready(&still, &prompt);
        let spec = KillShotSpec::new(
            &request.id,
            &request.killer,
            &request.victim,
            input.clone(),
            clip_recipe(Variant::KillShot),
        );
        let lane = Lane {
            key: ResourceKey::of(&spec),
            file: clip_spec::file_key(&spec),
        };
        let slot = KillSlot {
            request: request.clone(),
            lane: lane.clone(),
            still_path,
            started: Instant::now(),
        };
        locked(&self.kill_shots).insert(request.id.clone(), slot);
        *locked(&self.latest_kill) = Some(request.id.clone());
        if self.lane_state(&lane) != LaneState::Forgotten || !self.claim_launch(&lane.key) {
            return;
        }
        audit_kill(
            "still_ready",
            &request.killer,
            &request.victim,
            0,
            "photo_check_passed",
        );
        audit_kill(
            "requested",
            &request.killer,
            &request.victim,
            0,
            "killing_blow",
        );
        let job = ClipJob {
            variant: Variant::KillShot,
            resource_subject: spec.subject(),
            file: lane.file.clone(),
            input,
            unavailable: None,
            still: StillBytes::Plate(still),
            reason: "killing-blow".to_owned(),
            kill_names: Some((request.killer.clone(), request.victim.clone())),
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            locked(&self.launching).remove(&lane.key);
            audit_kill(
                "failed",
                &request.killer,
                &request.victim,
                0,
                "runtime_unavailable",
            );
            return;
        };
        let engine = Arc::clone(self);
        handle.spawn(async move { engine.fly_clip(spec, lane.key, job).await });
    }

    /// The view's kill shot for the latest blow, if any. The first ready
    /// delivery is audited as shown once per (id, epoch); the client's own
    /// DISPLAY records are the playback evidence.
    fn kill_shot_for_view(&self) -> Option<rpc::KillShot> {
        let id = locked(&self.latest_kill).clone()?;
        let slot = locked(&self.kill_shots).get(&id).cloned()?;
        let (state, path) = match self.lane_state(&slot.lane) {
            LaneState::Ready => ("ready", Some(format!("{ROUTE}/{}", slot.lane.file))),
            LaneState::Pending => ("pending", None),
            LaneState::Failed => ("failed", None),
            LaneState::Forgotten => return None,
        };
        if state == "ready"
            && locked(&self.kill_shown_audited)
                .insert((slot.request.id.clone(), slot.request.epoch))
        {
            audit_kill(
                "shown",
                &slot.request.killer,
                &slot.request.victim,
                slot.started.elapsed().as_millis(),
                "delivered_ready",
            );
        }
        Some(kill_shot::view_projection(
            &slot.request,
            path,
            state,
            &slot.still_path,
        ))
    }

    fn scene_clip_view(&self, plate_key: &str) -> SceneClipView {
        if !valid_plate_key(plate_key) {
            return SceneClipView::None;
        }
        let Some(lane) = locked(&self.scenes).get(plate_key).cloned() else {
            return SceneClipView::None;
        };
        match self.lane_state(&lane) {
            LaneState::Ready => SceneClipView::Ready {
                path: format!("{ROUTE}/{}", lane.file),
            },
            LaneState::Pending => SceneClipView::Pending,
            LaneState::Failed => SceneClipView::Failed,
            LaneState::Forgotten => SceneClipView::None,
        }
    }

    /// The task of a painting: prepare its still, key the clip by it, run the clip, then wake the
    /// displays.
    async fn fly_home(self: Arc<Self>, painting: String) {
        let (input, unavailable, still) = match prepare_still(&self.root, &painting).await {
            Ok(still) => (ClipInput::ready(&still, &home_video_prompt()), None, still),
            Err(Failure::Still(reason)) => (
                ClipInput::refused(None, reason),
                Some(Failure::Still(reason)),
                Vec::new(),
            ),
            Err(other) => (
                ClipInput::refused(None, "still unavailable"),
                Some(other),
                Vec::new(),
            ),
        };
        let spec = HomeLoopSpec::new(&painting, input.clone(), clip_recipe(Variant::Home));
        let lane = Lane {
            key: ResourceKey::of(&spec),
            file: clip_spec::file_key(&spec),
        };
        // A run of this clip may already be going (another painting with the same still).
        let started = self.claim_launch(&lane.key);
        locked(&self.home).insert(painting.clone(), HomeSlot::Keyed(lane.clone()));
        if started {
            let job = ClipJob {
                variant: Variant::Home,
                resource_subject: spec.subject(),
                file: lane.file.clone(),
                input,
                unavailable,
                still: StillBytes::Prepared(still),
                reason: "home-loop".to_owned(),
                kill_names: None,
            };
            self.fly_clip(spec, lane.key, job).await;
        }
        // The display streams watch the home wake, and the living scenes' wake when it has one.
        WAKE.get_or_init(|| watch::channel(0).0)
            .send_modify(|count| *count += 1);
    }

    /// The one run of a clip, through the generation hub: identical asks share it, the hub
    /// remembers its failure and lists it, and the outcome is read back from the hub by the view
    /// (and written to the log by [`Self::generate`]).
    async fn fly_clip<S>(self: Arc<Self>, spec: S, key: ResourceKey, job: ClipJob)
    where
        S: ResourceSpec + Send + Sync + 'static,
    {
        let kit = self.kit.get();
        let engine = Arc::clone(&self);
        let work = async move { engine.generate(job).await };
        // The outcome is in the hub and the log; the views read it from the hub.
        let (_outcome, _role) = kit.produce(&self.flights, spec, work).await;
        locked(&self.launching).remove(&key);
        if let Some(living) = &self.living {
            living.signal_art();
        }
    }

    /// Makes the clip, or finds it in the store, and writes the log lines of the attempt.
    async fn generate(self: Arc<Self>, job: ClipJob) -> Result<Produced<()>, ResourceFailure> {
        let started = Instant::now();
        log_requested(&job);
        let mut paid = None;
        let outcome = self.produce_clip(&job, &mut paid).await;
        let elapsed = started.elapsed();
        match outcome {
            Ok(done) => {
                if let Some((killer, victim)) = &job.kill_names {
                    audit_kill("clip_ready", killer, victim, elapsed.as_millis(), "ready");
                }
                log_ready(&job, &done, paid.as_ref(), elapsed);
                Ok(Produced {
                    generated: Generated {
                        handle: job.file.clone(),
                        note: format!("{} bytes", done.bytes),
                    },
                    payload: Some(()),
                    from_store: done.cached,
                })
            }
            Err(failure) => {
                if let Some((killer, victim)) = &job.kill_names {
                    audit_kill(
                        "failed",
                        killer,
                        victim,
                        elapsed.as_millis(),
                        &failure.label(),
                    );
                }
                log_failed(&job, &failure, paid.as_ref(), elapsed);
                Err(classify(&failure))
            }
        }
    }

    async fn produce_clip(
        &self,
        job: &ClipJob,
        paid: &mut Option<PaidCall>,
    ) -> Result<ClipDone, Failure> {
        if let Some(failure) = &job.unavailable {
            return Err(failure.clone());
        }
        let prompt = job
            .input
            .prompt()
            .ok_or(Failure::Clip("clip prompt missing"))?;
        if let Some(path) = cached_clip(&self.cache_dir, &job.file) {
            let bytes = std::fs::metadata(path)
                .map_err(|_| Failure::Clip("cached clip unavailable"))?
                .len();
            return Ok(ClipDone {
                cached: true,
                bytes,
            });
        }
        let still = match &job.still {
            StillBytes::Prepared(bytes) => bytes.clone(),
            StillBytes::Plate(plate) => prepare_scene_still(plate.clone()).await?,
        };
        let provider = self.provider()?;
        let ledger = Arc::clone(self.kit.get().hub().ledger());
        let micro_per_second = ledger.costs().quote(
            cost::Provider::Fal,
            MODEL,
            &[(cost::Category::VideoSeconds, 1)],
        );
        // The request is checked before anything is charged, so a request refused locally costs
        // nothing.
        let (request, reference) = clip_request(job.variant, &still, &job.file, prompt)?;
        // The call is about to be sent: the clip is paid for whatever its outcome, and a call of
        // unknown outcome may have been billed, so it is never given back.
        self.kit.get().charge(Unit::VideoClip, 1);
        let billed = ledger.begin_call(
            cost::video_call(
                MODEL,
                cost::Cause::HomeVideo,
                match job.variant {
                    Variant::Home => "home",
                    Variant::Scene => "scene",
                    Variant::KillShot => "kill_shot",
                },
                u64::from(DURATION_SECONDS),
            ),
            &[],
        );
        let started = Instant::now();
        let fetched = self.fetch(&provider, &request, &reference).await;
        // A clip that came back was billed; a failure is not classified by this client (a refusal
        // at submit and a clip that never finished look alike), so it is of unknown outcome and
        // booked at the clip's length.
        match &fetched {
            Ok(_) => cost::settle_counted(
                billed,
                vec![(cost::Category::VideoSeconds, u64::from(DURATION_SECONDS))],
            ),
            Err(_) => drop(billed),
        }
        let stored = match fetched {
            Ok(bytes) => match LoopFinish::CURRENT.apply(bytes) {
                Ok(finished) => {
                    let length = finished.len() as u64;
                    write_clip(&self.cache_dir, &job.file, &finished)
                        .await
                        .map(|()| length)
                }
                Err(failure) => Err(failure),
            },
            Err(failure) => Err(failure),
        };
        *paid = Some(PaidCall {
            micro_per_second,
            elapsed: started.elapsed(),
        });
        stored.map(|bytes| ClipDone {
            cached: false,
            bytes,
        })
    }

    fn provider(&self) -> Result<NativeFalMediaProvider, Failure> {
        let credential = self
            .credential
            .as_deref()
            .ok_or_else(|| Failure::Provider("no FAL_KEY".to_owned()))?;
        let config = match &self.endpoints {
            Endpoints::Live { pins } if pins.is_empty() => {
                return Err(Failure::Provider(
                    "fal media hosts do not resolve".to_owned(),
                ));
            }
            Endpoints::Live { pins } => FalMediaConfig::live(FalMediaFamily::Video, pins.clone()),
            Endpoints::Fixture { api, media } => {
                FalMediaConfig::loopback_fixture(FalMediaFamily::Video, *api, *media)
                    .map_err(|_| Failure::Provider("fixture address refused".to_owned()))?
            }
        };
        let config = config
            .with_video_model(MODEL)
            .map_err(|_| Failure::Provider("model refused".to_owned()))?;
        registered_fal_media_provider(config, credential, limits())
            .map_err(|error| Failure::Provider(format!("{:?}", error.reason)))
    }

    /// Submits the request, waits for the clip and checks it, within the engine's deadline.
    async fn fetch(
        &self,
        provider: &NativeFalMediaProvider,
        request: &FalVideoRequest,
        reference: &RequestBinding<FalVideoBasis>,
    ) -> Result<Vec<u8>, Failure> {
        tokio::time::timeout(self.deadline, self.run_job(provider, request, reference))
            .await
            .map_err(|_| Failure::Timeout)?
    }

    async fn run_job(
        &self,
        provider: &NativeFalMediaProvider,
        request: &FalVideoRequest,
        reference: &RequestBinding<FalVideoBasis>,
    ) -> Result<Vec<u8>, Failure> {
        let mut job = provider
            .submit_video(request, owner(reference), std::future::pending())
            .await
            .map_err(provider_failure)?;
        loop {
            match job
                .status(owner(reference), std::future::pending())
                .await
                .map_err(provider_failure)?
            {
                FalQueueObservation::Completed { .. } => break,
                FalQueueObservation::Failed {
                    error_type, error, ..
                } => {
                    let detail = error_type
                        .or(error)
                        .unwrap_or_else(|| "unspecified".to_owned());
                    return Err(Failure::Provider(format!("refused or failed: {detail}")));
                }
                FalQueueObservation::Unknown(_) => {
                    return Err(Failure::Provider("status unknown".to_owned()));
                }
                FalQueueObservation::Pending(_) => tokio::time::sleep(POLL_INTERVAL).await,
            }
        }
        let event = job
            .result(owner(reference), std::future::pending())
            .await
            .map_err(provider_failure)?
            .complete(owner(reference))
            .map_err(provider_failure)?
            .into_video_event()
            .map_err(provider_failure)?;
        let VideoEventKind::Completed { output, .. } = event.kind else {
            return Err(Failure::Provider("no completed output".to_owned()));
        };
        let [clip] = output.as_slice() else {
            return Err(Failure::Clip("not exactly one output"));
        };
        let bytes = clip.bytes().to_vec();
        match clip.facts() {
            EncodedMediaFacts::Mp4 {
                width,
                height,
                timebase,
                ..
            } => {
                let milliseconds = timebase
                    .milliseconds_ceil()
                    .map_err(|_| Failure::Clip("clock unreadable"))?;
                if !acceptable_frame(width, height)
                    || !(MINIMUM_CLIP_MILLISECONDS..=MAXIMUM_CLIP_MILLISECONDS)
                        .contains(&milliseconds)
                {
                    return Err(Failure::Clip("size or duration outside the expected class"));
                }
            }
            _ => return Err(Failure::Clip("not an MP4")),
        }
        if bytes.len() > MAXIMUM_CLIP_BYTES {
            return Err(Failure::Clip("larger than the clip bound"));
        }
        Ok(bytes)
    }
}

/// Whether a clip's frame is the shape the display wants: 16:9 within two percent, at least 640
/// pixels wide. Another provider may return 720p for a 1080p request, and a real clip of the
/// right shape is not discarded for its size.
fn acceptable_frame(width: u32, height: u32) -> bool {
    let (width, height) = (u64::from(width), u64::from(height));
    if width < 640 {
        return false;
    }
    let (left, right) = (width * 9, height * 16);
    left.abs_diff(right) * 50 <= right
}

/// The adapter's reason, with the queue request it concerned: the diagnostic that names a job
/// the provider may still be running.
///
/// When the supplier answered over HTTP, its status and a bounded, redacted excerpt of its body
/// follow (the adapter has already removed the credential and any opaque token), so a refusal
/// reads as the supplier worded it.
fn provider_failure(error: FalMediaError) -> Failure {
    let mut detail = format!("{:?}", error.reason);
    if let Some(id) = &error.request_id {
        detail.push_str(&format!(" request={}", id.as_str()));
    }
    if let Some(http) = &error.http {
        detail.push_str(&format!(
            " http_status={} body={:?}",
            http.status, http.body
        ));
    }
    Failure::Provider(detail)
}

fn valid_plate_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn limits() -> NativeHttpLimits {
    // The response bound is the adapter's ceiling; the clip bound below is the same size.
    ProviderLimits::new(REQUEST_DEADLINE, 8 * 1024 * 1024, 16 * 1024 * 1024)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(60))
        .chunks(65_536)
        .build()
}

fn owner(binding: &RequestBinding<FalVideoBasis>) -> RequestOwnerState<'_, FalVideoBasis> {
    RequestOwnerState {
        current: Some(binding),
        elapsed: Duration::ZERO,
        cancelled: false,
    }
}

fn request_refused<E>(_: E) -> Failure {
    Failure::Provider("request refused before submit".to_owned())
}

/// The request basis for one still: the still as both frames, five seconds, the adapter's own
/// profile for the endpoint.
fn video_basis_with(
    still: &[u8],
    key: &str,
    prompt: &str,
    asset_key: &str,
    rights_revision: &str,
    recipe: &str,
) -> Result<RequestBinding<FalVideoBasis>, Failure> {
    let digest: [u8; 32] = Sha256::digest(still).into();
    let mut ids = [0u8; 16];
    for (target, source) in ids.iter_mut().zip(key.as_bytes().chunks(2)) {
        let text = std::str::from_utf8(source).unwrap_or("00");
        *target = u8::from_str_radix(text, 16).unwrap_or(0);
    }
    let locator = format!("data:image/jpeg;base64,{}", base64_encode(still));
    let basis = Basis {
        session: SessionId::from_bytes(&ids).map_err(request_refused)?,
        run: RunId::from_bytes(&ids).map_err(request_refused)?,
        revision: SessionRevision::new(RecoveryEpoch::new(1).map_err(request_refused)?, 1),
    };
    let profile = FalVideoProfile {
        model: MODEL.to_owned(),
        duration_seconds: DURATION_SECONDS,
        resolution: "1080p".to_owned(),
        width: WIDTH,
        height: HEIGHT,
        accepted_duration_min: Duration::from_millis(4_500),
        accepted_duration_max: Duration::from_millis(5_500),
        seed: 0,
        safety: true,
        prompt_expansion: false,
    };
    let semantic = VideoSemantics {
        locale: None,
        prompt: prompt.to_owned(),
        reference_frames: vec![FalReferenceFrame {
            asset: AssetReference {
                key: RevisionLabel::new(Some(asset_key)).map_err(request_refused)?,
                digest: ContentDigest(digest),
                byte_length: still.len() as u64,
                kind: AssetKind::Image,
            },
            locator,
        }],
        output_profile: profile,
    };
    let binding = RequestBinding {
        identity: RequestIdentity {
            basis,
            job: JobId::from_bytes(&ids).map_err(request_refused)?,
            operation: OperationId::from_bytes(&ids).map_err(request_refused)?,
            generation: 1,
        },
        semantic_basis: RequestBasis {
            modality: RequestModality::Video,
            source: ContentDigest(digest),
            rights_revision: RevisionLabel::new(Some(rights_revision)).map_err(request_refused)?,
            output_format_revision: RevisionLabel::new(Some(recipe)).map_err(request_refused)?,
            semantic,
        },
        mode: ExecutionMode::Live,
        deadline: REQUEST_DEADLINE,
    };
    Ok(binding)
}

/// The request for one still of either loop, and the binding it was checked against (the owner's
/// view of it).
fn clip_request(
    variant: Variant,
    still: &[u8],
    key: &str,
    prompt: &str,
) -> Result<(FalVideoRequest, RequestBinding<FalVideoBasis>), Failure> {
    let (asset_key, rights_revision, recipe) = match variant {
        Variant::Home => ("home-still", "home-video", RECIPE),
        Variant::Scene => ("scene-plate", "scene-video", SCENE_RECIPE),
        Variant::KillShot => ("kill-shot-still", "kill-shot", KILL_RECIPE),
    };
    let reference = video_basis_with(still, key, prompt, asset_key, rights_revision, recipe)?;
    let request_basis = video_basis_with(still, key, prompt, asset_key, rights_revision, recipe)?;
    make_video_request(request_basis, reference)
}

fn make_video_request(
    request_basis: RequestBinding<FalVideoBasis>,
    reference: RequestBinding<FalVideoBasis>,
) -> Result<(FalVideoRequest, RequestBinding<FalVideoBasis>), Failure> {
    let request = FalVideoRequest::new(
        request_basis,
        b"",
        RequestUsage::new(
            0,
            Duration::from_secs(u64::from(DURATION_SECONDS)),
            Usage::new(5_000, UsageUnit::VideoMillisecond),
        ),
        RequestLimits::new(
            8 * 1024 * 1024,
            0,
            Duration::from_secs(10),
            Usage::new(10_000, UsageUnit::VideoMillisecond),
        )
        .map_err(request_refused)?,
        RequestOwnerState {
            current: Some(&reference),
            elapsed: Duration::ZERO,
            cancelled: false,
        },
    )
    .map_err(request_refused)?;
    Ok((request, reference))
}

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        out.push(char::from(ALPHABET[usize::from(first >> 2)]));
        out.push(char::from(
            ALPHABET[usize::from(((first & 3) << 4) | (second >> 4))],
        ));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[usize::from(((second & 15) << 2) | (third >> 6))])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[usize::from(third & 63)])
        } else {
            '='
        });
    }
    out
}

/// The painting as the model sees it: 1920 by 1080, JPEG at quality 95, decoded and encoded once.
async fn prepare_still(root: &Path, painting: &str) -> Result<Vec<u8>, Failure> {
    let path = home_theme::painting_file(root, painting)
        .map_err(|_| Failure::Still("painting file unavailable"))?;
    tokio::task::spawn_blocking(move || {
        let image = image::ImageReader::open(&path)
            .and_then(|reader| reader.with_guessed_format())
            .map_err(|_| Failure::Still("painting unreadable"))?
            .decode()
            .map_err(|_| Failure::Still("painting does not decode"))?;
        let rgb = image
            .resize_exact(WIDTH, HEIGHT, image::imageops::FilterType::Triangle)
            .to_rgb8();
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, STILL_QUALITY)
            .encode(rgb.as_raw(), WIDTH, HEIGHT, image::ExtendedColorType::Rgb8)
            .map_err(|_| Failure::Still("still does not encode"))?;
        Ok(bytes)
    })
    .await
    .map_err(|_| Failure::Still("still task interrupted"))?
}

async fn prepare_scene_still(still: Vec<u8>) -> Result<Vec<u8>, Failure> {
    tokio::task::spawn_blocking(move || {
        let image = image::load_from_memory(&still)
            .map_err(|_| Failure::Still("scene plate does not decode"))?;
        let rgb = image
            .resize_exact(WIDTH, HEIGHT, image::imageops::FilterType::Triangle)
            .to_rgb8();
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, STILL_QUALITY)
            .encode(rgb.as_raw(), WIDTH, HEIGHT, image::ExtendedColorType::Rgb8)
            .map_err(|_| Failure::Still("scene plate does not encode"))?;
        Ok(bytes)
    })
    .await
    .map_err(|_| Failure::Still("scene plate task interrupted"))?
}

#[cfg(test)]
pub(crate) async fn prepare_still_for_test(
    root: &Path,
    painting: &str,
) -> Result<Vec<u8>, Failure> {
    prepare_still(root, painting).await
}

/// The home loop's file key for a prepared still: the whole digest of its spec.
#[cfg(test)]
pub(crate) fn clip_key(still: &[u8]) -> String {
    let input = ClipInput::ready(still, &home_video_prompt());
    clip_spec::file_key(&HomeLoopSpec::new("", input, clip_recipe(Variant::Home)))
}

/// The scene loop's spec for a plate, with `edit` applied to the recipe, for the key checks.
#[cfg(test)]
pub(crate) fn scene_spec_for_test(
    plate_key: &str,
    plate: &[u8],
    prompt: &str,
    edit: impl FnOnce(&mut ClipRecipe),
) -> SceneLoopSpec {
    let mut recipe = clip_recipe(Variant::Scene);
    edit(&mut recipe);
    SceneLoopSpec::new(plate_key, ClipInput::ready(plate, prompt), recipe)
}

/// The scene loop's file key for a plate and a description, as `request_scene_clip` computes it.
#[cfg(test)]
pub(crate) fn scene_clip_key(plate: &[u8], description: &str) -> Option<String> {
    let prompt = scene_video_prompt::ambient(description).ok()?;
    Some(clip_spec::file_key(&scene_spec_for_test(
        "",
        plate,
        &prompt,
        |_| {},
    )))
}

fn clip_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(format!("{key}.mp4"))
}

/// A cached clip that is present, bounded, and an MP4. The bytes were validated when written.
fn cached_clip(cache_dir: &Path, key: &str) -> Option<PathBuf> {
    let root = std::fs::canonicalize(cache_dir).ok()?;
    let path = std::fs::canonicalize(clip_path(cache_dir, key)).ok()?;
    if !is_within_cache(&root, &path) {
        return None;
    }
    let metadata = std::fs::metadata(&path).ok()?;
    (metadata.is_file() && metadata.len() > 12 && metadata.len() <= MAXIMUM_CLIP_BYTES as u64)
        .then_some(path)
}

pub(super) fn is_within_cache(root: &Path, resolved: &Path) -> bool {
    resolved.starts_with(root)
}

/// Writes a validated clip beside its key through the shared atomic write, so a reader never sees
/// half a file. The write is blocking I/O, so it runs off the async workers.
async fn write_clip(cache_dir: &Path, key: &str, bytes: &[u8]) -> Result<(), Failure> {
    let directory = cache_dir.to_path_buf();
    let name = format!("{key}.mp4");
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        write_atomic(&directory, &name, &bytes).map_err(|_| Failure::Clip("cache write failed"))
    })
    .await
    .map_err(|_| Failure::Clip("cache task interrupted"))?
}

/// The byte span a range request names, for a file of `length` bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Span {
    Whole,
    Part { first: u64, last: u64 },
    Unsatisfiable,
}

/// `bytes=a-b`, `bytes=a-`, `bytes=-n`. Several ranges, another unit, or a syntactically invalid
/// range is ignored and the whole file is served, as RFC 9110 allows; a range that names no byte
/// of the file is unsatisfiable.
pub(crate) fn span_for(range: Option<&str>, length: u64) -> Span {
    let Some(spec) = range.and_then(|value| value.trim().strip_prefix("bytes=")) else {
        return Span::Whole;
    };
    if spec.contains(',') {
        return Span::Whole;
    }
    let Some((start, end)) = spec.split_once('-') else {
        return Span::Whole;
    };
    if length == 0 {
        return Span::Unsatisfiable;
    }
    if start.is_empty() {
        let Ok(suffix) = end.parse::<u64>() else {
            return Span::Whole;
        };
        if suffix == 0 {
            return Span::Unsatisfiable;
        }
        return Span::Part {
            first: length.saturating_sub(suffix),
            last: length - 1,
        };
    }
    let Ok(first) = start.parse::<u64>() else {
        return Span::Whole;
    };
    if first >= length {
        return Span::Unsatisfiable;
    }
    let last = if end.is_empty() {
        length - 1
    } else {
        match end.parse::<u64>() {
            Ok(last) if last >= first => last.min(length - 1),
            _ => return Span::Whole,
        }
    };
    Span::Part { first, last }
}

async fn serve_clip(RoutePath(key): RoutePath<String>, headers: HeaderMap, uri: Uri) -> Response {
    let Some(grant) = super::media_grant::verify_request(&headers, &uri, &key) else {
        return super::media_grant::concealed_not_found();
    };
    let started = Instant::now();
    let response = match ENGINE.lock().ok().and_then(|engine| engine.clone()) {
        Some(engine) => respond(&engine.cache_dir, &key, &headers).await,
        None => StatusCode::NOT_FOUND.into_response(),
    };
    let status_code = response.status().as_u16();
    let (status, failure_category, cache_result, stage) = match status_code {
        200 | 206 => ("ready", None, "server_hit", "delivered"),
        304 => ("not_modified", None, "client_revalidated", "delivered"),
        404 => ("not_found", Some("not_found"), "miss", "failed"),
        416 => (
            "range_refused",
            Some("range_refused"),
            "server_hit",
            "failed",
        ),
        _ => ("server_error", Some("server_error"), "error", "failed"),
    };
    let byte_length = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let browser_asset_id = df_observe::presentation::local_identity(&format!("{ROUTE}/{key}"));
    record_video_resource(
        "gameplay.media.delivery",
        &safe_video_resource_id(&key),
        stage,
        if failure_category.is_some() {
            "failed"
        } else {
            "delivered"
        },
        failure_category,
        "video_cache",
        MODEL,
        DURATION_SECONDS,
        Some(started.elapsed().as_millis()),
        None,
        Some(byte_length),
        cache_result,
        Some(status),
        Some(browser_asset_id),
    );
    super::media_grant::secure_media_response(response, grant.remaining)
}

/// The response for one cached clip: the whole file, a byte range, or a refusal of a range that
/// names no byte. Only lowercase hex keys that are cached are served.
pub(crate) async fn respond(cache_dir: &Path, key: &str, headers: &HeaderMap) -> Response {
    if key.len() != 64
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(path) = cached_clip(cache_dir, key) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let key = key.to_owned();
    let headers = headers.clone();
    let etag = format!("\"{key}\"");
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(etag.as_str())
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    let Ok(Ok(bytes)) = tokio::task::spawn_blocking(move || std::fs::read(&path)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let length = bytes.len() as u64;
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let base = |builder: axum::http::response::Builder| {
        builder
            .header(header::CONTENT_TYPE, "video/mp4")
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::ETAG, etag.as_str())
            .header(header::CACHE_CONTROL, "private, max-age=86400, immutable")
    };
    let response = match span_for(range, length) {
        Span::Whole => base(Response::builder())
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, length)
            .body(Body::from(bytes)),
        Span::Part { first, last } => {
            let slice = bytes
                .get(first as usize..=last as usize)
                .unwrap_or_default()
                .to_vec();
            base(Response::builder())
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_LENGTH, slice.len())
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {first}-{last}/{length}"),
                )
                .body(Body::from(slice))
        }
        Span::Unsatisfiable => base(Response::builder())
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{length}"))
            .body(Body::empty()),
    };
    response.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn context() -> OperationContext {
    OperationContext {
        trace_parent: String::new(),
        build: BUILD_ID.to_owned(),
    }
}

/// One paid call, with its time, model, price and outcome, through the shared span path.
fn record_paid(
    key: &str,
    micro_per_second: Option<u64>,
    elapsed: Duration,
    request_elapsed_ms: u128,
    outcome: &str,
    failure_category: Option<&'static str>,
) {
    let mut span = df_observe::begin(&context(), "gameplay.home_video_render");
    span.event(
        "gameplay.home_video.paid",
        &[
            ("gameplay.home_video.model", MODEL.to_owned()),
            ("gameplay.home_video.key", key[..16].to_owned()),
            (
                "gameplay.home_video.price_usd_per_second",
                micro_per_second.map_or_else(|| "unpriced".to_owned(), cost::usd),
            ),
            (
                "gameplay.home_video.seconds",
                format!("{:.1}", elapsed.as_secs_f64()),
            ),
            (
                "gameplay.home_video.request_elapsed_ms",
                request_elapsed_ms.to_string(),
            ),
            (
                "gameplay.home_video.failure_category",
                failure_category.unwrap_or("none").to_owned(),
            ),
            (
                "gameplay.home_video.estimated_usd",
                micro_per_second.map_or_else(
                    || "unpriced".to_owned(),
                    |micro| cost::usd(micro.saturating_mul(u64::from(DURATION_SECONDS))),
                ),
            ),
        ],
    );
    span.finish_unmeasured(if outcome == "ready" {
        "ready"
    } else {
        "failed"
    });
}

/// How the hub remembers a failure: what would fail again is permanent, what may pass is
/// transient, and a deadline has its own class. The label is stable and holds no
/// detail of the request.
fn classify(failure: &Failure) -> ResourceFailure {
    match failure {
        Failure::Still("still task interrupted") => {
            ResourceFailure::Transient("still task interrupted")
        }
        Failure::Still(reason) | Failure::Clip(reason) => ResourceFailure::Permanent(*reason),
        Failure::Timeout => ResourceFailure::TimedOut,
        Failure::Provider(detail) => provider_class(detail),
    }
}

/// The provider failures the engine itself words, and the adapter's own reason for the rest.
fn provider_class(detail: &str) -> ResourceFailure {
    match detail {
        "no FAL_KEY" => ResourceFailure::Permanent("provider: no FAL_KEY"),
        "fal media hosts do not resolve" => {
            ResourceFailure::Permanent("provider: fal media hosts do not resolve")
        }
        "request refused before submit" => {
            ResourceFailure::Permanent("provider: request refused before submit")
        }
        "fixture address refused" | "model refused" => {
            ResourceFailure::Permanent("provider: configuration refused")
        }
        "status unknown" => ResourceFailure::Transient("provider: status unknown"),
        "no completed output" => ResourceFailure::Transient("provider: no completed output"),
        other if other.starts_with("refused or failed") => {
            ResourceFailure::Transient("provider: refused or failed")
        }
        _ => ResourceFailure::Transient("provider: call failed"),
    }
}

fn log_requested(job: &ClipJob) {
    match job.variant {
        Variant::Home => record_video_resource(
            "gameplay.home_video.requested",
            &job.file,
            "requested",
            "requested",
            None,
            "fal",
            MODEL,
            DURATION_SECONDS,
            None,
            None,
            None,
            "unknown",
            None,
            None,
        ),
        Variant::Scene => record_scene_video(
            "request",
            &job.file,
            MODEL,
            DURATION_SECONDS,
            &job.reason,
            None,
            None,
            None,
        ),
        Variant::KillShot => record_scene_video(
            "request",
            &job.file,
            MODEL,
            DURATION_SECONDS,
            &job.reason,
            None,
            None,
            None,
        ),
    }
}

fn log_ready(job: &ClipJob, done: &ClipDone, paid: Option<&PaidCall>, elapsed: Duration) {
    let request_ms = elapsed.as_millis();
    if !done.cached
        && let Some(paid) = paid
    {
        record_paid(
            &job.file,
            paid.micro_per_second,
            paid.elapsed,
            request_ms,
            "ready",
            None,
        );
    }
    match (job.variant, done.cached) {
        (Variant::Home, true) => {
            record_event("cached", &job.file);
            record_video_resource(
                "gameplay.home_video.ready",
                &job.file,
                "ready",
                "cached",
                None,
                "cache",
                MODEL,
                DURATION_SECONDS,
                Some(0),
                None,
                None,
                "hit",
                None,
                None,
            );
        }
        (Variant::Home, false) => record_video_resource(
            "gameplay.home_video.ready",
            &job.file,
            "ready",
            "ready",
            None,
            "fal",
            MODEL,
            DURATION_SECONDS,
            Some(request_ms),
            paid.map(|paid| paid.elapsed.as_millis()),
            Some(done.bytes),
            "stored",
            None,
            None,
        ),
        (Variant::Scene, true) => record_scene_video(
            "ready",
            &job.file,
            MODEL,
            DURATION_SECONDS,
            "cached",
            Some(0),
            Some(0),
            Some(done.bytes),
        ),
        (Variant::Scene, false) => record_scene_video(
            "ready",
            &job.file,
            MODEL,
            DURATION_SECONDS,
            "ambient-scene-loop",
            paid.map(|paid| paid.elapsed.as_millis()),
            Some(request_ms),
            Some(done.bytes),
        ),
        (Variant::KillShot, true) => record_scene_video(
            "ready",
            &job.file,
            MODEL,
            DURATION_SECONDS,
            "cached",
            Some(0),
            Some(0),
            Some(done.bytes),
        ),
        (Variant::KillShot, false) => record_scene_video(
            "ready",
            &job.file,
            MODEL,
            DURATION_SECONDS,
            "ambient-scene-loop",
            paid.map(|paid| paid.elapsed.as_millis()),
            Some(request_ms),
            Some(done.bytes),
        ),
    }
}

fn log_failed(job: &ClipJob, failure: &Failure, paid: Option<&PaidCall>, elapsed: Duration) {
    let request_ms = elapsed.as_millis();
    let category = video_failure_category(failure);
    record_failure_detail(job, failure);
    if let Some(paid) = paid {
        record_paid(
            &job.file,
            paid.micro_per_second,
            paid.elapsed,
            request_ms,
            "failed",
            Some(category),
        );
    }
    match job.variant {
        Variant::Home => record_video_resource(
            "gameplay.home_video.failed",
            &job.file,
            "failed",
            "failed",
            Some(category),
            "fal",
            MODEL,
            DURATION_SECONDS,
            Some(request_ms),
            paid.map(|paid| paid.elapsed.as_millis()),
            None,
            "miss",
            None,
            None,
        ),
        Variant::Scene => record_scene_video(
            "failed",
            &job.file,
            MODEL,
            DURATION_SECONDS,
            &failure.label(),
            None,
            Some(request_ms),
            None,
        ),
        Variant::KillShot => record_scene_video(
            "failed",
            &job.file,
            MODEL,
            DURATION_SECONDS,
            &failure.label(),
            None,
            Some(request_ms),
            None,
        ),
    }
}

/// The full wording of a failure, which the hub's stable label leaves out: the adapter's reason,
/// the queue request it concerned (the diagnostic that names a job the provider may still be
/// running) and, when the supplier answered over HTTP, its status and a bounded, redacted excerpt
/// of its body. It holds no credential. It is written in the hub's `RESOURCE` line format beside
/// the hub's own `failed` line for the same key, as `failed_detail`.
fn record_failure_detail(job: &ClipJob, failure: &Failure) {
    let detail: String = failure
        .label()
        .chars()
        .map(|character| match character {
            '"' => '\'',
            character if character.is_control() => ' ',
            character => character,
        })
        .collect();
    super::actor::record_audit_line(
        "gameplay.resource",
        format!(
            "RESOURCE failed_detail kind={} key={} subject={} ms=0 handle= error=\"{detail}\"",
            ResourceKind::VideoClip.label(),
            job.file.chars().take(32).collect::<String>(),
            job.resource_subject,
        ),
    );
    let mut span = df_observe::begin(&context(), "gameplay.home_video_event");
    span.event(
        "gameplay.home_video.failure",
        &[
            (
                "gameplay.home_video.key",
                job.file.chars().take(16).collect(),
            ),
            (
                "gameplay.home_video.variant",
                match job.variant {
                    Variant::Home => "home",
                    Variant::Scene => "scene",
                    Variant::KillShot => "kill_shot",
                }
                .to_owned(),
            ),
            ("gameplay.home_video.detail", failure.label()),
        ],
    );
    span.finish_unmeasured("failed");
}

fn record_event(kind: &str, subject: &str) {
    let mut span = df_observe::begin(&context(), "gameplay.home_video_event");
    let fields = vec![
        ("gameplay.home_video.event", kind.to_owned()),
        (
            "gameplay.home_video.subject",
            subject.chars().take(16).collect(),
        ),
    ];
    span.event("gameplay.home_video.event", &fields);
    span.finish_unmeasured("recorded");
}

fn record_scene_video(
    kind: &str,
    plate: &str,
    model: &str,
    duration_seconds: u32,
    reason: &str,
    provider_ms: Option<u128>,
    elapsed_ms: Option<u128>,
    bytes: Option<u64>,
) {
    let resource_id = safe_video_resource_id(plate);
    let (stage, status, failure_category, cache_result) = match kind {
        "request" => ("requested", "requested", None, "unknown"),
        "ready" if reason == "cached" => ("ready", "cached", None, "hit"),
        "ready" => ("ready", "ready", None, "stored"),
        _ => (
            "failed",
            "failed",
            Some(video_failure_reason_category(reason)),
            "miss",
        ),
    };
    let mut span = df_observe::begin(&context(), "gameplay.scene_video");
    span.event(
        "gameplay.scene_video.event",
        &[
            ("gameplay.scene_video.resource_id", resource_id.clone()),
            ("gameplay.scene_video.model", model.to_owned()),
            ("gameplay.scene_video.kind", kind.to_owned()),
            ("gameplay.scene_video.status", status.to_owned()),
        ],
    );
    span.finish_unmeasured(kind);
    record_video_resource(
        match kind {
            "request" => "gameplay.scene_video.requested",
            "ready" => "gameplay.scene_video.ready",
            _ => "gameplay.scene_video.failed",
        },
        &resource_id,
        stage,
        status,
        failure_category,
        "fal",
        model,
        duration_seconds,
        elapsed_ms,
        provider_ms,
        bytes,
        cache_result,
        Some(video_reason_category(reason)),
        None,
    );
}

fn safe_video_resource_id(key: &str) -> String {
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

fn video_failure_category(failure: &Failure) -> &'static str {
    match failure {
        Failure::Still(_) => "still_failure",
        Failure::Provider(_) => "provider_failure",
        Failure::Timeout => "deadline",
        Failure::Clip(_) => "clip_failure",
    }
}

fn video_failure_reason_category(reason: &str) -> &'static str {
    match reason {
        "still: invalid" | "still: unavailable" => "still_failure",
        "timeout" => "deadline",
        value if value.starts_with("provider:") => "provider_failure",
        value if value.starts_with("clip:") => "clip_failure",
        _ => "failed",
    }
}

fn video_reason_category(reason: &str) -> &'static str {
    match reason {
        "cached" => "cached",
        "ambient-scene-loop" => "ambient_loop",
        "opening-scene" => "opening_scene",
        _ => "unspecified",
    }
}

#[allow(clippy::too_many_arguments)]
fn record_video_resource(
    event: &'static str,
    resource_id: &str,
    stage: &'static str,
    status: &'static str,
    failure_category: Option<&'static str>,
    provider: &'static str,
    model: &str,
    duration_seconds: u32,
    elapsed_ms: Option<u128>,
    provider_ms: Option<u128>,
    byte_length: Option<u64>,
    cache_result: &'static str,
    status_category: Option<&'static str>,
    browser_asset_id: Option<u64>,
) {
    let line = format!(
        "VIDEO {stage} status={status} provider={provider} model={model} resource_id={resource_id} duration_seconds={duration_seconds} elapsed_ms={} provider_ms={} bytes={} cache_result={cache_result} failure_category={} status_category={}",
        elapsed_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        provider_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        byte_length.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        failure_category.unwrap_or("none"),
        status_category.unwrap_or("none"),
    );
    let fields = serde_json::Map::from_iter([
        ("resource_type".to_owned(), serde_json::json!("video")),
        ("resource_id".to_owned(), serde_json::json!(resource_id)),
        ("provider".to_owned(), serde_json::json!(provider)),
        ("model".to_owned(), serde_json::json!(model)),
        ("stage".to_owned(), serde_json::json!(stage)),
        ("status".to_owned(), serde_json::json!(status)),
        (
            "failure_category".to_owned(),
            serde_json::json!(failure_category),
        ),
        (
            "duration_seconds".to_owned(),
            serde_json::json!(duration_seconds),
        ),
        ("elapsed_ms".to_owned(), serde_json::json!(elapsed_ms)),
        ("provider_ms".to_owned(), serde_json::json!(provider_ms)),
        ("byte_length".to_owned(), serde_json::json!(byte_length)),
        ("cache_result".to_owned(), serde_json::json!(cache_result)),
        (
            "status_category".to_owned(),
            serde_json::json!(status_category),
        ),
        (
            "browser_asset_id".to_owned(),
            serde_json::json!(browser_asset_id),
        ),
    ]);
    let attributes = [
        ("gameplay.resource.type", "video".to_owned()),
        ("gameplay.resource.id", resource_id.to_owned()),
        ("gameplay.video.provider", provider.to_owned()),
        ("gameplay.video.model", model.to_owned()),
        ("gameplay.resource.stage", stage.to_owned()),
        ("gameplay.resource.status", status.to_owned()),
        (
            "gameplay.resource.failure_category",
            failure_category.unwrap_or("none").to_owned(),
        ),
        (
            "gameplay.video.duration_seconds",
            duration_seconds.to_string(),
        ),
        (
            "gameplay.resource.elapsed_ms",
            elapsed_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
        (
            "gameplay.video.provider_ms",
            provider_ms.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
        (
            "gameplay.resource.byte_length",
            byte_length.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
        ("gameplay.media.cache_result", cache_result.to_owned()),
        (
            "gameplay.media.status_category",
            status_category.unwrap_or("none").to_owned(),
        ),
        (
            "gameplay.media.browser_asset_id",
            browser_asset_id.map_or_else(|| "none".to_owned(), |value| value.to_string()),
        ),
    ];
    super::actor::record_audit_diagnostic(event, line, fields, &attributes);
}

/// Test helper: an engine for the home loop, over a fixture or live endpoints.
#[cfg(test)]
pub(crate) fn test_engine(
    root: PathBuf,
    cache_dir: PathBuf,
    endpoints: Endpoints,
    deadline: Duration,
) -> Arc<Engine> {
    Arc::new(Engine::new(EngineSetup {
        root,
        cache_dir,
        credential: Some("fixture-private-key".to_owned()),
        endpoints,
        deadline,
        home_enabled: true,
        scene_enabled: false,
        living: None,
    }))
}

/// Test helper: installs the engine the view and the route use. Only one test installs one.
#[cfg(test)]
pub(crate) fn install_for_test(engine: Arc<Engine>) -> Result<(), Arc<Engine>> {
    match ENGINE.lock() {
        Ok(mut slot) if slot.is_none() => {
            *slot = Some(engine);
            Ok(())
        }
        _ => Err(engine),
    }
}

/// Test helper for the live check: the same engine with the live endpoints and a given credential.
#[cfg(test)]
pub(crate) fn live_test_engine(
    root: PathBuf,
    cache_dir: PathBuf,
    credential: String,
    pins: Vec<FalCdnPin>,
    deadline: Duration,
) -> Arc<Engine> {
    Arc::new(Engine::new(EngineSetup {
        root,
        cache_dir,
        credential: Some(credential),
        endpoints: Endpoints::Live { pins },
        deadline,
        home_enabled: true,
        scene_enabled: false,
        living: None,
    }))
}

/// Test helper: an engine for scene loops.
#[cfg(test)]
pub(crate) fn scene_test_engine(
    cache_dir: PathBuf,
    endpoints: Endpoints,
    deadline: Duration,
    credential: Option<String>,
) -> Arc<Engine> {
    Arc::new(Engine::new(EngineSetup {
        root: PathBuf::new(),
        cache_dir,
        credential,
        endpoints,
        deadline,
        home_enabled: false,
        scene_enabled: true,
        living: None,
    }))
}

#[cfg(test)]
impl Engine {
    fn stage_text(&self, lane: &Lane) -> String {
        if locked(&self.launching).contains(&lane.key) {
            return "pending".to_owned();
        }
        match self.kit.get().hub().status(&lane.key) {
            ResourceStatus::Ready(generated) => format!("ready:{}", generated.handle),
            ResourceStatus::Failed(failure, _) => format!("failed:{}", failure.label()),
            ResourceStatus::Pending(_) | ResourceStatus::Fallback(_) => "pending".to_owned(),
        }
    }

    pub(crate) fn stage_of(&self, painting: &str) -> Option<String> {
        let slot = locked(&self.home).get(painting).cloned()?;
        Some(match slot {
            HomeSlot::Preparing => "pending".to_owned(),
            HomeSlot::Keyed(lane) => self.stage_text(&lane),
        })
    }

    pub(crate) fn observe_for_test(self: &Arc<Self>, painting: &str) -> LoopView {
        self.observe(painting)
    }

    /// Clips charged on the hub's ledger so far.
    pub(crate) fn clips_charged(&self) -> u64 {
        self.kit
            .get()
            .hub()
            .ledger()
            .snapshot(Unit::VideoClip)
            .committed
    }

    /// The kit the engine runs on, for the hub's listing.
    pub(crate) fn kit_for_test(&self) -> Kit {
        self.kit.get()
    }

    pub(crate) fn request_scene_for_test(
        self: &Arc<Self>,
        plate_key: &str,
        still: Vec<u8>,
        description: &str,
        reason: &str,
        blocked_by_combat_or_roll: bool,
    ) {
        if blocked_by_combat_or_roll {
            return;
        }
        self.request_scene_clip(plate_key, still, description, reason);
    }

    pub(crate) fn scene_stage_of(&self, plate_key: &str) -> Option<String> {
        let lane = locked(&self.scenes).get(plate_key).cloned()?;
        Some(self.stage_text(&lane))
    }

    pub(crate) fn scene_view_for_test(&self, plate_key: &str) -> SceneClipView {
        self.scene_clip_view(plate_key)
    }

    pub(crate) fn request_kill_for_test(
        self: &Arc<Self>,
        request: kill_shot::KillShotRequest,
        still: Vec<u8>,
    ) {
        self.request_kill_shot(request, still, "/gameplay/imagery/test-still".to_owned());
    }

    pub(crate) fn kill_stage_of(&self, kill_id: &str) -> Option<String> {
        let lane = locked(&self.kill_shots).get(kill_id).cloned()?.lane;
        Some(self.stage_text(&lane))
    }

    pub(crate) fn kill_view_for_test(&self) -> Option<rpc::KillShot> {
        self.kill_shot_for_view()
    }
}

#[cfg(test)]
pub(crate) async fn prepare_scene_still_for_test(still: Vec<u8>) -> Result<Vec<u8>, Failure> {
    prepare_scene_still(still).await
}

#[cfg(test)]
mod prompt_tests {
    use super::home_video_prompt;

    #[test]
    fn home_video_prompt_is_a_photographic_look_of_a_place_with_no_cast() {
        let prompt = home_video_prompt().to_ascii_lowercase();
        for marker in [
            "documentary photograph of a place",
            "real camera",
            "prime lens",
            "natural practical light",
            "natural depth of field",
            "fine film grain",
            "no people and no creatures",
        ] {
            assert!(prompt.contains(marker), "missing {marker:?} in {prompt}");
        }
        // The prompt names only what the photograph is; naming another medium, even to forbid
        // it, steers the model toward it (owner, 2026-10-17).
        for style_word in super::super::imagery::NON_PHOTOGRAPHIC_WORDS {
            assert!(!prompt.contains(style_word), "{style_word:?} in {prompt}");
        }
        for cast_word in ["performers", "costume", "skin pores", "anyone in the frame"] {
            assert!(
                !prompt.contains(cast_word),
                "a loop of a place must not invite {cast_word:?}: {prompt}"
            );
        }
    }

    #[test]
    fn the_home_loop_prompt_asks_for_a_gentle_ambience_loop_and_nothing_abrupt() {
        let prompt = home_video_prompt().to_ascii_lowercase();
        for marker in [
            "locked-off camera",
            "lamplight and candle flicker",
            "drifting smoke or dust",
            "cloth and banners stirring",
            "water shimmer",
            "nothing enters or leaves the frame",
            "loops seamlessly",
            "documentary photograph",
        ] {
            assert!(prompt.contains(marker), "missing {marker:?} in {prompt}");
        }
        assert!(!prompt.contains("push-in"), "the camera does not move");
    }

    #[test]
    fn the_home_loop_spec_names_its_still_frames_prompt_variant_model_duration_and_recipe() {
        use crate::gameplay::resources::ResourceSpec;

        let spec = super::HomeLoopSpec::new(
            "assets/ui/generated/scene-farm",
            super::ClipInput::ready(b"a still", &home_video_prompt()),
            super::clip_recipe(super::Variant::Home),
        );

        let canonical: serde_json::Value = serde_json::from_str(&spec.canonical()).unwrap();

        assert_eq!(canonical["variant"], "home_loop");
        assert_eq!(canonical["prompt_variant"], "ambient");
        assert_eq!(canonical["frames"], "first_and_last_frame_same_still");
        assert_eq!(canonical["model"], super::MODEL);
        assert_eq!(canonical["duration_seconds"], super::DURATION_SECONDS);
        assert_eq!(canonical["recipe"], super::RECIPE);
        assert_eq!(canonical["finish"], "as_delivered");
        assert_eq!(canonical["still"].as_str().map(str::len), Some(64));
        assert_eq!(spec.subject(), "video:home:scene-farm");
    }
}
