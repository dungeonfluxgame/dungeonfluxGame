//! Place music: the server decides, per place, what the adaptive score should sound like, asks
//! the generation hub for a matching set, and tells the display what to play meanwhile.
//!
//! * The spec ([`MusicSpec`]: mood, energy, setting family, time of day, space, palette,
//!   ambience) comes from the pack's `music` block for that place, or else from Jev choosing
//!   among closed labels. No word list, name match or asset-name guess decides anything. With no
//!   block and no Jev answer the place has no spec: the shared score plays with no ambience bed
//!   and the plan says why.
//! * The set (four sample-aligned pacing-band layers and an ambience bed) is a resource of the
//!   generation hub (`hub`): the hub owns the cache, the single in-flight request per key, the
//!   spend ledger, retries and the RESOURCE audit lines. `generator` is the adapter it invokes.
//!   The first view requests the set; a ready set requests its neighbouring pacing bands.
//! * Until the set is ready, or when it failed, the plan is FALLBACK: the shared score plus the
//!   existing bed that the spec (or Jev) named as fitting, if any. The display crossfades to the
//!   place's own layers when the plan turns READY. The tempo engine still decides the mix.
//!
//! Every display view carries the plan (`DisplayGameplayView.place_music`); phones do not.
//! The service is installed once per server process ([`install`]) and applied where the
//! gameplay service finishes a view, so the plan is a presentation layer on top of the
//! authoritative view and changes nothing else in it.

mod ambience;
mod generator;
mod hub;
mod master;
mod pacing;
mod resource;
mod spec;

use generator::MusicSetSpec;
pub(in crate::gameplay) use hub::{MusicHub, MusicStatus, ReadyMusicSet};
pub(in crate::gameplay) use resource::{HubMusic, MusicInputs};
pub(in crate::gameplay) use spec::{MusicSpec, PlaceFacts, SpecSource};
pub(crate) use spec::{PackMusic, SpecError};

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError},
    time::{Duration, Instant},
};

use df_protocol::common as rpc;
use serde_json::Value;

use crate::music_score::{
    BPM, GENERATED_MUSIC_ROUTE, LOOP_SAMPLES, PacingBand, SAMPLE_RATE, SHARED_MUSIC_ROUTE,
};

use super::{approach_model::ApproachModel, pack};

/// Places remembered per run. A place beyond this gets no plan.
const MAXIMUM_PLACES: usize = 256;
/// A place whose spec could not be decided is asked about again no sooner than this.
const UNAVAILABLE_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone)]
enum Place {
    Deciding,
    Decided(Decided),
    Unavailable { reason: String, since: Instant },
}

#[derive(Clone)]
struct Decided {
    spec: MusicSpec,
    source: SpecSource,
}

