//! Scene plates: one composed film still for every story scene, made just in time (PLAY-V09).
//!
//! A plate is the whole scene as one photographic picture: the heroes seen from behind, the
//! speaker the lit focal point, extras and foreground objects, one light. It replaces the painted
//! backdrop with separate cut-outs on the shared display (the display draws those cut-outs only
//! when a plate has failed or none is planned; until the plate is ready the scene shows its
//! establishing shot, PLAY-V12).
//!
//! The director here watches the display's view. The moment a scene becomes current it requests
//! that scene's plate, and the plates of the scenes that can follow it (the beat script knows the
//! successors, a won fight included), so a successor is usually ready when the party arrives.
//! Nothing in the game waits for a plate or changes with it, and a
//! request that cannot be queued is dropped and made again by a later view. When the mood tracker
//! settles on a lasting face for the speaker, the current plate is edited (the experiments showed
//! an edit keeps the room and a fresh render re-stages it) instead of re-drawing the cut-out.
//!
//! The one worker task owns every render it starts (a `JoinSet`), ends with the last handle, and
//! reports each outcome to the plate book, never to the game.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use df_protocol::common as rpc;
use futures::{StreamExt, stream::FuturesUnordered};
use sha2::Digest;
use tokio::{sync::mpsc, task::JoinSet};

use super::{
    super::{
        beat_script::{self, Beat, BeatKind, BeatScript, BeatTransition, BeatTrigger},
        imagery::{
            CastIdentity, ImageFailure, ImageKey, ImageKind, ImageRequest, Lookup, OutputSpec,
            PlaceFacts, PlateReferences, ReferenceImage, cast_sheet, hero_locator_phrase,
            party_sheet, plate_moment, scene_plate,
        },
        pack::{self, Age, Gender, SceneKey},
        scene_mood::Face,
        shot::{
            self, ActorHead, HeroHead, PlateLayout, PlatePlacement, ShotError, ShotInput, ShotSpec,
            SpeakerInput,
        },
    },
    Inner, WorkerGuard, locked, record,
};

mod direct;
mod state_images;

/// Plates kept in the book; the oldest not in use are forgotten first.
const MAXIMUM_ENTRIES: usize = 32;
/// Requests that may wait for the worker; a further one is dropped and made again later.
const QUEUE_CAPACITY: usize = 16;
/// The most successor scenes whose plates are requested ahead of time.
const MAXIMUM_SUCCESSORS: usize = 6;
/// The image edit adapter accepts at most four reference images. Keep the party, speaker and
/// place references first; additional cast figures are useful context but are already named in
/// the prompt and must not make a valid plate request fail locally.
const MAXIMUM_PLATE_REFERENCES: usize = 4;
/// A plate for a scene that may come next waits this long before it is made, so the plate of the
/// scene on screen is always the first to reach the image service (the service runs two calls at
/// a time and serves them in the order they arrive).
const AHEAD_DELAY: Duration = Duration::from_secs(2);
/// How often the worker looks for a lasting face to edit into the current plate.
const POLL: Duration = Duration::from_secs(1);

/// What the plate director needs to know about one scene, from the pack.
#[derive(Clone, Debug)]
struct PlateScene {
    title: String,
    key: SceneKey,
    source_variant_id: Option<String>,
    asset: String,
    place_id: String,
    place_kind: String,
    /// The adventure record's `exterior` for the place.
    exterior: bool,
    place: Option<PlaceFacts>,
    story_text: String,
    speaker: Option<Speaker>,
    others: Vec<IdentityReference>,
    cast: Vec<pack::SceneCastMember>,
    identity_cast: Vec<IdentityReference>,
    snapshot: Option<crate::gameplay::adventure::scene_model::SceneNow>,
    hero_names: Vec<String>,
    authored_room: Option<crate::gameplay::adventure::scene_model::SceneModel>,
    room_revision: Option<String>,
}

#[derive(Clone, Debug)]
struct IdentityReference {
    id: String,
    name: String,
    sprite: String,
    portrait: String,
}

#[derive(Clone, Debug)]
struct Speaker {
    id: String,
    sprite: String,
    gender: Option<Gender>,
    age: Option<Age>,
}

impl PlateScene {
    /// `None` for a scene that never gets a plate: the lobby and a fight.
    fn of(scene: &pack::Scene) -> Option<Self> {
        if matches!(scene.key, SceneKey::Room | SceneKey::Combat) {
            return None;
        }
        let setting = scene.setting.as_ref()?;
        let place_kind = serde_json::to_value(setting.kind)
            .ok()?
            .as_str()
            .map(str::to_owned)?;
        let speaker = scene
            .cast
            .iter()
            .find(|member| !scene.speaker.is_empty() && member.name == scene.speaker);
        let identity_cast: Vec<IdentityReference> = scene
            .cast
            .iter()
            .map(|member| IdentityReference {
                id: format!("npc:{}", member.id),
                name: member.name.clone(),
                sprite: member.sprite.clone(),
                portrait: member.portrait.clone(),
            })
            .collect();
        let others = identity_cast
            .iter()
            .filter(|member| {
                speaker.is_none_or(|speaker| {
                    member.id.strip_prefix("npc:").unwrap_or(&member.id) != speaker.id
                })
            })
            .cloned()
            .collect();
        Some(Self {
            title: scene.title.clone(),
            key: scene.key,
            source_variant_id: scene.source_variant_id.clone(),
            asset: scene.asset.clone(),
            place_id: setting.place_id.clone(),
            place_kind,
            exterior: setting.exterior,
            place: PlaceFacts::new(&setting.name, &setting.short),
            story_text: format!("{} {}", scene.narration, scene.dialogue),
            speaker: speaker.map(|member| Speaker {
                id: member.id.clone(),
                sprite: member.sprite.clone(),
                gender: member.gender,
                age: member.age,
            }),
            others,
            cast: scene.cast.clone(),
            identity_cast,
            snapshot: crate::gameplay::adventure::scene_model::current_scene(scene),
            hero_names: Vec::new(),
            authored_room: setting.model.clone(),
            room_revision: setting
                .model
                .as_ref()
                .and_then(super::canonical::room_revision),
        })
    }

    /// The committed NPC reply is the canonical runtime focus for the next plate. Authored
    /// scenes start on their patron, but a later directed reply must promote that exact cast ID
    /// so the camera and locator can keep the addressed person readable.
    fn for_view(view: &View, scene: &pack::Scene) -> Option<Self> {
        let mut plate = Self::of(scene)?;
        let Some(story) = view.story.as_ref() else {
            return Some(plate);
        };
        let actor_id = story
            .actor_id
            .strip_prefix("npc:")
            .unwrap_or(&story.actor_id);
        let Some(actor) = scene.cast.iter().find(|actor| actor.id == actor_id) else {
            return Some(plate);
        };
        plate.speaker = Some(Speaker {
            id: actor.id.clone(),
            sprite: actor.sprite.clone(),
            gender: actor.gender,
            age: actor.age,
        });
        plate.others = plate
            .identity_cast
            .iter()
            .filter(|other| other.id.strip_prefix("npc:").unwrap_or(&other.id) != actor.id)
            .cloned()
            .collect();
        Some(plate)
    }

    fn locator_actors(&self) -> Vec<(String, String)> {
        self.identity_cast
            .iter()
            .map(|actor| {
                (
                    actor
                        .id
                        .strip_prefix("npc:")
                        .unwrap_or(&actor.id)
                        .to_owned(),
                    format!(
                        "{} ({}), the same canonical person in this photograph",
                        actor.name, actor.sprite
                    ),
                )
            })
            .collect()
    }

    fn source_identity(&self) -> String {
        projection_scene_identity(
            self.key.source_label(),
            self.source_variant_id.as_deref().unwrap_or_default(),
            &self.title,
            &self.asset,
        )
    }

    /// The plate of this scene for this party: the same scene and party always have one name.
    fn plate_id(&self, heroes: &[u8], faces: &[(u8, ImageKey)], _graph_revision: u64) -> String {
        let party: String = heroes.iter().map(|art| art.to_string()).collect();
        // A hurt hero's health variant makes another picture, so it is part of the name.
        let hurt: String = faces
            .iter()
            .map(|(art, key)| format!("{art}{}", &key.as_str()[..8]))
            .collect();
        let source_variant = self
            .source_variant_id
            .as_deref()
            .map_or_else(|| "base".to_owned(), |id| format!("variant={id}"));
        format!(
            "{}|{}|{:?}|{source_variant}|speaker={}|{party}|{hurt}|room={}",
            self.title,
            self.place_id,
            self.key,
            self.speaker
                .as_ref()
                .map_or("none", |speaker| speaker.id.as_str()),
            self.room_revision.as_deref().unwrap_or("legacy"),
        )
    }

