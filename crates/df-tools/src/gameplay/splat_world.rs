//! Real splat worlds for places: the generator behind the hub's
//! [`SplatWorld`](super::resources::ResourceKind::SplatWorld) kind.
//!
//! # Flow (World Labs Marble)
//!
//! One submit, a poll loop, then a free PLY export and asset downloads (see
//! `planning/worldlabs-splat-worlds-research.md`): the place's canonical room photograph
//! (JPEG, at most 1600 px) plus a short photographic text prompt built from the place facts
//! are submitted to `marble-1.1`; the operation is polled every 5 s for up to 15 minutes;
//! the 500k PLY is exported and the collider GLB downloaded. Splat files are PLY (or SOG)
//! only: an SPZ URL is never stored or read. Worlds are photographic; no prompt names
//! another medium.
//!
//! # Storage and serving
//!
//! A finished world is stored under the cache dir as
//! `artifacts/cache/splat-worlds/<resource-key>/{world.ply, collider.glb, world.json}` with
//! atomic writes (temp file, then rename). A stored world is reused across runs by key and
//! is never regenerated while present. Once stored, its two files are registered with
//! [`super::splat_assets`] so they are served at once at
//! `/assets/splat/worlds/<key>/world.ply` and `.../collider.glb`.
//!
//! # Decisions and spend
//!
//! The narrative engine (Jev) decides WHEN a world is worth making and WHERE a fight
//! happens; this module only makes a world when asked through [`request_world`]. A fight
//! that starts before its world is ready uses the flat photographic battlefield, so
//! [`ready_world`] simply returns `None` until the world is stored. Credits are recorded
//! in the hub's [`SpendLedger`](super::resources::SpendLedger), which never blocks: there
//! is no spend cap. Every transition is one audit line:
//! `SPLAT_WORLD status=submitted|ready|failed place=<id> key=<k> elapsed_ms=<n>
//! reason=<label>`.
//!
//! No live World Labs call is made here: production wires a `df-providers`
//! `NativeWorldLabs` behind [`WorldMaker`]; tests use loopback fakes only.
use std::{
    collections::HashMap,
    future::Future,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::resources::{
    Generated, Hub, ResourceFailure, ResourceKey, SplatWorldFormat, SplatWorldSpec, Unit,
};

/// Longest side of the JPEG sent to Marble, in pixels.
pub(in crate::gameplay) const MAXIMUM_SOURCE_PIXELS: u32 = 1600;
/// Download bound for a 500k PLY (the proof world is 28,000,362 bytes).
pub(in crate::gameplay) const MAXIMUM_PLY_BYTES: usize = 32 * 1024 * 1024;
/// Download bound for a collider GLB (the proof world is 2,490,376 bytes).
pub(in crate::gameplay) const MAXIMUM_COLLIDER_BYTES: usize = 32 * 1024 * 1024;
const MAXIMUM_PROMPT_FACTS_CHARACTERS: usize = 960;
/// How often a generation is polled.
pub(in crate::gameplay) const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Longest wait for one generation.
pub(in crate::gameplay) const POLL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// The cache directory below the cache root that holds every stored world.
pub(in crate::gameplay) const SPLAT_WORLDS_DIRECTORY: &str = "splat-worlds";
/// The place the seeded proof world belongs to.
pub(in crate::gameplay) const PROOF_PLACE_ID: &str = "place-mareks-cloth-shop";
/// The proof world's Marble identity and scale (from its saved operation JSON).
const PROOF_WORLD_ID: &str = "14303792-4851-4bb8-ab28-ae93eb84d706";
const PROOF_METRIC_SCALE_FACTOR: f64 = 1.5014322;
const PROOF_GROUND_PLANE_OFFSET: f64 = 1.2088797;

/// A Marble model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum MarbleModel {
    Draft,
    Marble11,
}

impl MarbleModel {
    pub(in crate::gameplay) const fn api_name(self) -> &'static str {
        match self {
            Self::Draft => "marble-1.0-draft",
            Self::Marble11 => "marble-1.1",
        }
    }
}

/// A PLY export resolution. Only the 500k export is requested by the generator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PlyResolution {
    HundredK,
    FiveHundredK,
    FullRes,
}

impl PlyResolution {
    pub(in crate::gameplay) const fn api_name(self) -> &'static str {
        match self {
            Self::HundredK => "100k",
            Self::FiveHundredK => "500k",
            Self::FullRes => "full_res",
        }
    }
}