#[derive(Default)]
struct State {
    entry_memory: crate::music_score::EntryMemory,
    soundscapes: HashMap<String, Option<ambience::Soundscape>>,
    soundscape_status: HashMap<String, String>,
    audio_place: String,
    audio_intensity: u16,
    places: HashMap<String, Place>,
    overrides: HashMap<String, MusicSpec>,
    band: PacingBand,
    playing: HashMap<String, MusicSetSpec>,
    variant_status: HashMap<String, (String, &'static str)>,
    pacing_review: Option<pacing::Review>,
}

struct Inner {
    jev: ApproachModel,
    hub: Option<Arc<dyn MusicHub>>,
    runtime: tokio::runtime::Handle,
    /// Called when a spec is decided, so watchers rebuild their view promptly.
    changed: Box<dyn Fn() + Send + Sync>,
    state: Mutex<State>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The place-music director: a cheap handle shared by the view path and the dev surface.
#[derive(Clone)]
pub(in crate::gameplay) struct PlaceMusic {
    inner: Arc<Inner>,
}

static INSTALLED: OnceLock<PlaceMusic> = OnceLock::new();

/// Installs the process's place-music director. The gameplay service has one per process and
/// its test doubles never install one, so views then carry no plan.
pub(in crate::gameplay) fn install(music: PlaceMusic) {
    // A second install keeps the first: one process, one game.
    let _kept = INSTALLED.set(music);
}

pub(in crate::gameplay) fn installed() -> Option<&'static PlaceMusic> {
    INSTALLED.get()
}

/// Adds the place's music plan to a display view; any other audience is left alone.
pub(in crate::gameplay) fn apply(message: &mut rpc::ViewMessage) {
    if let Some(music) = installed() {
        music.apply(message);
    }
}

/// Projects only decided specs and existing cache status; never starts Jev or generation.
pub(in crate::gameplay) fn apply_read(message: &mut rpc::ViewMessage) {
    if let Some(music) = installed() {
        music.apply_read(message);
    }
}

impl PlaceMusic {
    /// Projects the existing spec/cache only. No resolution or generation side effects.
    fn apply_read(&self, message: &mut rpc::ViewMessage) {
        let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
            return;
        };
        let Some(scene) = &display.scene else {
            return;
        };
        let Ok(decided) = self.decided(&scene.location_id) else {
            return;
        };
        let set = self.playing_set(&scene.location_id, &decided.spec);
        let status = self
            .inner
            .hub
            .as_ref()
            .map_or(MusicStatus::Fallback("no generation hub".into()), |hub| {
                hub.status(&set)
            });
        display.place_music = Some(match status {
            MusicStatus::Ready(ready) => ready_plan(&decided.spec, &set, ready),
            MusicStatus::Pending => fallback_plan(Some(&decided.spec), "pending".into()),
            MusicStatus::Failed(reason) | MusicStatus::Fallback(reason) => {
                fallback_plan(Some(&decided.spec), reason)
            }
        });
        self.project_audio(display);
    }