    fn stage(&self, heroes: &[u8]) -> Result<ShotSpec, ShotError> {
        let others: Vec<&str> = self
            .others
            .iter()
            .map(|member| member.sprite.as_str())
            .collect();
        shot::direct(&ShotInput {
            scene_key: self.key,
            place_id: &self.place_id,
            place_kind: &self.place_kind,
            exterior: self.exterior,
            title: &self.title,
            story_text: &self.story_text,
            speaker: self.speaker.as_ref().map(|speaker| SpeakerInput {
                sprite: &speaker.sprite,
                gender: speaker.gender,
                age: speaker.age,
            }),
            others: &others,
            heroes,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LookupFailure {
    InvalidSceneKey,
    InvalidSceneVariant,
    MissingSceneVariant,
    SceneVariantMismatch,
    MissingBaseScene,
    AmbiguousBaseScene,
}

impl LookupFailure {
    const fn label(self) -> &'static str {
        match self {
            Self::InvalidSceneKey => "invalid_scene_key",
            Self::InvalidSceneVariant => "invalid_scene_variant",
            Self::MissingSceneVariant => "missing_scene_variant",
            Self::SceneVariantMismatch => "scene_variant_mismatch",
            Self::MissingBaseScene => "missing_base_scene",
            Self::AmbiguousBaseScene => "ambiguous_base_scene",
        }
    }
}

fn record_lookup_not_planned(view: &View, failure: LookupFailure) {
    let scene_id = df_observe::presentation::local_identity(&view.scene_key);
    let scene_key = pack::SceneKey::parse_source_label(&view.source_scene_key)
        .map_or("unknown", pack::SceneKey::source_label);
    let variant = if view.source_scene_variant.is_empty() {
        "base"
    } else {
        pack::SceneVariantId::parse(&view.source_scene_variant)
            .map_or("invalid", |variant| variant.as_str())
    };
    record_plate_diagnostic(
        "gameplay.living_scene.plate_not_planned",
        format!(
            "PLATE not_planned scene_id={scene_id} source_scene_key={scene_key} source_scene_variant={variant} reason={}",
            failure.label()
        ),
        serde_json::Map::from_iter([
            ("scene_id".to_owned(), serde_json::json!(scene_id)),
            ("source_scene_key".to_owned(), serde_json::json!(scene_key)),
            (
                "source_scene_variant".to_owned(),
                serde_json::json!(variant),
            ),
            ("status".to_owned(), serde_json::json!("not_planned")),
            (
                "failure_category".to_owned(),
                serde_json::json!(failure.label()),
            ),
        ]),
    );
}

/// Resolves a compiled base scene or the exact BeatScript variant from the public projection.
/// A variant never falls back to another variant or a base scene with matching display text.
pub(super) fn locate(
    title: &str,
    asset: &str,
    source_scene_key: &str,
    source_scene_variant: &str,
) -> Result<pack::Scene, LookupFailure> {
    let source = pack::SceneKey::parse_source_label(source_scene_key)
        .ok_or(LookupFailure::InvalidSceneKey)?;
    pack::with_content(|content| {
        let wanted = asset.trim_start_matches('/');
        let matches = |scene: &pack::Scene, actual_asset: &str| {
            scene.key == source
                && actual_asset.trim_start_matches('/') == wanted
                && scene.title == title
        };
        let overrides = pack::active().map(|loaded| loaded.script_scene_assets.clone());
        if source_scene_variant.is_empty() {
            let mut candidates = content
                .scenes
                .iter()
                .filter(|scene| matches(scene, &scene.asset));
            let Some(scene) = candidates.next() else {
                return Err(LookupFailure::MissingBaseScene);
            };
            if candidates.next().is_some() {
                return Err(LookupFailure::AmbiguousBaseScene);
            }
            return Ok(scene.clone());
        }

        let variant = pack::SceneVariantId::parse(source_scene_variant)
            .ok_or(LookupFailure::InvalidSceneVariant)?;
        let script = content
            .beat_script
            .as_ref()
            .ok_or(LookupFailure::MissingSceneVariant)?;
        let scene = script
            .scenes
            .get(variant.as_str())
            .ok_or(LookupFailure::MissingSceneVariant)?;
        let actual_asset = overrides
            .as_ref()
            .and_then(|assets| assets.get(variant.as_str()))
            .unwrap_or(&scene.asset);
        if !matches(scene, actual_asset) {
            return Err(LookupFailure::SceneVariantMismatch);
        }
        let mut resolved = scene.clone();
        resolved.asset = actual_asset.clone();
        resolved.source_variant_id = Some(variant.as_str().to_owned());
        Ok(resolved)
    })
}

/// How many transitions deep the scenes that can follow are looked for: a travel scene's own
/// destination, and the ending after it, are made while the party is still on the way.
const MAXIMUM_LOOKAHEAD: usize = 3;

/// The scenes that can follow `current`, nearest first, from the beat script's transitions (the
/// canonical script when the pack gives none). A fight is never plated, so its win leads on to the
/// scene after it (a loss is not made ahead of time); a journey and an ending lead on to what
/// they lead to, so the first picture of a new place is requested while the party can still
/// choose to go there.
fn successors(current: &pack::Scene) -> Vec<pack::Scene> {
    pack::with_content(|content| {
        let script = content
            .beat_script
            .as_ref()
            .unwrap_or_else(|| beat_script::canonical());
        successors_in(script, &content.scenes, current)
    })
}

fn successors_in(
    script: &BeatScript,
    scenes: &[pack::Scene],
    current: &pack::Scene,
) -> Vec<pack::Scene> {
    {
        let scene_of = |beat: &Beat| match &beat.scene_variant {
            Some(variant) => script.scenes.get(variant).cloned().map(|mut scene| {
                scene.source_variant_id = Some(variant.clone());
                scene
            }),
            None => scenes
                .iter()
                .find(|candidate| candidate.key == beat.scene)
                .cloned(),
        };
        let mut found: Vec<pack::Scene> = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        let mut queue: VecDeque<(&Beat, usize)> = VecDeque::new();
        for beat in &script.beats {
            let here = scene_of(beat).is_some_and(|scene| {
                scene.title == current.title
                    && scene.key == current.key
                    && scene.source_variant_id == current.source_variant_id
            });
            if here {
                queue.extend(
                    beat.transitions
                        .iter()
                        .filter_map(|transition| Some((script.find(&transition.target)?, 1))),
                );
            }
        }
        while let Some((beat, depth)) = queue.pop_front() {
            if seen.contains(&beat.id.as_str()) {
                continue;
            }
            seen.push(&beat.id);
            let onward: Vec<&BeatTransition> = match beat.kind {
                BeatKind::Fight => beat
                    .transitions
                    .iter()
                    .filter(|won| won.trigger == BeatTrigger::FightWon)
                    .collect(),
                BeatKind::Travel | BeatKind::Ending if depth < MAXIMUM_LOOKAHEAD => {
                    beat.transitions.iter().collect()
                }
                _ => Vec::new(),
            };
            if beat.kind != BeatKind::Fight
                && let Some(next) = scene_of(beat)
                && next.title != current.title
                && !found.iter().any(|known| {
                    known.key == next.key
                        && known.title == next.title
                        && known.asset == next.asset
                        && known.source_variant_id == next.source_variant_id
                })
            {
                found.push(next);
            }
            let deeper = if beat.kind == BeatKind::Fight {
                depth
            } else {
                depth + 1
            };
            queue.extend(
                onward
                    .into_iter()
                    .filter_map(|transition| Some((script.find(&transition.target)?, deeper))),
            );
        }
        found
    }
}

/// What one display view says about plates.
struct View {
    /// Names the authored source scene as well as its title and painting.
    scene_key: String,
    source_scene_key: String,
    source_scene_variant: String,
    title: String,
    asset: String,
    in_combat: bool,
    in_roll: bool,
    graph_revision: u64,
    /// The art numbers of the heroes present, in party order.
    heroes: Vec<u8>,
    /// Their names, to find a hero's health variant.
    hero_names: Vec<(u8, String)>,
    story: Option<StoryBeat>,
}

#[derive(Clone, Debug)]
struct StoryBeat {
    actor_id: String,
    sequence: u64,
    speaker: String,
    text: String,
    sha256: String,
    tempo: shot::PublicBeat,
    dialogue: shot::PublicDialogue,
}

impl StoryBeat {
    fn actor_key(&self) -> String {
        if self.actor_id.is_empty() {
            format!("speaker:{}", self.speaker)
        } else {
            self.actor_id.clone()
        }
    }
}

fn public_beat(journey: &rpc::JourneyView) -> shot::PublicBeat {
    match journey
        .tempo
        .as_ref()
        .and_then(|cue| rpc::TempoPhase::try_from(cue.phase).ok())
    {
        Some(rpc::TempoPhase::Build) => shot::PublicBeat::Build,
        Some(rpc::TempoPhase::Anticipation) => shot::PublicBeat::Anticipation,
        Some(rpc::TempoPhase::Peak) => shot::PublicBeat::Peak,
        Some(rpc::TempoPhase::Release) => shot::PublicBeat::Release,
        _ => shot::PublicBeat::Calm,
    }
}

fn committed_story(journey: &rpc::JourneyView) -> Option<StoryBeat> {
    use sha2::{Digest, Sha256};
    let scene_boundary = journey
        .table_log
        .iter()
        .filter(|line| {
            rpc::TableRole::try_from(line.role).ok() == Some(rpc::TableRole::Hero)
                && rpc::TableKind::try_from(line.kind).ok() == Some(rpc::TableKind::Action)
        })
        .map(|line| line.sequence)
        .max()
        .unwrap_or(0);
    let line = journey.table_log.iter().rev().find(|line| {
        line.sequence > scene_boundary
            && rpc::TableRole::try_from(line.role).ok() == Some(rpc::TableRole::Npc)
            && rpc::TableKind::try_from(line.kind).ok() == Some(rpc::TableKind::Speech)
            && !line.text.trim().is_empty()
    })?;
    if line.text.chars().count() > 4096 {
        return None;
    }
    let text = line.text.trim().to_owned();
    Some(StoryBeat {
        sequence: line.sequence,
        actor_id: line.actor_id.clone(),
        speaker: line.speaker.clone(),
        sha256: format!(
            "{:x}",
            Sha256::digest(format!("{}\0{}\0{text}", line.actor_id, line.speaker).as_bytes())
        ),
        text,
        tempo: public_beat(journey),
        dialogue: shot::PublicDialogue::FirstReply,
    })
}

pub(super) fn projection_scene_identity(
    source_scene_key: &str,
    source_scene_variant: &str,
    title: &str,
    asset: &str,
) -> String {
    let source = pack::SceneKey::parse_source_label(source_scene_key)
        .map_or("unknown", pack::SceneKey::source_label);
    let prefix = if source_scene_variant.is_empty() {
        source.to_owned()
    } else if let Some(variant) = pack::SceneVariantId::parse(source_scene_variant) {
        format!("{source}|variant={}", variant.as_str())
    } else {
        format!(
            "{source}|variant=invalid-{}",
            df_observe::presentation::local_identity(source_scene_variant)
        )
    };
    format!("{prefix}|{title}|{asset}")
}

fn scene_identity(display_asset: &str, scene: &rpc::GameplayScene) -> (String, String) {
    let asset = if scene.scene_asset.is_empty() {
        display_asset.to_owned()
    } else {
        scene.scene_asset.clone()
    };
    (
        projection_scene_identity(
            &scene.source_scene_key,
            &scene.source_scene_variant,
            &scene.title,
            &asset,
        ),
        asset,
    )
}

fn view_of(message: &rpc::ViewMessage, graph_revision: u64) -> Option<View> {
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        return None;
    };
    let scene = display.scene.as_ref()?;
    let journey = display.journey.as_ref();
    let in_combat = journey.and_then(|journey| rpc::JourneyPhase::try_from(journey.phase).ok())
        == Some(rpc::JourneyPhase::Combat);
    let in_roll = journey.is_some_and(|journey| {
        journey.table_log.last().is_some_and(|line| {
            rpc::TableKind::try_from(line.kind).ok() == Some(rpc::TableKind::Action)
                && rpc::TableRole::try_from(line.role).ok() == Some(rpc::TableRole::Narrator)
                && line.speaker == "Roll"
        })
    });
    let hero_names: Vec<(u8, String)> = journey
        .map(|journey| {
            journey
                .party
                .iter()
                .enumerate()
                .take(shot::MAXIMUM_HEROES)
                .filter(|(_, member)| member.character_ready)
                .filter_map(|(index, member)| {
                    Some((u8::try_from(index + 1).ok()?, member.name.clone()))
                })
                .collect()
        })
        .unwrap_or_default();
    let heroes = hero_names.iter().map(|(art, _)| *art).collect();
    let (scene_key, asset) = scene_identity(&display.scene_asset, scene);
    Some(View {
        scene_key,
        source_scene_key: scene.source_scene_key.clone(),
        source_scene_variant: scene.source_scene_variant.clone(),
        title: scene.title.clone(),
        asset,
        in_combat,
        in_roll,
        graph_revision,
        heroes,
        hero_names,
        story: journey.and_then(committed_story),
    })
}

#[derive(Clone, Debug)]
enum Entry {
    /// Asked for; the render is queued or flying.
    Pending,
    Ready {
        key: ImageKey,
        placement: Option<PlatePlacement>,
    },
    Failed,
    /// The scene cannot be staged (a creature speaks, nobody is present): no plate, no failure.
    NotPlanned,
}

#[derive(Clone, Debug)]
enum Edit {
    Idle,
    Pending(Face),
    Ready { face: Face, key: ImageKey },
    Failed(Face),
}

/// The scene on screen and what is known about its plate.
struct Current {
    scene_key: String,
    id: Option<String>,
    scene: Option<PlateScene>,
    heroes: Vec<u8>,
    /// The health variants of hurt heroes the scene's plate was named with.
    faces: Vec<(u8, ImageKey)>,
    /// A story scene that never gets a plate (it has no place to stage): the display holds its
    /// establishing picture instead of drawing cut-outs.
    unplated: bool,
    in_combat: bool,
    in_roll: bool,
    graph_revision: u64,
    request_started: Option<std::time::Instant>,
    first_projection_ms: Option<u128>,
    last_projection: Option<PlateProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PlateProjection {
    graph_revision: u64,
    plate_graph_revision: u64,
    plate_id: Option<String>,
    plate_state: i32,
    source: String,
    asset: String,
}

/// Modelled scenes coalesce visible expression changes rather than restaging every line.
#[derive(Default)]
struct FaceRefresh {
    target: Option<(Option<Face>, Duration)>,
    last_started: Option<Duration>,
}

impl FaceRefresh {
    fn ready(&mut self, wanted: Option<Face>, now: Duration) -> bool {
        if self.target.is_none_or(|(target, _)| target != wanted) {
            self.target = Some((wanted, now));
        }
        self.target
            .is_some_and(|(_, changed)| now.saturating_sub(changed) >= Duration::from_secs(3))
            && self
                .last_started
                .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(10))
    }
}

struct Book {
    image_layouts: BTreeMap<String, PlatePlacement>,
    current: Option<Current>,
    entries: BTreeMap<String, Entry>,
    order: VecDeque<String>,
    request_started: BTreeMap<String, std::time::Instant>,
    edit: Edit,
    direct: direct::Book,
    state_images: state_images::Book,
    face_refresh: FaceRefresh,
    refresh_decisions: BTreeMap<&'static str, (u64, &'static str)>,
}

fn refresh_decision(book: &mut Book, pipeline: &'static str, reason: &'static str) {
    let revision = book
        .current
        .as_ref()
        .map_or(0, |current| current.graph_revision);
    if book.refresh_decisions.get(pipeline) == Some(&(revision, reason)) {
        return;
    }
    book.refresh_decisions.insert(pipeline, (revision, reason));
    let attributes = [
        ("refresh.pipeline", pipeline.to_owned()),
        ("refresh.reason", reason.to_owned()),
        ("graph.revision", revision.to_string()),
    ];
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.living_scene.plate_refresh_decision",
        format!("PLATE refresh pipeline={pipeline} reason={reason} revision={revision}"),
        serde_json::Map::from_iter([
            ("pipeline".to_owned(), serde_json::json!(pipeline)),
            ("reason".to_owned(), serde_json::json!(reason)),
            ("graph_revision".to_owned(), serde_json::json!(revision)),
        ]),
        &attributes,
    );
}

impl Book {
    fn new() -> Self {
        Self {
            image_layouts: BTreeMap::new(),
            current: None,
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            request_started: BTreeMap::new(),
            edit: Edit::Idle,
            direct: direct::Book::default(),
            state_images: state_images::Book::default(),
            face_refresh: FaceRefresh::default(),
            refresh_decisions: BTreeMap::new(),
        }
    }

    fn insert(&mut self, id: String, entry: Entry) {
        if self.entries.insert(id.clone(), entry).is_none() {
            self.order.push_back(id);
        }
        let in_use = self.current.as_ref().and_then(|current| current.id.clone());
        while self.order.len() > MAXIMUM_ENTRIES {
            let Some(position) = self
                .order
                .iter()
                .position(|known| Some(known) != in_use.as_ref())
            else {
                break;
            };
            if let Some(forgotten) = self.order.remove(position) {
                self.entries.remove(&forgotten);
                self.request_started.remove(&forgotten);
            }
        }
    }

    fn current_entry(&self) -> Option<&Entry> {
        let id = self.current.as_ref()?.id.as_ref()?;
        self.entries.get(id)
    }

    /// The plate the display should show now, with its layout: the edited plate while an edit of
    /// the wanted face is ready, otherwise the first one.
    fn shown(&self) -> Option<(ImageKey, Option<PlatePlacement>)> {
        if let Some(shown) = &self.state_images.shown {
            return Some(match &self.edit {
                Edit::Ready { key, .. } => {
                    (key.clone(), self.image_layouts.get(key.as_str()).cloned())
                }
                _ => shown.clone(),
            });
        }
        if let Some(shown) = &self.direct.shown {
            return match &self.edit {
                Edit::Ready { key, .. } => {
                    Some((key.clone(), self.image_layouts.get(key.as_str()).cloned()))
                }
                _ => Some(shown.clone()),
            };
        }
        let Some(Entry::Ready { key, placement }) = self.current_entry() else {
            return None;
        };
        match &self.edit {
            Edit::Ready { key: edited, .. } => Some((
                edited.clone(),
                self.image_layouts.get(edited.as_str()).cloned(),
            )),
            _ => Some((key.clone(), placement.clone())),
        }
    }

    fn covers(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| !current.in_combat)
            && (self.direct.shown.is_some()
                || matches!(self.current_entry(), Some(Entry::Ready { .. })))
    }

    fn current_update_pending(&self, scene_key: &str) -> bool {
        let Some(current) = self.current.as_ref() else {
            return false;
        };
        current.scene_key == scene_key
            && current.scene.is_some()
            && !current.in_combat
            && (matches!(self.current_entry(), Some(Entry::Pending))
                || matches!(self.edit, Edit::Pending(_))
                || self.state_images.pending.is_some()
                || self.state_images.flying
                || self.direct.pending)
    }
}

