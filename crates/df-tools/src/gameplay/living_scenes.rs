//! Living scenes: the shared display's backdrop and characters follow the mood of the story.
//!
//! The director watches the display views the server sends and the social ledger's record of
//! how the last exchange landed. When the story's mood changes it asks the image service for
//! (1) a changed expression on the character in focus, cached forever per sprite and
//! expression, and (2) at most once per interval, the backdrop re-framed and re-lit. A finished
//! image is published only as presentation: the display view the server sends carries the new
//! art path and watchers are woken the way a new table line wakes them, with the same revision,
//! the same offers and the same game state. Nothing here waits for an image, changes an
//! outcome, or runs during combat, a dice roll or the first seconds of a scene.
//!
//! The director's one worker task owns every render task it starts (a `JoinSet`), ends when
//! the last handle is dropped, and reports each outcome to its state, never to the game.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use df_protocol::common as rpc;
use tokio::{sync::watch, task::JoinSet, time::Instant};

use crate::health_tier::{Vitals, tier_of};

// PLAY-V09: scene plates (the whole scene as one composed picture), made just in time.
mod canonical;
mod plates;

use super::{
    adventure::{dossier::Attitude, setting::SettingKind},
    health_faces::{Book, Character, PREWARM_ORDER},
    imagery::{
        CharacterFacts, CharacterRole, ImageFailure, ImageKey, ImageKind, ImageRequest, Imagery,
        LifeStage, OutputSpec, PlaceFacts, ReferenceImage, ReferenceStore, cast_expression,
        party_sheet, scene_refresh,
    },
    pack::{self, SceneKey},
    scene_mood::{
        Beat, CastState, ExchangeSignal, Expression, Face, HealthTier, Hysteresis, Look,
        MoodInputs, MoodTracker, SceneTone, TableKind, Tempo, TempoLevel,
    },
    shot::{self, ShotInput, SpeakerInput, scene_graph},
    social::SocialLedger,
    startup_configuration::ContentRating,
};

use super::shot::scene_graph::{ActorInput, SceneGraph, SceneGraphInput};

/// The authored cut-out of the party's hero at this position; the health variants are made from
/// it.
fn hero_art(index: usize) -> String {
    format!("assets/ui/generated/sprite-hero-{}-photo", index + 1)
}

/// The cut-out the battlefield dresses the hero at this party position in (the dwarf fighter, the
/// archer, then the second fighter). SWITCH POINT: this must match `snapshot_of` in
/// `gameplay_browser/live_battlefield.rs`, so a health variant is made from the art the fight
/// shows.
fn battle_hero_art(index: usize) -> String {
    let art = [1, 3, 2][index % 3];
    format!("assets/ui/generated/sprite-hero-{art}-photo")
}

/// What a hero's health variants are made from on each screen, and which are cached.
#[derive(Clone, Debug, Eq, PartialEq)]
struct HeroFace {
    /// The variant of the exploration stage's art.
    stage: Option<ImageKey>,
    /// The variant of the battlefield's art (the same one when the seat wears the same art).
    battle: Option<ImageKey>,
}

/// How many of the latest table lines the mood reads (kinds only).
const RECENT_TABLE_LINES: usize = 6;

/// The timing of living scenes. Production values come from the validated interval.
#[derive(Clone, Copy, Debug)]
pub(super) struct Timing {
    /// The least time between two scene refreshes.
    pub(super) interval: Duration,
    /// No refresh starts or lands this soon after a scene begins.
    pub(super) scene_start_quiet: Duration,
    /// No refresh starts or lands this soon after an accepted action (which may be a roll).
    pub(super) roll_quiet: Duration,
    /// With no story event for this many intervals, a gentle alternate angle is allowed.
    pub(super) idle_after_intervals: u32,
    /// An alternate angle comes at most once per this many intervals.
    pub(super) idle_every_intervals: u32,
    /// How often the worker re-evaluates.
    pub(super) tick: Duration,
    pub(super) hysteresis: Hysteresis,
}

impl Timing {
    pub(super) fn from_interval(interval: Duration) -> Self {
        Self {
            interval,
            scene_start_quiet: Duration::from_secs(8),
            roll_quiet: Duration::from_secs(6),
            idle_after_intervals: 4,
            idle_every_intervals: 3,
            tick: Duration::from_secs(1),
            hysteresis: Hysteresis::default(),
        }
    }
}

/// Why a scene refresh was started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RefreshReason {
    /// The mood or shot differs from what is on screen.
    Change,
    /// Nothing has happened for a long while: a gentle alternate angle.
    AlternateAngle,
}

/// When a repaint may start and land. Pure: every method takes the time explicitly.
#[derive(Debug)]
pub(super) struct Pacer {
    timing: Timing,
    scene_started: Duration,
    last_refresh_started: Option<Duration>,
    last_action: Option<Duration>,
    last_story_event: Duration,
    last_alternate: Option<Duration>,
    alternates: usize,
}

impl Pacer {
    pub(super) fn new(timing: Timing, now: Duration) -> Self {
        Self {
            timing,
            scene_started: now,
            last_refresh_started: None,
            last_action: None,
            last_story_event: now,
            last_alternate: None,
            alternates: 0,
        }
    }

    pub(super) fn begin_scene(&mut self, now: Duration) {
        *self = Self::new(self.timing, now);
    }

    /// An accepted action happened (it may be a dice roll).
    pub(super) fn note_action(&mut self, now: Duration) {
        self.last_action = Some(now);
        self.last_story_event = now;
    }

    /// A social exchange happened.
    pub(super) fn note_story_event(&mut self, now: Duration) {
        self.last_story_event = now;
    }

    fn quiet(&self, now: Duration) -> bool {
        now.saturating_sub(self.scene_started) < self.timing.scene_start_quiet
            || self
                .last_action
                .is_some_and(|action| now.saturating_sub(action) < self.timing.roll_quiet)
    }

    /// Whether a refresh may start now, and why. Never inside the interval, never in combat,
    /// never in the quiet windows.
    pub(super) fn refresh_due(
        &self,
        now: Duration,
        in_combat: bool,
        worth_rendering: bool,
    ) -> Option<RefreshReason> {
        if in_combat || self.quiet(now) {
            return None;
        }
        if self
            .last_refresh_started
            .is_some_and(|started| now.saturating_sub(started) < self.timing.interval)
        {
            return None;
        }
        if worth_rendering {
            return Some(RefreshReason::Change);
        }
        let idle_for = now.saturating_sub(self.last_story_event);
        let idle_long = idle_for >= self.timing.interval * self.timing.idle_after_intervals;
        let alternate_due = self.last_alternate.is_none_or(|last| {
            now.saturating_sub(last) >= self.timing.interval * self.timing.idle_every_intervals
        });
        (idle_long && alternate_due).then_some(RefreshReason::AlternateAngle)
    }

    pub(super) fn refresh_started(&mut self, now: Duration, reason: RefreshReason) {
        self.last_refresh_started = Some(now);
        if reason == RefreshReason::AlternateAngle {
            self.last_alternate = Some(now);
            self.alternates += 1;
        }
    }

    /// How many alternate angles have been started in this scene.
    pub(super) fn alternates(&self) -> usize {
        self.alternates
    }

    /// Whether a finished repaint may land now: not in combat, not in a quiet window.
    pub(super) fn may_publish(&self, now: Duration, in_combat: bool) -> bool {
        !in_combat && !self.quiet(now)
    }
}

/// One character on stage, from the view and the pack.
#[derive(Clone, Debug)]
struct Member {
    id: String,
    role: CharacterRole,
    life_stage: LifeStage,
    name: String,
    /// The cut-out's asset stem, as the scene cast names it.
    sprite: String,
    /// The social ledger's id of this character's dossier, when the adventure gives one.
    dossier_id: Option<String>,
    opposes: bool,
    starting_attitude: Option<Attitude>,
}

/// The facts of the current scene that do not change while it lasts.
#[derive(Clone, Debug)]
struct SceneFacts {
    place: Option<PlaceFacts>,
    tone: SceneTone,
    beat: Beat,
    members: Vec<Member>,
    /// Named cast of the adventure's fights that is not on this stage, so a fight's foes have
    /// their health variants ready before the fight starts.
    fights: Vec<Member>,
}

/// What one display view says about the stage.
#[derive(Clone, Debug)]
struct Stage {
    /// Names the scene: the title and the authored backdrop path.
    scene_key: String,
    scene_asset: String,
    revision: Option<(u64, u64)>,
    in_combat: bool,
    in_roll: bool,
    tempo: Option<Tempo>,
    speaking_ids: BTreeSet<String>,
    recent_kinds: Vec<TableKind>,
    title: String,
    story_scene: Option<pack::Scene>,
    cast: Vec<StageCast>,
    speaker_id: Option<String>,
    listener_id: Option<String>,
    line_length: usize,
    measured_speaker_layout: Option<shot::PlateLayout>,
    /// What the view says about the health of the named combatants it still carries.
    vitals: Vec<(String, Vitals)>,
    /// The party's ready heroes, in party order.
    heroes: Vec<StageHero>,
}