    /// `hub` is the generation hub; `None` leaves every place on its fallback with that reason.
    pub(in crate::gameplay) fn new(
        jev: ApproachModel,
        hub: Option<Arc<dyn MusicHub>>,
        runtime: tokio::runtime::Handle,
        changed: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                jev,
                hub,
                runtime,
                changed: Box::new(changed),
                state: Mutex::new(State::default()),
            }),
        }
    }

    pub(in crate::gameplay) fn startup_line(&self) -> String {
        match &self.inner.hub {
            Some(_) => "MUSIC on (place sets from the generation hub)".to_owned(),
            None => {
                "MUSIC spec only (no generation hub connected): shared score and beds".to_owned()
            }
        }
    }

    /// Sets the display view's plan from its scene.
    pub(in crate::gameplay) fn apply(&self, message: &mut rpc::ViewMessage) {
        let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
            return;
        };
        let Some(scene) = display.scene.as_ref() else {
            return;
        };
        if scene.location_id.is_empty() {
            return;
        }
        let facts = place_facts(scene);
        display.place_music = self.plan(&facts);
        self.project_audio(display);
    }

    /// Primes a proposed place through the normal Jev and resource-hub path without
    /// publishing a view or changing the active mix. The returned plan reports current
    /// readiness only; pending work completes even if no display visits the place yet.
    /// A scene without a setting has no place to prepare. Public projection errors are
    /// returned before any work is queued.
    pub(in crate::gameplay) fn prepare_scene(
        &self,
        scene: &pack::Scene,
    ) -> Result<Option<rpc::PlaceMusicPlan>, super::adventure::scene_model::SceneModelError> {
        let Some(setting) = &scene.setting else {
            return Ok(None);
        };
        let contents = setting
            .model
            .as_ref()
            .map(|model| model.for_scene(scene.key).public_contents())
            .transpose()?
            .unwrap_or_default();
        let projection = rpc::GameplayScene {
            location_id: setting.place_id.clone(),
            title: scene.title.clone(),
            scene_contents_json: contents,
            ..Default::default()
        };
        Ok(self.plan(&facts_with_setting(&projection, Some(setting))))
    }

    fn plan(&self, facts: &PlaceFacts) -> Option<rpc::PlaceMusicPlan> {
        match self.resolve(facts)? {
            Resolved::Decided(decided) => {
                let plan = self.plan_for(&facts.place_id, &decided.spec);
                self.request_soundscape(facts.clone());
                Some(plan)
            }
            Resolved::Waiting(note) => Some(fallback_plan(None, note)),
        }
    }

    /// The place's spec, deciding it from content at once or starting Jev's choice.
    fn resolve(&self, facts: &PlaceFacts) -> Option<Resolved> {
        let id = facts.place_id.as_str();
        let content = pack::with_content(|pack| {
            pack.music
                .iter()
                .find(|block| block.place_id == id)
                .cloned()
        });
        let (resolved, audit) = {
            let mut state = locked(&self.inner.state);
            if let Some(spec) = state.overrides.get(id) {
                return Some(Resolved::Decided(Decided {
                    spec: spec.clone(),
                    source: SpecSource::Override,
                }));
            }
            match state.places.get(id).cloned() {
                Some(Place::Decided(decided)) => return Some(Resolved::Decided(decided)),
                Some(Place::Deciding) => {
                    return Some(Resolved::Waiting("choosing the music".into()));
                }
                Some(Place::Unavailable { reason, since })
                    if since.elapsed() < UNAVAILABLE_COOLDOWN =>
                {
                    return Some(Resolved::Waiting(format!("no spec: {reason}")));
                }
                Some(Place::Unavailable { .. }) | None => {}
            }
            if state.places.len() >= MAXIMUM_PLACES && !state.places.contains_key(id) {
                return None;
            }
            match content.as_ref().map(PackMusic::spec) {
                Some(Ok(spec)) => {
                    let decided = Decided {
                        spec,
                        source: SpecSource::Content,
                    };
                    state
                        .places
                        .insert(id.to_owned(), Place::Decided(decided.clone()));
                    let line = spec_line(id, &decided, 0);
                    (Resolved::Decided(decided), Some(line))
                }
                Some(Err(error)) => {
                    let reason = error.to_string();
                    state.places.insert(
                        id.to_owned(),
                        Place::Unavailable {
                            reason: reason.clone(),
                            since: Instant::now(),
                        },
                    );
                    let line =
                        format!("MUSIC spec place={id} source=content failed reason=\"{reason}\"");
                    (Resolved::Waiting(format!("no spec: {reason}")), Some(line))
                }
                None => {
                    state.places.insert(id.to_owned(), Place::Deciding);
                    drop(state);
                    self.start_jev(facts.clone());
                    return Some(Resolved::Waiting("choosing the music".into()));
                }
            }
        };
        if let Some(line) = audit {
            super::actor::record_audit_line("gameplay.music.spec", line);
        }
        Some(resolved)
    }

    fn start_jev(&self, facts: PlaceFacts) {
        let inner = Arc::clone(&self.inner);
        self.inner.runtime.spawn(async move {
            let started = Instant::now();
            let outcome = spec::choose_with_jev(&inner.jev, &facts).await;
            let millis = started.elapsed().as_millis();
            let line = {
                let mut state = locked(&inner.state);
                match outcome {
                    Ok(spec) => {
                        let decided = Decided {
                            spec,
                            source: SpecSource::Jev,
                        };
                        let line = spec_line(&facts.place_id, &decided, millis);
                        state
                            .places
                            .insert(facts.place_id.clone(), Place::Decided(decided));
                        line
                    }
                    Err(spec::SpecUnavailable(why)) => {
                        state.places.insert(
                            facts.place_id.clone(),
                            Place::Unavailable {
                                reason: why.label().to_owned(),
                                since: Instant::now(),
                            },
                        );
                        format!(
                            "MUSIC spec place={} source=jev failed reason={} ms={millis}",
                            facts.place_id,
                            why.label()
                        )
                    }
                }
            };
            super::actor::record_audit_line("gameplay.music.spec", line);
            // A prepared destination may have no view watcher. Continue its hub requests
            // after Jev resolves, rather than waiting for the party to arrive.
            let music = Self {
                inner: Arc::clone(&inner),
            };
            if music.decided(&facts.place_id).is_ok() {
                let _plan = music.plan(&facts);
            }
            (inner.changed)();
        });
    }

    /// What the hub says about the spec's set, as a plan. Asking is idempotent: the hub starts
    /// generation on the first request only.
    fn plan_for(&self, place: &str, spec: &MusicSpec) -> rpc::PlaceMusicPlan {
        let Some(hub) = &self.inner.hub else {
            return fallback_plan(Some(spec), "generation hub not connected".into());
        };
        let desired = MusicSetSpec::in_band(spec.clone(), self.band());
        let requested = self.request_variant(place, &desired, hub.as_ref());
        let (set, status) = if matches!(requested, MusicStatus::Ready(_)) {
            (desired, requested)
        } else {
            let playing = self.playing_set(place, spec);
            if playing == desired {
                (desired, requested)
            } else {
                let status = self.request_variant(place, &playing, hub.as_ref());
                (playing, status)
            }
        };
        match status {
            MusicStatus::Ready(ready) => {
                let newly_playing = locked(&self.inner.state)
                    .playing
                    .insert(place.to_owned(), set.clone())
                    .as_ref()
                    != Some(&set);
                if newly_playing {
                    for band in set.band().neighbours() {
                        self.request_variant(
                            place,
                            &MusicSetSpec::in_band(spec.clone(), *band),
                            hub.as_ref(),
                        );
                    }
                }
                ready_plan(spec, &set, ready)
            }
            MusicStatus::Pending => fallback_plan(Some(spec), "generating".into()),
            MusicStatus::Failed(reason) => fallback_plan(Some(spec), format!("failed: {reason}")),
            MusicStatus::Fallback(reason) => {
                fallback_plan(Some(spec), format!("fallback: {reason}"))
            }
        }
    }

    pub(in crate::gameplay) fn band(&self) -> PacingBand {
        locked(&self.inner.state).band
    }

    pub(in crate::gameplay) fn set_band(&self, band: PacingBand) {
        locked(&self.inner.state).band = band;
        (self.inner.changed)();
    }

    fn playing_set(&self, place: &str, spec: &MusicSpec) -> MusicSetSpec {
        let state = locked(&self.inner.state);
        state
            .playing
            .get(place)
            .filter(|set| set.spec() == spec)
            .cloned()
            .unwrap_or_else(|| MusicSetSpec::in_band(spec.clone(), state.band))
    }

    fn request_variant(&self, place: &str, set: &MusicSetSpec, hub: &dyn MusicHub) -> MusicStatus {
        let status = hub.get_or_request(set);
        let record = (set.key(), status.label());
        let changed = locked(&self.inner.state)
            .variant_status
            .insert(format!("{place}:{}", set.band().label()), record.clone())
            != Some(record);
        if changed {
            super::actor::record_audit_line(
                "gameplay.music.variant",
                format!(
                    "MUSIC variant place={place} band={} status={} key={}",
                    set.band().label(),
                    status.label(),
                    set.key(),
                ),
            );
        }
        status
    }

    /// Read-only status of all pacing variants; this never starts generation.
    pub(in crate::gameplay) fn variants(&self, place: &str) -> Result<Value, NoSpec> {
        let decided = self.decided(place)?;
        let playing = locked(&self.inner.state).playing.get(place).cloned();
        Ok(Value::Array(
            PacingBand::ALL
                .into_iter()
                .map(|band| {
                    let set = MusicSetSpec::in_band(decided.spec.clone(), band);
                    let status = self.inner.hub.as_ref().map(|hub| hub.status(&set));
                    serde_json::json!({"band": band.label(), "bpm": band.bpm(),
                "energy_permille": band.energy(), "bars": band.bars(), "loop_samples": LOOP_SAMPLES,
                "set_key": set.key(), "playing": playing.as_ref() == Some(&set),
                "status": status.as_ref().map_or("unavailable", MusicStatus::label)})
                })
                .collect(),
        ))
    }
}