/// One request for the worker.
enum Job {
    State(state_images::Job),
    Directed(Box<direct::Work>),
    Fresh {
        id: String,
        graph_revision: u64,
        scene: PlateScene,
        spec: Box<ShotSpec>,
        heroes: Vec<u8>,
        /// The health variants (already made and cached) of hurt heroes, by art number.
        faces: Vec<(u8, ImageKey)>,
        /// A plate for a scene that may come next, not the one on screen.
        ahead: bool,
    },
    Edit {
        scene_key: String,
        scene: PlateScene,
        base: ImageKey,
        face: Face,
        heroes: Vec<u8>,
    },
    /// Finds the heads in a plate that is already shown with its predicted layout.
    Locate {
        id: String,
        work: Box<Locate>,
    },
}

/// What finding the heads in a finished plate needs.
struct Locate {
    focus_actor_id: Option<String>,
    actors: Vec<(String, String)>,
    key: ImageKey,
    spec: ShotSpec,
    speaker: shot::SpeakerShot,
    actor_references: BTreeMap<String, ReferenceImage>,
}

enum Done {
    State(state_images::Done),
    Directed {
        generation: u64,
        scene_key: String,
        key: Option<ImageKey>,
        result: Result<(ImageKey, Option<PlatePlacement>), ImageFailure>,
    },
    Located {
        id: String,
        key: ImageKey,
        elapsed: Duration,
        placement: PlatePlacement,
    },
    Fresh {
        id: String,
        graph_revision: u64,
        ahead: bool,
        elapsed: Duration,
        result: Result<(ImageKey, Option<PlatePlacement>), ImageFailure>,
    },
    Edit {
        scene_key: String,
        speaker_id: Option<String>,
        face: Face,
        elapsed: Duration,
        result: Result<ImageKey, ImageFailure>,
    },
}

struct Shared {
    inner: Arc<Inner>,
    book: Mutex<Book>,
    jobs: mpsc::Sender<Job>,
}

/// The plate director: a cheap handle owned by the living-scenes handle.
#[derive(Clone)]
pub(super) struct Plates {
    shared: Arc<Shared>,
    _worker: Arc<WorkerGuard>,
}

impl Plates {
    /// Starts the director and its worker task. Must be called inside the runtime.
    pub(super) fn start(inner: Arc<Inner>) -> Self {
        let (jobs, queue) = mpsc::channel(QUEUE_CAPACITY);
        let shared = Arc::new(Shared {
            inner,
            book: Mutex::new(Book::new()),
            jobs,
        });
        let worker = tokio::spawn(Arc::clone(&shared).run(queue));
        Self {
            shared,
            _worker: Arc::new(WorkerGuard(worker)),
        }
    }

    /// Notes a display view: a new scene gets its plate and its successors' plates requested.
    pub(super) fn observe(&self, message: &rpc::ViewMessage, graph_revision: u64) {
        let Some(view) = view_of(message, graph_revision) else {
            return;
        };
        self.shared.observe(&view);
    }

    /// Adds the plate to the scene of a view about to be sent (to the display or a phone).
    pub(super) fn apply_to_message(&self, message: &mut rpc::ViewMessage) {
        match message.audience.as_mut() {
            Some(rpc::view_message::Audience::Display(display)) => {
                let Some(scene) = display.scene.as_mut() else {
                    return;
                };
                if let Some(item) = scene.item.as_ref() {
                    let _ = self.request_hand_item(item, true, &scene.scene_contents_json);
                }
                for item in scene.hand_items.clone() {
                    let _ = self.request_hand_item(&item, true, &scene.scene_contents_json);
                }
                let (scene_key, _) = scene_identity(&display.scene_asset, scene);
                display.live_plate_graph_revision =
                    self.shared.apply(&scene_key, scene).unwrap_or_default();
                self.shared.project_scene_clip(&scene_key, scene);
                self.shared.project_scene_contents(scene, None);
                self.shared
                    .record_projection(&scene_key, scene, display.live_plate_graph_revision);
            }
            Some(rpc::view_message::Audience::Player(player)) => {
                let Some(scene) = player.scene.as_mut() else {
                    return;
                };
                if let Some(item) = scene.item.as_ref() {
                    let _ = self.request_hand_item(item, false, &scene.scene_contents_json);
                }
                for item in scene.hand_items.clone() {
                    let _ = self.request_hand_item(&item, false, &scene.scene_contents_json);
                }
                let owned_items = player
                    .journey
                    .as_ref()
                    .and_then(|journey| journey.own_character.as_ref())
                    .map(|sheet| sheet.owned_items.clone())
                    .unwrap_or_default();
                for item in &owned_items {
                    let _ = self.request_hand_item(item, false, &scene.scene_contents_json);
                }
                let (scene_key, _) = scene_identity("", scene);
                let _revision = self.shared.apply(&scene_key, scene);
                scene.scene_video_state = rpc::HomeVideoState::None as i32;
                scene.scene_video_path.clear();
                let hero = player
                    .journey
                    .as_ref()
                    .and_then(|journey| journey.own_character.as_ref())
                    .map(|hero| hero.name.as_str());
                self.shared.project_scene_contents(scene, hero);
                if let Some(sheet) = player
                    .journey
                    .as_mut()
                    .and_then(|journey| journey.own_character.as_mut())
                {
                    self.shared.project_item_cards(&mut sheet.owned_items);
                }
            }
            None => {}
        }
    }
}

fn projection_request_latency(
    first: &mut Option<u128>,
    scene_age_ms: Option<u128>,
    projected: Option<&str>,
    original: Option<&str>,
) -> Option<u128> {
    if projected.is_none() || projected != original {
        return None;
    }
    if first.is_none() {
        *first = scene_age_ms;
    }
    *first
}

fn record_plate_diagnostic(
    event: &'static str,
    line: String,
    fields: serde_json::Map<String, serde_json::Value>,
) {
    crate::gameplay::actor::record_audit_diagnostic(event, line, fields, &[]);
}

fn record_plate_render_diagnostic(
    event: &'static str,
    status: &'static str,
    scene: &PlateScene,
    spec: &ShotSpec,
    plate_id: &str,
    graph_revision: u64,
    ahead: bool,
    image_key: Option<&ImageKey>,
    failure: Option<ImageFailure>,
    elapsed_ms: u128,
) {
    let scene_key = scene.source_identity();
    let scene_id = df_observe::presentation::local_identity(&scene_key);
    let plate_id = df_observe::presentation::local_identity(plate_id);
    let image_key = image_key.map(ImageKey::as_str);
    let image_key_id = image_key.map(df_observe::presentation::local_identity);
    let failure_category = failure.map(ImageFailure::label);
    let focus_subject = if spec.speaker.is_some() {
        "primary-speaker"
    } else if !spec.dressing.is_empty() {
        "foreground-goods"
    } else {
        "place-context"
    };
    let context_scope = if spec.camera.kind() == "establishing-wide" {
        "full-place-cast-and-goods"
    } else {
        "speaker-with-place-context"
    };
    let line = format!(
        "PLATE {status} scene_id={scene_id} revision={graph_revision} ahead={ahead} plate_id={plate_id} shot_kind={} angle={} focus_reason={} focus_subject={focus_subject} context_scope={context_scope} image_key={} image_key_id={} failure_category={} elapsed_ms={elapsed_ms}",
        spec.camera.kind(),
        spec.camera.angle(),
        spec.camera.focus_reason(),
        image_key.unwrap_or("none"),
        image_key_id.map_or_else(|| "none".to_owned(), |id| id.to_string()),
        failure_category.unwrap_or("none"),
    );
    let fields = serde_json::Map::from_iter([
        ("scene_id".to_owned(), serde_json::json!(scene_id)),
        ("plate_id".to_owned(), serde_json::json!(plate_id)),
        (
            "graph_revision".to_owned(),
            serde_json::json!(graph_revision),
        ),
        ("ahead".to_owned(), serde_json::json!(ahead)),
        (
            "requested_source".to_owned(),
            serde_json::json!("scene_plate"),
        ),
        ("status".to_owned(), serde_json::json!(status)),
        ("image_key".to_owned(), serde_json::json!(image_key)),
        ("image_key_id".to_owned(), serde_json::json!(image_key_id)),
        (
            "shot_kind".to_owned(),
            serde_json::json!(spec.camera.kind()),
        ),
        ("angle".to_owned(), serde_json::json!(spec.camera.angle())),
        (
            "focus_reason".to_owned(),
            serde_json::json!(spec.camera.focus_reason()),
        ),
        ("focus_subject".to_owned(), serde_json::json!(focus_subject)),
        ("context_scope".to_owned(), serde_json::json!(context_scope)),
        (
            "failure_category".to_owned(),
            serde_json::json!(failure_category),
        ),
        ("elapsed_ms".to_owned(), serde_json::json!(elapsed_ms)),
    ]);
    record_plate_diagnostic(event, line, fields);
}

/// The locator accepts four short plain IDs per call. Canonical IDs remain in the indexed mapping.
fn locator_targets(
    spec: &ShotSpec,
    speaker: &shot::SpeakerShot,
    actors: &[(String, String)],
    focus_actor_id: Option<&str>,
) -> (String, Vec<(String, String)>) {
    let focus = actors
        .iter()
        .position(|(id, _)| Some(id.as_str()) == focus_actor_id);
    let focus_id = focus.map_or_else(|| "speaker".to_owned(), |index| format!("npc-{index}"));
    let bounded = |text: String| text.chars().take(300).collect::<String>();
    let mut targets = vec![(
        focus_id.clone(),
        bounded(format!("{}, {}", speaker.look, speaker.task)),
    )];
    targets.extend(spec.heroes.iter().map(|hero| {
        (
            format!("hero-{}", hero.art),
            bounded(hero_locator_phrase(hero.art).to_owned()),
        )
    }));
    targets.extend(
        actors
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != focus)
            .map(|(index, (_, description))| {
                (format!("npc-{index}"), bounded(description.clone()))
            }),
    );
    (focus_id, targets)
}

fn reference_hashes(references: &BTreeMap<String, ReferenceImage>) -> BTreeMap<String, String> {
    references
        .iter()
        .map(|(id, reference)| (id.clone(), reference.sha256.clone()))
        .collect()
}

struct LocatorBatch<'a> {
    targets: Vec<(String, String)>,
    reference: &'a ReferenceImage,
}

/// Each person is the sole target matched to their own pinned reference. Four calls per image;
/// absent references and people beyond that cap deliberately have no attributed head box.
fn referenced_locator_batches<'a>(
    targets: &[(String, String)],
    actors: &[(String, String)],
    references: &'a BTreeMap<String, ReferenceImage>,
) -> Vec<LocatorBatch<'a>> {
    targets.iter().take(16).filter_map(|target| {
        // `speaker` is an unbound compatibility slot, never a canonical actor identity.
        if target.0 == "speaker" { return None; }
        let id = if let Some(index) = target.0.strip_prefix("npc-").and_then(|id| id.parse::<usize>().ok()) {
            actors.get(index)?.0.as_str()
        } else { target.0.as_str() };
        Some(LocatorBatch {
            targets: vec![(target.0.clone(), "The exact person in the second reference picture; match their face, hair and costume, never their name, role or position. Omit this target if identity cannot be matched confidently.".to_owned())],
            reference: references.get(id)?,
        })
    }).take(4).collect()
}

const HEAD_IMAGE_DEADLINE: Duration = Duration::from_secs(25);

struct HeadResult {
    elapsed_ms: u128,
    heads: Result<Vec<df_providers::LocatedHead>, ImageFailure>,
}

/// The image job owns every borrowed request; one deadline drops all unfinished requests.
async fn collect_head_results<F>(
    mut pending: FuturesUnordered<F>,
    count: usize,
    deadline: tokio::time::Instant,
) -> Vec<Option<HeadResult>>
where
    F: std::future::Future<Output = (usize, HeadResult)>,
{
    let mut results: Vec<Option<HeadResult>> = (0..count.min(4)).map(|_| None).collect();
    while let Ok(Some((index, result))) = tokio::time::timeout_at(deadline, pending.next()).await {
        if let Some(slot) = results.get_mut(index) {
            *slot = Some(result);
        }
    }
    results
}

/// Bounded plate diagnostics share the audit mirror used by the development CLI.
struct HeadDiagnostic<'a> {
    key: &'a ImageKey,
    stage: &'static str,
    reference_sha256: Option<&'a str>,
    source: &'static str,
    batch: Option<usize>,
    targets: Vec<String>,
    located: Vec<String>,
    actor_ids: Vec<(String, String)>,
    elapsed_ms: u128,
    failure: Option<&'static str>,
}

