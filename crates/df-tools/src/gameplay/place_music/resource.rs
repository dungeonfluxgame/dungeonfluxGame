//! Place music as a resource kind of the generation hub (`gameplay/resources.rs`).
//!
//! * [`MusicSetResource`] is the hub's [`ResourceSpec`] for a set: kind `MusicSet`, the
//!   generator version as the contract version, the canonical spec as the description. The hub
//!   key is therefore the identity of a set everywhere (the plan's `spec_key`, the served
//!   route, the audit lines).
//! * [`HubMusic`] implements the music port over the hub: it asks `get_or_request`, maps the
//!   hub's status, drives `invalidate` and `regenerate`, and runs the hub's queued jobs one at
//!   a time on a worker it owns. The hub owns the registry, the one generation in flight per
//!   key, the timeout, the attempt limit, the per-run set cap and the `RESOURCE` audit lines.
//! * [`MusicStore`] holds the mastered MP3 bytes on disk under `<cache>/<key>/`, the way the
//!   imagery cache holds image bytes: the hub entry's handle is the key, and a set found on
//!   disk is not paid for again across runs. The route
//!   `GET /gameplay/music/{key}/{bed|motif|tension|combat|ambience}` serves only keys the hub
//!   reports ready.
//!
//! Spend: there is no budget, cap or limit on place music (the owner's decision). The hub's
//! `SpendLedger` still records every set (unit `MusicCall`, five provider calls), reserved
//! before the calls and committed once they may have gone out, and the hub's `RESOURCE` lines
//! record every request. One attempt per set is not a cap on spending but on blind repeats: a
//! failed set stays failed until `regenerate` asks again.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use axum::{
    Router,
    extract::{Path as RoutePath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bytes::Bytes;
use df_render::art_contract::Neutral;
use serde_json::json;

use super::{
    generator::{
        FailureReason, GENERATOR_VERSION, GeneratedMusicSet, MusicGenerator, MusicSetSpec,
        PROVIDER_CALLS,
    },
    hub::{MusicHub, MusicStatus, ReadyMusicSet},
    master::{Codec, Ffmpeg},
    spec::Layer,
};
use crate::gameplay::resource_adapters::{slug, write_atomic};
use crate::gameplay::resources::{
    Chrome, Generated, Hub, Invalidate, Limits, Maker, Reservation, ResourceFailure, ResourceKey,
    ResourceKind, ResourceSpec, ResourceStatus, Unit,
};

/// Where the display loads a set's files from.
pub(crate) const ROUTE: &str = "/gameplay/music";
const MAXIMUM_TRACK_BYTES: u64 = 6 * 1024 * 1024;
const CACHE_CONTROL: &str = "private, max-age=86400, immutable";
const AMBIENCE: &str = "ambience";
const MANIFEST: &str = "manifest.json";
const DEFAULT_CACHE: &str = "artifacts/tmp/play/music-cache";

pub(super) struct AmbientResource(pub(super) super::ambience::AmbientSpec);

impl AmbientResource {
    pub(super) fn key(&self) -> ResourceKey {
        ResourceKey::of(self)
    }
}

impl ResourceSpec for AmbientResource {
    const KIND: ResourceKind = ResourceKind::AmbientLayer;
    const CONTRACT_VERSION: u32 = 1;
    fn canonical(&self) -> String {
        json!(self.0).to_string()
    }
    fn subject(&self) -> String {
        format!(
            "ambient:{}:{}",
            slug(&self.0.place, 48),
            self.0.kind.label()
        )
    }
    fn chrome(&self) -> Chrome {
        Neutral::Nothing
    }
}

/// One provider call may take this long, including its body.
pub(in crate::gameplay) const CALL_DEADLINE: Duration = Duration::from_secs(120);
/// The whole set (five calls, two at a time, then mastering) must finish within this; the
/// generator ends it with `set: deadline` and every provider call and ffmpeg run gets only the
/// time that remains.
pub(in crate::gameplay) const SET_DEADLINE: Duration = Duration::from_secs(300);
/// The hub's own bound on a job, a little past the generator's so the generator reports first.
const HUB_TIMEOUT: Duration = Duration::from_secs(330);

/// The hub's description of one place's music set.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct MusicSetResource {
    set: MusicSetSpec,
}