/// The handle of one submitted generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct OperationId(pub(in crate::gameplay) String);

/// One generation request: the place photograph and its photographic prompt.
pub(in crate::gameplay) struct WorldRequest {
    pub(in crate::gameplay) image_jpeg: Vec<u8>,
    pub(in crate::gameplay) text_prompt: String,
    pub(in crate::gameplay) model: MarbleModel,
    pub(in crate::gameplay) display_name: String,
}

/// A finished generation, as the completed operation reports it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct WorldResult {
    pub(in crate::gameplay) world_id: String,
    pub(in crate::gameplay) collider_url: String,
    pub(in crate::gameplay) caption: String,
    pub(in crate::gameplay) metric_scale_factor: Option<f64>,
    pub(in crate::gameplay) ground_plane_offset: Option<f64>,
    pub(in crate::gameplay) credits: u64,
}

/// The state of one polled generation.
pub(in crate::gameplay) enum WorldPoll {
    Pending,
    Done(WorldResult),
    Failed(String),
}

/// Why a maker call did not succeed. `label` is a short audit token, never prose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct WorldMakerError {
    pub(in crate::gameplay) label: &'static str,
    pub(in crate::gameplay) permanent: bool,
}

impl WorldMakerError {
    pub(in crate::gameplay) const fn transient(label: &'static str) -> Self {
        Self {
            label,
            permanent: false,
        }
    }

    pub(in crate::gameplay) const fn permanent(label: &'static str) -> Self {
        Self {
            label,
            permanent: true,
        }
    }

    fn failure(&self) -> ResourceFailure {
        if self.permanent {
            ResourceFailure::Permanent(self.label)
        } else {
            ResourceFailure::Transient(self.label)
        }
    }
}

type MakerFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, WorldMakerError>> + Send + 'a>>;

/// The World Labs port. Its shapes match `df-providers`' `NativeWorldLabs`
/// (`submit`/`poll`/`export_ply`/`download`); production implements this trait for that
/// type once it lands, and tests implement it with a fake. No implementation in this
/// module calls the live API.
pub(in crate::gameplay) trait WorldMaker: Send + Sync {
    fn submit<'a>(&'a self, request: &'a WorldRequest) -> MakerFuture<'a, OperationId>;
    fn poll<'a>(&'a self, operation: &'a OperationId) -> MakerFuture<'a, WorldPoll>;
    fn export_ply<'a>(
        &'a self,
        world_id: &'a str,
        resolution: PlyResolution,
    ) -> MakerFuture<'a, String>;
    fn download<'a>(&'a self, url: &'a str, max_bytes: usize) -> MakerFuture<'a, Vec<u8>>;
}

#[cfg(not(target_arch = "wasm32"))]
impl WorldMaker for df_providers::NativeWorldLabs {
    fn submit<'a>(&'a self, request: &'a WorldRequest) -> MakerFuture<'a, OperationId> {
        Box::pin(async move {
            let provider_request = df_providers::WorldRequest {
                display_name: &request.display_name,
                image_jpeg: &request.image_jpeg,
                text_prompt: &request.text_prompt,
                model: match request.model {
                    MarbleModel::Draft => df_providers::MarbleModel::Draft,
                    MarbleModel::Marble11 => df_providers::MarbleModel::Marble11,
                },
            };
            df_providers::NativeWorldLabs::submit(self, &provider_request)
                .await
                .map(|operation| OperationId(operation.to_string()))
                .map_err(map_provider_error)
        })
    }

    fn poll<'a>(&'a self, operation: &'a OperationId) -> MakerFuture<'a, WorldPoll> {
        Box::pin(async move {
            let operation =
                df_providers::OperationId::new(operation.0.clone()).map_err(map_provider_error)?;
            df_providers::NativeWorldLabs::poll(self, &operation)
                .await
                .map(|poll| match poll {
                    df_providers::WorldPoll::Pending => WorldPoll::Pending,
                    df_providers::WorldPoll::Failed(reason) => WorldPoll::Failed(reason),
                    df_providers::WorldPoll::Done(result) => {
                        match (result.collider_url, result.credits) {
                            (Some(collider_url), Some(credits)) => WorldPoll::Done(WorldResult {
                                world_id: result.world_id,
                                collider_url,
                                caption: result.caption,
                                metric_scale_factor: result.metric_scale_factor,
                                ground_plane_offset: result.ground_plane_offset,
                                credits,
                            }),
                            _ => WorldPoll::Failed("world_metadata_missing".to_owned()),
                        }
                    }
                })
                .map_err(map_provider_error)
        })
    }