impl HeadDiagnostic<'_> {
    fn fields(&self) -> serde_json::Map<String, serde_json::Value> {
        let partial = self.located.len() < self.targets.len() || self.failure.is_some();
        let status = if matches!(self.stage, "requested" | "queued") {
            self.stage
        } else if self.failure.is_some() && self.located.is_empty() {
            "failed"
        } else if partial {
            "partial"
        } else {
            "ready"
        };
        let bounded = |ids: &[String]| {
            ids.iter()
                .take(16)
                .map(|id| id.chars().take(96).collect::<String>())
                .collect::<Vec<_>>()
        };
        serde_json::Map::from_iter([
            ("resource_type".into(), serde_json::json!("plate")),
            ("stage".into(), serde_json::json!(self.stage)),
            ("status".into(), serde_json::json!(status)),
            ("outcome".into(), serde_json::json!(status)),
            ("image_key".into(), serde_json::json!(self.key.as_str())),
            ("cache_source".into(), serde_json::json!(self.source)),
            (
                "reference_sha256".into(),
                serde_json::json!(self.reference_sha256),
            ),
            ("locator_revision".into(), serde_json::json!(2)),
            ("batch".into(), serde_json::json!(self.batch)),
            (
                "target_ids".into(),
                serde_json::json!(bounded(&self.targets)),
            ),
            (
                "located_ids".into(),
                serde_json::json!(bounded(&self.located)),
            ),
            (
                "actor_id_mapping".into(),
                serde_json::json!(
                    self.actor_ids
                        .iter()
                        .take(16)
                        .map(|(target, actor)| (
                            target.chars().take(96).collect::<String>(),
                            actor.chars().take(96).collect::<String>()
                        ))
                        .collect::<std::collections::BTreeMap<_, _>>()
                ),
            ),
            ("target_count".into(), serde_json::json!(self.targets.len())),
            (
                "located_count".into(),
                serde_json::json!(self.located.len()),
            ),
            (
                "ids_truncated".into(),
                serde_json::json!(
                    self.targets.len() > 16
                        || self.located.len() > 16
                        || self
                            .targets
                            .iter()
                            .chain(&self.located)
                            .any(|id| id.chars().count() > 96)
                ),
            ),
            ("partial".into(), serde_json::json!(partial)),
            ("elapsed_ms".into(), serde_json::json!(self.elapsed_ms)),
            ("failure_category".into(), serde_json::json!(self.failure)),
            (
                "missing_head_surface".into(),
                serde_json::json!("named_lane"),
            ),
        ])
    }

    fn record(self) {
        let fields = self.fields();
        let status = fields["status"].as_str().unwrap_or("failed");
        crate::gameplay::actor::record_audit_diagnostic(
            if self.batch.is_some() {
                "gameplay.living_scene.head_located"
            } else {
                "gameplay.living_scene.actor_heads"
            },
            format!(
                "PLATE heads stage={} key={} source={} status={} targets={} located={} elapsed_ms={}",
                self.stage,
                self.key.as_str(),
                self.source,
                status,
                self.targets.len(),
                self.located.len(),
                self.elapsed_ms
            ),
            fields.clone(),
            &[
                ("gameplay.resource.type", "plate".to_owned()),
                ("plate.stage", self.stage.to_owned()),
                ("plate.status", status.to_owned()),
                ("plate.image_key", self.key.as_str().to_owned()),
                ("plate.cache_source", self.source.to_owned()),
                ("plate.target_ids", fields["target_ids"].to_string()),
                ("plate.located_ids", fields["located_ids"].to_string()),
                ("plate.elapsed_ms", self.elapsed_ms.to_string()),
                (
                    "plate.failure_category",
                    self.failure.unwrap_or("none").to_owned(),
                ),
            ],
        );
    }
}

fn record_head_result(
    key: &ImageKey,
    batch: usize,
    work: &LocatorBatch<'_>,
    actors: &[(String, String)],
    result: &HeadResult,
) {
    HeadDiagnostic {
        key,
        stage: "head_location",
        reference_sha256: Some(work.reference.sha256.as_str()),
        source: "provider",
        batch: Some(batch),
        actor_ids: work
            .targets
            .iter()
            .filter_map(|(id, _)| {
                let index = id.strip_prefix("npc-")?.parse::<usize>().ok()?;
                Some((id.clone(), actors.get(index)?.0.clone()))
            })
            .collect(),
        targets: work.targets.iter().map(|(id, _)| id.clone()).collect(),
        located: result
            .heads
            .as_ref()
            .map(|heads| heads.iter().map(|head| head.id.clone()).collect())
            .unwrap_or_default(),
        elapsed_ms: result.elapsed_ms,
        failure: result.heads.as_ref().err().map(|failure| failure.label()),
    }
    .record();
}

fn record_actor_heads(
    key: &ImageKey,
    actors: &[(String, String)],
    placement: &PlatePlacement,
    source: &'static str,
    elapsed_ms: u128,
    failure: Option<&'static str>,
) {
    HeadDiagnostic {
        key,
        stage: "actor_heads",
        reference_sha256: None,
        source,
        batch: None,
        targets: actors.iter().map(|(id, _)| id.clone()).collect(),
        actor_ids: Vec::new(),
        located: placement
            .actors
            .iter()
            .map(|head| head.actor_id.clone())
            .collect(),
        elapsed_ms,
        failure,
    }
    .record();
}

fn wire_side(side: shot::Side) -> i32 {
    (match side {
        shot::Side::Left => rpc::PlateSide::Left,
        shot::Side::Right => rpc::PlateSide::Right,
    }) as i32
}

fn wire_layout(placement: &PlatePlacement) -> rpc::PlateLayout {
    let speaker = &placement.speaker;
    rpc::PlateLayout {
        bubble_side: wire_side(speaker.bubble_side),
        head_x_permille: u32::from(speaker.head_x_permille),
        head_y_permille: u32::from(speaker.head_y_permille),
        head_width_permille: u32::from(speaker.head_width_permille),
        head_height_permille: u32::from(speaker.head_height_permille),
        actors: placement
            .actors
            .iter()
            .map(|actor| rpc::PlateActorHead {
                actor_id: actor.actor_id.clone(),
                bubble_side: wire_side(actor.layout.bubble_side),
                head_x_permille: u32::from(actor.layout.head_x_permille),
                head_y_permille: u32::from(actor.layout.head_y_permille),
                head_width_permille: u32::from(actor.layout.head_width_permille),
                head_height_permille: u32::from(actor.layout.head_height_permille),
            })
            .collect(),
        heroes: placement
            .heroes
            .iter()
            .map(|hero| rpc::PlateHead {
                hero_index: u32::from(hero.art.saturating_sub(1)),
                bubble_side: wire_side(hero.layout.bubble_side),
                head_x_permille: u32::from(hero.layout.head_x_permille),
                head_y_permille: u32::from(hero.layout.head_y_permille),
                head_width_permille: u32::from(hero.layout.head_width_permille),
                head_height_permille: u32::from(hero.layout.head_height_permille),
            })
            .collect(),
    }
}

/// One head as it is kept beside the cached plate (art 0 is the speaker).
#[derive(serde::Deserialize, serde::Serialize)]
struct StoredHead {
    art: u8,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    bubble_left: bool,
}

/// The measured placement as it is kept beside the cached plate, so a later server run does not
/// ask the vision model again.
#[derive(serde::Deserialize, serde::Serialize)]
struct StoredPlacement {
    speaker: StoredHead,
    heroes: Vec<StoredHead>,
    #[serde(default)]
    actors: Vec<StoredActorHead>,
    #[serde(default)]
    actor_ids: Vec<String>,
    #[serde(default)]
    locator_revision: u8,
    #[serde(default)]
    actor_reference_sha256: BTreeMap<String, String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct StoredActorHead {
    actor_id: String,
    head: StoredHead,
}

impl StoredHead {
    fn of(art: u8, layout: &PlateLayout) -> Self {
        Self {
            art,
            x: layout.head_x_permille,
            y: layout.head_y_permille,
            width: layout.head_width_permille,
            height: layout.head_height_permille,
            bubble_left: layout.bubble_side == shot::Side::Left,
        }
    }

    /// `None` for numbers outside the plate: a note that has been edited or damaged is not trusted.
    fn layout(&self) -> Option<PlateLayout> {
        (self.x <= 1000
            && self.y <= 1000
            && (1..=400).contains(&self.width)
            && (1..=500).contains(&self.height))
        .then_some(PlateLayout {
            bubble_side: if self.bubble_left {
                shot::Side::Left
            } else {
                shot::Side::Right
            },
            head_x_permille: self.x,
            head_y_permille: self.y,
            head_width_permille: self.width,
            head_height_permille: self.height,
        })
    }
}

impl StoredPlacement {
    fn of(placement: &PlatePlacement) -> Self {
        Self {
            speaker: StoredHead::of(0, &placement.speaker),
            locator_revision: 2,
            actor_reference_sha256: BTreeMap::new(),
            actor_ids: placement
                .actors
                .iter()
                .map(|actor| actor.actor_id.clone())
                .collect(),
            actors: placement
                .actors
                .iter()
                .map(|actor| StoredActorHead {
                    actor_id: actor.actor_id.clone(),
                    head: StoredHead::of(0, &actor.layout),
                })
                .collect(),
            heroes: placement
                .heroes
                .iter()
                .map(|hero| StoredHead::of(hero.art, &hero.layout))
                .collect(),
        }
    }

    fn placement(&self) -> Option<PlatePlacement> {
        let heroes = self
            .heroes
            .iter()
            .filter_map(|head| {
                Some(HeroHead {
                    art: head.art,
                    layout: head.layout()?,
                })
            })
            .collect();
        Some(PlatePlacement {
            speaker: self.speaker.layout()?,
            actors: self
                .actors
                .iter()
                .filter_map(|actor| {
                    Some(ActorHead {
                        actor_id: actor.actor_id.clone(),
                        layout: actor.head.layout()?,
                    })
                })
                .collect(),
            heroes,
        })
    }
}

impl Shared {
    fn current_clip_context(
        &self,
        scene_key: &str,
        plate_key: &str,
    ) -> Option<(String, &'static str, bool)> {
        let book = locked(&self.book);
        let current = book.current.as_ref()?;
        if current.scene_key != scene_key
            || book
                .shown()
                .as_ref()
                .is_none_or(|(key, _)| key.as_str() != plate_key)
        {
            return None;
        }
        let directed = book.direct.shown.is_some();
        let (description, reason) =
            if directed && let Some(description) = book.direct.description.clone() {
                (description, "directed-plate")
            } else {
                let scene = current.scene.as_ref()?;
                let spec = scene.stage(&current.heroes).ok()?;
                (
                    scene.snapshot.as_ref().map_or_else(
                        || super::super::home_video::scene_description(&spec),
                        |snapshot| super::super::home_video::model_staging(&snapshot.model),
                    ),
                    if matches!(&book.edit, Edit::Ready { key, .. } if key.as_str() == plate_key) {
                        "plate-edit"
                    } else {
                        "scene-plate"
                    },
                )
            };
        Some((description, reason, current.in_combat || current.in_roll))
    }

    fn project_scene_clip(&self, scene_key: &str, scene: &mut rpc::GameplayScene) {
        scene.scene_video_state = rpc::HomeVideoState::None as i32;
        scene.scene_video_path.clear();
        let Some(plate_key) = scene
            .live_plate
            .strip_prefix("/gameplay/imagery/")
            .filter(|key| key.len() == 64)
        else {
            return;
        };
        let Some((description, reason, blocked)) = self.current_clip_context(scene_key, plate_key)
        else {
            return;
        };
        if blocked {
            super::super::home_video::request_scene_clip(
                plate_key,
                Vec::new(),
                &description,
                reason,
                true,
            );
            return;
        }
        let view = super::super::home_video::scene_clip_view(plate_key);
        scene.scene_video_state = view.proto_state() as i32;
        scene.scene_video_path = view.path();
        if view.proto_state() != rpc::HomeVideoState::None {
            return;
        }
        let imagery = self.inner.imagery.clone();
        let plate_key = plate_key.to_owned();
        tokio::spawn(async move {
            if let Lookup::Image { bytes, .. } = imagery.image(&plate_key).await {
                super::super::home_video::request_scene_clip(
                    &plate_key,
                    bytes.to_vec(),
                    &description,
                    reason,
                    false,
                );
            }
        });
    }

    fn wake(&self) {
        self.inner.wake.send_modify(|count| *count += 1);
    }

    fn observe(&self, view: &View) {
        let changed = locked(&self.book).current.as_ref().is_none_or(|current| {
            current.scene_key != view.scene_key || current.heroes != view.heroes
        });
        let located = match locate(
            &view.title,
            &view.asset,
            &view.source_scene_key,
            &view.source_scene_variant,
        ) {
            Ok(scene) => Some(scene),
            Err(failure) => {
                if changed {
                    record_lookup_not_planned(view, failure);
                }
                None
            }
        };
        let next = if changed {
            located.as_ref().map(successors).unwrap_or_default()
        } else {
            Vec::new()
        };
        self.observe_with(view, located, next);
    }