impl MusicSetResource {
    pub(in crate::gameplay) fn new(set: MusicSetSpec) -> Self {
        Self { set }
    }

    pub(super) fn key(&self) -> ResourceKey {
        ResourceKey::of(self)
    }
}

impl ResourceSpec for MusicSetResource {
    const KIND: ResourceKind = ResourceKind::MusicSet;
    const CONTRACT_VERSION: u32 = GENERATOR_VERSION;

    fn canonical(&self) -> String {
        json!({"spec": self.set.spec().canonical(), "band": self.set.band().label()}).to_string()
    }

    fn subject(&self) -> String {
        format!("music:{}", slug(&self.set.spec().label(), 48))
    }

    fn chrome(&self) -> Chrome {
        Neutral::Nothing
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Raw `DF_GAMEPLAY_MUSIC*` and `ELEVENLABS_API_KEY` inputs.
pub(in crate::gameplay) struct MusicInputs {
    pub(in crate::gameplay) credential: Result<String, std::env::VarError>,
    pub(in crate::gameplay) mode: Result<String, std::env::VarError>,
    pub(in crate::gameplay) cache: Result<String, std::env::VarError>,
    pub(in crate::gameplay) ffmpeg: Result<String, std::env::VarError>,
}

impl MusicInputs {
    /// The credential is the same `ELEVENLABS_API_KEY` the voice reads.
    pub(in crate::gameplay) fn from_environment() -> Self {
        Self {
            credential: std::env::var("ELEVENLABS_API_KEY"),
            mode: crate::gameplay::media_policy::gates()
                .input(crate::gameplay::media_policy::Capability::Music),
            cache: std::env::var("DF_GAMEPLAY_MUSIC_CACHE"),
            ffmpeg: std::env::var("DF_FFMPEG"),
        }
    }
}

fn present(input: Result<String, std::env::VarError>) -> Option<String> {
    input.ok().filter(|value| !value.is_empty())
}

/// The byte store of generated sets.
pub(in crate::gameplay) struct MusicStore {
    root: PathBuf,
}

impl MusicStore {
    fn new(root: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn set_directory(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }

    /// Whether a complete set is on disk, and whether it has its own ambience. The manifest is
    /// written last, so it marks the set complete.
    fn cached(&self, key: &str) -> Option<&'static str> {
        let directory = self.set_directory(key);
        let complete = directory.join(MANIFEST).is_file()
            && Layer::ALL
                .iter()
                .all(|layer| directory.join(format!("{}.mp3", layer.name())).is_file());
        complete.then(|| {
            if directory.join(format!("{AMBIENCE}.mp3")).is_file() {
                "ambience"
            } else {
                "no_ambience"
            }
        })
    }

    /// Writes each file atomically with the shared helper, the manifest last. A set being
    /// rewritten has no manifest until it is whole again, so a reader never takes it for ready.
    fn write(&self, key: &str, set: &GeneratedMusicSet) -> io::Result<()> {
        let directory = self.set_directory(key);
        std::fs::create_dir_all(&directory)?;
        // Best effort: an old manifest that cannot be removed is replaced below.
        let _unmarked = std::fs::remove_file(directory.join(MANIFEST));
        for (layer, bytes) in Layer::ALL.iter().zip(&set.layers) {
            write_atomic(&directory, &format!("{}.mp3", layer.name()), bytes)?;
        }
        match &set.ambience {
            Some(ambience) => write_atomic(&directory, &format!("{AMBIENCE}.mp3"), ambience)?,
            None => {
                // Best effort: a stale ambience from an earlier set of this key.
                let _removed = std::fs::remove_file(directory.join(format!("{AMBIENCE}.mp3")));
            }
        }
        let manifest = serde_json::to_vec_pretty(&set.manifest).map_err(io::Error::other)?;
        write_atomic(&directory, MANIFEST, &manifest)
    }

    fn remove(&self, key: &str) {
        // Best effort: a set that cannot be removed is regenerated over.
        let _removed = std::fs::remove_dir_all(self.set_directory(key));
    }

    fn read(&self, key: &str, name: &str) -> Option<Bytes> {
        let path = self.set_directory(key).join(format!("{name}.mp3"));
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAXIMUM_TRACK_BYTES {
            return None;
        }
        std::fs::read(path).ok().map(Bytes::from)
    }
}

/// The hub-backed music port and its worker.
pub(in crate::gameplay) struct HubMusic {
    hub: Hub,
    store: Arc<MusicStore>,
    generator: Option<Arc<MusicGenerator>>,
    /// Why generation is off, shown as the fallback reason.
    off_reason: String,
    summary: String,
    /// The last failure detail per key, for the dev status; the hub keeps only its label.
    last_errors: Arc<Mutex<BTreeMap<String, String>>>,
    worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for HubMusic {
    fn drop(&mut self) {
        if let Some(worker) = locked(&self.worker).take() {
            worker.abort();
        }
    }
}

impl HubMusic {
    /// Builds the adapter from validated inputs. With no credential, no ffmpeg or
    /// `DF_GAMEPLAY_MUSIC=off` it still answers, with a Fallback that says why.
    pub(in crate::gameplay) fn start(
        inputs: MusicInputs,
        current: &Path,
        hub: Hub,
    ) -> io::Result<Arc<Self>> {
        let cache =
            PathBuf::from(present(inputs.cache).unwrap_or_else(|| DEFAULT_CACHE.to_owned()));
        let cache = if cache.is_absolute() {
            cache
        } else {
            current.join(cache)
        };
        let store = Arc::new(MusicStore::new(cache.join("sets"))?);
        let ffmpeg = PathBuf::from(present(inputs.ffmpeg).unwrap_or_else(|| "ffmpeg".to_owned()));
        let disabled = match present(inputs.mode).as_deref() {
            None | Some("on") => None,
            Some("off") => Some("DF_GAMEPLAY_MUSIC=off".to_owned()),
            Some(_) => return Err(io::Error::other("DF_GAMEPLAY_MUSIC refused")),
        };
        let credential = present(inputs.credential)
            .filter(|value| value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)));
        let (generator, off_reason) = match (disabled, credential) {
            (Some(reason), _) => (None, reason),
            (None, None) => (None, "ELEVENLABS_API_KEY not set".to_owned()),
            (None, Some(_)) if !Ffmpeg::available(&ffmpeg) => {
                (None, "ffmpeg not found (set DF_FFMPEG)".to_owned())
            }
            (None, Some(credential)) => {
                let codec = Codec::Ffmpeg(Ffmpeg::new(ffmpeg, cache.join("scratch")));
                match MusicGenerator::live(&credential, CALL_DEADLINE, codec) {
                    Ok(generator) => (Some(Arc::new(generator)), String::new()),
                    Err(_) => (None, "music provider configuration refused".to_owned()),
                }
            }
        };
        Self::tune(&hub, &store);
        let summary = if generator.is_some() {
            format!(
                "MUSIC generation on, no budget (x{} provider calls per set) call_deadline={}s set_deadline={}s cache={}",
                super::generator::PROVIDER_CALLS,
                CALL_DEADLINE.as_secs(),
                SET_DEADLINE.as_secs(),
                cache.display()
            )
        } else {
            format!("MUSIC generation off ({off_reason})")
        };
        Ok(Arc::new(Self {
            hub,
            store,
            generator,
            off_reason,
            summary,
            last_errors: Arc::default(),
            worker: Mutex::new(None),
        }))
    }

    /// The kind's bounds and checks, set once at startup: the hub's timeout (a little past the
    /// generator's own), one attempt, and the
    /// freshness check that a ready set is still on disk.
    fn tune(hub: &Hub, store: &Arc<MusicStore>) {
        hub.set_limits(
            ResourceKind::AmbientLayer,
            Limits {
                timeout: Duration::from_secs(180),
                attempts: 1,
            },
        );
        let ambient_store = Arc::clone(store);
        hub.set_validator(
            ResourceKind::AmbientLayer,
            Arc::new(move |generated: &Generated| {
                ambient_store
                    .set_directory(&generated.handle)
                    .join(MANIFEST)
                    .is_file()
                    && ambient_store
                        .set_directory(&generated.handle)
                        .join("ambient.mp3")
                        .is_file()
            }),
        );
        hub.set_limits(
            ResourceKind::MusicSet,
            Limits {
                timeout: HUB_TIMEOUT,
                // A retry would spend five more provider calls; `regenerate` is the way to retry.
                attempts: 1,
            },
        );
        let store = Arc::clone(store);
        hub.set_validator(
            ResourceKind::MusicSet,
            Arc::new(move |generated: &Generated| store.cached(&generated.handle).is_some()),
        );
    }

    pub(in crate::gameplay) fn startup_line(&self) -> &str {
        &self.summary
    }

    /// Starts the worker that runs the hub's queued jobs one at a time and calls `changed`
    /// after each, so watchers show the new plan promptly. A second call does nothing.
    pub(in crate::gameplay) fn start_worker(
        &self,
        runtime: &tokio::runtime::Handle,
        changed: impl Fn() + Send + Sync + 'static,
    ) {
        let mut worker = locked(&self.worker);
        if worker.is_some() {
            return;
        }
        let hub = self.hub.clone();
        *worker = Some(runtime.spawn(async move {
            loop {
                let jobs = hub.take_jobs(1);
                if jobs.is_empty() {
                    hub.wait_for_work().await;
                    continue;
                }
                for job in jobs {
                    hub.complete(job.execute().await);
                    changed();
                }
            }
        }));
    }

    /// `GET /gameplay/music/{key}/{name}`, for keys the hub reports ready only.
    pub(in crate::gameplay) fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/gameplay/music/{key}/{name}", get(serve))
            .with_state(Arc::clone(self))
    }