enum Resolved {
    Decided(Decided),
    /// No spec yet (or none possible); the plan is the shared score with this note.
    Waiting(String),
}

fn spec_line(place: &str, decided: &Decided, millis: u128) -> String {
    let set = MusicSetSpec::new(decided.spec.clone());
    format!(
        "MUSIC spec place={place} source={} key={} label=\"{}\" fallback_bed={} ms={millis}",
        decided.source.label(),
        set.key(),
        decided.spec.label(),
        decided
            .spec
            .fallback_bed()
            .map_or("none", crate::music_score::Bed::label),
    )
}

fn base_plan(spec: Option<&MusicSpec>) -> rpc::PlaceMusicPlan {
    rpc::PlaceMusicPlan {
        spec_label: spec.map(MusicSpec::label).unwrap_or_default(),
        setting_family: spec
            .map(|spec| spec.setting().label().to_owned())
            .unwrap_or_default(),
        state: rpc::PlaceMusicState::Fallback as i32,
        spec_key: spec
            .map(|spec| MusicSetSpec::new(spec.clone()).key())
            .unwrap_or_default(),
        layer_paths: Vec::new(),
        ambience_path: String::new(),
        fallback_note: String::new(),
        bpm: BPM as u32,
        loop_samples: LOOP_SAMPLES,
        sample_rate: SAMPLE_RATE,
    }
}