    /// [`Self::observe`] with the pack's scene and its successors given, so tests need no pack.
    fn observe_with(&self, view: &View, located: Option<pack::Scene>, next: Vec<pack::Scene>) {
        let mut book = locked(&self.book);
        if let Some(scene) = located
            .as_ref()
            .and_then(|scene| PlateScene::for_view(view, scene))
        {
            state_images::observe(&mut book, view, &scene, self.inner.now());
        } else {
            state_images::reset_scene(&mut book, view);
        }
        let canonical_changed = located
            .as_ref()
            .and_then(|scene| PlateScene::for_view(view, scene))
            .is_some_and(|scene| {
                if let Some(model) = &scene.authored_room {
                    self.inner
                        .admit_canonical_room(&scene.place_id, model, &scene.cast);
                }
                let faces = self.inner.hero_face_keys(&view.hero_names);
                let id = self.canonical_plate_id(&scene, &view.heroes, &faces, view.graph_revision);
                book.current
                    .as_ref()
                    .and_then(|current| current.id.as_ref())
                    .is_some_and(|current| *current != id)
            });
        let scene_changed = book
            .current
            .as_ref()
            .is_none_or(|current| current.scene_key != view.scene_key);
        let party_changed = book
            .current
            .as_ref()
            .is_some_and(|current| current.heroes != view.heroes);
        let graph_changed = book
            .current
            .as_ref()
            .is_some_and(|current| current.graph_revision != view.graph_revision);
        let runtime_speaker = located
            .as_ref()
            .and_then(|scene| PlateScene::for_view(view, scene))
            .and_then(|scene| scene.speaker.map(|speaker| speaker.id));
        let speaker_changed = book.current.as_ref().is_some_and(|current| {
            current
                .scene
                .as_ref()
                .and_then(|scene| scene.speaker.as_ref().map(|speaker| speaker.id.as_str()))
                != runtime_speaker.as_deref()
        });
        let combat_changed = book
            .current
            .as_ref()
            .is_some_and(|current| current.in_combat != view.in_combat);
        let roll_changed = book
            .current
            .as_ref()
            .is_some_and(|current| current.in_roll != view.in_roll);
        let mut woke = false;
        // A scene keeps its plate while it lasts: who speaks and their expressions are drawn on
        // the character layers, never by a new plate. A plate per graph change replaced the whole
        // background every line, with a new composition and the figures gone (the owner,
        // 2026-10-14: "changes background too often where all the characters disappear").
        // Visible changes to a modelled scene's state are settled edits (state_images).
        if scene_changed || party_changed || speaker_changed || canonical_changed {
            let mut scene = located
                .as_ref()
                .and_then(|scene| PlateScene::for_view(view, scene))
                .or_else(|| {
                    (!scene_changed && !party_changed)
                        .then(|| {
                            book.current
                                .as_ref()
                                .and_then(|current| current.scene.clone())
                        })
                        .flatten()
                });
            if let Some(scene) = scene.as_mut() {
                scene.hero_names = view
                    .hero_names
                    .iter()
                    .map(|(_, name)| name.clone())
                    .collect();
            }
            let faces = self.inner.hero_face_keys(&view.hero_names);
            let id = scene.as_ref().map(|scene| {
                self.canonical_plate_id(scene, &view.heroes, &faces, view.graph_revision)
            });
            let old_id = graph_changed
                .then(|| book.current.as_ref().and_then(|current| current.id.clone()))
                .flatten();
            if let Some(old_id) = old_id
                && id.as_ref() != Some(&old_id)
                && matches!(book.entries.get(&old_id), Some(Entry::Pending))
            {
                book.entries.remove(&old_id);
                book.order.retain(|known| known != &old_id);
                book.request_started.remove(&old_id);
            }
            if scene_changed || graph_changed {
                book.edit = Edit::Idle;
                book.face_refresh = FaceRefresh::default();
                book.refresh_decisions.clear();
                book.direct.reset();
                // A plate that failed is asked for again once when its scene comes back.
                if let Some(id) = &id
                    && matches!(book.entries.get(id), Some(Entry::Failed))
                {
                    book.entries.remove(id);
                    book.order.retain(|known| known != id);
                    book.request_started.remove(id);
                }
            }
            let unplated = scene.is_none()
                && located.as_ref().is_none_or(|located| {
                    !matches!(located.key, SceneKey::Room | SceneKey::Combat)
                });
            let request_started = id
                .as_ref()
                .and_then(|id| book.request_started.get(id).copied());
            book.current = Some(Current {
                scene_key: view.scene_key.clone(),
                id,
                scene: scene.clone(),
                heroes: view.heroes.clone(),
                faces: faces.clone(),
                unplated,
                in_combat: view.in_combat,
                in_roll: view.in_roll,
                graph_revision: view.graph_revision,
                request_started,
                first_projection_ms: None,
                last_projection: None,
            });
            // The lobby has no plate; the opening plate is requested when that scene becomes current.
            if !view.in_combat {
                if let Some(scene) = &scene {
                    self.ensure(
                        &mut book,
                        scene,
                        &view.heroes,
                        &faces,
                        view.graph_revision,
                        false,
                    );
                }
                if scene.is_some() && (scene_changed || party_changed) {
                    for mut next in next
                        .iter()
                        .take(MAXIMUM_SUCCESSORS)
                        .filter_map(|next| PlateScene::for_view(view, next))
                    {
                        next.hero_names = view
                            .hero_names
                            .iter()
                            .map(|(_, name)| name.clone())
                            .collect();
                        self.ensure(
                            &mut book,
                            &next,
                            &view.heroes,
                            &faces,
                            view.graph_revision,
                            scene.is_some(),
                        );
                    }
                }
            }
            woke = true;
        } else if (combat_changed || roll_changed || graph_changed)
            && let Some(current) = book.current.as_mut()
        {
            current.in_combat = view.in_combat;
            current.in_roll = view.in_roll;
            current.graph_revision = view.graph_revision;
            if let Some(mut scene) = located
                .as_ref()
                .and_then(|scene| PlateScene::for_view(view, scene))
            {
                scene.hero_names = view
                    .hero_names
                    .iter()
                    .map(|(_, name)| name.clone())
                    .collect();
                current.scene = Some(scene);
            }
            woke = true;
        }
        if !view.in_combat && book.current_entry().is_none() {
            // The scene's request was dropped (a full queue): it is made again by this view.
            let retry = book.current.as_ref().and_then(|current| {
                Some((
                    current.scene.clone()?,
                    current.heroes.clone(),
                    current.faces.clone(),
                    current.graph_revision,
                ))
            });
            if let Some((scene, heroes, faces, graph_revision)) = retry {
                self.ensure(&mut book, &scene, &heroes, &faces, graph_revision, false);
            }
        }
        let covers = book.covers();
        drop(book);
        self.inner.set_plate_covers(covers);
        if woke {
            self.wake();
        }
    }

    fn canonical_plate_id(
        &self,
        scene: &PlateScene,
        heroes: &[u8],
        faces: &[(u8, ImageKey)],
        graph_revision: u64,
    ) -> String {
        let id = scene.plate_id(heroes, faces, graph_revision);
        match self.inner.canonical_cast_binding(
            heroes,
            scene
                .identity_cast
                .iter()
                .map(|identity| identity.id.strip_prefix("npc:").unwrap_or(&identity.id)),
        ) {
            Some(binding) => format!("{id}|cast={binding}"),
            None => id,
        }
    }

    /// Asks for the plate of `scene` for this party unless the book already knows it.
    fn ensure(
        &self,
        book: &mut Book,
        scene: &PlateScene,
        heroes: &[u8],
        faces: &[(u8, ImageKey)],
        graph_revision: u64,
        ahead: bool,
    ) {
        if let Some(model) = &scene.authored_room {
            self.inner
                .admit_canonical_room(&scene.place_id, model, &scene.cast);
        }
        let id = self.canonical_plate_id(scene, heroes, faces, graph_revision);
        if book.entries.contains_key(&id) {
            return;
        }
        let mut dependencies = heroes
            .iter()
            .map(|seat| self.inner.canonical_hero_status(*seat))
            .collect::<Vec<_>>();
        dependencies.extend(scene.identity_cast.iter().map(|identity| {
            self.inner
                .canonical_npc_status(identity.id.strip_prefix("npc:").unwrap_or(&identity.id))
        }));
        if let Some(revision) = &scene.room_revision {
            dependencies.push(
                self.inner
                    .canonical_room_status_at(&scene.place_id, revision),
            );
        }
        if let Some(failure) = dependencies.iter().find_map(|status| match status {
            super::canonical::ReferenceStatus::Failed(failure) => Some(*failure),
            _ => None,
        }) {
            record_plate_diagnostic(
                "gameplay.living_scene.plate_reference_failed",
                format!(
                    "PLATE failed scene_id={} reason=canonical_reference_{}",
                    df_observe::presentation::local_identity(&scene.source_identity()),
                    failure.label()
                ),
                serde_json::Map::from_iter([
                    ("status".to_owned(), serde_json::json!("failed")),
                    (
                        "failure_category".to_owned(),
                        serde_json::json!(failure.label()),
                    ),
                    (
                        "requested_source".to_owned(),
                        serde_json::json!("canonical_reference"),
                    ),
                    (
                        "plate_id".to_owned(),
                        serde_json::json!(df_observe::presentation::local_identity(&id)),
                    ),
                ]),
            );
            book.insert(id, Entry::Failed);
            return;
        }
        if dependencies.contains(&super::canonical::ReferenceStatus::Pending) {
            return;
        }
        let spec = match scene.stage(heroes) {
            Ok(spec) => spec,
            Err(_) => {
                record_plate_diagnostic(
                    "gameplay.living_scene.plate_not_planned",
                    format!(
                        "PLATE not_planned scene_id={} revision={} reason=invalid_stage",
                        df_observe::presentation::local_identity(&scene.source_identity()),
                        graph_revision
                    ),
                    serde_json::Map::from_iter([
                        (
                            "scene_id".to_owned(),
                            serde_json::json!(df_observe::presentation::local_identity(
                                &scene.source_identity()
                            )),
                        ),
                        (
                            "plate_id".to_owned(),
                            serde_json::json!(df_observe::presentation::local_identity(&id)),
                        ),
                        (
                            "graph_revision".to_owned(),
                            serde_json::json!(graph_revision),
                        ),
                        (
                            "requested_source".to_owned(),
                            serde_json::json!("scene_plate"),
                        ),
                        (
                            "failure_category".to_owned(),
                            serde_json::json!("invalid_stage"),
                        ),
                    ]),
                );
                book.insert(id, Entry::NotPlanned);
                return;
            }
        };
        let job = Job::Fresh {
            id: id.clone(),
            graph_revision,
            scene: scene.clone(),
            spec: Box::new(spec),
            heroes: heroes.to_vec(),
            faces: faces.to_vec(),
            ahead,
        };
        // A full queue (the worker is busy or gone) drops the request: the next view asks again,
        // and nothing waits.
        if self.jobs.try_send(job).is_ok() {
            record(
                "gameplay.living_scene.plate_requested",
                &[("scene.kind", "plate".to_owned())],
            );
            let scene_key = scene.source_identity();
            let scene_id = df_observe::presentation::local_identity(&scene_key);
            let plate_id = df_observe::presentation::local_identity(&id);
            record_plate_diagnostic(
                "gameplay.living_scene.plate_requested",
                format!(
                    "PLATE requested scene_id={scene_id} revision={graph_revision} ahead={ahead} plate_id={plate_id}"
                ),
                serde_json::Map::from_iter([
                    ("scene_id".to_owned(), serde_json::json!(scene_id)),
                    ("plate_id".to_owned(), serde_json::json!(plate_id)),
                    (
                        "graph_revision".to_owned(),
                        serde_json::json!(graph_revision),
                    ),
                    ("ahead".to_owned(), serde_json::json!(ahead)),
                    (
                        "requested_source".to_owned(),
                        serde_json::json!("scene_plate"),
                    ),
                    ("status".to_owned(), serde_json::json!("queued")),
                ]),
            );
            book.request_started
                .insert(id.clone(), std::time::Instant::now());
            if let Some(current) = book
                .current
                .as_mut()
                .filter(|current| current.id.as_deref() == Some(id.as_str()))
            {
                current.request_started = book.request_started.get(&id).copied();
                current.last_projection = None;
            }
            book.insert(id, Entry::Pending);
        } else {
            record_plate_diagnostic(
                "gameplay.living_scene.plate_queue_dropped",
                format!(
                    "PLATE dropped scene_id={} revision={} reason=queue_full plate_id={}",
                    df_observe::presentation::local_identity(&scene.source_identity()),
                    graph_revision,
                    df_observe::presentation::local_identity(&id),
                ),
                serde_json::Map::from_iter([
                    (
                        "scene_id".to_owned(),
                        serde_json::json!(df_observe::presentation::local_identity(
                            &scene.source_identity()
                        )),
                    ),
                    (
                        "plate_id".to_owned(),
                        serde_json::json!(df_observe::presentation::local_identity(&id)),
                    ),
                    (
                        "graph_revision".to_owned(),
                        serde_json::json!(graph_revision),
                    ),
                    ("ahead".to_owned(), serde_json::json!(ahead)),
                    (
                        "requested_source".to_owned(),
                        serde_json::json!("scene_plate"),
                    ),
                    (
                        "failure_category".to_owned(),
                        serde_json::json!("queue_full"),
                    ),
                ]),
            );
        }
    }

    fn apply(&self, scene_key: &str, scene: &mut rpc::GameplayScene) -> Option<u64> {
        let book = locked(&self.book);
        let Some(current) = &book.current else {
            return None;
        };
        if current.scene_key != scene_key || current.in_combat {
            return None;
        }
        scene.plate_update_pending = book.current_update_pending(scene_key);
        if book.direct.shown.is_some()
            && let Some((key, placement)) = book.shown()
        {
            scene.live_plate = key.path();
            scene.plate_layout = placement.as_ref().map(wire_layout);
            scene.plate_state = rpc::PlateState::Ready as i32;
            return Some(
                book.state_images
                    .baked_revision
                    .unwrap_or(current.graph_revision),
            );
        }
        match book.current_entry() {
            Some(Entry::Ready { .. }) => {
                if let Some((key, placement)) = book.shown() {
                    scene.live_plate = key.path();
                    scene.plate_layout = placement.as_ref().map(wire_layout);
                    scene.plate_state = rpc::PlateState::Ready as i32;
                    return Some(
                        book.state_images
                            .baked_revision
                            .unwrap_or(current.graph_revision),
                    );
                }
            }
            Some(Entry::Failed) => {
                scene.plate_state = rpc::PlateState::Failed as i32;
            }
            Some(Entry::NotPlanned) => {
                scene.plate_state = rpc::PlateState::Unavailable as i32;
            }
            // A planned scene whose request is not in the book yet is pending too: the display
            // never mistakes a plate that is on its way for a scene without one.
            Some(Entry::Pending) | None => {
                if current.scene.is_some() {
                    scene.plate_state = rpc::PlateState::Pending as i32;
                } else if current.unplated {
                    scene.plate_state = rpc::PlateState::Unavailable as i32;
                }
            }
        }
        None
    }