    fn maker(&self, generator: Arc<MusicGenerator>, set: MusicSetSpec) -> Maker {
        let store = Arc::clone(&self.store);
        let errors = Arc::clone(&self.last_errors);
        let ledger = Arc::clone(self.hub.ledger());
        Arc::new(move || {
            let (generator, store, errors, ledger, set) = (
                Arc::clone(&generator),
                Arc::clone(&store),
                Arc::clone(&errors),
                Arc::clone(&ledger),
                set.clone(),
            );
            Box::pin(async move {
                let key = set.key();
                let probe = (Arc::clone(&store), key.clone());
                let cached = tokio::task::spawn_blocking(move || probe.0.cached(&probe.1))
                    .await
                    .map_err(|_| ResourceFailure::Interrupted)?;
                if let Some(note) = cached {
                    return Ok(Generated {
                        handle: key,
                        note: note.to_owned(),
                    });
                }
                // The ledger records the charge and never refuses it: music has no budget.
                let reservation = ledger.reserve(Unit::MusicCall, u64::from(PROVIDER_CALLS));
                let generated = match generator.generate(&set, SET_DEADLINE).await {
                    Ok(generated) => {
                        settle(reservation, true);
                        generated
                    }
                    Err(failure) => {
                        // Calls may have gone out unless the request was rejected locally.
                        settle(reservation, failure.reason != FailureReason::InvalidRequest);
                        locked(&errors).insert(key, failure.to_string());
                        return Err(if failure.reason.is_permanent() {
                            ResourceFailure::Permanent(failure.reason.label())
                        } else {
                            ResourceFailure::Transient(failure.reason.label())
                        });
                    }
                };
                let note = if generated.ambience.is_some() {
                    "ambience"
                } else {
                    "no_ambience"
                };
                let written = (Arc::clone(&store), key.clone());
                tokio::task::spawn_blocking(move || written.0.write(&written.1, &generated))
                    .await
                    .map_err(|_| ResourceFailure::Interrupted)?
                    .map_err(|_| ResourceFailure::Transient("store_failed"))?;
                locked(&errors).remove(&key);
                Ok(Generated {
                    handle: key,
                    note: note.to_owned(),
                })
            })
        })
    }