/// A ready hero of the party, from the view's party list.
#[derive(Clone, Debug)]
struct StageHero {
    index: usize,
    name: String,
    vitals: Vitals,
}

fn vitals_of(member: &rpc::PartyMemberView) -> Vitals {
    Vitals {
        current: member.hit_points,
        maximum: member.maximum_hit_points,
        unconscious: member.unconscious,
        dying: false,
    }
}

#[derive(Clone, Debug)]
struct StageCast {
    id: String,
    name: String,
    sprite: String,
}

fn stage_of(message: &rpc::ViewMessage) -> Option<Stage> {
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        return None;
    };
    let scene = display.scene.as_ref()?;
    let journey = display.journey.as_ref();
    let phase = journey.and_then(|journey| rpc::JourneyPhase::try_from(journey.phase).ok());
    let in_combat = phase == Some(rpc::JourneyPhase::Combat);
    let tempo = journey
        .and_then(|journey| journey.tempo.as_ref())
        .and_then(|cue| {
            let phase = match rpc::TempoPhase::try_from(cue.phase).ok()? {
                rpc::TempoPhase::Calm => TempoLevel::Calm,
                rpc::TempoPhase::Build => TempoLevel::Build,
                rpc::TempoPhase::Anticipation => TempoLevel::Anticipation,
                rpc::TempoPhase::Peak => TempoLevel::Peak,
                rpc::TempoPhase::Release => TempoLevel::Release,
                rpc::TempoPhase::Unspecified => return None,
            };
            Some(Tempo {
                phase,
                intensity_permille: u16::try_from(cue.intensity_permille).unwrap_or(u16::MAX),
            })
        });
    let recent_kinds = journey
        .map(|journey| {
            let lines = &journey.table_log;
            lines
                .iter()
                .skip(lines.len().saturating_sub(RECENT_TABLE_LINES))
                .filter_map(|line| match rpc::TableKind::try_from(line.kind).ok()? {
                    rpc::TableKind::Speech => Some(TableKind::Speech),
                    rpc::TableKind::Action => Some(TableKind::Action),
                    rpc::TableKind::Unspecified => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let asset = if scene.scene_asset.is_empty() {
        display.scene_asset.clone()
    } else {
        scene.scene_asset.clone()
    };
    let source_scene_key = pack::SceneKey::parse_source_label(&scene.source_scene_key)
        .map_or("unknown", pack::SceneKey::source_label);
    let story_scene = plates::locate(&scene.title, &asset, &scene.source_scene_key);
    let hero_names: Vec<(String, String)> = journey
        .map(|journey| {
            journey
                .party
                .iter()
                .enumerate()
                .take(shot::MAXIMUM_HEROES)
                .filter(|(_, member)| member.character_ready)
                .map(|(index, member)| (format!("hero-{}", index + 1), member.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let last_speaker = journey.and_then(|journey| {
        journey.table_log.iter().rev().find_map(|line| {
            (rpc::TableKind::try_from(line.kind).ok() == Some(rpc::TableKind::Speech))
                .then_some(line.speaker.as_str())
        })
    });
    let cast_speaker = scene.cast.iter().find(|actor| actor.speaking);
    let speaker_id = last_speaker
        .and_then(|name| {
            hero_names
                .iter()
                .find(|(_, hero_name)| hero_name == name)
                .map(|(id, _)| id.clone())
                .or_else(|| {
                    scene
                        .cast
                        .iter()
                        .find(|actor| actor.name == name)
                        .map(|actor| actor.id.clone())
                })
        })
        .or_else(|| cast_speaker.map(|actor| actor.id.clone()));
    let listener_id = speaker_id.as_ref().and_then(|speaker| {
        if speaker.starts_with("hero-") {
            cast_speaker.map(|actor| actor.id.clone())
        } else {
            hero_names.first().map(|(id, _)| id.clone())
        }
    });
    let line_length = journey
        .and_then(|journey| journey.table_log.last())
        .map(|line| line.text.chars().count())
        .filter(|length| *length > 0)
        .or_else(|| journey.map(|journey| journey.dialogue.chars().count()))
        .unwrap_or_default();
    let measured_speaker_layout = scene.plate_layout.as_ref().and_then(shot_layout_from_wire);
    let in_roll = journey
        .and_then(|journey| journey.table_log.last())
        .is_some_and(|line| {
            rpc::TableKind::try_from(line.kind).ok() == Some(rpc::TableKind::Action)
        });
    Some(Stage {
        scene_key: format!("{source_scene_key}|{}|{}", scene.title, asset),
        scene_asset: asset,
        revision: message
            .revision
            .as_ref()
            .and_then(|revision| Some((revision.epoch.as_ref()?.value?, revision.sequence?))),
        in_combat,
        in_roll,
        tempo,
        speaking_ids: scene
            .cast
            .iter()
            .filter(|actor| actor.speaking)
            .map(|actor| actor.id.clone())
            .collect(),
        recent_kinds,
        title: scene.title.clone(),
        story_scene,
        cast: scene
            .cast
            .iter()
            .map(|actor| StageCast {
                id: actor.id.clone(),
                name: actor.name.clone(),
                sprite: actor.sprite.clone(),
            })
            .collect(),
        speaker_id,
        listener_id,
        line_length,
        measured_speaker_layout,
        vitals: journey
            .and_then(|journey| journey.combat.as_ref())
            .map(|combat| {
                combat
                    .participants
                    .iter()
                    .map(|member| (member.name.clone(), vitals_of(member)))
                    .collect()
            })
            .unwrap_or_default(),
        heroes: journey
            .map(|journey| {
                journey
                    .party
                    .iter()
                    .enumerate()
                    .take(2)
                    .filter(|(_, member)| member.character_ready && member.maximum_hit_points > 0)
                    .map(|(index, member)| StageHero {
                        index,
                        name: member.name.clone(),
                        vitals: vitals_of(member),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn shot_layout_from_wire(layout: &rpc::PlateLayout) -> Option<shot::PlateLayout> {
    Some(shot::PlateLayout {
        bubble_side: match rpc::PlateSide::try_from(layout.bubble_side).ok()? {
            rpc::PlateSide::Left => shot::Side::Left,
            rpc::PlateSide::Right => shot::Side::Right,
            rpc::PlateSide::Unspecified => return None,
        },
        head_x_permille: u16::try_from(layout.head_x_permille).ok()?,
        head_y_permille: u16::try_from(layout.head_y_permille).ok()?,
        head_width_permille: u16::try_from(layout.head_width_permille).ok()?,
        head_height_permille: u16::try_from(layout.head_height_permille).ok()?,
    })
}

fn stage_asset(path: &str) -> String {
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

fn graph_of(stage: &Stage, state: Option<&State>) -> Option<SceneGraph> {
    if stage.in_combat || stage.in_roll {
        return None;
    }
    let scene = stage.story_scene.as_ref()?;
    let (place_id, place_name, place_kind, exterior): (&str, &str, String, bool) =
        if let Some(setting) = scene.setting.as_ref() {
            let place_kind = serde_json::to_value(setting.kind)
                .ok()?
                .as_str()?
                .to_owned();
            (
                &setting.place_id,
                &setting.name,
                place_kind,
                setting.exterior,
            )
        } else {
            let (place_id, place_name, place_kind, exterior) = match scene.asset.as_str() {
                "assets/ui/scenes/mara-harbor-v4.png" => {
                    ("lantern-wharf", "Lantern Wharf", "quay", true)
                }
                "assets/concept-art/scene-tavern-barkeep-talk-rain.webp" => {
                    ("harbor-inn", "Harbor Inn", "inn", false)
                }
                _ => return None,
            };
            (place_id, place_name, place_kind.to_owned(), exterior)
        };
    let speaker = scene
        .cast
        .iter()
        .find(|actor| Some(actor.id.as_str()) == stage.speaker_id.as_deref());
    let other_sprites: Vec<&str> = scene
        .cast
        .iter()
        .filter(|actor| Some(actor.id.as_str()) != stage.speaker_id.as_deref())
        .map(|actor| actor.sprite.as_str())
        .collect();
    let heroes: Vec<u8> = stage
        .heroes
        .iter()
        .filter_map(|hero| u8::try_from(hero.index + 1).ok())
        .collect();
    let story_text = format!("{} {}", scene.narration, scene.dialogue);
    let shot = shot::direct(&ShotInput {
        scene_key: scene.key,
        place_id,
        place_kind: &place_kind,
        exterior,
        title: &scene.title,
        story_text: &story_text,
        speaker: speaker.map(|actor| SpeakerInput {
            sprite: &actor.sprite,
            gender: actor.gender,
            age: actor.age,
        }),
        others: &other_sprites,
        heroes: &heroes,
    })
    .ok()?;
    let mut owned = Vec::with_capacity(stage.heroes.len() + stage.cast.len());
    for hero in &stage.heroes {
        let art = u8::try_from(hero.index + 1).ok()?;
        let fallback_art = match art {
            1 | 2 => format!("/assets/ui/generated/figure-hero-{art}-documentary-photo"),
            _ => format!("/assets/ui/generated/figure-hero-{art}-documentary-photo"),
        };
        let live_art = state.and_then(|state| {
            state
                .canonical
                .hero_at_seat(art)
                .and_then(|(entity, _)| state.canonical.hero(entity))
                .map(|key| key.path())
                .or_else(|| state.hero_cutouts.get(&hero.name).map(ImageKey::path))
        });
        owned.push((
            format!("hero-{art}"),
            hero.name.clone(),
            fallback_art,
            live_art,
            Some(art),
            Some(format!("hero-{art}") == stage.speaker_id.as_deref().unwrap_or_default()),
            Some(Expression::Neutral),
        ));
    }
    for actor in &stage.cast {
        let fallback_art = match actor.sprite.rsplit('/').next().unwrap_or_default() {
            "sprite-courier" => "/assets/ui/generated/sprite-courier-documentary-photo".to_owned(),
            "sprite-bandit" => "/assets/ui/generated/sprite-bandit-documentary-photo".to_owned(),
            _ => stage_asset(&actor.sprite),
        };
        let overlay = state
            .and_then(|state| state.health_sprites.get(&actor.id))
            .or_else(|| state.and_then(|state| state.sprites.get(&actor.id)))
            .filter(|overlay| overlay.source_sprite == actor.sprite);
        let live_art = state
            .and_then(|state| state.canonical.npc(&actor.id))
            .map(|key| key.path())
            .or_else(|| overlay.map(|overlay| overlay.key.path()));
        let expression = state
            .and_then(|state| state.plate_wants.get(&actor.id))
            .map(|face| face.expression)
            .or_else(|| overlay.map(|overlay| overlay.expression))
            .or(Some(Expression::Neutral));
        owned.push((
            actor.id.clone(),
            actor.name.clone(),
            fallback_art,
            live_art,
            None,
            Some(Some(actor.id.as_str()) == stage.speaker_id.as_deref()),
            expression,
        ));
    }
    let actor_inputs: Vec<ActorInput<'_>> = owned
        .iter()
        .map(
            |(id, name, fallback_art, live_art, hero_art, speaking, expression)| ActorInput {
                id,
                name,
                fallback_art,
                live_art: live_art.as_deref(),
                expression: *expression,
                hero_art: *hero_art,
                speaking: speaking.unwrap_or(false),
            },
        )
        .collect();
    let canonical_set = state
        .and_then(|state| state.canonical.room(place_id))
        .map(|key| key.path());
    let graph = scene_graph::build(&SceneGraphInput {
        scene_key: scene.key,
        place_id,
        place_name,
        fallback_set: &stage_asset(&scene.asset),
        refreshed_set: canonical_set.as_deref().or_else(|| {
            state
                .filter(|state| state.backdrop.is_some())
                .and_then(|state| state.backdrop.as_ref())
                .map(|backdrop| backdrop.key.as_str())
        }),
        exterior,
        weather: None,
        shot: &shot,
        actors: &actor_inputs,
        speaker_id: stage.speaker_id.as_deref(),
        listener_id: stage.listener_id.as_deref(),
        measured_speaker_layout: stage.measured_speaker_layout,
        scene_open: scene.key == SceneKey::Opening,
        narrator_speaks: stage.speaker_id.is_none(),
        line_length: stage.line_length,
    })
    .ok()?;
    match super::adventure::scene_model::current_scene(scene) {
        Some(current) => scene_graph::with_model(graph, &current.model).ok(),
        None => Some(graph),
    }
}

fn graph_to_wire(graph: &SceneGraph) -> rpc::DisplaySceneGraph {
    rpc::DisplaySceneGraph {
        revision: graph.revision,
        scene_key: graph.scene_key.clone(),
        set: Some(rpc::DisplaySceneSet {
            place_id: graph.set.place_id.clone(),
            place_name: graph.set.place_name.clone(),
            asset: graph.set.asset.clone(),
            fallback_asset: graph.set.fallback_asset.clone(),
            exterior: graph.set.exterior,
            time_of_day: graph.set.time_of_day.clone(),
            weather: graph.set.weather.map_or_else(String::new, |weather| {
                match weather {
                    scene_graph::Weather::Clear => "clear",
                    scene_graph::Weather::Fog => "fog",
                    scene_graph::Weather::Rain => "rain",
                    scene_graph::Weather::Snow => "snow",
                    scene_graph::Weather::Wind => "wind",
                }
                .to_owned()
            }),
            lights: graph.set.lights.clone(),
        }),
        camera: match graph.camera {
            scene_graph::CameraFraming::Wide => rpc::DisplaySceneCamera::Wide,
            scene_graph::CameraFraming::TwoShot => rpc::DisplaySceneCamera::TwoShot,
            scene_graph::CameraFraming::Speaker => rpc::DisplaySceneCamera::Speaker,
        } as i32,
        actors: graph
            .actors
            .iter()
            .map(|actor| rpc::DisplaySceneActor {
                id: actor.id.clone(),
                name: actor.name.clone(),
                art: actor.art.clone(),
                fallback_art: actor.fallback_art.clone(),
                spot: actor.spot.clone(),
                anchor_x_permille: actor.anchor_x_permille,
                anchor_y_permille: actor.anchor_y_permille,
                depth_permille: actor.depth_permille,
                pose: actor.pose.clone(),
                expression: actor.expression.clone(),
                look_at: actor.look_at.clone(),
                speaking: actor.speaking,
            })
            .collect(),
    }
}

fn tone_of(kind: SettingKind) -> SceneTone {
    match kind {
        SettingKind::GraveyardGate | SettingKind::UndercroftStudy | SettingKind::CiderCellar => {
            SceneTone::Grim
        }
        SettingKind::Inn
        | SettingKind::TeaHouse
        | SettingKind::FarmKitchen
        | SettingKind::Shop
        | SettingKind::LanternShop
        | SettingKind::WaystationHall
        | SettingKind::Bothy
        | SettingKind::RefugeHall
        | SettingKind::PilgrimsHall
        | SettingKind::Apothecary => SceneTone::Cozy,
        _ => SceneTone::Neutral,
    }
}

fn role_of(role: pack::CastRole) -> CharacterRole {
    match role {
        pack::CastRole::Patron => CharacterRole::Patron,
        pack::CastRole::Antagonist => CharacterRole::Antagonist,
        pack::CastRole::Contact => CharacterRole::Contact,
        pack::CastRole::Bystander => CharacterRole::Bystander,
    }
}

/// The facts of the pack scene a display view shows: matched by its authored backdrop and title.
fn facts_of(stage: &Stage, social: &SocialLedger) -> SceneFacts {
    pack::with_content(|content| {
        let wanted = stage.scene_asset.trim_start_matches('/');
        let matches = |scene: &&pack::Scene, asset: &str| {
            asset.trim_start_matches('/') == wanted && scene.title == stage.title
        };
        // The pack's own scenes first, then the scenes a beat script gives its peaceful beats
        // (their painting may be replaced by the adventure's own art for that variant).
        let overrides = pack::active().map(|loaded| loaded.script_scene_assets.clone());
        let scene = content
            .scenes
            .iter()
            .find(|scene| matches(scene, &scene.asset))
            .or_else(|| {
                content.beat_script.as_ref().and_then(|script| {
                    script.scenes.iter().find_map(|(id, scene)| {
                        let asset = overrides
                            .as_ref()
                            .and_then(|assets| assets.get(id))
                            .unwrap_or(&scene.asset);
                        matches(&scene, asset).then_some(scene)
                    })
                })
            });
        let Some(scene) = scene else {
            return SceneFacts {
                place: None,
                tone: SceneTone::Neutral,
                beat: Beat::Conversation,
                members: Vec::new(),
                fights: Vec::new(),
            };
        };
        let member_of = |member: &pack::SceneCastMember| {
            let role = match member.role {
                pack::CastRole::Patron => rpc::SceneRole::Patron,
                pack::CastRole::Antagonist => rpc::SceneRole::Antagonist,
                pack::CastRole::Contact => rpc::SceneRole::Contact,
                pack::CastRole::Bystander => rpc::SceneRole::Bystander,
            };
            let dossier = social.dossier_of(role, &member.id);
            Member {
                id: member.id.clone(),
                role: role_of(member.role),
                life_stage: LifeStage::from_age(member.age),
                name: member.name.clone(),
                sprite: member.sprite.clone(),
                dossier_id: dossier.map(|dossier| dossier.npc_id.clone()),
                opposes: dossier.is_some_and(|dossier| dossier.role.opposes_party()),
                starting_attitude: dossier.map(|dossier| dossier.starting_attitude),
            }
        };
        let members: Vec<Member> = scene.cast.iter().map(member_of).collect();
        let mut fights: Vec<Member> = Vec::new();
        for fight in content
            .scenes
            .iter()
            .filter(|other| other.key == SceneKey::Combat)
        {
            for member in fight.cast.iter().map(member_of) {
                let known = members
                    .iter()
                    .chain(&fights)
                    .any(|other| other.sprite == member.sprite);
                if !known {
                    fights.push(member);
                }
            }
        }
        SceneFacts {
            place: scene
                .setting
                .as_ref()
                .and_then(|setting| PlaceFacts::new(&setting.name, &setting.short)),
            tone: scene
                .setting
                .as_ref()
                .map_or(SceneTone::Neutral, |setting| tone_of(setting.kind)),
            fights,
            beat: match scene.key {
                SceneKey::Victory => Beat::Victory,
                SceneKey::Defeat => Beat::Defeat,
                SceneKey::Room | SceneKey::Combat => Beat::Confrontation,
                SceneKey::Opening
                | SceneKey::DialogueSeal
                | SceneKey::DialogueEscort
                | SceneKey::Inn => Beat::Conversation,
            },
            members,
        }
    })
}

/// A published character overlay: the expression variant of one cut-out.
#[derive(Clone, Debug)]
struct SpriteOverlay {
    /// The authored cut-out this variant replaces; it only applies while the cast names it.
    source_sprite: String,
    expression: Expression,
    key: ImageKey,
}

#[derive(Clone, Debug)]
struct BackdropOverlay {
    key: ImageKey,
}

/// A finished scene refresh waiting for a quiet moment to land.
#[derive(Debug)]
struct ReadyBackdrop {
    key: ImageKey,
    look: Look,
    alternate: bool,
}

struct State {
    canonical: canonical::References,
    stage: Option<Stage>,
    facts: Option<SceneFacts>,
    tracker: MoodTracker,
    pacer: Pacer,
    backdrop: Option<BackdropOverlay>,
    sprites: BTreeMap<String, SpriteOverlay>,
    ready_backdrop: Option<ReadyBackdrop>,
    refresh_in_flight: bool,
    expression_in_flight: BTreeSet<String>,
    /// Faces that failed in this scene are not asked for again.
    failed_faces: BTreeSet<(String, Face)>,
    last_revision: Option<(u64, u64)>,
    budget_exhausted: bool,
    /// PLAY-V09: the scene's plate is on screen and covers the cut-outs and the backdrop.
    plate_covers: bool,
    /// PLAY-V09: the face the mood last asked for on each character, for the plate director.
    plate_wants: BTreeMap<String, Face>,
    /// The health variants known to exist in the cache, and what has been tried.
    book: Book,
    /// The health variant each cast member shows now, by cast id.
    health_sprites: BTreeMap<String, SpriteOverlay>,
    /// The health variant each hero shows now, by name.
    hero_faces: BTreeMap<String, HeroFace>,
    /// The standing cut-outs, requested once per hero from the opening party sheet.
    hero_cutouts: BTreeMap<String, ImageKey>,
    hero_cutouts_in_flight: BTreeSet<String>,
    failed_hero_cutouts: BTreeSet<String>,
    scene_graph: Option<SceneGraph>,
}

/// What the worker should start.
enum Job {
    Canonical(canonical::Work),
    HeroCutout {
        name: String,
        index: usize,
        party: Vec<usize>,
    },
    Expression {
        member: Member,
        face: Face,
    },
    Refresh {
        place: PlaceFacts,
        scene_asset: String,
        look: Look,
        reason: RefreshReason,
    },
    /// Makes one health variant: the only job that can spend on health art, and never started in
    /// a fight.
    Warm {
        character: Character,
        tier: HealthTier,
    },
    /// Looks through the cache (never the provider) for a character's health variants.
    Peek {
        character: Character,
    },
}

enum Done {
    Canonical {
        work: canonical::Work,
        result: Result<ImageKey, ImageFailure>,
    },
    HeroCutout {
        name: String,
        result: Result<ImageKey, ImageFailure>,
    },
    Expression {
        npc_id: String,
        face: Face,
        source_sprite: String,
        result: Result<ImageKey, ImageFailure>,
    },
    Refresh {
        look: Look,
        reason: RefreshReason,
        scene_key: String,
        result: Result<ImageKey, ImageFailure>,
    },
    Warm {
        id: String,
        tier: HealthTier,
        result: Result<ImageKey, ImageFailure>,
    },
    Peek {
        id: String,
        found: Vec<(HealthTier, ImageKey)>,
    },
}

struct Inner {
    imagery: Imagery,
    references: ReferenceStore,
    social: Arc<SocialLedger>,
    rating: ContentRating,
    timing: Timing,
    state: Mutex<State>,
    /// Counts overlay changes so watchers rebuild their view.
    wake: watch::Sender<u64>,
    /// The scene key of the last published overlay change, for tests and diagnostics.
    origin: Instant,
}

/// Ends the worker with the last handle.
struct WorkerGuard(tokio::task::JoinHandle<()>);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The living-scenes director: a cheap handle shared by the view path.
#[derive(Clone)]
pub(super) struct LivingScenes {
    inner: Option<Arc<Inner>>,
    _worker: Option<Arc<WorkerGuard>>,
    /// PLAY-V09: the plate director, when plates are on.
    plates: Option<plates::Plates>,
    last_graph_revision: Arc<Mutex<Option<u64>>>,
}

/// The health variants of the heroes, by name, on a view's journey.
fn apply_hero_faces(state: &State, journey: Option<&mut rpc::JourneyView>) {
    if let Some(journey) = journey {
        journey.live_faces = state
            .hero_faces
            .iter()
            .map(|(name, face)| rpc::LiveFace {
                name: name.clone(),
                sprite: face.stage.as_ref().map(ImageKey::path).unwrap_or_default(),
                battle_sprite: face.battle.as_ref().map(ImageKey::path).unwrap_or_default(),
            })
            .collect();
    }
}

/// The characters whose art has health variants in this stage: the party's heroes first, then
/// the scene's cast.
fn characters_of(stage: &Stage, facts: &SceneFacts) -> Vec<Character> {
    let heroes = stage.heroes.iter().flat_map(|hero| {
        let mut arts = vec![hero_art(hero.index)];
        let battle = battle_hero_art(hero.index);
        if !arts.contains(&battle) {
            arts.push(battle);
        }
        arts.into_iter().map(|art| Character {
            id: art.clone(),
            art,
            facts: CharacterFacts {
                role: CharacterRole::Hero,
                life_stage: LifeStage::Adult,
            },
        })
    });
    let cast = facts
        .members
        .iter()
        .filter(|member| stage.cast.iter().any(|cast| cast.id == member.id))
        .map(|member| Character {
            id: member.sprite.clone(),
            art: member.sprite.clone(),
            facts: CharacterFacts {
                role: member.role,
                life_stage: member.life_stage,
            },
        });
    let fights = facts
        .fights
        .iter()
        .filter(|member| {
            !facts
                .members
                .iter()
                .any(|other| other.sprite == member.sprite)
        })
        .map(|member| Character {
            id: member.sprite.clone(),
            art: member.sprite.clone(),
            facts: CharacterFacts {
                role: member.role,
                life_stage: member.life_stage,
            },
        });
    heroes.chain(cast).chain(fights).collect()
}

/// How hurt the named combatant of this stage is; healthy when the view carries no hit points.
fn member_tier(stage: &Stage, member: &Member) -> HealthTier {
    stage
        .vitals
        .iter()
        .find(|(name, _)| *name == member.name)
        .and_then(|(_, vitals)| tier_of(*vitals))
        .unwrap_or(HealthTier::Healthy)
}

/// Recomputes which health variant every hero and cast member shows from the view's hit points
/// and the variants known to exist: the variant of the exact tier, otherwise the nearest cached
/// one, otherwise the authored art. Pure presentation, no provider call; returns whether
/// anything shown changed.
fn refresh_health(state: &mut State) -> bool {
    let (Some(stage), Some(facts)) = (state.stage.as_ref(), state.facts.as_ref()) else {
        return false;
    };
    let mut hero_faces = BTreeMap::new();
    for hero in &stage.heroes {
        if let Some(tier) = tier_of(hero.vitals) {
            let nearest = |art: String| state.book.nearest(&art, tier).map(|(_, key)| key.clone());
            let face = HeroFace {
                stage: nearest(hero_art(hero.index)),
                battle: nearest(battle_hero_art(hero.index)),
            };
            if face.stage.is_some() || face.battle.is_some() {
                hero_faces.insert(hero.name.clone(), face);
            }
        }
    }
    let mut health_sprites = BTreeMap::new();
    let mut superseded = Vec::new();
    for member in &facts.members {
        if !stage.cast.iter().any(|cast| cast.id == member.id) {
            continue;
        }
        let tier = member_tier(stage, member);
        // Bloodied or worse outranks the conversation's mood face; Hurt only fills in where no
        // mood face is showing.
        let health_face = tier.is_bloodied_or_worse()
            || (tier == HealthTier::Hurt && !state.sprites.contains_key(&member.id));
        if tier.is_bloodied_or_worse() {
            superseded.push(member.id.clone());
        }
        if health_face && let Some((_, key)) = state.book.nearest(&member.sprite, tier) {
            health_sprites.insert(
                member.id.clone(),
                SpriteOverlay {
                    source_sprite: member.sprite.clone(),
                    expression: Expression::Neutral,
                    key: key.clone(),
                },
            );
        }
    }
    let mut changed = false;
    for id in superseded {
        changed |= state.sprites.remove(&id).is_some();
    }
    let shown_keys = |sprites: &BTreeMap<String, SpriteOverlay>| -> Vec<(String, ImageKey)> {
        sprites
            .iter()
            .map(|(id, overlay)| (id.clone(), overlay.key.clone()))
            .collect()
    };
    changed |= shown_keys(&health_sprites) != shown_keys(&state.health_sprites)
        || hero_faces != state.hero_faces;
    state.health_sprites = health_sprites;
    state.hero_faces = hero_faces;
    changed
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One diagnostic event. Its fields are closed-vocabulary labels (a mood, a shot, an outcome)
/// and never a prompt, a reference or an image.
fn record(event: &'static str, fields: &[(&'static str, String)]) {
    let context = df_observe::OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, "gameplay.living_scene");
    span.event(event, fields);
    span.finish_unmeasured("recorded");
}

impl LivingScenes {
    /// Register the committed identity before projecting a view that can request its plate.
    pub(super) fn observe_checkpoint(&self, checkpoint: &df_model::checkpoint::Checkpoint) {
        if let Some(inner) = &self.inner {
            inner.observe_checkpoint(checkpoint);
        }
    }
    /// Living scenes that do nothing: imagery is off.
    pub(super) fn disabled() -> Self {
        Self {
            inner: None,
            _worker: None,
            plates: None,
            last_graph_revision: Arc::new(Mutex::new(None)),
        }
    }

    /// Starts the director and its worker task. Must be called inside the runtime.
    pub(super) fn start(
        imagery: Imagery,
        references: ReferenceStore,
        social: Arc<SocialLedger>,
        rating: ContentRating,
        timing: Timing,
        plates_on: bool,
        checkpoints: watch::Receiver<df_model::checkpoint::Checkpoint>,
    ) -> Self {
        let origin = Instant::now();
        let inner = Arc::new(Inner {
            imagery,
            references,
            social,
            rating,
            timing,
            state: Mutex::new(State::new(timing)),
            wake: watch::channel(0).0,
            origin,
        });
        inner.observe_checkpoint(&checkpoints.borrow().clone());
        let worker = tokio::spawn(Arc::clone(&inner).run(checkpoints));
        let plates = plates_on.then(|| plates::Plates::start(Arc::clone(&inner)));
        Self {
            inner: Some(inner),
            _worker: Some(Arc::new(WorkerGuard(worker))),
            plates,
            last_graph_revision: Arc::new(Mutex::new(None)),
        }
    }

    /// Wakes when the published art changed, so watchers rebuild their view.
    pub(super) fn subscribe(&self) -> Option<watch::Receiver<u64>> {
        self.inner.as_ref().map(|inner| inner.wake.subscribe())
    }

    /// PLAY-V10: wakes the watchers when a generated loop lands outside this worker, as art does.
    pub(super) fn signal_art(&self) {
        if let Some(inner) = &self.inner {
            inner.wake.send_modify(|count| *count += 1);
        }
    }

    pub(super) fn direct_plate(
        &self,
        text: String,
        keep: bool,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<serde_json::Value, ImageFailure>>, &'static str>
    {
        self.plates
            .as_ref()
            .ok_or("imagery or scene plates are off")?
            .direct(text, keep)
    }

    pub(super) fn clear_direction(&self) {
        if let Some(plates) = &self.plates {
            plates.clear_direction();
        }
    }

    pub(super) fn direct_status(&self, key: &str) -> Option<serde_json::Value> {
        self.plates.as_ref()?.direct_status(key)
    }

    pub(super) fn request_item(
        &self,
        hero: &str,
        thing_id: &str,
        detail: Option<String>,
        shared: bool,
    ) -> Result<(), &'static str> {
        self.plates
            .as_ref()
            .ok_or("scene pictures are unavailable")?
            .request_item(hero, thing_id, detail, shared)
    }

    /// Notes a display view the server is sending: its scene, cast, tempo, revision and the
    /// health of the party and the cast. Player views are ignored.
    pub(super) fn observe(&self, message: &rpc::ViewMessage) {
        let Some(inner) = &self.inner else {
            return;
        };
        let Some(stage) = stage_of(message) else {
            return;
        };
        let graph_revision = inner.observe(stage).unwrap_or_default();
        if let Some(plates) = &self.plates {
            plates.observe(message, graph_revision);
        }
    }

    /// Adds the published art to a view about to be sent. Presentation only: no other field of
    /// the view changes. The backdrop and mood faces are never added in combat or for another
    /// scene; the health variants of the heroes go to every audience.
    pub(super) fn apply(&self, message: &mut rpc::ViewMessage) {
        // PLAY-V09: the scene plate goes to the display and to phones alike.
        if let Some(plates) = &self.plates {
            plates.apply_to_message(message);
        }
        let stage = stage_of(message);
        if let Some(inner) = &self.inner {
            inner.apply(stage.as_ref(), message);
        } else if let (Some(stage), Some(rpc::view_message::Audience::Display(display))) =
            (stage.as_ref(), message.audience.as_mut())
        {
            let graph = graph_of(stage, None);
            self.note_graph(graph.as_ref());
            display.scene_graph = graph.as_ref().map(graph_to_wire);
        }
    }

    fn note_graph(&self, graph: Option<&SceneGraph>) {
        let revision = graph.map(|graph| graph.revision);
        let mut known = locked(&self.last_graph_revision);
        if *known == revision {
            return;
        }
        *known = revision;
        drop(known);
        if let Some(graph) = graph {
            record("SCENE GRAPH", &scene_graph::change_log(graph));
        }
    }
}

impl Inner {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn observe(&self, stage: Stage) -> Option<u64> {
        self.observe_at(stage, self.now());
        locked(&self.state)
            .scene_graph
            .as_ref()
            .map(|graph| graph.revision)
    }

    fn observe_at(&self, stage: Stage, now: Duration) {
        let mut state = locked(&self.state);
        if let Some(scene) = &stage.story_scene {
            state.canonical.observe_room(scene);
        }
        let changed_scene = state
            .stage
            .as_ref()
            .is_none_or(|known| known.scene_key != stage.scene_key);
        let mut had_overlay = false;
        if changed_scene {
            had_overlay = state.backdrop.is_some() || !state.sprites.is_empty();
            state.facts = Some(facts_of(&stage, &self.social));
            state.tracker.reset_scene();
            state.pacer.begin_scene(now);
            state.backdrop = None;
            state.sprites.clear();
            state.health_sprites.clear();
            state.ready_backdrop = None;
            state.failed_faces.clear();
            state.plate_wants.clear();
            state.last_revision = stage.revision;
        } else if stage.revision != state.last_revision {
            state.last_revision = stage.revision;
            state.pacer.note_action(now);
        }
        state.stage = Some(stage);
        let health_changed = refresh_health(&mut state);
        let graph = state
            .stage
            .as_ref()
            .and_then(|stage| graph_of(stage, Some(&state)));
        let graph_changed = graph.as_ref().map(|graph| graph.revision)
            != state.scene_graph.as_ref().map(|graph| graph.revision);
        state.scene_graph = graph;
        let graph_log = graph_changed
            .then(|| state.scene_graph.as_ref().map(scene_graph::change_log))
            .flatten();
        drop(state);
        if let Some(detail) = graph_log {
            record("SCENE GRAPH", &detail);
        }
        if had_overlay || health_changed || graph_changed {
            self.wake.send_modify(|count| *count += 1);
        }
    }

    fn apply(&self, stage: Option<&Stage>, message: &mut rpc::ViewMessage) {
        let state = locked(&self.state);
        match message.audience.as_mut() {
            Some(rpc::view_message::Audience::Display(display)) => {
                let Some(stage) = stage else {
                    return;
                };
                let current = state
                    .stage
                    .as_ref()
                    .is_some_and(|known| known.scene_key == stage.scene_key);
                if !current {
                    return;
                }
                if let Some(scene) = display.scene.as_mut() {
                    if let Some(key) = stage
                        .story_scene
                        .as_ref()
                        .and_then(|scene| scene.setting.as_ref())
                        .and_then(|setting| state.canonical.room(&setting.place_id))
                    {
                        scene.live_backdrop = key.path();
                    }
                    // The backdrop and the mood faces are never published in combat; a health
                    // variant is, because it shows how the fight is going.
                    if !stage.in_combat
                        && let Some(backdrop) = &state.backdrop
                    {
                        scene.live_backdrop = backdrop.key.path();
                    }
                    for actor in &mut scene.cast {
                        let overlay = state.health_sprites.get(&actor.id).or_else(|| {
                            (!stage.in_combat)
                                .then(|| state.sprites.get(&actor.id))
                                .flatten()
                        });
                        if let Some(overlay) = overlay
                            && overlay.source_sprite == actor.sprite
                        {
                            actor.live_sprite = overlay.key.path();
                        }
                    }
                }
                display.scene_graph = state.scene_graph.as_ref().map(graph_to_wire);
                apply_hero_faces(&state, display.journey.as_mut());
            }
            Some(rpc::view_message::Audience::Player(player)) => {
                apply_hero_faces(&state, player.journey.as_mut());
            }
            None => {}
        }
    }

    /// The worker: ticks, follows the social ledger, and owns every render task it starts.
    async fn run(
        self: Arc<Self>,
        mut checkpoints: watch::Receiver<df_model::checkpoint::Checkpoint>,
    ) {
        let _canonical_lifetime = canonical::WorkerLifetime(Arc::clone(&self));
        self.observe_checkpoint(&checkpoints.borrow_and_update().clone());
        let mut checkpoints_open = true;
        let mut ticker = tokio::time::interval(self.timing.tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut exchanges = self.social.subscribe_exchanges();
        let mut tasks: JoinSet<Done> = JoinSet::new();
        loop {
            tokio::select! {
                changed = checkpoints.changed(), if checkpoints_open => {
                    if changed.is_ok() { self.observe_checkpoint(&checkpoints.borrow_and_update().clone()); }
                    else { checkpoints_open = false; }
                }
                _ = ticker.tick() => {}
                _ = exchanges.changed() => {}
                Some(done) = tasks.join_next(), if !tasks.is_empty() => {
                    // A render task that panicked or was cancelled has nothing to report.
                    if let Ok(done) = done {
                        self.finish(done);
                    }
                }
            }
            let canonical_jobs = locked(&self.state)
                .canonical
                .take_jobs_up_to(super::imagery::MAXIMUM_IN_FLIGHT.saturating_sub(tasks.len()));
            for job in canonical_jobs {
                self.spawn(&mut tasks, job);
            }
            for job in self.evaluate() {
                self.spawn(&mut tasks, job);
            }
        }
    }

    fn spawn(self: &Arc<Self>, tasks: &mut JoinSet<Done>, job: Job) {
        let inner = Arc::clone(self);
        let started = Instant::now();
        match job {
            Job::Canonical(work) => {
                tasks.spawn(async move {
                    use futures::FutureExt;
                    let result = std::panic::AssertUnwindSafe(inner.render_canonical(&work))
                        .catch_unwind()
                        .await
                        .unwrap_or(Err(ImageFailure::Interrupted));
                    Done::Canonical { work, result }
                });
            }
            Job::HeroCutout { name, index, party } => {
                tasks.spawn(async move {
                    let result = inner.render_hero_cutout(index, &party).await;
                    super::actor::record_audit_line(
                        "gameplay.imagery_complete",
                        image_completion_line("hero_cutout", started.elapsed(), result.as_ref()),
                    );
                    Done::HeroCutout { name, result }
                });
            }
            Job::Expression { member, face } => {
                tasks.spawn(async move {
                    let result = inner.render_expression(&member, face).await;
                    super::actor::record_audit_line(
                        "gameplay.imagery_complete",
                        image_completion_line("expression", started.elapsed(), result.as_ref()),
                    );
                    Done::Expression {
                        npc_id: member.id,
                        face,
                        source_sprite: member.sprite,
                        result,
                    }
                });
            }
            Job::Warm { character, tier } => {
                tasks.spawn(async move {
                    let result = match inner.load_figure(&character.art).await {
                        Ok(sprite) => inner.render_health(sprite, character.facts, tier).await,
                        Err(failure) => Err(failure),
                    };
                    super::actor::record_audit_line(
                        "gameplay.imagery_complete",
                        image_completion_line("health", started.elapsed(), result.as_ref()),
                    );
                    Done::Warm {
                        id: character.id,
                        tier,
                        result,
                    }
                });
            }
            Job::Peek { character } => {
                tasks.spawn(async move {
                    let mut found = Vec::new();
                    if let Ok(sprite) = inner.load_figure(&character.art).await {
                        for tier in PREWARM_ORDER {
                            if let Some(key) = inner
                                .peek_health(sprite.clone(), character.facts, tier)
                                .await
                            {
                                found.push((tier, key));
                            }
                        }
                    }
                    Done::Peek {
                        id: character.id,
                        found,
                    }
                });
            }
            Job::Refresh {
                place,
                scene_asset,
                look,
                reason,
            } => {
                let scene_key = locked(&self.state)
                    .stage
                    .as_ref()
                    .map(|stage| stage.scene_key.clone())
                    .unwrap_or_default();
                tasks.spawn(async move {
                    let result = inner.render_refresh(&place, &scene_asset, look).await;
                    super::actor::record_audit_line(
                        "gameplay.imagery_complete",
                        image_completion_line("refresh", started.elapsed(), result.as_ref()),
                    );
                    Done::Refresh {
                        look,
                        reason,
                        scene_key,
                        result,
                    }
                });
            }
        }
    }

    async fn load(&self, name: String) -> Result<ReferenceImage, ImageFailure> {
        let references = self.references.clone();
        tokio::task::spawn_blocking(move || references.load(&name))
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .map_err(|_| ImageFailure::InvalidRequest)
    }

    async fn load_photographic(&self, name: String) -> Result<ReferenceImage, ImageFailure> {
        let references = self.references.clone();
        tokio::task::spawn_blocking(move || references.load_photographic(&name))
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .map_err(|_| ImageFailure::InvalidRequest)
    }

    async fn load_plate_reference(&self, name: String) -> Result<ReferenceImage, ImageFailure> {
        if self.references.photographic_only() {
            self.load_photographic(name).await
        } else {
            self.load(name).await
        }
    }

    async fn neutral_photographic_reference(&self) -> Result<ReferenceImage, ImageFailure> {
        let references = self.references.clone();
        tokio::task::spawn_blocking(move || references.neutral_photographic())
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .map_err(|_| ImageFailure::InvalidRequest)
    }

    async fn render_hero_cutout(
        &self,
        index: usize,
        party: &[usize],
    ) -> Result<ImageKey, ImageFailure> {
        let mut figures = Vec::with_capacity(party.len());
        for hero_index in party {
            figures.push(self.load_figure(&hero_art(*hero_index)).await?);
        }
        let sheet = party_sheet(&figures).map_err(|_| ImageFailure::InvalidRequest)?;
        self.imagery
            .render(scene_graph::hero_cutout_request(
                self.rating,
                sheet,
                index,
                party.len(),
            )?)
            .await
    }

    /// The art the stage shows for this character: the full-body twin of the cut-out where there
    /// is one (every non-player character stands with their feet on screen), otherwise the
    /// cut-out. The expression is made from that art so it replaces it in place, floor contact
    /// and framing included.
    async fn load_figure(&self, sprite: &str) -> Result<ReferenceImage, ImageFailure> {
        let stem = sprite.rsplit('/').next().unwrap_or(sprite);
        if let Some(name) = stem.strip_prefix("sprite-")
            && let Ok(figure) = if self.references.photographic_only() {
                self.load_photographic(format!("figure-{name}")).await
            } else {
                self.load(format!("figure-{name}")).await
            }
        {
            return Ok(figure);
        }
        if self.references.photographic_only() {
            self.load_photographic(sprite.to_owned()).await
        } else {
            self.load(sprite.to_owned()).await
        }
    }

    async fn render_expression(
        &self,
        member: &Member,
        face: Face,
    ) -> Result<ImageKey, ImageFailure> {
        let sprite = self.load_figure(&member.sprite).await?;
        let prompt = cast_expression(
            self.rating,
            CharacterFacts {
                role: member.role,
                life_stage: member.life_stage,
            },
            face.expression,
            face.tier,
            // No character's art derives from a real person's likeness today.
            false,
        )
        .map_err(|_| ImageFailure::InvalidRequest)?;
        self.imagery
            .render(ImageRequest {
                kind: ImageKind::CastExpression,
                references: vec![sprite],
                prompt,
                output: OutputSpec::CUTOUT,
            })
            .await
    }

    /// The request for a character's health variant: the authored figure, the closed health
    /// phrases, the same cut-out output as every other character art.
    fn health_request(
        &self,
        sprite: ReferenceImage,
        facts: CharacterFacts,
        tier: HealthTier,
    ) -> Result<ImageRequest, ImageFailure> {
        let prompt = cast_expression(
            self.rating,
            facts,
            Expression::Neutral,
            tier,
            // No character's art derives from a real person's likeness today.
            false,
        )
        .map_err(|_| ImageFailure::InvalidRequest)?;
        Ok(ImageRequest {
            kind: ImageKind::CastExpression,
            references: vec![sprite],
            prompt,
            output: OutputSpec::CUTOUT,
        })
    }

    async fn render_health(
        &self,
        sprite: ReferenceImage,
        facts: CharacterFacts,
        tier: HealthTier,
    ) -> Result<ImageKey, ImageFailure> {
        self.imagery
            .render(self.health_request(sprite, facts, tier)?)
            .await
    }

    /// The key of a health variant if it is already cached. Never calls the provider.
    async fn peek_health(
        &self,
        sprite: ReferenceImage,
        facts: CharacterFacts,
        tier: HealthTier,
    ) -> Option<ImageKey> {
        let request = self.health_request(sprite, facts, tier).ok()?;
        self.imagery.peek(&request).await
    }

    async fn render_refresh(
        &self,
        place: &PlaceFacts,
        scene_asset: &str,
        look: Look,
    ) -> Result<ImageKey, ImageFailure> {
        let backdrop = self.load_plate_reference(scene_asset.to_owned()).await?;
        self.imagery
            .render(ImageRequest {
                kind: ImageKind::SceneRefresh,
                references: vec![backdrop],
                prompt: scene_refresh(self.rating, place, look),
                output: OutputSpec::BACKDROP,
            })
            .await
    }

    /// Records a finished render. A scene refresh waits for a quiet moment before it lands.
    fn finish(&self, done: Done) {
        if let Done::Canonical { work, result } = done {
            self.finish_canonical(work, result);
            return;
        }
        let label_of = |result: &Result<ImageKey, ImageFailure>| {
            result
                .as_ref()
                .map_or_else(|failure| failure.label(), |_| "ready")
                .to_owned()
        };
        let (kind, outcome) = match &done {
            Done::Canonical { .. } => return,
            Done::HeroCutout { result, .. } => ("hero_cutout", label_of(result)),
            Done::Expression { result, .. } => ("expression", label_of(result)),
            Done::Refresh { result, .. } => ("refresh", label_of(result)),
            Done::Warm { result, .. } => ("health", label_of(result)),
            Done::Peek { .. } => ("health_peek", "ready".to_owned()),
        };
        record(
            "gameplay.living_scene.render_finished",
            &[("scene.kind", kind.to_owned()), ("scene.outcome", outcome)],
        );
        let mut state = locked(&self.state);
        match done {
            Done::Canonical { .. } => return,
            Done::HeroCutout { name, result } => {
                state.hero_cutouts_in_flight.remove(&name);
                match result {
                    Ok(key) => {
                        state.hero_cutouts.insert(name, key);
                        drop(state);
                        self.wake.send_modify(|count| *count += 1);
                    }
                    Err(failure) => {
                        state.failed_hero_cutouts.insert(name);
                        if failure == ImageFailure::BudgetExhausted {
                            state.budget_exhausted = true;
                        }
                    }
                }
            }
            Done::Warm { id, tier, result } => {
                match result {
                    Ok(key) => state.book.learn(&id, tier, key),
                    Err(ImageFailure::BudgetExhausted) => {
                        state.book.release(&id, tier);
                        state.budget_exhausted = true;
                    }
                    Err(_) => state.book.fail(&id, tier),
                }
                let changed = refresh_health(&mut state);
                drop(state);
                if changed {
                    self.wake.send_modify(|count| *count += 1);
                }
            }
            Done::Peek { id, found } => {
                for (tier, key) in found {
                    state.book.learn(&id, tier, key);
                }
                let changed = refresh_health(&mut state);
                drop(state);
                if changed {
                    self.wake.send_modify(|count| *count += 1);
                }
            }
            Done::Expression {
                npc_id,
                face,
                source_sprite,
                result,
            } => {
                state.expression_in_flight.remove(&npc_id);
                match result {
                    Ok(key) => {
                        let current = state.stage.as_ref().is_some_and(|stage| {
                            stage
                                .cast
                                .iter()
                                .any(|cast| cast.id == npc_id && cast.sprite == source_sprite)
                        });
                        if current {
                            state.sprites.insert(
                                npc_id.clone(),
                                SpriteOverlay {
                                    source_sprite,
                                    expression: face.expression,
                                    key,
                                },
                            );
                            state.tracker.mark_face_shown(&npc_id, face);
                            drop(state);
                            self.wake.send_modify(|count| *count += 1);
                        }
                    }
                    Err(failure) => {
                        state.failed_faces.insert((npc_id, face));
                        if failure == ImageFailure::BudgetExhausted {
                            state.budget_exhausted = true;
                        }
                    }
                }
            }
            Done::Refresh {
                look,
                reason,
                scene_key,
                result,
            } => {
                state.refresh_in_flight = false;
                let current = state
                    .stage
                    .as_ref()
                    .is_some_and(|stage| stage.scene_key == scene_key);
                match result {
                    Ok(key) if current => {
                        state.ready_backdrop = Some(ReadyBackdrop {
                            key,
                            look,
                            alternate: reason == RefreshReason::AlternateAngle,
                        });
                    }
                    Ok(_) => {}
                    Err(ImageFailure::BudgetExhausted) => state.budget_exhausted = true,
                    Err(_) => {}
                }
            }
        }
    }

    /// One decision pass: lands a ready backdrop when quiet, reverts to the authored art when
    /// the mood has calmed, and returns the renders to start.
    fn evaluate(&self) -> Vec<Job> {
        self.evaluate_at(self.now())
    }

    /// [`Self::evaluate`] at an explicit time, so tests control the clock.
    fn evaluate_at(&self, now: Duration) -> Vec<Job> {
        let mut state = locked(&self.state);
        let (Some(stage), Some(facts)) = (state.stage.clone(), state.facts.clone()) else {
            return Vec::new();
        };
        let in_combat = stage.in_combat;
        let mut wake = false;
        if let Some(ready) = state.ready_backdrop.take() {
            if state.pacer.may_publish(now, in_combat) {
                state.backdrop = Some(BackdropOverlay { key: ready.key });
                if ready.alternate {
                    // An alternate angle is not owed again: the mood is already on screen.
                    let target = state.tracker.shown_look();
                    state.tracker.mark_scene_shown(Look {
                        shot: ready.look.shot,
                        ..target
                    });
                } else {
                    state.tracker.mark_scene_shown(ready.look);
                }
                wake = true;
            } else {
                state.ready_backdrop = Some(ready);
            }
        }
        let cast: Vec<CastState> = facts
            .members
            .iter()
            .filter(|member| stage.cast.iter().any(|cast| cast.id == member.id))
            .map(|member| CastState {
                id: member.id.clone(),
                attitude: member.starting_attitude,
                opposes: member.opposes,
                speaking: stage.speaking_ids.contains(&member.id),
                tier: member_tier(&stage, member),
            })
            .collect();
        // The ledger names a dossier ("patron:merchant"); the cast names its member by id.
        let exchange = self.social.latest_exchange().map(|record| ExchangeSignal {
            sequence: record.sequence,
            npc_id: facts
                .members
                .iter()
                .find(|member| member.dossier_id.as_deref() == Some(record.npc_id.as_str()))
                .map_or(record.npc_id, |member| member.id.clone()),
            approach: record.approach,
            effect: record.effect,
            attitude: record.attitude,
            opposes: record.opposes,
            emotional_state: record.emotional_state.clone(),
        });
        if exchange
            .as_ref()
            .is_some_and(|signal| signal.sequence > state.tracker_handled())
        {
            state.pacer.note_story_event(now);
        }
        let inputs = MoodInputs {
            tempo: stage.tempo,
            tone: facts.tone,
            beat: facts.beat,
            cast: &cast,
            exchange: exchange.as_ref(),
            recent_kinds: &stage.recent_kinds,
        };
        let verdict = state.tracker.update(now, &inputs);
        let mut jobs = Vec::new();
        if !in_combat
            && stage
                .story_scene
                .as_ref()
                .is_some_and(|scene| scene.key == SceneKey::Opening)
        {
            let party: Vec<usize> = stage.heroes.iter().map(|hero| hero.index).collect();
            for (index, hero) in stage.heroes.iter().enumerate() {
                if u8::try_from(hero.index + 1)
                    .ok()
                    .and_then(|seat| state.canonical.hero_at_seat(seat))
                    .is_some()
                {
                    continue;
                }
                if state.hero_cutouts.contains_key(&hero.name)
                    || state.hero_cutouts_in_flight.contains(&hero.name)
                    || state.failed_hero_cutouts.contains(&hero.name)
                    || state.budget_exhausted
                {
                    continue;
                }
                state.hero_cutouts_in_flight.insert(hero.name.clone());
                jobs.push(Job::HeroCutout {
                    name: hero.name.clone(),
                    index,
                    party: party.clone(),
                });
            }
        }
        if !in_combat {
            for (id, face) in &verdict.face_changes {
                // PLAY-V09: the plate director edits the plate; a cut-out nobody sees is not drawn.
                state.plate_wants.insert(id.clone(), *face);
                if state.plate_covers && *face != Face::AUTHORED {
                    continue;
                }
                if *face == Face::AUTHORED {
                    // The authored cut-out is the neutral, unhurt face: reverting costs nothing.
                    if state.sprites.remove(id).is_some() {
                        wake = true;
                    }
                    state.tracker.mark_face_shown(id, Face::AUTHORED);
                    continue;
                }
                if face.tier != HealthTier::Healthy {
                    // A health face is made and shown by the health pipeline below, not as a
                    // mood render; it supersedes any mood face on this character.
                    if state.sprites.remove(id).is_some() {
                        wake = true;
                    }
                    state.tracker.mark_face_shown(id, *face);
                    continue;
                }
                let Some(member) = facts.members.iter().find(|member| member.id == *id) else {
                    continue;
                };
                if state.expression_in_flight.contains(id)
                    || state.failed_faces.contains(&(id.clone(), *face))
                    || state.budget_exhausted
                {
                    continue;
                }
                state.expression_in_flight.insert(id.clone());
                record(
                    "gameplay.living_scene.face_requested",
                    &[
                        ("scene.expression", face.expression.label().to_owned()),
                        ("scene.tier", face.tier.label().to_owned()),
                    ],
                );
                jobs.push(Job::Expression {
                    member: member.clone(),
                    face: *face,
                });
            }
        }
        jobs.extend(self.health_jobs(&mut state, &stage, &facts));
        let authored = verdict.look == Look::AUTHORED;
        if authored && verdict.scene_worth_rendering && state.pacer.may_publish(now, in_combat) {
            // The mood has calmed back to the authored framing: show the authored painting.
            if state.backdrop.take().is_some() {
                wake = true;
            }
            state.ready_backdrop = None;
            state.tracker.mark_scene_shown(Look::AUTHORED);
        } else if let Some(place) = &facts.place
            && !state.refresh_in_flight
            && state.ready_backdrop.is_none()
            && !state.budget_exhausted
            && !state.plate_covers
        {
            let worth = verdict.scene_worth_rendering;
            if let Some(reason) = state.pacer.refresh_due(now, in_combat, worth) {
                let look = match reason {
                    RefreshReason::Change => verdict.look,
                    RefreshReason::AlternateAngle => {
                        alternate_look(verdict.look, state.pacer.alternates())
                    }
                };
                state.pacer.refresh_started(now, reason);
                state.refresh_in_flight = true;
                record(
                    "gameplay.living_scene.refresh_requested",
                    &[
                        ("scene.mood", look.mood.label().to_owned()),
                        ("scene.shot", look.shot.label().to_owned()),
                        ("scene.lighting", look.lighting.label().to_owned()),
                        (
                            "scene.reason",
                            match reason {
                                RefreshReason::Change => "change",
                                RefreshReason::AlternateAngle => "alternate_angle",
                            }
                            .to_owned(),
                        ),
                    ],
                );
                jobs.push(Job::Refresh {
                    place: place.clone(),
                    scene_asset: stage.scene_asset.clone(),
                    look,
                    reason,
                });
            }
        }
        drop(state);
        if wake {
            self.wake.send_modify(|count| *count += 1);
        }
        jobs
    }
}

impl Inner {
    /// The health variants to look for or make now. Looking is always allowed; making is never
    /// done in a fight, so combat shows only what is already cached.
    fn health_jobs(&self, state: &mut State, stage: &Stage, facts: &SceneFacts) -> Vec<Job> {
        let characters = characters_of(stage, facts);
        let mut jobs = Vec::new();
        for character in &characters {
            if state.book.needs_peek(&character.id) {
                state.book.mark_peeked(&character.id);
                jobs.push(Job::Peek {
                    character: character.clone(),
                });
            }
        }
        if stage.in_combat || state.budget_exhausted {
            return jobs;
        }
        // What the party and the cast look like right now comes before background warming.
        let now_needed = stage
            .heroes
            .iter()
            .flat_map(|hero| {
                let tier = tier_of(hero.vitals);
                [hero_art(hero.index), battle_hero_art(hero.index)]
                    .into_iter()
                    .filter_map(move |art| Some((art, tier?)))
            })
            .chain(
                facts
                    .members
                    .iter()
                    .filter(|member| stage.cast.iter().any(|cast| cast.id == member.id))
                    .map(|member| (member.sprite.clone(), member_tier(stage, member))),
            )
            .filter(|(_, tier)| *tier != HealthTier::Healthy);
        for (art, tier) in now_needed {
            if state.book.is_open(&art, tier)
                && let Some(character) = characters.iter().find(|character| character.id == art)
            {
                state.book.start_warming(&art, tier);
                jobs.push(Job::Warm {
                    character: character.clone(),
                    tier,
                });
            }
        }
        if let Some((index, tier)) =
            state
                .book
                .next_warm(&characters, self.imagery.budget_remaining(), false)
            && let Some(character) = characters.get(index)
        {
            state.book.start_warming(&character.id, tier);
            jobs.push(Job::Warm {
                character: character.clone(),
                tier,
            });
        }
        jobs
    }
}

impl State {
    fn new(timing: Timing) -> Self {
        Self {
            canonical: canonical::References::default(),
            stage: None,
            facts: None,
            tracker: MoodTracker::new(timing.hysteresis),
            pacer: Pacer::new(timing, Duration::ZERO),
            backdrop: None,
            sprites: BTreeMap::new(),
            ready_backdrop: None,
            refresh_in_flight: false,
            expression_in_flight: BTreeSet::new(),
            failed_faces: BTreeSet::new(),
            last_revision: None,
            budget_exhausted: false,
            plate_covers: false,
            plate_wants: BTreeMap::new(),
            book: Book::default(),
            health_sprites: BTreeMap::new(),
            hero_faces: BTreeMap::new(),
            hero_cutouts: BTreeMap::new(),
            hero_cutouts_in_flight: BTreeSet::new(),
            failed_hero_cutouts: BTreeSet::new(),
            scene_graph: None,
        }
    }

    /// The newest exchange sequence the mood has already handled.
    fn tracker_handled(&self) -> u64 {
        self.tracker.handled_sequence()
    }
}

/// A gentle change of angle for a quiet scene: the same mood and light, another camera.
fn alternate_look(target: Look, alternates: usize) -> Look {
    const ANGLES: [super::scene_mood::Shot; 4] = [
        super::scene_mood::Shot::MediumConversation,
        super::scene_mood::Shot::LowAngle,
        super::scene_mood::Shot::HighAngle,
        super::scene_mood::Shot::OverShoulderTowardSpeaker,
    ];
    Look {
        shot: ANGLES[alternates % ANGLES.len()],
        ..target
    }
}

fn image_completion_line(
    kind: &'static str,
    elapsed: Duration,
    result: Result<&ImageKey, &ImageFailure>,
) -> String {
    let (outcome, key, status) = match result {
        Ok(key) => ("ready", key.as_str(), "ready"),
        Err(failure) => ("failed", "none", failure.label()),
    };
    format!(
        "IMAGERY {outcome} kind={kind} id={key} ms={} status={status} boundary=scene-owner",
        elapsed.as_millis()
    )
}

#[cfg(test)]
#[path = "living_scenes_tests.rs"]
mod tests;