    fn record_projection(
        &self,
        scene_key: &str,
        scene: &rpc::GameplayScene,
        plate_graph_revision: u64,
    ) {
        let (line, fields) = {
            let mut book = locked(&self.book);
            let original_key = match book.current_entry() {
                Some(Entry::Ready { key, .. }) => Some(key.as_str().to_owned()),
                _ => None,
            };
            let Some(current) = book.current.as_mut() else {
                return;
            };
            if current.scene_key != scene_key {
                return;
            }
            let ready =
                scene.plate_state == rpc::PlateState::Ready as i32 && !scene.live_plate.is_empty();
            let (source, asset) = if ready {
                ("live_plate", scene.live_plate.as_str())
            } else if !scene.live_backdrop.is_empty() {
                ("live_backdrop", scene.live_backdrop.as_str())
            } else {
                ("scene_asset", scene.scene_asset.as_str())
            };
            let projection = PlateProjection {
                graph_revision: current.graph_revision,
                plate_graph_revision,
                plate_id: current.id.clone(),
                plate_state: scene.plate_state,
                source: source.to_owned(),
                asset: asset.to_owned(),
            };
            if current.last_projection.as_ref() == Some(&projection) {
                return;
            }
            current.last_projection = Some(projection);
            let requested_source = if current.id.is_some() {
                "scene_plate"
            } else {
                "scene_asset"
            };
            let scene_id = df_observe::presentation::local_identity(scene_key);
            let state = rpc::PlateState::try_from(scene.plate_state)
                .map_or("unrecognized", |state| state.as_str_name());
            let scene_request_age_ms = current
                .request_started
                .map(|started| started.elapsed().as_millis());
            let live_plate_key = scene
                .live_plate
                .strip_prefix("/gameplay/imagery/")
                .filter(|key| key.len() == 64);
            let image_key = ready.then_some(live_plate_key).flatten();
            let elapsed_ms = projection_request_latency(
                &mut current.first_projection_ms,
                scene_request_age_ms,
                image_key,
                original_key.as_deref(),
            );
            let image_key_id = image_key.map(df_observe::presentation::local_identity);
            let plate_id = current
                .id
                .as_deref()
                .map(df_observe::presentation::local_identity);
            let effective_asset_id = df_observe::presentation::local_identity(asset);
            let line = format!(
                "PLATE projected scene_id={scene_id} revision={} plate_id={} state={} requested_source={} effective_source={} effective_asset_id={effective_asset_id} image_key={} image_key_id={} request_to_projection_ms={} scene_request_age_ms={}",
                current.graph_revision,
                plate_id.map_or_else(|| "none".to_owned(), |id| id.to_string()),
                state,
                requested_source,
                source,
                image_key.unwrap_or("none"),
                image_key_id.map_or_else(|| "none".to_owned(), |id| id.to_string()),
                elapsed_ms.map_or_else(|| "none".to_owned(), |ms| ms.to_string()),
                scene_request_age_ms.map_or_else(|| "none".to_owned(), |ms| ms.to_string()),
            );
            let fields = serde_json::Map::from_iter([
                ("scene_id".to_owned(), serde_json::json!(scene_id)),
                (
                    "graph_revision".to_owned(),
                    serde_json::json!(current.graph_revision),
                ),
                (
                    "plate_graph_revision".to_owned(),
                    serde_json::json!(plate_graph_revision),
                ),
                ("plate_id".to_owned(), serde_json::json!(plate_id)),
                ("plate_state".to_owned(), serde_json::json!(state)),
                (
                    "requested_source".to_owned(),
                    serde_json::json!(requested_source),
                ),
                (
                    "server_effective_source".to_owned(),
                    serde_json::json!(source),
                ),
                (
                    "effective_asset_id".to_owned(),
                    serde_json::json!(effective_asset_id),
                ),
                ("image_key".to_owned(), serde_json::json!(image_key)),
                ("image_key_id".to_owned(), serde_json::json!(image_key_id)),
                (
                    "scene_request_age_ms".to_owned(),
                    serde_json::json!(scene_request_age_ms),
                ),
                (
                    "request_to_projection_ms".to_owned(),
                    serde_json::json!(elapsed_ms),
                ),
            ]);
            (line, fields)
        };
        record_plate_diagnostic("gameplay.living_scene.plate_projected", line, fields);
    }