fn fallback_bed_path(spec: &MusicSpec) -> String {
    spec.fallback_bed()
        .map(|bed| format!("{SHARED_MUSIC_ROUTE}{}", bed.track()))
        .unwrap_or_default()
}

/// The shared score plus the bed the spec names, with the reason the place's own set is not
/// playing.
fn fallback_plan(spec: Option<&MusicSpec>, note: String) -> rpc::PlaceMusicPlan {
    rpc::PlaceMusicPlan {
        ambience_path: spec.map(fallback_bed_path).unwrap_or_default(),
        fallback_note: note,
        ..base_plan(spec)
    }
}

fn ready_plan(spec: &MusicSpec, set: &MusicSetSpec, ready: ReadyMusicSet) -> rpc::PlaceMusicPlan {
    debug_assert!(
        ready
            .layer_paths
            .iter()
            .all(|path| path.starts_with(GENERATED_MUSIC_ROUTE)),
        "the hub serves place music under {GENERATED_MUSIC_ROUTE}"
    );
    rpc::PlaceMusicPlan {
        state: rpc::PlaceMusicState::Ready as i32,
        bpm: set.band().bpm(),
        spec_key: set.key(),
        layer_paths: ready.layer_paths.to_vec(),
        ambience_path: ready
            .ambience_path
            .unwrap_or_else(|| fallback_bed_path(spec)),
        ..base_plan(Some(spec))
    }
}

/// The content and view facts Jev is shown. The pack's scene setting supplies the explicit
/// fields; the public view supplies the rest.
fn place_facts(scene: &rpc::GameplayScene) -> PlaceFacts {
    let setting = pack::active()
        .and_then(|loaded| {
            loaded
                .pack
                .scenes
                .iter()
                .filter_map(|authored| authored.setting.as_ref())
                .find(|setting| setting.place_id == scene.location_id)
                .cloned()
        })
        // A place the run created is not in the pack; the session owner published its setting.
        .or_else(|| super::place::registered_setting(&scene.location_id));
    facts_with_setting(scene, setting.as_ref())
}

fn facts_with_setting(
    scene: &rpc::GameplayScene,
    setting: Option<&super::adventure::setting::SceneSetting>,
) -> PlaceFacts {
    let mut contents: Option<Value> = serde_json::from_str(&scene.scene_contents_json).ok();
    if let Some(object) = contents.as_mut().and_then(Value::as_object_mut) {
        object.remove("tempo_audio");
    }
    let field = |name: &str| {
        contents
            .as_ref()
            .and_then(|contents| contents.get(name))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    PlaceFacts {
        place_id: scene.location_id.clone(),
        name: setting
            .as_ref()
            .map_or_else(|| scene.place.clone(), |setting| setting.name.clone()),
        description: format!(
            "{} Public scene: {}",
            setting
                .as_ref()
                .map(|setting| setting.short.clone())
                .unwrap_or_default(),
            contents.as_ref().unwrap_or(&Value::Null)
        ),
        kind: setting
            .as_ref()
            .and_then(|setting| serde_json::to_value(setting.kind).ok())
            .and_then(|kind| kind.as_str().map(str::to_owned))
            .unwrap_or_default(),
        scene_title: scene.title.clone(),
        exterior: setting.as_ref().map(|setting| setting.exterior),
        time_of_day: field("time_of_day"),
        weather: field("weather"),
    }
}

// ---- Dev surface: thin reads and calls into the spec state and the hub ----

/// How a place's spec was decided, and the spec itself.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct SpecReport {
    pub(in crate::gameplay) place_id: String,
    /// `content`, `jev` or `override`.
    pub(in crate::gameplay) source: &'static str,
    pub(in crate::gameplay) label: String,
    /// The set's key, which names it in the hub.
    pub(in crate::gameplay) set_key: String,
    pub(in crate::gameplay) fallback_bed: &'static str,
    pub(in crate::gameplay) spec: Value,
}