    fn mapped(&self, key: &ResourceKey, status: ResourceStatus) -> MusicStatus {
        match status {
            ResourceStatus::Ready(generated) => MusicStatus::Ready(ReadyMusicSet {
                layer_paths: Layer::ALL.map(|layer| format!("{ROUTE}/{key}/{}", layer.name())),
                ambience_path: (generated.note == "ambience")
                    .then(|| format!("{ROUTE}/{key}/{AMBIENCE}")),
            }),
            ResourceStatus::Pending(_) => MusicStatus::Pending,
            ResourceStatus::Failed(failure, _) => MusicStatus::Failed(
                locked(&self.last_errors)
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or_else(|| failure.label().to_owned()),
            ),
            ResourceStatus::Fallback(_) => MusicStatus::Fallback("not requested".to_owned()),
        }
    }
}

impl MusicHub for HubMusic {
    fn ambient_request(
        &self,
        spec: &super::ambience::AmbientSpec,
    ) -> super::ambience::AmbientStatus {
        use super::ambience::AmbientStatus;
        let Some(generator) = &self.generator else {
            return AmbientStatus::Fallback(self.off_reason.clone());
        };
        let resource = AmbientResource(spec.clone());
        let (generator, store, ledger, layer_spec) = (
            Arc::clone(generator),
            Arc::clone(&self.store),
            Arc::clone(self.hub.ledger()),
            spec.clone(),
        );
        let maker: Maker = Arc::new(move || {
            let (generator, store, ledger, spec) = (
                generator.clone(),
                store.clone(),
                ledger.clone(),
                layer_spec.clone(),
            );
            Box::pin(async move {
                let key = spec.key();
                let directory = store.set_directory(&key);
                if directory.join(MANIFEST).is_file() && directory.join("ambient.mp3").is_file() {
                    return Ok(Generated {
                        handle: key,
                        note: "ambient".into(),
                    });
                }
                let reservation = ledger.reserve(Unit::MusicCall, 1);
                let bytes = match generator.ambient_layer(&spec, CALL_DEADLINE).await {
                    Ok(bytes) => {
                        reservation.commit(1);
                        bytes
                    }
                    Err(reason) => {
                        if reason == FailureReason::InvalidRequest {
                            reservation.refund();
                        } else {
                            reservation.commit(1);
                        }
                        return Err(if reason.is_permanent() {
                            ResourceFailure::Permanent(reason.label())
                        } else {
                            ResourceFailure::Transient(reason.label())
                        });
                    }
                };
                tokio::task::spawn_blocking(move || {
                    std::fs::create_dir_all(&directory)?;
                    write_atomic(&directory, "ambient.mp3", &bytes)?;
                    write_atomic(
                        &directory,
                        MANIFEST,
                        json!({"version":1,"spec":spec}).to_string().as_bytes(),
                    )
                })
                .await
                .map_err(|_| ResourceFailure::Interrupted)?
                .map_err(|_| ResourceFailure::Transient("store_failed"))?;
                Ok(Generated {
                    handle: key,
                    note: "ambient".into(),
                })
            })
        });
        self.hub.get_or_request(&resource, maker);
        self.ambient_status(spec)
    }