    fn export_ply<'a>(
        &'a self,
        world_id: &'a str,
        resolution: PlyResolution,
    ) -> MakerFuture<'a, String> {
        Box::pin(async move {
            let resolution = match resolution {
                PlyResolution::HundredK => df_providers::PlyResolution::K100,
                PlyResolution::FiveHundredK => df_providers::PlyResolution::K500,
                PlyResolution::FullRes => df_providers::PlyResolution::FullRes,
            };
            df_providers::NativeWorldLabs::export_ply(self, world_id, resolution)
                .await
                .map_err(map_provider_error)
        })
    }

    fn download<'a>(&'a self, url: &'a str, max_bytes: usize) -> MakerFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let max_bytes = u64::try_from(max_bytes)
                .map_err(|_| WorldMakerError::permanent("download_bound_invalid"))?;
            df_providers::NativeWorldLabs::download(self, url, max_bytes)
                .await
                .map_err(map_provider_error)
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn map_provider_error(error: df_providers::WorldLabsError) -> WorldMakerError {
    if error.is_transient() {
        WorldMakerError::transient(error.label())
    } else {
        WorldMakerError::permanent(error.label())
    }
}

/// What a ready world offers the battle lanes.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct ReadyWorld {
    pub(in crate::gameplay) ply_url: String,
    pub(in crate::gameplay) collider_url: String,
    pub(in crate::gameplay) metric_scale_factor: Option<f64>,
    pub(in crate::gameplay) ground_plane_offset: Option<f64>,
}

/// The `world.json` stored beside a world's two files.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(in crate::gameplay) struct StoredWorld {
    pub(in crate::gameplay) world_id: String,
    pub(in crate::gameplay) caption: String,
    pub(in crate::gameplay) metric_scale_factor: Option<f64>,
    pub(in crate::gameplay) ground_plane_offset: Option<f64>,
    pub(in crate::gameplay) ply_bytes: u64,
    pub(in crate::gameplay) splats: u64,
    pub(in crate::gameplay) created_unix: u64,
}

fn audit(place_id: &str, key: &ResourceKey, status: &str, elapsed_ms: u128, reason: &str) {
    super::actor::record_audit_line(
        "gameplay.splat_world",
        format!(
            "SPLAT_WORLD status={status} place={place_id} key={key} elapsed_ms={elapsed_ms} \
             reason={reason}"
        ),
    );
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The photographic text prompt for a place, built only from its facts. Every wording is
/// photographic; no other medium is named, even to forbid it.
pub(in crate::gameplay) fn build_text_prompt(place_facts: &[String]) -> String {
    let facts = place_facts
        .iter()
        .map(|fact| fact.trim().trim_end_matches('.'))
        .filter(|fact| !fact.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    if facts.is_empty() {
        "An empty room interior, photographic, natural light.".to_owned()
    } else {
        let facts: String = facts
            .chars()
            .take(MAXIMUM_PROMPT_FACTS_CHARACTERS)
            .collect();
        format!("{facts}, photographic, natural light.")
    }
}

/// JPEG-encodes `bytes` at a longest side of at most [`MAXIMUM_SOURCE_PIXELS`].
pub(in crate::gameplay) fn prepare_source_image(bytes: &[u8]) -> Result<Vec<u8>, ResourceFailure> {
    let image = image::load_from_memory(bytes)
        .map_err(|_| ResourceFailure::Permanent("source_image_unreadable"))?;
    let image = if image.width() > MAXIMUM_SOURCE_PIXELS || image.height() > MAXIMUM_SOURCE_PIXELS {
        image.thumbnail(MAXIMUM_SOURCE_PIXELS, MAXIMUM_SOURCE_PIXELS)
    } else {
        image
    };
    let mut jpeg = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .map_err(|_| ResourceFailure::Permanent("source_image_unreadable"))?;
    Ok(jpeg)
}

/// The number of splats a binary PLY declares (`element vertex <n>`), if it says.
pub(in crate::gameplay) fn ply_splat_count(ply: &[u8]) -> Option<u64> {
    const HEADER_MARKER: &[u8] = b"end_header";
    let end = ply
        .windows(HEADER_MARKER.len())
        .position(|window| window == HEADER_MARKER)?;
    if end + HEADER_MARKER.len() > 4096 {
        return None;
    }
    let header = std::str::from_utf8(&ply[..end + HEADER_MARKER.len()]).ok()?;
    for line in header.lines() {
        if line == "end_header" {
            break;
        }
        if let Some(count) = line
            .strip_prefix("element vertex ")
            .and_then(|rest| rest.trim().parse::<u64>().ok())
        {
            return Some(count);
        }
    }
    None
}

/// The directory one key's world is stored in.
pub(in crate::gameplay) fn world_directory(cache_dir: &Path, key: &ResourceKey) -> PathBuf {
    cache_dir.join(SPLAT_WORLDS_DIRECTORY).join(key.as_str())
}

fn write_atomic(directory: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let staging = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&staging, bytes)?;
    let target = directory.join(name);
    match std::fs::rename(&staging, &target) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _removed = std::fs::remove_file(&staging);
            Err(error)
        }
    }
}