    /// The worker: takes requests, owns every render task, and every tick looks for a lasting face
    /// to edit into the current plate.
    async fn run(self: Arc<Self>, mut queue: mpsc::Receiver<Job>) {
        let mut ticker = tokio::time::interval(POLL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut tasks: JoinSet<Done> = JoinSet::new();
        loop {
            tokio::select! {
                Some(job) = queue.recv() => self.spawn(&mut tasks, job),
                Some(done) = tasks.join_next(), if !tasks.is_empty() => {
                    // A render task that panicked or was cancelled has nothing to report.
                    if let Ok(done) = done {
                        self.finish(done);
                    }
                }
                _ = ticker.tick() => {
                    self.poll_state_images();
                    self.poll_face();
                },
            }
        }
    }

    fn spawn(self: &Arc<Self>, tasks: &mut JoinSet<Done>, job: Job) {
        let shared = Arc::clone(self);
        match job {
            Job::State(job) => {
                tasks.spawn(async move { Done::State(shared.render_state_image(job).await) });
            }
            Job::Directed(work) => {
                tasks.spawn(async move { shared.render_directed(*work).await });
            }
            Job::Fresh {
                id,
                graph_revision,
                scene,
                spec,
                heroes,
                faces,
                ahead,
            } => {
                tasks.spawn(async move {
                    if ahead {
                        tokio::time::sleep(AHEAD_DELAY).await;
                    }
                    let started = tokio::time::Instant::now();
                    let result = shared
                        .render_fresh(&scene, &spec, &heroes, &faces, &id, graph_revision, ahead)
                        .await;
                    let result = result.map(|(key, placement, locate)| {
                        // The plate is shown with its predicted layout as soon as it lands; the
                        // measured one follows. A full queue keeps the prediction.
                        if let Some(work) = locate {
                            let _kept_prediction = shared.jobs.try_send(Job::Locate {
                                id: id.clone(),
                                work,
                            });
                        }
                        (key, placement)
                    });
                    Done::Fresh {
                        id,
                        graph_revision,
                        ahead,
                        elapsed: started.elapsed(),
                        result,
                    }
                });
            }
            Job::Locate { id, work } => {
                tasks.spawn(async move {
                    let started = tokio::time::Instant::now();
                    let placement = shared
                        .measured_or_predicted(
                            &work.key,
                            &work.spec,
                            &work.speaker,
                            &work.actor_references,
                            &work.actors,
                            work.focus_actor_id.as_deref(),
                        )
                        .await;
                    Done::Located {
                        id,
                        key: work.key,
                        elapsed: started.elapsed(),
                        placement,
                    }
                });
            }
            Job::Edit {
                scene_key,
                scene,
                base,
                face,
                heroes,
            } => {
                tasks.spawn(async move {
                    let started = tokio::time::Instant::now();
                    let result = shared.render_edit(&scene, &base, face, &heroes).await;
                    if let Ok(key) = &result {
                        shared.measure_scene_plate(key, &scene, &heroes).await;
                    }
                    Done::Edit {
                        scene_key,
                        speaker_id: scene.speaker.map(|speaker| speaker.id),
                        face,
                        elapsed: started.elapsed(),
                        result,
                    }
                });
            }
        }
    }

    /// The art that shows a speaker in a plate: the full-length portrait (or figure) and the close
    /// portrait of the face, each when it exists.
    async fn speaker_art(&self, sprite: &str) -> (Option<ReferenceImage>, Option<ReferenceImage>) {
        // Generated campaigns can carry a registered immutable identity directly instead of
        // a checked-in archetype stem. Pin those exact bytes in the same sheet contract.
        if let Some(key) = sprite.strip_prefix("/gameplay/imagery/") {
            return match self.inner.imagery.image(key).await {
                Lookup::Image { bytes, format } => (Some(ReferenceImage::new(format, bytes)), None),
                Lookup::Unregistered | Lookup::NotReady => (None, None),
            };
        }
        let stem = sprite.rsplit('/').next().unwrap_or(sprite);
        let Some(name) = stem.strip_prefix("sprite-") else {
            return (
                self.inner
                    .load_plate_reference(sprite.to_owned())
                    .await
                    .ok(),
                None,
            );
        };
        let mut figure = None;
        for candidate in [
            format!("portrait-{name}-full"),
            format!("figure-{name}"),
            stem.to_owned(),
        ] {
            if let Ok(art) = self.inner.load_plate_reference(candidate).await {
                figure = Some(art);
                break;
            }
        }
        let face = self
            .inner
            .load_plate_reference(format!("portrait-{name}"))
            .await
            .ok();
        (figure, face)
    }

    /// All canonical identities fit in one contact sheet, so no cast member loses their
    /// reference when the provider's four-image budget is reached. Missing art is a failure,
    /// distinct from a provider failure; the display retains its known good photograph.
    async fn identity_sheet(
        &self,
        scene: &PlateScene,
        heroes: &[u8],
    ) -> Result<(ReferenceImage, String), ImageFailure> {
        let mut wanted = scene.identity_cast.clone();
        let mut cast = Vec::new();
        for art in heroes {
            if let Some((entity, figure)) = self.inner.canonical_hero_reference(*art).await {
                let entity = format!("{:x}", sha2::Sha256::digest(entity.as_bytes()));
                cast.push(CastIdentity {
                    id: format!("hero:{entity}"),
                    figure,
                });
            } else if self.inner.canonical_hero_status(*art)
                == super::canonical::ReferenceStatus::Missing
            {
                wanted.push(IdentityReference {
                    id: format!("hero:{art}"),
                    name: String::new(),
                    sprite: format!("sprite-hero-{art}"),
                    portrait: String::new(),
                });
            } else {
                return Err(ImageFailure::Interrupted);
            }
        }
        for IdentityReference {
            id,
            sprite,
            portrait,
            name: _,
        } in wanted
        {
            let canonical_id = id.strip_prefix("npc:").unwrap_or(&id);
            if let Some(figure) = self.inner.canonical_npc_reference(canonical_id).await {
                cast.push(CastIdentity { id, figure });
                continue;
            }
            if self.inner.canonical_npc_status(canonical_id)
                != super::canonical::ReferenceStatus::Missing
            {
                return Err(ImageFailure::Interrupted);
            }
            let (figure, _) = self.speaker_art(&sprite).await;
            let figure = match figure {
                Some(figure) => Some(figure),
                None if !portrait.is_empty() => self.speaker_art(&portrait).await.0,
                None => None,
            };
            let Some(figure) = figure else {
                let scene_id = df_observe::presentation::local_identity(&scene.source_identity());
                crate::gameplay::actor::record_audit_diagnostic(
                    "gameplay.living_scene.plate_identity_missing",
                    format!("PLATE identity_missing scene_id={scene_id} cast_id={id}"),
                    serde_json::Map::from_iter([
                        ("scene_id".to_owned(), serde_json::json!(scene_id)),
                        ("cast_id".to_owned(), serde_json::json!(id)),
                        (
                            "status".to_owned(),
                            serde_json::json!("missing_photographic_identity"),
                        ),
                    ]),
                    &[
                        ("cast.id", id),
                        (
                            "reference.status",
                            "missing_photographic_identity".to_owned(),
                        ),
                    ],
                );
                return Err(ImageFailure::InvalidRequest);
            };
            cast.push(CastIdentity { id, figure });
        }
        let sources: Vec<serde_json::Value> = cast.iter().map(|identity| serde_json::json!({"cast_id": identity.id, "source_sha256": identity.figure.sha256})).collect();
        let result = tokio::task::spawn_blocking(move || cast_sheet(cast))
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .map_err(|_| ImageFailure::InvalidRequest)?;
        crate::gameplay::actor::record_audit_diagnostic(
            "gameplay.living_scene.plate_identity_ready",
            format!(
                "PLATE identity sheet_sha256={} cast_count={}",
                result.0.sha256,
                sources.len()
            ),
            serde_json::Map::from_iter([
                (
                    "sheet_sha256".to_owned(),
                    serde_json::json!(result.0.sha256),
                ),
                ("sources".to_owned(), serde_json::json!(sources)),
                ("status".to_owned(), serde_json::json!("complete_cast")),
            ]),
            &[
                ("reference.sha256", result.0.sha256.clone()),
                ("reference.status", "complete_cast".to_owned()),
            ],
        );
        Ok(result)
    }

    async fn photographic_request(
        &self,
        scene: &PlateScene,
        spec: &ShotSpec,
        heroes: &[u8],
        direction: Option<&str>,
    ) -> Result<(ImageRequest, Option<ReferenceImage>), ImageFailure> {
        let (sheet, legend) = self.identity_sheet(scene, heroes).await?;
        let edit_harbor = scene.exterior
            && scene.place_id == "salt-quay"
            && scene.asset.contains("scene-salt-quay-arrival");
        let base = if edit_harbor {
            "scene-salt-quay-candid".to_owned()
        } else {
            scene.asset.clone()
        };
        let canonical = match scene.room_revision.as_deref() {
            Some(revision) => {
                self.inner
                    .canonical_room_reference_at(&scene.place_id, revision)
                    .await
            }
            None => None,
        };
        let place = match canonical {
            Some(reference) => Ok(reference),
            None if scene.room_revision.is_some() => {
                return Err(ImageFailure::Interrupted);
            }
            None => self.inner.load_plate_reference(base).await,
        };
        let (place, style_only) = match place {
            Ok(place) => (place, false),
            Err(_) if !edit_harbor => (
                self.inner
                    .load_plate_reference("scene-salt-quay-arrival".to_owned())
                    .await?,
                true,
            ),
            Err(error) => return Err(error),
        };
        // Stable ordering on fresh builds and rebuilds: base photograph, complete identity
        // sheet, then optional speaker close-up. Never let a close-up displace another person.
        let mut references = vec![place, sheet];
        let (figure, face) = match &scene.speaker {
            Some(speaker) => match self.inner.canonical_npc_reference(&speaker.id).await {
                Some(figure) => (Some(figure), None),
                None => self.speaker_art(&speaker.sprite).await,
            },
            None => (None, None),
        };
        let locator_face = face.clone().or_else(|| figure.clone());
        references.extend(face.or(figure));
        let prompt = if edit_harbor {
            super::super::imagery::harbor_photo_edit(
                self.inner.rating,
                spec,
                scene.snapshot.as_ref().map(|snapshot| &snapshot.model),
                &scene.hero_names,
            )?
        } else if let Some(snapshot) = &scene.snapshot {
            super::super::imagery::model_plate(
                self.inner.rating,
                &snapshot.model,
                &scene.place_id,
                &scene.hero_names,
            )?
        } else {
            scene_plate(
                self.inner.rating,
                spec,
                scene.place.as_ref(),
                PlateReferences {
                    party_sheet: false,
                    speaker: false,
                    speaker_face: false,
                    place: true,
                },
            )
            .map_err(|_| ImageFailure::InvalidRequest)?
        };
        let prompt = if style_only {
            prompt.with_photographic_style_reference()
        } else {
            prompt
        };
        let prompt = if let Some(speaker) = &scene.speaker {
            let identity = scene.identity_cast.iter().find(|identity| {
                identity.id.strip_prefix("npc:").unwrap_or(&identity.id) == speaker.id
            });
            identity.map_or(prompt.clone(), |identity| {
                prompt.with_focus_actor(&speaker.id, &identity.name)
            })
        } else {
            prompt
        };
        let prompt = prompt.with_cast_sheet(2, &legend);
        let prompt = match direction {
            Some(direction) => prompt.with_development_direction(direction),
            None => prompt,
        };
        Ok((
            ImageRequest {
                kind: ImageKind::ScenePlate,
                references,
                prompt,
                output: OutputSpec::PLATE,
            },
            locator_face,
        ))
    }

    async fn fresh_request(
        &self,
        scene: &PlateScene,
        spec: &ShotSpec,
        heroes: &[u8],
        faces: &[(u8, ImageKey)],
        direction: Option<&str>,
    ) -> Result<(ImageRequest, Option<ReferenceImage>), ImageFailure> {
        if self.inner.references.photographic_only() {
            return self
                .photographic_request(scene, spec, heroes, direction)
                .await;
        }
        let mut references = Vec::new();
        let edit_documentary_photo = self.inner.references.photographic_only()
            && scene.exterior
            && scene.place_id == "salt-quay"
            && scene.asset.contains("scene-salt-quay-arrival");
        if edit_documentary_photo {
            references.push(
                self.inner
                    .load_plate_reference("scene-salt-quay-candid".to_owned())
                    .await?,
            );
        }
        let mut has_party_sheet = false;
        let mut sorted = heroes.to_vec();
        sorted.sort_unstable();
        if !sorted.is_empty() {
            let mut figures = Vec::with_capacity(sorted.len());
            for art in &sorted {
                if self.inner.references.photographic_only() {
                    let Ok(figure) = self
                        .inner
                        .load_plate_reference(format!("figure-hero-{art}"))
                        .await
                    else {
                        figures.clear();
                        break;
                    };
                    figures.push(figure);
                    continue;
                }
                // A hurt hero stands in the plate as the cut-out stage shows them: their health
                // variant, taken only from what is already made and cached.
                let variant = match faces.iter().find(|(known, _)| known == art) {
                    Some((_, key)) => match self.inner.imagery.image(key.as_str()).await {
                        Lookup::Image { bytes, format } => Some(ReferenceImage::new(format, bytes)),
                        Lookup::Unregistered | Lookup::NotReady => None,
                    },
                    None => None,
                };
                figures.push(match variant {
                    Some(variant) => variant,
                    None => self.inner.load(format!("figure-hero-{art}")).await?,
                });
            }
            if figures.len() == sorted.len() {
                let sheet = tokio::task::spawn_blocking(move || party_sheet(&figures))
                    .await
                    .map_err(|_| ImageFailure::Interrupted)?
                    .map_err(|_| ImageFailure::InvalidRequest)?;
                references.push(sheet);
                has_party_sheet = true;
            }
        }
        // The cast's art leads: the full-length figure and a close portrait of the face. Both
        // go into every plate that shows the speaker, because a figure alone let the same person
        // drift between two scenes' plates.
        let (figure, face) = match &scene.speaker {
            Some(speaker) => self.speaker_art(&speaker.sprite).await,
            None => (None, None),
        };
        let has_speaker_art = figure.is_some();
        let has_speaker_face = has_speaker_art && face.is_some();
        let locator_face = face.clone().or_else(|| figure.clone());
        references.extend(figure);
        if has_speaker_face {
            references.extend(face);
        }
        let place = if edit_documentary_photo {
            None
        } else {
            self.inner
                .load_plate_reference(scene.asset.clone())
                .await
                .ok()
        };
        let has_place = place.is_some();
        references.extend(place);
        // A reviewed location photograph anchors the medium even when this scene has no
        // photographic place art. It supplies no scene facts; the prompt makes that explicit.
        let photographic_style_reference =
            !edit_documentary_photo && !has_place && self.inner.references.photographic_only();
        if photographic_style_reference {
            references.push(
                self.inner
                    .load_plate_reference("scene-salt-quay-arrival".to_owned())
                    .await?,
            );
        }
        // Keep the remaining cast figures only while the provider's reference limit allows them.
        // They are also named in the prompt, so dropping the lowest-priority art preserves the
        // request's scene facts while keeping the party, speaker and place references intact.
        for sprite in scene
            .others
            .iter()
            .take(MAXIMUM_PLATE_REFERENCES.saturating_sub(references.len()))
        {
            let (figure, _) = self.speaker_art(sprite).await;
            references.extend(figure);
        }
        // The image edit adapter requires at least one reference even when the scene has no
        // generated photographic identity art yet. Use the checked-in neutral photo as a carrier
        // so the request reaches the provider; the prompt explicitly tells it to rebuild the
        // entire plate from the written scene direction.
        let mut neutral_reference = false;
        if references.is_empty() && self.inner.references.photographic_only() {
            references.push(self.inner.neutral_photographic_reference().await?);
            neutral_reference = true;
        }
        let prompt = if edit_documentary_photo {
            super::super::imagery::harbor_photo_edit(
                self.inner.rating,
                spec,
                scene.snapshot.as_ref().map(|snapshot| &snapshot.model),
                &scene.hero_names,
            )?
        } else if let Some(snapshot) = &scene.snapshot {
            super::super::imagery::model_plate(
                self.inner.rating,
                &snapshot.model,
                &scene.place_id,
                &scene.hero_names,
            )?
        } else {
            scene_plate(
                self.inner.rating,
                spec,
                scene.place.as_ref(),
                PlateReferences {
                    party_sheet: has_party_sheet,
                    speaker: has_speaker_art,
                    speaker_face: has_speaker_face,
                    place: has_place,
                },
            )
            .map_err(|_| ImageFailure::InvalidRequest)?
        };
        let prompt = if neutral_reference {
            prompt.with_neutral_reference()
        } else if photographic_style_reference {
            prompt.with_photographic_style_reference()
        } else {
            prompt
        };
        let prompt = if let Some(speaker) = &scene.speaker {
            let identity = scene.identity_cast.iter().find(|identity| {
                identity.id.strip_prefix("npc:").unwrap_or(&identity.id) == speaker.id
            });
            identity.map_or(prompt.clone(), |identity| {
                prompt.with_focus_actor(&speaker.id, &identity.name)
            })
        } else {
            prompt
        };
        let prompt = match direction {
            Some(direction) => prompt.with_development_direction(direction),
            None => prompt,
        };
        Ok((
            ImageRequest {
                kind: ImageKind::ScenePlate,
                references,
                prompt,
                output: OutputSpec::PLATE,
            },
            locator_face,
        ))
    }

    async fn render_fresh(
        &self,
        scene: &PlateScene,
        spec: &ShotSpec,
        heroes: &[u8],
        faces: &[(u8, ImageKey)],
        plate_id: &str,
        graph_revision: u64,
        ahead: bool,
    ) -> Result<(ImageKey, Option<PlatePlacement>, Option<Box<Locate>>), ImageFailure> {
        let started = std::time::Instant::now();
        let mut image_key = None;
        let result = async {
            let (request, _locator_face) =
                self.fresh_request(scene, spec, heroes, faces, None).await?;
            let key = self.inner.imagery.key_for(&request);
            image_key = Some(key.clone());
            record_plate_render_diagnostic(
                "gameplay.living_scene.plate_render_requested",
                "render_requested",
                scene,
                spec,
                plate_id,
                graph_revision,
                ahead,
                Some(&key),
                None,
                started.elapsed().as_millis(),
            );
            let key = self.inner.imagery.render(request).await?;
            let Some(speaker) = &spec.speaker else {
                return Ok((key, None, None));
            };
            let actor_references = self.locator_references(scene, heroes).await;
            if let Some(stored) = self
                .stored_placement(&key, &scene.locator_actors(), &actor_references)
                .await
            {
                return Ok((key, Some(stored), None));
            }
            let actors = scene.locator_actors();
            let predicted = PlatePlacement {
                speaker: speaker.layout,
                heroes: Vec::new(),
                actors: Vec::new(),
            };
            let work = Locate {
                focus_actor_id: scene.speaker.as_ref().map(|speaker| speaker.id.clone()),
                actors,
                key: key.clone(),
                spec: spec.clone(),
                speaker: speaker.clone(),
                actor_references,
            };
            Ok((key, Some(predicted), Some(Box::new(work))))
        }
        .await;
        let failure = result.as_ref().err().copied();
        let event = if failure.is_some() {
            "gameplay.living_scene.plate_failed"
        } else {
            "gameplay.living_scene.plate_ready"
        };
        record_plate_render_diagnostic(
            event,
            if failure.is_some() { "failed" } else { "ready" },
            scene,
            spec,
            plate_id,
            graph_revision,
            ahead,
            image_key.as_ref(),
            failure,
            started.elapsed().as_millis(),
        );
        crate::gameplay::actor::record_audit_line(
            "gameplay.imagery_complete",
            super::image_completion_line(
                "scene_plate",
                started.elapsed(),
                result.as_ref().map(|(key, _, _)| key),
            ),
        );
        result
    }

    /// The placement kept beside a cached plate by an earlier measurement, when it is intact.
    async fn stored_placement(
        &self,
        key: &ImageKey,
        actors: &[(String, String)],
        references: &BTreeMap<String, ReferenceImage>,
    ) -> Option<PlatePlacement> {
        let started = tokio::time::Instant::now();
        let note = self.inner.imagery.read_note(key).await?;
        let stored = serde_json::from_slice::<StoredPlacement>(&note).ok()?;
        if stored.actor_reference_sha256 != reference_hashes(references)
            || actors.iter().any(|(id, _)| !stored.actor_ids.contains(id))
            || stored.locator_revision != 2
        {
            return None;
        }
        let mut placement = stored.placement()?;
        placement
            .actors
            .retain(|head| actors.iter().any(|(id, _)| id == &head.actor_id));
        record_actor_heads(
            key,
            actors,
            &placement,
            "durable_note",
            started.elapsed().as_millis(),
            None,
        );
        Some(placement)
    }

    /// Where the heads are in the finished plate, as the vision model finds them: the speaker's
    /// and those of the heroes it asked for. A placement found before is kept beside the cached
    /// plate and read back, so a later server run does not ask again. When the speaker cannot be
    /// found (no provider, a failure, an implausible answer) the layout the director predicted
    /// stands, with no hero positions. The image model does not place anyone where it is asked.
    async fn measured_or_predicted(
        &self,
        key: &ImageKey,
        spec: &ShotSpec,
        speaker: &shot::SpeakerShot,
        actor_references: &BTreeMap<String, ReferenceImage>,
        actors: &[(String, String)],
        focus_actor_id: Option<&str>,
    ) -> PlatePlacement {
        let missing_focus_identity =
            !focus_actor_id.is_some_and(|id| actors.iter().any(|(actor, _)| actor == id));
        if missing_focus_identity {
            HeadDiagnostic {
                key,
                stage: "head_location",
                reference_sha256: None,
                source: "identity_validation",
                batch: None,
                actor_ids: Vec::new(),
                targets: vec!["speaker".to_owned()],
                located: Vec::new(),
                elapsed_ms: 0,
                failure: Some("missing_focus_identity"),
            }
            .record();
        }
        let known = locked(&self.book).image_layouts.get(key.as_str()).cloned();
        if let Some(known) = known {
            record_actor_heads(key, actors, &known, "in_process", 0, None);
            return known;
        }
        if let Some(stored) = self.stored_placement(key, actors, actor_references).await {
            return stored;
        }
        let (focus_id, targets) = locator_targets(spec, speaker, actors, focus_actor_id);
        let started = tokio::time::Instant::now();
        let mut located = Vec::new();
        let batches = referenced_locator_batches(&targets, actors, actor_references);
        let mut measured_all_batches = targets.len() <= 16
            && actors
                .iter()
                .all(|(id, _)| actor_references.contains_key(id))
            && batches
                .iter()
                .map(|batch| batch.targets.len())
                .sum::<usize>()
                == targets.len();
        let mut failure = actors
            .iter()
            .any(|(id, _)| !actor_references.contains_key(id))
            .then_some("missing_reference");
        if !measured_all_batches {
            failure = Some("missing_reference_or_batch_limit");
        }
        if missing_focus_identity {
            failure = Some("missing_focus_identity");
        }
        for stage in ["requested", "queued"] {
            HeadDiagnostic {
                key,
                stage,
                reference_sha256: None,
                source: "provider",
                batch: None,
                actor_ids: Vec::new(),
                targets: targets.iter().map(|(id, _)| id.clone()).collect(),
                located: Vec::new(),
                elapsed_ms: 0,
                failure: None,
            }
            .record();
        }
        let pending = FuturesUnordered::new();
        for (batch, work) in batches.iter().enumerate() {
            for stage in ["requested", "queued"] {
                HeadDiagnostic {
                    key,
                    stage,
                    reference_sha256: Some(work.reference.sha256.as_str()),
                    source: "provider",
                    batch: Some(batch),
                    actor_ids: work
                        .targets
                        .iter()
                        .filter_map(|(id, _)| {
                            let index = id.strip_prefix("npc-")?.parse::<usize>().ok()?;
                            Some((id.clone(), actors.get(index)?.0.clone()))
                        })
                        .collect(),
                    targets: work.targets.iter().map(|(id, _)| id.clone()).collect(),
                    located: Vec::new(),
                    elapsed_ms: 0,
                    failure: None,
                }
                .record();
            }
            pending.push(async move {
                let borrowed: Vec<(&str, &str)> = work
                    .targets
                    .iter()
                    .map(|(id, description)| (id.as_str(), description.as_str()))
                    .collect();
                let heads = self
                    .inner
                    .imagery
                    .locate_heads(key, Some(work.reference), &borrowed)
                    .await;
                let result = HeadResult {
                    elapsed_ms: started.elapsed().as_millis(),
                    heads,
                };
                record_head_result(key, batch, work, actors, &result);
                (batch, result)
            });
        }
        let results =
            collect_head_results(pending, batches.len(), started + HEAD_IMAGE_DEADLINE).await;
        for (batch, (work, result)) in batches.iter().zip(results).enumerate() {
            let result = match result {
                Some(result) => result,
                None => {
                    let result = HeadResult {
                        elapsed_ms: started.elapsed().as_millis(),
                        heads: Err(ImageFailure::DeadlineExceeded),
                    };
                    record_head_result(key, batch, work, actors, &result);
                    result
                }
            };
            failure = failure.or(result.heads.as_ref().err().map(|failure| failure.label()));
            measured_all_batches &= result.heads.is_ok();
            located.extend(result.heads.unwrap_or_default());
        }
        if targets.len() > 16 {
            record(
                "gameplay.living_scene.head_overflow",
                &[
                    ("image_key", key.as_str().to_owned()),
                    ("target_count", targets.len().to_string()),
                    ("status", "named_lane_after_16_people".to_owned()),
                ],
            );
        }
        let focus_head = located.iter().find(|head| head.id == focus_id);
        let heroes = spec
            .heroes
            .iter()
            .filter_map(|hero| {
                let id = format!("hero-{}", hero.art);
                let found = located.iter().find(|head| head.id == id)?;
                Some(HeroHead {
                    art: hero.art,
                    layout: shot::measured_layout(
                        found.head.left,
                        found.head.top,
                        found.head.right,
                        found.head.bottom,
                    ),
                })
            })
            .collect();
        let actor_heads = actors
            .iter()
            .enumerate()
            .filter_map(|(index, (id, _))| {
                let found = located
                    .iter()
                    .find(|head| head.id == format!("npc-{index}"))?;
                Some(ActorHead {
                    actor_id: id.clone(),
                    layout: shot::measured_layout(
                        found.head.left,
                        found.head.top,
                        found.head.right,
                        found.head.bottom,
                    ),
                })
            })
            .collect();
        let placement = PlatePlacement {
            actors: actor_heads,
            speaker: focus_head.map_or(speaker.layout, |head| {
                shot::measured_layout(
                    head.head.left,
                    head.head.top,
                    head.head.right,
                    head.head.bottom,
                )
            }),
            heroes,
        };
        record_actor_heads(
            key,
            actors,
            &placement,
            "provider",
            started.elapsed().as_millis(),
            failure,
        );
        let mut stored = StoredPlacement::of(&placement);
        stored.actor_ids = actors.iter().map(|(id, _)| id.clone()).collect();
        stored.actor_reference_sha256 = reference_hashes(actor_references);
        stored.locator_revision = 2;
        if measured_all_batches && let Ok(note) = serde_json::to_vec(&stored) {
            self.inner.imagery.write_note(key, note).await;
        }
        let mut book = locked(&self.book);
        if book.image_layouts.len() >= MAXIMUM_ENTRIES {
            book.image_layouts.pop_first();
        }
        book.image_layouts
            .insert(key.as_str().to_owned(), placement.clone());
        placement
    }

    /// Use the pinned character bytes, never a role/name guess, for each NPC's lookup.
    async fn locator_references(
        &self,
        scene: &PlateScene,
        heroes: &[u8],
    ) -> BTreeMap<String, ReferenceImage> {
        let mut references = BTreeMap::new();
        for actor in scene.identity_cast.iter().take(16) {
            let (figure, face) = self.speaker_art(&actor.sprite).await;
            let reference = match face.or(figure) {
                Some(reference) => Some(reference),
                None if !actor.portrait.is_empty() => {
                    let (figure, face) = self.speaker_art(&actor.portrait).await;
                    face.or(figure)
                }
                None => None,
            };
            if let Some(reference) = reference {
                references.insert(
                    actor
                        .id
                        .strip_prefix("npc:")
                        .unwrap_or(&actor.id)
                        .to_owned(),
                    reference,
                );
            }
        }
        for art in heroes.iter().take(16) {
            let (figure, face) = self.speaker_art(&format!("sprite-hero-{art}")).await;
            if let Some(reference) = face.or(figure) {
                references.insert(format!("hero-{art}"), reference);
            }
        }
        references
    }

    /// Every edited image owns its measured heads; an earlier photograph's boxes are never reused.
    async fn measure_scene_plate(&self, key: &ImageKey, scene: &PlateScene, heroes: &[u8]) {
        let Ok(spec) = scene.stage(heroes) else {
            return;
        };
        let Some(speaker) = spec.speaker.as_ref() else {
            return;
        };
        let _placement = self
            .measured_or_predicted(
                key,
                &spec,
                speaker,
                &self.locator_references(scene, heroes).await,
                &scene.locator_actors(),
                scene.speaker.as_ref().map(|speaker| speaker.id.as_str()),
            )
            .await;
    }

    async fn render_edit(
        &self,
        scene: &PlateScene,
        base: &ImageKey,
        face: Face,
        heroes: &[u8],
    ) -> Result<ImageKey, ImageFailure> {
        let Lookup::Image { bytes, format } = self.inner.imagery.image(base.as_str()).await else {
            return Err(ImageFailure::InvalidRequest);
        };
        let mut references = vec![ReferenceImage::new(format, bytes)];
        let (figure, _) = match &scene.speaker {
            Some(speaker) => self.speaker_art(&speaker.sprite).await,
            None => (None, None),
        };
        let has_speaker_art = figure.is_some();
        references.extend(figure);
        let prompt = plate_moment(
            self.inner.rating,
            face.expression,
            face.tier,
            has_speaker_art,
        )
        .map_err(|_| ImageFailure::InvalidRequest)?;
        let prompt = if self.inner.references.photographic_only() {
            let (sheet, legend) = self.identity_sheet(scene, heroes).await?;
            references.push(sheet);
            prompt.with_cast_sheet(references.len(), &legend)
        } else {
            prompt
        };
        let direction = locked(&self.book).direct.keep.clone();
        let prompt = match direction {
            Some(direction) => prompt.with_development_direction(&direction),
            None => prompt,
        };
        self.inner
            .imagery
            .render(ImageRequest {
                kind: ImageKind::ScenePlateEdit,
                references,
                prompt,
                output: OutputSpec::PLATE,
            })
            .await
    }

    /// Records a finished render. Every generation is logged with its time; the run manifest
    /// holds its cost.
    fn finish(&self, done: Done) {
        if let Done::State(done) = done {
            self.finish_state_image(done);
            return;
        }
        if let Done::Directed {
            generation,
            scene_key,
            key,
            result,
        } = done
        {
            self.finish_directed(generation, &scene_key, key, result);
            return;
        }
        let (kind, elapsed, outcome) = match &done {
            Done::State(_) => unreachable!("handled above"),
            Done::Directed { .. } => unreachable!("handled above"),
            Done::Fresh {
                elapsed, result, ..
            } => ("plate", *elapsed, result.as_ref().map(|_| ())),
            Done::Edit {
                elapsed, result, ..
            } => ("plate_edit", *elapsed, result.as_ref().map(|_| ())),
            Done::Located { elapsed, .. } => ("plate_heads", *elapsed, Ok(())),
        };
        record(
            "gameplay.living_scene.render_finished",
            &[
                ("scene.kind", kind.to_owned()),
                (
                    "scene.outcome",
                    outcome
                        .map_or_else(|failure| failure.label(), |()| "ready")
                        .to_owned(),
                ),
                ("scene.elapsed_ms", elapsed.as_millis().to_string()),
            ],
        );
        let mut book = locked(&self.book);
        match done {
            Done::State(_) => unreachable!("handled above"),
            Done::Directed { .. } => unreachable!("handled above"),
            Done::Located {
                id, key, placement, ..
            } => {
                // The measured heads replace the prediction of the same plate, if it is still there.
                if let Some(Entry::Ready {
                    key: shown,
                    placement: kept,
                }) = book.entries.get_mut(&id)
                    && *shown == key
                {
                    *kept = Some(placement);
                }
            }
            Done::Fresh {
                id,
                graph_revision,
                ahead,
                result,
                ..
            } => {
                let entry = match result {
                    Ok((key, placement)) => Entry::Ready { key, placement },
                    Err(_) => Entry::Failed,
                };
                let current_graph = book
                    .current
                    .as_ref()
                    .is_some_and(|current| current.id.as_deref() == Some(id.as_str()));
                // Speech changes the graph revision while the room's plate is still loading.
                // Only scene/party identity supersedes that plate, never a speaker's expression.
                if (current_graph || ahead) && book.entries.contains_key(&id) {
                    book.entries.insert(id, entry);
                    if current_graph {
                        book.state_images.baked_revision = Some(graph_revision);
                    }
                }
            }
            Done::Edit {
                scene_key,
                speaker_id,
                face,
                result,
                ..
            } => {
                let current = book
                    .current
                    .as_ref()
                    .is_some_and(|current| current.scene_key == scene_key);
                if current && matches!(&book.edit, Edit::Pending(pending) if *pending == face) {
                    match result {
                        Ok(key) => {
                            book.edit = Edit::Ready { face, key };
                            if let Some(speaker_id) = speaker_id {
                                self.inner.note_plate_face_shown(&speaker_id, face);
                            }
                        }
                        Err(_) => book.edit = Edit::Failed(face),
                    }
                }
            }
        }
        let covers = book.covers();
        drop(book);
        self.inner.set_plate_covers(covers);
        self.wake();
    }

    /// One look at the speaker's wanted face: a lasting change of it edits the current plate, and
    /// a return to the neutral face returns to the first plate.
    fn poll_face(&self) {
        self.poll_face_at(self.inner.now());
    }

    fn poll_face_at(&self, now: Duration) {
        let mut book = locked(&self.book);
        if book.state_images.pending.is_some() || book.state_images.flying {
            refresh_decision(&mut book, "face", "visible_state_pending");
            return;
        }
        if book.direct.pending {
            refresh_decision(&mut book, "face", "directed_render_pending");
            return;
        }
        let Some(current) = &book.current else {
            return;
        };
        let (Some(scene), false) = (current.scene.clone(), current.in_combat) else {
            return;
        };
        let Some(speaker) = scene.speaker.clone() else {
            return;
        };
        let base = book
            .state_images
            .shown
            .as_ref()
            .map(|shown| shown.0.clone())
            .or_else(|| {
                if book.direct.keep.is_some() {
                    book.direct.shown.as_ref().map(|shown| shown.0.clone())
                } else {
                    None
                }
            })
            .or_else(|| match book.current_entry() {
                Some(Entry::Ready { key, .. }) => Some(key.clone()),
                _ => None,
            });
        let Some(base) = base else {
            return;
        };
        let scene_key = current.scene_key.clone();
        let heroes = current.heroes.clone();
        let wanted = self
            .inner
            .plate_wanted_face(&speaker.id)
            .filter(|face| *face != Face::AUTHORED);
        let state_rate_limited = book
            .state_images
            .last_started
            .is_some_and(|last| now.saturating_sub(last) < Duration::from_secs(10));
        if scene.snapshot.is_some() && (!book.face_refresh.ready(wanted, now) || state_rate_limited)
        {
            refresh_decision(&mut book, "face", "settling_or_rate_limited");
            return;
        }
        match (wanted, book.edit.clone()) {
            (None, Edit::Idle) => refresh_decision(&mut book, "face", "unchanged"),
            (None, _) => {
                let had_edit = matches!(book.edit, Edit::Ready { .. });
                book.edit = Edit::Idle;
                drop(book);
                if had_edit {
                    self.inner
                        .note_plate_face_shown(&speaker.id, Face::AUTHORED);
                    self.wake();
                }
            }
            (Some(face), Edit::Pending(pending)) if pending == face => {
                refresh_decision(&mut book, "face", "render_in_flight")
            }
            (Some(face), Edit::Ready { face: shown, .. } | Edit::Failed(shown))
                if shown == face =>
            {
                refresh_decision(&mut book, "face", "unchanged_or_failed_target")
            }
            (Some(face), _) => {
                let job = Job::Edit {
                    heroes,
                    scene_key,
                    scene,
                    base,
                    face,
                };
                if self.jobs.try_send(job).is_ok() {
                    if book.direct.keep.is_none() {
                        book.direct.shown = None;
                    }
                    book.edit = Edit::Pending(face);
                    book.face_refresh.last_started = Some(now);
                    refresh_decision(&mut book, "face", "queued");
                    record(
                        "gameplay.living_scene.plate_edit_requested",
                        &[
                            ("scene.expression", face.expression.label().to_owned()),
                            ("scene.tier", face.tier.label().to_owned()),
                        ],
                    );
                    drop(book);
                    self.wake();
                    return;
                }
            }
        }
    }
}

impl Inner {
    /// The health variants the health pipeline has made for these heroes, by art number.
    fn hero_face_keys(&self, heroes: &[(u8, String)]) -> Vec<(u8, ImageKey)> {
        let state = locked(&self.state);
        heroes
            .iter()
            .filter_map(|(art, name)| Some((*art, state.hero_faces.get(name)?.stage.clone()?)))
            .collect()
    }

    /// Whether the plate on screen covers the cut-outs: their expression and the re-lit
    /// backdrop are then never drawn (nobody would see them).
    fn set_plate_covers(&self, covers: bool) {
        locked(&self.state).plate_covers = covers;
    }

    /// The face the mood last asked for on this character, if any.
    fn plate_wanted_face(&self, id: &str) -> Option<Face> {
        locked(&self.state).plate_wants.get(id).copied()
    }

    /// The plate now shows `face`, so the mood tracker owes nothing more for it.
    fn note_plate_face_shown(&self, id: &str, face: Face) {
        locked(&self.state).tracker.mark_face_shown(id, face);
    }
}

#[cfg(test)]
#[path = "plates_tests.rs"]
mod tests;