    fn ambient_status(
        &self,
        spec: &super::ambience::AmbientSpec,
    ) -> super::ambience::AmbientStatus {
        use super::ambience::AmbientStatus;
        if self.generator.is_none() {
            return AmbientStatus::Fallback(self.off_reason.clone());
        }
        let key = AmbientResource(spec.clone()).key();
        match self.hub.status(&key) {
            ResourceStatus::Ready(_) => AmbientStatus::Ready(format!("{ROUTE}/{key}/ambient")),
            ResourceStatus::Pending(_) => AmbientStatus::Pending,
            ResourceStatus::Failed(reason, _) => AmbientStatus::Failed(reason.label().into()),
            ResourceStatus::Fallback(_) => AmbientStatus::Fallback("not requested".into()),
        }
    }
    fn get_or_request(&self, set: &MusicSetSpec) -> MusicStatus {
        let Some(generator) = &self.generator else {
            return MusicStatus::Fallback(self.off_reason.clone());
        };
        let resource = MusicSetResource::new(set.clone());
        let status = self
            .hub
            .get_or_request(&resource, self.maker(Arc::clone(generator), set.clone()));
        self.mapped(&resource.key(), status)
    }

    fn status(&self, set: &MusicSetSpec) -> MusicStatus {
        let key = MusicSetResource::new(set.clone()).key();
        self.mapped(&key, self.hub.status(&key))
    }