/// Why a place has no spec to report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum NoSpec {
    UnknownPlace,
    Deciding,
    /// Neither content nor Jev decided; the shared score plays with no bed.
    Unavailable(String),
}

/// One layer's or the ambience's state, from the hub's report on the whole set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct TrackReport {
    pub(in crate::gameplay) track: &'static str,
    /// `ready`, `pending`, `failed` or `fallback`.
    pub(in crate::gameplay) status: &'static str,
    /// The served path when ready; the last error when failed.
    pub(in crate::gameplay) detail: String,
}

/// A place the director knows, for the list read.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct PlaceSummary {
    pub(in crate::gameplay) place_id: String,
    pub(in crate::gameplay) source: &'static str,
    pub(in crate::gameplay) label: String,
    pub(in crate::gameplay) set_key: String,
    pub(in crate::gameplay) status: &'static str,
}

impl PlaceMusic {
    fn decided(&self, place_id: &str) -> Result<Decided, NoSpec> {
        let state = locked(&self.inner.state);
        if let Some(spec) = state.overrides.get(place_id) {
            return Ok(Decided {
                spec: spec.clone(),
                source: SpecSource::Override,
            });
        }
        match state.places.get(place_id) {
            Some(Place::Decided(decided)) => Ok(decided.clone()),
            Some(Place::Deciding) => Err(NoSpec::Deciding),
            Some(Place::Unavailable { reason, .. }) => Err(NoSpec::Unavailable(reason.clone())),
            None => Err(NoSpec::UnknownPlace),
        }
    }

    /// The place's MusicSpec and how it was decided.
    pub(in crate::gameplay) fn spec_report(&self, place_id: &str) -> Result<SpecReport, NoSpec> {
        let decided = self.decided(place_id)?;
        Ok(SpecReport {
            place_id: place_id.to_owned(),
            source: decided.source.label(),
            label: decided.spec.label(),
            set_key: self.playing_set(place_id, &decided.spec).key(),
            fallback_bed: decided
                .spec
                .fallback_bed()
                .map_or("none", crate::music_score::Bed::label),
            spec: serde_json::to_value(&decided.spec).unwrap_or(Value::Null),
        })
    }

    /// Per-track status of the place's set, from the hub. The four layers and the ambience share
    /// the set's state; a ready set lists each served path.
    pub(in crate::gameplay) fn track_status(
        &self,
        place_id: &str,
    ) -> Result<Vec<TrackReport>, NoSpec> {
        let decided = self.decided(place_id)?;
        let set = self.playing_set(place_id, &decided.spec);
        let status = self
            .inner
            .hub
            .as_ref()
            .map_or(MusicStatus::Fallback("no generation hub".into()), |hub| {
                hub.status(&set)
            });
        let names = ["bed", "motif", "tension", "combat", "ambience"];
        Ok(names
            .iter()
            .enumerate()
            .map(|(index, track)| {
                let (status_label, detail) = match &status {
                    MusicStatus::Ready(ready) => (
                        "ready",
                        if index < 4 {
                            ready.layer_paths[index].clone()
                        } else {
                            ready.ambience_path.clone().unwrap_or_default()
                        },
                    ),
                    MusicStatus::Pending => ("pending", String::new()),
                    MusicStatus::Failed(reason) => ("failed", reason.clone()),
                    MusicStatus::Fallback(reason) => ("fallback", reason.clone()),
                };
                TrackReport {
                    track,
                    status: status_label,
                    detail,
                }
            })
            .collect())
    }