/// Stores a finished world and registers its files for serving.
pub(in crate::gameplay) fn store_world(
    cache_dir: &Path,
    key: &ResourceKey,
    result: &WorldResult,
    ply: &[u8],
    collider: &[u8],
) -> Result<StoredWorld, ResourceFailure> {
    if ply.is_empty()
        || ply.len() > MAXIMUM_PLY_BYTES
        || collider.is_empty()
        || collider.len() > MAXIMUM_COLLIDER_BYTES
    {
        return Err(ResourceFailure::Permanent("world_asset_size_invalid"));
    }
    if let Some(stored) = read_stored_world(cache_dir, key) {
        register_stored_world(key, &world_directory(cache_dir, key));
        return Ok(stored);
    }
    let splats = ply_splat_count(ply).ok_or(ResourceFailure::Permanent("ply_unreadable"))?;
    let stored = StoredWorld {
        world_id: result.world_id.clone(),
        caption: result.caption.clone(),
        metric_scale_factor: result.metric_scale_factor,
        ground_plane_offset: result.ground_plane_offset,
        ply_bytes: ply.len() as u64,
        splats,
        created_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_secs()),
    };
    let json = serde_json::to_vec(&stored)
        .map_err(|_| ResourceFailure::Permanent("world_store_failed"))?;
    let directory = world_directory(cache_dir, key);
    let parent = directory
        .parent()
        .ok_or(ResourceFailure::Permanent("world_store_failed"))?;
    std::fs::create_dir_all(parent)
        .map_err(|_| ResourceFailure::Transient("world_store_failed"))?;
    let staging = parent.join(format!(".{}.{}.tmp", key.as_str(), std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|_| ResourceFailure::Transient("world_store_failed"))?;
    }
    std::fs::create_dir(&staging).map_err(|_| ResourceFailure::Transient("world_store_failed"))?;
    let write_result = write_atomic(&staging, "world.ply", ply)
        .and_then(|()| write_atomic(&staging, "collider.glb", collider))
        .and_then(|()| write_atomic(&staging, "world.json", &json));
    if write_result.is_err() {
        let _removed = std::fs::remove_dir_all(&staging);
        return Err(ResourceFailure::Transient("world_store_failed"));
    }
    if directory.exists() {
        let _removed = std::fs::remove_dir_all(&staging);
        if let Some(existing) = read_stored_world(cache_dir, key) {
            register_stored_world(key, &directory);
            return Ok(existing);
        }
        return Err(ResourceFailure::Permanent("world_store_conflict"));
    }
    if std::fs::rename(&staging, &directory).is_err() {
        let _removed = std::fs::remove_dir_all(&staging);
        if let Some(existing) = read_stored_world(cache_dir, key) {
            register_stored_world(key, &directory);
            return Ok(existing);
        }
        return Err(ResourceFailure::Transient("world_store_failed"));
    }
    register_stored_world(key, &directory);
    Ok(stored)
}

/// Reads a stored world, present only when all three files are there.
pub(in crate::gameplay) fn read_stored_world(
    cache_dir: &Path,
    key: &ResourceKey,
) -> Option<StoredWorld> {
    let directory = world_directory(cache_dir, key);
    if !directory.join("world.ply").is_file() || !directory.join("collider.glb").is_file() {
        return None;
    }
    let json = std::fs::read(directory.join("world.json")).ok()?;
    serde_json::from_slice(&json).ok()
}

