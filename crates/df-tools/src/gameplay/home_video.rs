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
//! Spend is charged to a per-run budget (`DF_GAMEPLAY_VIDEO_BUDGET`) before each submit and is
//! never refunded. The live price is read before the first submit of a run and logged.
use std::{
    collections::BTreeMap,
    io,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

use axum::{
    Router,
    body::Body,
    extract::Path as RoutePath,
    http::{HeaderMap, StatusCode, header},
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
    NativeFalMediaProvider, NativeHttpLimits, registered_fal_media_provider,
};
use df_types::{
    OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage, UsageUnit,
};
use sha2::{Digest, Sha256};

use super::home_theme;
use super::living_scenes::LivingScenes;
use super::rpc;
use crate::BUILD_ID;

/// The endpoint the server requests. Only endpoints the adapter has a body and checks for are
/// accepted as the setting.
pub(crate) const MODEL: &str = df_providers::KLING_O1_IMAGE_TO_VIDEO;
/// The ambient-motion prompt from the PLAY-V04 spike. It names no film, franchise, studio or person.
const PROMPT: &str = "Locked-off camera with an almost imperceptible push-in. Ambient motion only: lantern and window light flicker, drifting mist over the water, slow clouds across the moon, water glints, leaves or banners stirring. No new people, creatures, objects, text or logos. Nothing enters or leaves the frame. The composition stays exactly as in the reference image.";
const DURATION_SECONDS: u32 = 5;
/// Changing how a clip is requested or cached changes this, so old loops are not reused for it.
const RECIPE: &str = "df-home-video/1";
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const STILL_QUALITY: u8 = 95;
const DEFAULT_BUDGET: u32 = 4;
const MAXIMUM_BUDGET: u32 = 1_000;
const DEFAULT_CACHE: &str = "artifacts/tmp/play/home-video-cache";
/// The fal media hosts the download is pinned to. The adapter refuses any other host.
const CDN_HOSTS: [&str; 2] = ["v3.fal.media", "v3b.fal.media"];
const MAXIMUM_CLIP_BYTES: usize = 16 * 1024 * 1024;
const MINIMUM_CLIP_MILLISECONDS: u128 = 4_500;
const MAXIMUM_CLIP_MILLISECONDS: u128 = 5_500;
const POLL_INTERVAL: Duration = Duration::from_secs(4);
const DEFAULT_DEADLINE: Duration = Duration::from_secs(15 * 60);
const REQUEST_DEADLINE: Duration = Duration::from_secs(170);
const PRICE_DEADLINE: Duration = Duration::from_secs(20);
const ROUTE: &str = "/gameplay/home-video";

/// The raw `DF_GAMEPLAY_HOME_VIDEO*` and `FAL_KEY` inputs.
pub(crate) struct HomeVideoInputs {
    pub(crate) mode: Result<String, std::env::VarError>,
    pub(crate) model: Result<String, std::env::VarError>,
    pub(crate) budget: Result<String, std::env::VarError>,
    pub(crate) cache: Result<String, std::env::VarError>,
    pub(crate) credential: Result<String, std::env::VarError>,
}

impl HomeVideoInputs {
    pub(crate) fn from_environment() -> Self {
        Self {
            mode: std::env::var("DF_GAMEPLAY_HOME_VIDEO"),
            model: std::env::var("DF_GAMEPLAY_HOME_VIDEO_MODEL"),
            budget: std::env::var("DF_GAMEPLAY_VIDEO_BUDGET"),
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
    pub(crate) credential: FalCredential,
    pub(crate) model: &'static str,
    /// Clips the run may request, charged before each submit.
    pub(crate) budget: u32,
    pub(crate) cache_dir: PathBuf,
}

#[derive(Debug)]
pub(crate) enum HomeVideoSettings {
    Off(HomeVideoOff),
    On(HomeVideoConfiguration),
}

/// A setting that is present and invalid is refused by name, never echoed.
fn refused(name: &str) -> io::Error {
    io::Error::other(format!("{name} invalid"))
}

/// Validates the home video settings. Off is a state, not an error: a missing key or a setting
/// of `off` turns the feature off with a reason the startup line states.
pub(crate) fn home_video_settings(
    inputs: HomeVideoInputs,
    current: &Path,
    qualification: bool,
) -> Result<HomeVideoSettings, io::Error> {
    if qualification {
        return Ok(HomeVideoSettings::Off(HomeVideoOff::Qualification));
    }
    let mode = match inputs.mode {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => String::new(),
        Err(_) => return Err(refused("DF_GAMEPLAY_HOME_VIDEO")),
    };
    let credential = match inputs.credential {
        Ok(value) if !value.is_empty() => value,
        Err(std::env::VarError::NotPresent) | Ok(_) => String::new(),
        Err(_) => return Err(refused("FAL_KEY")),
    };
    match mode.as_str() {
        "off" => return Ok(HomeVideoSettings::Off(HomeVideoOff::Disabled)),
        "" | "on" => {}
        _ => return Err(refused("DF_GAMEPLAY_HOME_VIDEO")),
    }
    if credential.is_empty() {
        return Ok(HomeVideoSettings::Off(HomeVideoOff::NoCredential));
    }
    if credential.len() > 8192 || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err(refused("FAL_KEY"));
    }
    let model = match inputs.model {
        Ok(value) if value.is_empty() => MODEL,
        Ok(value) if value == MODEL => MODEL,
        Ok(_) => return Err(refused("DF_GAMEPLAY_HOME_VIDEO_MODEL")),
        Err(std::env::VarError::NotPresent) => MODEL,
        Err(_) => return Err(refused("DF_GAMEPLAY_HOME_VIDEO_MODEL")),
    };
    let budget = match inputs.budget {
        Ok(value) if !value.is_empty() => value
            .parse::<u32>()
            .ok()
            .filter(|budget| *budget <= MAXIMUM_BUDGET)
            .ok_or_else(|| refused("DF_GAMEPLAY_VIDEO_BUDGET"))?,
        Ok(_) | Err(std::env::VarError::NotPresent) => DEFAULT_BUDGET,
        Err(_) => return Err(refused("DF_GAMEPLAY_VIDEO_BUDGET")),
    };
    let cache_dir = match inputs.cache {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) | Err(std::env::VarError::NotPresent) => current.join(DEFAULT_CACHE),
        Err(_) => return Err(refused("DF_GAMEPLAY_HOME_VIDEO_CACHE")),
    };
    Ok(HomeVideoSettings::On(HomeVideoConfiguration {
        credential: FalCredential(credential),
        model,
        budget,
        cache_dir,
    }))
}

/// The one startup line for the feature, printed with the other services.
pub(crate) fn startup_line(settings: &HomeVideoSettings) -> String {
    match settings {
        HomeVideoSettings::On(configuration) => format!(
            "HOME VIDEO on model={} budget={} duration={}s",
            configuration.model, configuration.budget, DURATION_SECONDS
        ),
        HomeVideoSettings::Off(reason) => format!("HOME VIDEO off ({})", reason.reason()),
    }
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

/// Why a loop was not made. The label is the diagnostic; the still stays on screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    Still(&'static str),
    Price(&'static str),
    BudgetSpent,
    Provider(String),
    Timeout,
    Clip(&'static str),
}

impl Failure {
    fn label(&self) -> String {
        match self {
            Self::Still(reason) => format!("still: {reason}"),
            Self::Price(reason) => format!("price: {reason}"),
            Self::BudgetSpent => "budget spent: no clip charged".to_owned(),
            Self::Provider(reason) => format!("provider: {reason}"),
            Self::Timeout => "timeout".to_owned(),
            Self::Clip(reason) => format!("clip: {reason}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Stage {
    Pending,
    Ready(String),
    Failed(Failure),
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
    credential: String,
    endpoints: Endpoints,
    deadline: Duration,
    budget: AtomicU32,
    stages: Mutex<BTreeMap<String, Stage>>,
    price: tokio::sync::Mutex<Option<f64>>,
    living: Option<LivingScenes>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HomeVideoEngine")
            .field("cache_dir", &self.cache_dir)
            .finish_non_exhaustive()
    }
}

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();

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
    let configuration = match settings {
        HomeVideoSettings::Off(reason) => return Ok(startup_line(&HomeVideoSettings::Off(reason))),
        HomeVideoSettings::On(configuration) => configuration,
    };
    let pins = resolve_pins();
    if pins.is_empty() {
        return Ok(startup_line(&HomeVideoSettings::Off(HomeVideoOff::NoHost)));
    }
    let line = startup_line(&HomeVideoSettings::On(HomeVideoConfiguration {
        credential: FalCredential(String::new()),
        model: configuration.model,
        budget: configuration.budget,
        cache_dir: configuration.cache_dir.clone(),
    }));
    let engine = Engine {
        root: asset_root.to_path_buf(),
        cache_dir: configuration.cache_dir,
        credential: configuration.credential.0,
        endpoints: Endpoints::Live { pins },
        deadline: DEFAULT_DEADLINE,
        budget: AtomicU32::new(configuration.budget),
        stages: Mutex::new(BTreeMap::new()),
        price: tokio::sync::Mutex::new(None),
        living: Some(living),
    };
    ENGINE
        .set(Arc::new(engine))
        .map_err(|_| io::Error::other("home video already installed"))?;
    Ok(line)
}

/// Resolves each pinned fal media host once, at startup. Only public addresses are kept.
fn resolve_pins() -> Vec<FalCdnPin> {
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
    match ENGINE.get() {
        Some(engine) => engine.observe(painting),
        None => LoopView::None,
    }
}

/// Sets the home loop on a view: the state and the served path, for the display's lobby only. It
/// runs on every view the service hands out, so a loop that lands is seen by the next view, and
/// the journey projection cached by revision does not hold a stale state.
pub(crate) fn apply(message: &mut rpc::ViewMessage) {
    let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
        return;
    };
    // The lobby's painting is the audience's own `scene_asset`, the one the stage draws.
    let painting = display.scene_asset.clone();
    let Some(journey) = display.journey.as_mut() else {
        return;
    };
    let lobby = journey.phase == rpc::JourneyPhase::Room as i32;
    let loop_view = if lobby && !painting.is_empty() {
        observe(&painting)
    } else {
        LoopView::None
    };
    journey.home_video_state = loop_view.proto_state() as i32;
    journey.home_video_path = loop_view.path();
}

/// Serves the cached loops as `GET /gameplay/home-video/<key>`.
pub(crate) fn router() -> Router {
    Router::new().route("/gameplay/home-video/{key}", get(serve_clip))
}

impl Engine {
    fn observe(self: &Arc<Self>, painting: &str) -> LoopView {
        let Ok(mut stages) = self.stages.lock() else {
            return LoopView::None;
        };
        let stage = match stages.get(painting) {
            Some(stage) => stage.clone(),
            None => {
                stages.insert(painting.to_owned(), Stage::Pending);
                drop(stages);
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    let engine = Arc::clone(self);
                    let painting = painting.to_owned();
                    handle.spawn(async move { engine.fly(painting).await });
                }
                return LoopView::Pending;
            }
        };
        match stage {
            Stage::Pending => LoopView::Pending,
            Stage::Ready(key) => LoopView::Ready {
                path: format!("{ROUTE}/{key}"),
            },
            Stage::Failed(_) => LoopView::Failed,
        }
    }

    /// The one task of a painting: prepare the still, serve a cached loop or make one, then
    /// record the outcome and wake the displays.
    async fn fly(self: Arc<Self>, painting: String) {
        let outcome = self.produce(&painting).await;
        let stage = match outcome {
            Ok(key) => Stage::Ready(key),
            Err(failure) => Stage::Failed(failure),
        };
        if let Ok(mut stages) = self.stages.lock() {
            stages.insert(painting, stage);
        }
        // The display streams watch the home wake, and the living scenes' wake when it has one.
        WAKE.get_or_init(|| watch::channel(0).0)
            .send_modify(|count| *count += 1);
        if let Some(living) = &self.living {
            living.signal_art();
        }
    }

    async fn produce(&self, painting: &str) -> Result<String, Failure> {
        let still = prepare_still(&self.root, painting).await?;
        let key = clip_key(&still);
        if cached_clip(&self.cache_dir, &key).is_some() {
            record_event("cached", &key, None);
            return Ok(key);
        }
        let provider = self.provider()?;
        let price = self.price(&provider).await?;
        self.charge()?;
        let started = Instant::now();
        let outcome = self.make(&provider, &still, &key).await;
        let elapsed = started.elapsed();
        match &outcome {
            Ok(()) => record_paid(&key, price, elapsed, "ready"),
            Err(failure) => record_paid(&key, price, elapsed, &failure.label()),
        }
        outcome.map(|()| key)
    }

    fn provider(&self) -> Result<NativeFalMediaProvider, Failure> {
        let config = match &self.endpoints {
            Endpoints::Live { pins } => FalMediaConfig::live(FalMediaFamily::Video, pins.clone()),
            Endpoints::Fixture { api, media } => {
                FalMediaConfig::loopback_fixture(FalMediaFamily::Video, *api, *media)
                    .map_err(|_| Failure::Provider("fixture address refused".to_owned()))?
            }
        };
        let config = config
            .with_video_model(MODEL)
            .map_err(|_| Failure::Provider("model refused".to_owned()))?;
        registered_fal_media_provider(config, &self.credential, limits())
            .map_err(|error| Failure::Provider(format!("{:?}", error.reason)))
    }

    /// The live per-second price, read once per run before its first submit and logged.
    async fn price(&self, provider: &NativeFalMediaProvider) -> Result<f64, Failure> {
        let mut known = self.price.lock().await;
        if let Some(price) = *known {
            return Ok(price);
        }
        let price = provider
            .video_price_per_second(PRICE_DEADLINE, std::future::pending())
            .await
            .map_err(|_| Failure::Price("live price unavailable"))?;
        record_event("price", MODEL, Some(price));
        *known = Some(price);
        Ok(price)
    }

    /// Charges one clip before the submit. Spend is never refunded, even when the clip fails.
    fn charge(&self) -> Result<(), Failure> {
        self.budget
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .map(|_| ())
            .map_err(|_| Failure::BudgetSpent)
    }

    async fn make(
        &self,
        provider: &NativeFalMediaProvider,
        still: &[u8],
        key: &str,
    ) -> Result<(), Failure> {
        let (request, reference) = video_request(still, key)?;
        let bytes =
            tokio::time::timeout(self.deadline, self.run_job(provider, &request, &reference))
                .await
                .map_err(|_| Failure::Timeout)??;
        write_clip(&self.cache_dir, key, &bytes).await
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
                if (width, height) != (WIDTH, HEIGHT)
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

/// The adapter's reason, with the queue request it concerned: the diagnostic that names a job
/// the provider may still be running.
fn provider_failure(error: FalMediaError) -> Failure {
    match error.request_id {
        Some(id) => Failure::Provider(format!("{:?} request={}", error.reason, id.as_str())),
        None => Failure::Provider(format!("{:?}", error.reason)),
    }
}

fn limits() -> NativeHttpLimits {
    NativeHttpLimits {
        connect_timeout: Duration::from_secs(10),
        read_timeout: Duration::from_secs(60),
        total_timeout: REQUEST_DEADLINE,
        maximum_request_bytes: 8 * 1024 * 1024,
        // The adapter's ceiling for a response; the clip bound below is the same size.
        maximum_response_bytes: 16 * 1024 * 1024,
        maximum_header_bytes: 16 * 1024,
        maximum_chunks: 65_536,
    }
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

/// The request for one still: the still as both frames, five seconds, the adapter's own profile
/// for the endpoint.
fn video_basis(still: &[u8], key: &str) -> Result<RequestBinding<FalVideoBasis>, Failure> {
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
        prompt: PROMPT.to_owned(),
        reference_frames: vec![FalReferenceFrame {
            asset: AssetReference {
                key: RevisionLabel::new(Some("home-still")).map_err(request_refused)?,
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
            rights_revision: RevisionLabel::new(Some("home-video")).map_err(request_refused)?,
            output_format_revision: RevisionLabel::new(Some(RECIPE)).map_err(request_refused)?,
            semantic,
        },
        mode: ExecutionMode::Live,
        deadline: REQUEST_DEADLINE,
    };
    Ok(binding)
}

/// The request for one still, and the binding it was checked against (the owner's view of it).
fn video_request(
    still: &[u8],
    key: &str,
) -> Result<(FalVideoRequest, RequestBinding<FalVideoBasis>), Failure> {
    let reference = video_basis(still, key)?;
    let request = FalVideoRequest::new(
        video_basis(still, key)?,
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

fn base64_encode(bytes: &[u8]) -> String {
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

#[cfg(test)]
pub(crate) async fn prepare_still_for_test(
    root: &Path,
    painting: &str,
) -> Result<Vec<u8>, Failure> {
    prepare_still(root, painting).await
}

/// The cache key: SHA-256 over the still, the model, the prompt, the duration and the recipe.
pub(crate) fn clip_key(still: &[u8]) -> String {
    clip_key_with(still, MODEL, PROMPT, DURATION_SECONDS, RECIPE)
}

pub(crate) fn clip_key_with(
    still: &[u8],
    model: &str,
    prompt: &str,
    duration_seconds: u32,
    recipe: &str,
) -> String {
    let mut hasher = Sha256::new();
    let duration = duration_seconds.to_string();
    for part in [
        still,
        model.as_bytes(),
        prompt.as_bytes(),
        duration.as_bytes(),
        recipe.as_bytes(),
    ] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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

/// Writes a validated clip beside its key, through a temporary name so a reader never sees half a
/// file. The write is blocking I/O, so it runs off the async workers.
async fn write_clip(cache_dir: &Path, key: &str, bytes: &[u8]) -> Result<(), Failure> {
    let directory = cache_dir.to_path_buf();
    let key = key.to_owned();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&directory)
            .map_err(|_| Failure::Clip("cache directory unavailable"))?;
        let path = clip_path(&directory, &key);
        let temporary = directory.join(format!(".{key}.{}.tmp", std::process::id()));
        std::fs::write(&temporary, &bytes).map_err(|_| Failure::Clip("cache write failed"))?;
        std::fs::rename(&temporary, &path).map_err(|_| Failure::Clip("cache rename failed"))
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

async fn serve_clip(RoutePath(key): RoutePath<String>, headers: HeaderMap) -> Response {
    let Some(engine) = ENGINE.get() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    respond(&engine.cache_dir, &key, &headers).await
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
fn record_paid(key: &str, price: f64, elapsed: Duration, outcome: &str) {
    let mut span = df_observe::begin(&context(), "gameplay.home_video_render");
    span.event(
        "gameplay.home_video.paid",
        &[
            ("gameplay.home_video.model", MODEL.to_owned()),
            ("gameplay.home_video.key", key[..16].to_owned()),
            (
                "gameplay.home_video.price_usd_per_second",
                format!("{price:.4}"),
            ),
            (
                "gameplay.home_video.seconds",
                format!("{:.1}", elapsed.as_secs_f64()),
            ),
            (
                "gameplay.home_video.estimated_usd",
                format!("{:.3}", price * f64::from(DURATION_SECONDS)),
            ),
        ],
    );
    span.finish_unmeasured(if outcome == "ready" {
        "ready"
    } else {
        "failed"
    });
}

fn record_event(kind: &str, subject: &str, price: Option<f64>) {
    let mut span = df_observe::begin(&context(), "gameplay.home_video_event");
    let mut fields = vec![
        ("gameplay.home_video.event", kind.to_owned()),
        (
            "gameplay.home_video.subject",
            subject.chars().take(16).collect(),
        ),
    ];
    if let Some(price) = price {
        fields.push((
            "gameplay.home_video.price_usd_per_second",
            format!("{price:.4}"),
        ));
    }
    span.event("gameplay.home_video.event", &fields);
    span.finish_unmeasured("recorded");
}

/// Test helper: a flight's outcome for a still, without the display.
#[cfg(test)]
pub(crate) fn test_engine(
    root: PathBuf,
    cache_dir: PathBuf,
    budget: u32,
    endpoints: Endpoints,
    deadline: Duration,
) -> Arc<Engine> {
    Arc::new(Engine {
        root,
        cache_dir,
        credential: "fixture-private-key".to_owned(),
        endpoints,
        deadline,
        budget: AtomicU32::new(budget),
        stages: Mutex::new(BTreeMap::new()),
        price: tokio::sync::Mutex::new(None),
        living: None,
    })
}

/// Test helper: installs the engine the view and the route use. Only one test installs one.
#[cfg(test)]
pub(crate) fn install_for_test(engine: Arc<Engine>) -> Result<(), Arc<Engine>> {
    ENGINE.set(engine)
}

/// Test helper for the live check: the same engine with the live endpoints and a given credential.
#[cfg(test)]
pub(crate) fn live_test_engine(
    root: PathBuf,
    cache_dir: PathBuf,
    budget: u32,
    credential: String,
    pins: Vec<FalCdnPin>,
    deadline: Duration,
) -> Arc<Engine> {
    Arc::new(Engine {
        root,
        cache_dir,
        credential,
        endpoints: Endpoints::Live { pins },
        deadline,
        budget: AtomicU32::new(budget),
        stages: Mutex::new(BTreeMap::new()),
        price: tokio::sync::Mutex::new(None),
        living: None,
    })
}

#[cfg(test)]
impl Engine {
    pub(crate) fn stage_of(&self, painting: &str) -> Option<String> {
        self.stages.lock().ok().and_then(|stages| {
            stages.get(painting).map(|stage| match stage {
                Stage::Pending => "pending".to_owned(),
                Stage::Ready(key) => format!("ready:{key}"),
                Stage::Failed(failure) => format!("failed:{}", failure.label()),
            })
        })
    }

    pub(crate) fn observe_for_test(self: &Arc<Self>, painting: &str) -> LoopView {
        self.observe(painting)
    }

    pub(crate) fn budget_left(&self) -> u32 {
        self.budget.load(Ordering::SeqCst)
    }
}