    /// Every place the director has met this run.
    pub(in crate::gameplay) fn places(&self) -> Vec<PlaceSummary> {
        let ids: Vec<String> = {
            let state = locked(&self.inner.state);
            let mut ids: Vec<String> = state
                .places
                .keys()
                .chain(state.overrides.keys())
                .cloned()
                .collect();
            ids.sort();
            ids.dedup();
            ids
        };
        ids.into_iter()
            .map(|place_id| {
                match self
                    .spec_report(&place_id)
                    .and_then(|report| Ok((report, self.decided(&place_id)?)))
                {
                    Ok((report, decided)) => {
                        let status = self.inner.hub.as_ref().map_or("fallback", |hub| {
                            hub.status(&self.playing_set(&place_id, &decided.spec))
                                .label()
                        });
                        PlaceSummary {
                            place_id,
                            source: report.source,
                            label: report.label,
                            set_key: report.set_key,
                            status,
                        }
                    }
                    Err(_) => PlaceSummary {
                        place_id,
                        source: "none",
                        label: String::new(),
                        set_key: String::new(),
                        status: "no_spec",
                    },
                }
            })
            .collect()
    }

    /// Forgets the place's set in the hub so the next view regenerates it.
    pub(in crate::gameplay) fn clear(&self, place_id: &str) -> Result<(), NoSpec> {
        let set = MusicSetSpec::in_band(self.decided(place_id)?.spec, self.band());
        if let Some(hub) = &self.inner.hub {
            hub.invalidate(&set);
        }
        Ok(())
    }

    /// Requests the place's set now; idempotent, bounded by the hub's own limits.
    pub(in crate::gameplay) fn request_now(&self, place_id: &str) -> Result<MusicStatus, NoSpec> {
        let set = MusicSetSpec::in_band(self.decided(place_id)?.spec, self.band());
        Ok(self
            .inner
            .hub
            .as_ref()
            .map_or(MusicStatus::Fallback("no generation hub".into()), |hub| {
                self.request_variant(place_id, &set, hub.as_ref())
            }))
    }

    /// Invalidates and requests the place's set in one step.
    pub(in crate::gameplay) fn regenerate(&self, place_id: &str) -> Result<(), NoSpec> {
        let set = MusicSetSpec::in_band(self.decided(place_id)?.spec, self.band());
        if let Some(hub) = &self.inner.hub {
            hub.regenerate(&set);
            self.request_variant(place_id, &set, hub.as_ref());
        }
        Ok(())
    }

    /// Replaces the place's spec for this run with a `music` block written like the pack's.
    pub(in crate::gameplay) fn override_spec(
        &self,
        place_id: &str,
        block: &PackMusic,
    ) -> Result<(), SpecError> {
        let spec = block.spec()?;
        locked(&self.inner.state)
            .overrides
            .insert(place_id.to_owned(), spec);
        (self.inner.changed)();
        Ok(())
    }

    /// Drops a dev override; the place returns to its content or Jev spec.
    pub(in crate::gameplay) fn clear_override(&self, place_id: &str) {
        locked(&self.inner.state).overrides.remove(place_id);
        (self.inner.changed)();
    }
}

#[cfg(test)]
#[path = "place_music_tests.rs"]
mod tests;

#[cfg(test)]
mod test_support {
    use super::{PackMusic, pack};

    /// The `music` block the shipped merchant-shop pack gives Marek's Cloth Shop, read through
    /// the real pack loader.
    pub(super) fn shop_block() -> PackMusic {
        let loaded = pack::load(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../content/merchant-shop"),
        )
        .expect("the shipped merchant-shop pack loads");
        loaded
            .pack
            .music
            .iter()
            .find(|block| block.place_id == "place-mareks-cloth-shop")
            .cloned()
            .expect("the shop pack has a music block")
    }
}

#[cfg(test)]
#[path = "place_music/generation_tests.rs"]
mod generation_tests;

#[cfg(test)]
mod variant_tests;

#[cfg(test)]
mod preparation_tests;