fn ready_world_for(cache_dir: &Path, key: &ResourceKey) -> Option<ReadyWorld> {
    let stored = read_stored_world(cache_dir, key)?;
    Some(ReadyWorld {
        ply_url: format!("/assets/splat/worlds/{}/world.ply", key.as_str()),
        collider_url: format!("/assets/splat/worlds/{}/collider.glb", key.as_str()),
        metric_scale_factor: stored.metric_scale_factor,
        ground_plane_offset: stored.ground_plane_offset,
    })
}

fn register_stored_world(key: &ResourceKey, directory: &Path) {
    super::splat_assets::register_world(
        key.as_str(),
        &directory.join("world.ply"),
        &directory.join("collider.glb"),
    );
}

/// Registers every world already stored under `cache_dir`, so a restart serves them.
pub(in crate::gameplay) fn register_stored_worlds(cache_dir: &Path) {
    let worlds = cache_dir.join(SPLAT_WORLDS_DIRECTORY);
    let Ok(entries) = std::fs::read_dir(&worlds) else {
        return;
    };
    for entry in entries.flatten() {
        let directory = entry.path();
        if directory.join("world.ply").is_file() && directory.join("collider.glb").is_file() {
            if let Some(key) = entry.file_name().to_str() {
                super::splat_assets::register_world(
                    key,
                    &directory.join("world.ply"),
                    &directory.join("collider.glb"),
                );
            }
        }
    }
}

/// Poll timing, injected so tests never wait.
#[derive(Clone, Copy)]
pub(in crate::gameplay) struct PollTiming {
    pub(in crate::gameplay) interval: Duration,
    pub(in crate::gameplay) timeout: Duration,
}

impl Default for PollTiming {
    fn default() -> Self {
        Self {
            interval: POLL_INTERVAL,
            timeout: POLL_TIMEOUT,
        }
    }
}

/// Asks `imagery_bytes` for the place's canonical room photograph. The production
/// handle reads the imagery cache's `canonical_room` image for the place; tests pass
/// fixture bytes. `None` means there is no photograph to build a world from.
pub(in crate::gameplay) type RoomPhotographFuture<'a> =
    Pin<Box<dyn Future<Output = Option<Vec<u8>>> + Send + 'a>>;
pub(in crate::gameplay) type RoomPhotograph =
    Arc<dyn for<'a> Fn(&'a str) -> RoomPhotographFuture<'a> + Send + Sync>;

/// The installed splat-world service: one hub, one maker, one photograph source.
pub(in crate::gameplay) struct SplatWorlds {
    hub: Hub,
    maker: Arc<dyn WorldMaker>,
    photograph: RoomPhotograph,
    cache_dir: PathBuf,
    timing: PollTiming,
    places: Mutex<HashMap<String, ResourceKey>>,
}

impl SplatWorlds {
    pub(in crate::gameplay) fn new(
        hub: Hub,
        maker: Arc<dyn WorldMaker>,
        photograph: RoomPhotograph,
        cache_dir: PathBuf,
    ) -> Self {
        Self::with_timing(hub, maker, photograph, cache_dir, PollTiming::default())
    }

    pub(in crate::gameplay) fn with_timing(
        hub: Hub,
        maker: Arc<dyn WorldMaker>,
        photograph: RoomPhotograph,
        cache_dir: PathBuf,
        timing: PollTiming,
    ) -> Self {
        let proof_spec = SplatWorldSpec::new(PROOF_PLACE_ID, Vec::new(), SplatWorldFormat::Ply);
        let proof_key = ResourceKey::of(&proof_spec);
        let mut places = HashMap::new();
        if read_stored_world(&cache_dir, &proof_key).is_some() {
            register_stored_world(&proof_key, &world_directory(&cache_dir, &proof_key));
            places.insert(PROOF_PLACE_ID.to_owned(), proof_key);
        }
        Self {
            hub,
            maker,
            photograph,
            cache_dir,
            timing,
            places: Mutex::new(places),
        }
    }

    /// The hub this service records spend and state in.
    pub(in crate::gameplay) fn hub(&self) -> &Hub {
        &self.hub
    }