    fn invalidate(&self, set: &MusicSetSpec) {
        let key = MusicSetResource::new(set.clone()).key();
        self.hub.invalidate(Invalidate::Key(&key));
        self.store.remove(key.as_str());
        locked(&self.last_errors).remove(key.as_str());
    }

    fn regenerate(&self, set: &MusicSetSpec) {
        let key = MusicSetResource::new(set.clone()).key();
        self.store.remove(key.as_str());
        locked(&self.last_errors).remove(key.as_str());
        if !self.hub.regenerate(&key) {
            self.get_or_request(set);
        }
    }
}

fn is_key(text: &str) -> bool {
    text.len() == 32
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

async fn serve(
    State(music): State<Arc<HubMusic>>,
    RoutePath((key, name)): RoutePath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let known_name = name == AMBIENCE
        || name == "ambient"
        || Layer::ALL.iter().any(|layer| layer.name() == name);
    let ready = is_key(&key)
        && known_name
        && music
            .hub
            .list(Some(if name == "ambient" {
                ResourceKind::AmbientLayer
            } else {
                ResourceKind::MusicSet
            }))
            .iter()
            .any(|row| row.state == "ready" && row.key.as_str() == key);
    if !ready {
        return StatusCode::NOT_FOUND.into_response();
    }
    let store = Arc::clone(&music.store);
    let (read_key, read_name) = (key.clone(), name.clone());
    let bytes = tokio::task::spawn_blocking(move || store.read(&read_key, &read_name))
        .await
        .ok()
        .flatten();
    let Some(bytes) = bytes else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let etag = format!("\"{key}-{name}\"");
    let revalidated = headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|value| value.as_bytes() == etag.as_bytes());
    let head = [
        (header::ETAG, etag),
        (header::CONTENT_TYPE, "audio/mpeg".to_owned()),
        (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
    ];
    if revalidated {
        return (StatusCode::NOT_MODIFIED, head).into_response();
    }
    (StatusCode::OK, head, bytes).into_response()
}

#[cfg(test)]
impl HubMusic {
    /// A test adapter over `hub` with a given generator and store directory.
    pub(in crate::gameplay) fn for_tests(
        hub: Hub,
        root: PathBuf,
        generator: MusicGenerator,
    ) -> Arc<Self> {
        let store = Arc::new(MusicStore::new(root).expect("test store"));
        Self::tune(&hub, &store);
        Arc::new(Self {
            hub,
            store,
            generator: Some(Arc::new(generator)),
            off_reason: String::new(),
            summary: String::new(),
            last_errors: Arc::default(),
            worker: Mutex::new(None),
        })
    }
}

/// Settles a set's five provider calls on the hub's ledger: committed when they may have gone
/// out, refunded when the request never left.
fn settle(reservation: Reservation, sent: bool) {
    if sent {
        reservation.commit(u64::from(PROVIDER_CALLS));
    } else {
        reservation.refund();
    }
}