    /// The key a place's world is (or would be) stored under.
    pub(in crate::gameplay) fn key_for(&self, spec: &SplatWorldSpec) -> ResourceKey {
        ResourceKey::of(spec)
    }

    /// The stored world for a place, once its generation has finished.
    pub(in crate::gameplay) fn ready_world(&self, place_id: &str) -> Option<ReadyWorld> {
        let key = locked(&self.places).get(place_id).cloned()?;
        ready_world_for(&self.cache_dir, &key)
    }

    /// Asks the hub for a place's world (idempotent per place): a stored world is
    /// registered and returned at once; otherwise one generation is queued. The spec's
    /// facts come from the caller, never from keyword rules.
    pub(in crate::gameplay) fn request_world(&self, spec: &SplatWorldSpec) -> ResourceKey {
        let place_id = spec.place_id().to_owned();
        let mut places = locked(&self.places);
        if let Some(key) = places.get(&place_id) {
            return key.clone();
        }
        let key = ResourceKey::of(spec);
        places.insert(place_id, key.clone());
        drop(places);
        if read_stored_world(&self.cache_dir, &key).is_some() {
            register_stored_world(&key, &world_directory(&self.cache_dir, &key));
        }
        let maker: super::resources::Maker = {
            let service = self.clone_handle();
            let spec = spec.clone();
            Arc::new(move || {
                let service = service.clone();
                let spec = spec.clone();
                Box::pin(async move { service.generate(&spec).await })
            })
        };
        let _status = self.hub.get_or_request(spec, maker);
        key
    }

    fn clone_handle(&self) -> ServiceHandle {
        ServiceHandle {
            hub: self.hub.clone(),
            maker: Arc::clone(&self.maker),
            photograph: Arc::clone(&self.photograph),
            cache_dir: self.cache_dir.clone(),
            timing: self.timing,
        }
    }

    /// Runs the queued generations this service owns.
    pub(in crate::gameplay) async fn run_queued(&self) {
        for job in self.hub.take_jobs(1) {
            self.hub.complete(job.execute().await);
        }
    }

    async fn generate(&self, spec: &SplatWorldSpec) -> Result<Generated, ResourceFailure> {
        self.clone_handle().generate(spec).await
    }
}

/// The owned pieces one generation needs, without the place index.
#[derive(Clone)]
struct ServiceHandle {
    hub: Hub,
    maker: Arc<dyn WorldMaker>,
    photograph: RoomPhotograph,
    cache_dir: PathBuf,
    timing: PollTiming,
}

impl ServiceHandle {
    async fn generate(&self, spec: &SplatWorldSpec) -> Result<Generated, ResourceFailure> {
        let started = Instant::now();
        let key = ResourceKey::of(spec);
        let place_id = spec.place_id();
        if let Some(stored) = read_stored_world(&self.cache_dir, &key) {
            register_stored_world(&key, &world_directory(&self.cache_dir, &key));
            audit(place_id, &key, "ready", 0, "reused");
            return Ok(Generated::new(stored.world_id));
        }
        let outcome = self.make_world(spec, &key).await;
        match &outcome {
            Ok(_generated) => audit(
                place_id,
                &key,
                "ready",
                started.elapsed().as_millis(),
                "stored",
            ),
            Err(failure) => audit(
                place_id,
                &key,
                "failed",
                started.elapsed().as_millis(),
                failure.label(),
            ),
        }
        outcome
    }

    async fn make_world(
        &self,
        spec: &SplatWorldSpec,
        key: &ResourceKey,
    ) -> Result<Generated, ResourceFailure> {
        let place_id = spec.place_id();
        let started = Instant::now();
        let photograph = (self.photograph)(place_id)
            .await
            .ok_or(ResourceFailure::Permanent("no_room_photograph"))?;
        let image_jpeg = prepare_source_image(&photograph)?;
        let request = WorldRequest {
            image_jpeg,
            text_prompt: build_text_prompt(spec.prompt_facts()),
            model: MarbleModel::Marble11,
            display_name: format!("dungeonflux-{}", &key.as_str()[..16]),
        };
        let operation = self
            .maker
            .submit(&request)
            .await
            .map_err(|error| error.failure())?;
        audit(
            place_id,
            key,
            "submitted",
            started.elapsed().as_millis(),
            "accepted",
        );
        let result = self.poll_until_done(&operation).await?;
        // The call was sent and finished: its credits are spent whatever happens next.
        self.hub
            .ledger()
            .reserve(Unit::SplatCredit, result.credits)
            .commit(result.credits);
        let ply_url = self
            .maker
            .export_ply(&result.world_id, PlyResolution::FiveHundredK)
            .await
            .map_err(|error| error.failure())?;
        let ply = self
            .maker
            .download(&ply_url, MAXIMUM_PLY_BYTES)
            .await
            .map_err(|error| error.failure())?;
        if ply_splat_count(&ply).is_none() {
            return Err(ResourceFailure::Permanent("ply_unreadable"));
        }
        let collider = self
            .maker
            .download(&result.collider_url, MAXIMUM_COLLIDER_BYTES)
            .await
            .map_err(|error| error.failure())?;
        if collider.is_empty() {
            return Err(ResourceFailure::Permanent("collider_unreadable"));
        }
        let stored = store_world(&self.cache_dir, key, &result, &ply, &collider)?;
        Ok(Generated {
            handle: stored.world_id,
            note: result.caption,
        })
    }

    async fn poll_until_done(
        &self,
        operation: &OperationId,
    ) -> Result<WorldResult, ResourceFailure> {
        tokio::time::timeout(self.timing.timeout, async {
            loop {
                tokio::time::sleep(self.timing.interval).await;
                match self.maker.poll(operation).await {
                    Ok(WorldPoll::Done(result)) => return Ok(result),
                    Ok(WorldPoll::Failed(_reason)) => {
                        return Err(ResourceFailure::Permanent("generation_failed"));
                    }
                    Ok(WorldPoll::Pending) => {}
                    Err(error) => return Err(error.failure()),
                }
            }
        })
        .await
        .unwrap_or(Err(ResourceFailure::TimedOut))
    }
}

static SERVICE: OnceLock<SplatWorlds> = OnceLock::new();

/// Installs the one splat-world service at startup. A second call keeps the first.
pub(in crate::gameplay) fn install(service: SplatWorlds) -> bool {
    SERVICE.set(service).is_ok()
}

/// The installed service, if startup wired one.
pub(in crate::gameplay) fn service() -> Option<&'static SplatWorlds> {
    SERVICE.get()
}

/// The ready world for a place, for the battle lanes: `None` while it is being made
/// (the flat photographic battlefield stands) or when no service is installed.
pub(in crate::gameplay) fn ready_world(place_id: &str) -> Option<ReadyWorld> {
    SERVICE.get()?.ready_world(place_id)
}

/// Asks the hub for a place's world (idempotent per place). Does nothing when no
/// service is installed; the narrative engine's judgement is the only caller.
pub(in crate::gameplay) fn request_world(spec: &SplatWorldSpec) -> Option<ResourceKey> {
    SERVICE.get().map(|service| service.request_world(spec))
}

/// Dev-only seed: copies the proof world on disk in as the stored world for
/// [`PROOF_PLACE_ID`], so the battle lanes can load a real level without a paid call.
/// Runs only when `DF_GAMEPLAY_SPLAT_SEED=m11` names it; any other value seeds nothing.
pub(in crate::gameplay) fn seed_proof_world(
    cache_dir: &Path,
    proof_directory: &Path,
) -> io::Result<Option<ReadyWorld>> {
    if std::env::var("DF_GAMEPLAY_SPLAT_SEED").as_deref() != Ok("m11") {
        return Ok(None);
    }
    let spec = SplatWorldSpec::new(PROOF_PLACE_ID, Vec::new(), SplatWorldFormat::Ply);
    let key = ResourceKey::of(&spec);
    if read_stored_world(cache_dir, &key).is_none() {
        let ply = std::fs::read(proof_directory.join("m11_500k.ply"))?;
        let collider = std::fs::read(proof_directory.join("m11_collider.glb"))?;
        let result = WorldResult {
            world_id: PROOF_WORLD_ID.to_owned(),
            collider_url: String::new(),
            caption: String::new(),
            metric_scale_factor: Some(PROOF_METRIC_SCALE_FACTOR),
            ground_plane_offset: Some(PROOF_GROUND_PLANE_OFFSET),
            credits: 0,
        };
        store_world(cache_dir, &key, &result, &ply, &collider)
            .map_err(|failure| io::Error::other(failure.label()))?;
    }
    Ok(ready_world_for(cache_dir, &key))
}

#[cfg(test)]
#[path = "splat_world_tests.rs"]
mod tests;
