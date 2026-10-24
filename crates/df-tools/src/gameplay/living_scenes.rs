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
use df_render::art_contract::{self, Role};
use tokio::{sync::watch, task::JoinSet, time::Instant};

use crate::health_tier::{Vitals, tier_of};

// PLAY-V09: scene plates (the whole scene as one composed picture), made just in time.
mod canonical;
mod plates;
pub(super) use plates::ahead::Source as PlateAheadSource;

use super::{
    adventure::{dossier::Attitude, setting::SettingKind},
    health_faces::{Book, Character, GENERATED_TIERS},
    imagery::{
        CharacterFacts, CharacterRole, ImageFailure, ImageKey, ImageKind, ImageRequest, Imagery,
        LifeStage, Lookup, OutputSpec, PlaceFacts, ReferenceImage, ReferenceStore, cast_expression,
        scene_refresh,
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
use super::{actor, home_video::kill_shot, resources};

/// The hero's accepted canonical picture is the source for every health variant and battle face.
fn live_hero_key(state: &State, hero: &StageHero) -> Option<ImageKey> {
    u8::try_from(hero.index + 1).ok().and_then(|seat| {
        state
            .canonical
            .hero_at_seat(seat)
            .and_then(|(entity, _)| state.canonical.hero(entity))
    })
}

/// Combat-only creatures do not occur in the story cast. Admit their stable public identity
/// through the same canonical owner and publish only completed cache handles, never guessed URLs.
fn apply_creature_faces(
    canonical: &mut canonical::References,
    journey: Option<&mut rpc::JourneyView>,
) {
    let Some(journey) = journey else {
        return;
    };
    let Some(combat) = &journey.combat else {
        return;
    };
    for participant in combat
        .participants
        .iter()
        .filter(|participant| participant.side == 2 && !participant.creature_id.is_empty())
    {
        let id = format!("combat-creature:{}", participant.creature_id);
        let sprite = format!("figure-ant-{}", participant.creature_id);
        let reviewed = art_contract::identity_photo(&sprite).is_some();
        if !reviewed {
            canonical.admit_cast(&Member {
                id: id.clone(),
                role: CharacterRole::Antagonist,
                life_stage: LifeStage::Adult,
                gender: None,
                age: None,
                name: participant.creature_id.replace('-', " "),
                sprite,
                dossier_id: None,
                opposes: true,
                starting_attitude: None,
            });
        }
        if let Some(key) = canonical.npc(&id) {
            let path = key.path();
            journey.live_faces.push(rpc::LiveFace {
                name: participant.name.clone(),
                sprite: path.clone(),
                battle_sprite: path,
            });
        }
    }
}

/// The characters whose art a hero's health variants are made from: one character sourced from
/// the hero's canonical picture when it exists (the same variant serves the stage and battlefield).
/// Without that picture, no variant is requested.
fn hero_characters(_hero: &StageHero, live: Option<ImageKey>) -> Vec<Character> {
    let facts = CharacterFacts {
        role: CharacterRole::Hero,
        life_stage: LifeStage::Adult,
    };
    if let Some(key) = live {
        let art = key.path();
        return vec![Character {
            id: art.clone(),
            art,
            facts,
        }];
    }
    Vec::new()
}

/// The picture a hero shows on each screen now: the health variant of their current tier when
/// one exists, otherwise their own live picture. `None` on a screen means no live picture
/// exists yet, so that screen shows a name placeholder.
#[derive(Clone, Debug, Eq, PartialEq)]
struct HeroFace {
    /// The picture for the exploration stage.
    stage: Option<ImageKey>,
    /// The picture for the battlefield (the same one when the variant is made from the live
    /// picture).
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
    gender: Option<pack::Gender>,
    age: Option<pack::Age>,
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
    /// Named cast of the adventure's fights that is not on this stage. Looked for in the cache
    /// only, so a foe who joins shows a variant already made; never made ahead of time.
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
    let story_scene = plates::locate(
        &scene.title,
        &asset,
        &scene.source_scene_key,
        &scene.source_scene_variant,
    )
    .ok();
    let hero_names: Vec<(String, String)> = journey
        .map(|journey| {
            journey
                .party
                .iter()
                .enumerate()
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
        scene_key: plates::projection_scene_identity(
            &scene.source_scene_key,
            &scene.source_scene_variant,
            &scene.title,
            &asset,
        ),
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
        let fallback_art = art_contract::NEUTRAL.to_owned();
        let live_art = state.and_then(|state| {
            state
                .hero_faces
                .get(&hero.name)
                .and_then(|face| face.stage.as_ref())
                .cloned()
                .or_else(|| live_hero_key(state, hero))
                .map(|key| key.path())
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
        // A place the run created is not in the pack; the stage found it where it is published.
        let Some(scene) = scene.or(stage.story_scene.as_ref()) else {
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
                gender: member.gender,
                age: member.age,
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

const MAXIMUM_PENDING_KILL_STILLS: usize = 32;
const KILL_REFERENCE_DEADLINE: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
enum KillSubject {
    Hero {
        entity: df_model::checkpoint::EntityId,
    },
    Npc(String),
}

#[derive(Clone, Debug)]
struct KillStill {
    request: kill_shot::KillShotRequest,
    subjects: [KillSubject; 2],
    room: String,
    references: [Option<ImageKey>; 3],
    started: Instant,
}

impl KillStill {
    fn resolve(&mut self, state: &State) -> Option<[ImageKey; 3]> {
        for (index, subject) in self.subjects.iter().enumerate() {
            if self.references[index].is_none() {
                self.references[index] = match subject {
                    KillSubject::Hero { entity, .. } => state.canonical.hero(*entity),
                    KillSubject::Npc(id) => state.canonical.npc(id),
                };
            }
        }
        if self.references[2].is_none() {
            self.references[2] = state.canonical.room(&self.room);
        }
        Some([
            self.references[0].clone()?,
            self.references[1].clone()?,
            self.references[2].clone()?,
        ])
    }
}

struct State {
    canonical: canonical::References,
    clip_reads: BTreeSet<String>,
    kill_stills_seen: kill_shot::ShownShots,
    kill_stills_pending: Vec<KillStill>,
    stage: Option<Arc<Stage>>,
    facts: Option<Arc<SceneFacts>>,
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
    scene_graph: Option<SceneGraph>,
}

/// What the worker should start.
enum Job {
    KillShot {
        still: KillStill,
        references: [ImageKey; 3],
    },
    Canonical(canonical::Work),
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
    /// Makes one health variant: the only job that can spend on health art, started only for a
    /// character who is at that tier in the view now.
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
    Resource(resources::Completed),
    KillShot {
        request: kill_shot::KillShotRequest,
        elapsed: Duration,
        result: Result<(Vec<u8>, String), ImageFailure>,
    },
    Canonical {
        work: canonical::Work,
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

/// Publish the accepted self-portrait, then the canonical image, to every personal surface.
/// Scene-graph health figures remain separate from a hero's sheet/party/turn portrait.
fn apply_hero_faces(state: &State, journey: Option<&mut rpc::JourneyView>) {
    let Some(journey) = journey else {
        return;
    };
    journey.live_faces.clear();
    for (index, member) in journey.party.iter_mut().enumerate() {
        let entity = if member.participant_id.is_empty() {
            u8::try_from(index + 1)
                .ok()
                .and_then(|seat| state.canonical.hero_at_seat(seat))
                .map(|(entity, _)| entity)
        } else {
            df_model::checkpoint::EntityId::from_bytes(&member.participant_id).ok()
        };
        let personal = art_contract::player_hero_image(&member.portrait)
            .filter(|path| path.starts_with("/gameplay/creator-portrait/"))
            .map(str::to_owned)
            .or_else(|| entity.and_then(|entity| state.canonical.hero_path(entity)))
            .or_else(|| art_contract::player_hero_image(&member.portrait).map(str::to_owned))
            .unwrap_or_default();
        member.portrait = personal.clone();
        journey.live_faces.push(rpc::LiveFace {
            name: member.name.clone(),
            sprite: personal.clone(),
            battle_sprite: personal,
        });
    }
    if let Some(combat) = &mut journey.combat {
        for member in combat
            .participants
            .iter_mut()
            .filter(|member| member.side == 1)
        {
            member.portrait = journey
                .party
                .iter()
                .find(|hero| hero.participant_id == member.participant_id)
                .map(|hero| hero.portrait.clone())
                .unwrap_or_default();
        }
    }
    if let Some(own) = &mut journey.own_character {
        // Names are unique at creation; personal art never comes from party position here.
        own.portrait_uri = journey
            .party
            .iter()
            .find(|member| member.name == own.name)
            .map(|member| member.portrait.clone())
            .unwrap_or_else(|| {
                art_contract::player_hero_image(&own.portrait_uri)
                    .unwrap_or("")
                    .to_owned()
            });
    }
}

/// The characters whose art has health variants in this stage: the party's heroes first, then
/// the scene's cast. A hero with a live picture contributes one character sourced from it;
/// a hero without one contributes no health-variant request.
fn characters_of(
    stage: &Stage,
    facts: &SceneFacts,
    hero_live: &[Option<ImageKey>],
) -> Vec<Character> {
    let heroes =
        stage.heroes.iter().enumerate().flat_map(|(index, hero)| {
            hero_characters(hero, hero_live.get(index).cloned().flatten())
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

/// The health variants to make now: one for each hero and cast member who is hurt in this view,
/// at the tier they are at, unless the cache knows it, it is being made or it was tried twice.
/// Making is on demand and never ahead of time, so a fresh scene of healthy characters makes
/// nothing. Pure apart from the book, which records what is being made.
fn warm_jobs(
    book: &mut Book,
    stage: &Stage,
    facts: &SceneFacts,
    characters: &[Character],
    hero_live: &[Option<ImageKey>],
) -> Vec<Job> {
    let heroes = stage.heroes.iter().enumerate().flat_map(|(index, hero)| {
        let tier = tier_of(hero.vitals);
        let live = hero_live.get(index).cloned().flatten();
        hero_characters(hero, live)
            .into_iter()
            .filter_map(move |character| Some((character.id, tier?)))
    });
    let cast = facts
        .members
        .iter()
        .filter(|member| stage.cast.iter().any(|cast| cast.id == member.id))
        .map(|member| (member.sprite.clone(), member_tier(stage, member)));
    let hurt: Vec<(String, HealthTier)> = heroes
        .chain(cast)
        .filter(|(_, tier)| *tier != HealthTier::Healthy)
        .collect();
    let mut jobs = Vec::new();
    for (art, tier) in hurt {
        if book.is_open(&art, tier)
            && let Some(character) = characters.iter().find(|character| character.id == art)
        {
            book.start_warming(&art, tier);
            jobs.push(Job::Warm {
                character: character.clone(),
                tier,
            });
        }
    }
    jobs
}

/// Recomputes which picture every hero and cast member shows from the view's hit points and
/// the variants known to exist: the variant of the exact tier, otherwise the nearest cached
/// one, otherwise (heroes) their own live picture or (everyone) the authored art. Pure
/// presentation, no provider call; returns whether anything shown changed.
fn refresh_health(state: &mut State) -> bool {
    let (Some(stage), Some(facts)) = (state.stage.as_ref(), state.facts.as_ref()) else {
        return false;
    };
    // Every hero is published, hurt or not: the health variant of their tier when one exists,
    // otherwise their own live picture, otherwise nothing (the screen shows their name).
    // A hero with a live picture has one variant source, so the stage and the battlefield
    // show the same variant; a hero without one waits for their own image.
    let mut hero_faces = BTreeMap::new();
    for hero in &stage.heroes {
        let live = live_hero_key(state, hero);
        let tier = tier_of(hero.vitals);
        let nearest = |id: &str| {
            tier.and_then(|tier| state.book.nearest(id, tier))
                .map(|(_, key)| key.clone())
        };
        let (stage_variant, battle_variant) = match &live {
            Some(key) => {
                let variant = nearest(&key.path());
                (variant.clone(), variant)
            }
            None => (None, None),
        };
        hero_faces.insert(
            hero.name.clone(),
            HeroFace {
                stage: stage_variant.or_else(|| live.clone()),
                battle: battle_variant.or(live),
            },
        );
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
    if event != "gameplay.living_scene.face_requested" {
        super::dev_control::timeline::record_structured(
            event,
            event,
            fields
                .iter()
                .map(|(key, value)| ((*key).to_owned(), serde_json::Value::String(value.clone())))
                .collect(),
        );
    }

    let context = df_observe::OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, "gameplay.living_scene");
    span.event(event, fields);
    span.finish_unmeasured("recorded");
}

impl super::Service {
    /// The plate-publication boundary admits evidence through the actor before any audience uses it.
    pub(super) async fn admit_picture_features(
        &self,
        credential: [u8; 32],
        role: df_persistence::local_demo_scope::LocalDemoRole,
        checkpoint: &df_model::checkpoint::Checkpoint,
        message: &mut rpc::ViewMessage,
    ) -> bool {
        use super::{imagery::ImageKey, journey::living_scene};
        let scene = match message.audience.as_mut() {
            Some(rpc::view_message::Audience::Display(view)) => view.scene.as_mut(),
            Some(rpc::view_message::Audience::Player(view)) => view.scene.as_mut(),
            None => None,
        };
        let Some(scene) = scene else {
            return false;
        };
        let Some(key) = scene
            .live_plate
            .strip_prefix("/gameplay/imagery/")
            .and_then(ImageKey::parse)
        else {
            return false;
        };
        let Some(inner) = &self.living.inner else {
            return false;
        };
        let evidence = match inner.imagery.photo_evidence(&key).await {
            Ok(Some(evidence)) => evidence,
            Ok(None) => return false,
            Err(error) => {
                actor::record_unavailable("picture-evidence-read", error.label());
                scene.live_plate.clear();
                scene.plate_state = rpc::PlateState::Pending as i32;
                return false;
            }
        };
        let admitted = living_scene::picture_features(checkpoint.state(), &evidence.place);
        if admitted.as_ref().is_ok_and(|records| {
            records
                .iter()
                .any(|record| record.plate_key == key.as_str())
                || records
                    .iter()
                    .map(|record| record.seen_in_picture.len())
                    .sum::<usize>()
                    >= living_scene::MAXIMUM_PICTURE_FEATURES
        }) {
            return false;
        }
        let public_scene = super::beat_script_runtime::scene(checkpoint.state());
        if let Ok(public_scene) = public_scene {
            if let Ok(Some(snapshot)) = living_scene::snapshot(checkpoint.state(), &public_scene) {
                let names = snapshot
                    .model
                    .people
                    .iter()
                    .map(|person| super::place::name_key(&person.name))
                    .chain(
                        snapshot
                            .model
                            .things
                            .iter()
                            .map(|thing| super::place::name_key(&thing.name)),
                    )
                    .chain(
                        snapshot
                            .model
                            .offers
                            .iter()
                            .map(|offer| super::place::name_key(&offer.name)),
                    )
                    .chain(
                        admitted
                            .as_ref()
                            .into_iter()
                            .flat_map(|records| records.iter())
                            .flat_map(|record| record.seen_in_picture.iter())
                            .map(|feature| super::place::name_key(&feature.name)),
                    )
                    .collect::<BTreeSet<_>>();
                let ids = snapshot
                    .model
                    .people
                    .iter()
                    .map(|person| person.id.as_str())
                    .chain(snapshot.model.things.iter().map(|thing| thing.id.as_str()))
                    .chain(snapshot.model.offers.iter().map(|offer| offer.id.as_str()))
                    .chain(snapshot.model.ways_out.iter().map(|way| way.id.as_str()))
                    .collect::<BTreeSet<_>>();
                if evidence.visible_features.iter().all(|feature| {
                    feature
                        .recorded_id
                        .as_deref()
                        .is_some_and(|id| ids.contains(id))
                        || names.contains(&super::place::name_key(&feature.name))
                }) {
                    return false;
                }
            }
        }
        let mut committed = false;
        if role == df_persistence::local_demo_scope::LocalDemoRole::Player {
            let (reply, wait) = tokio::sync::oneshot::channel();
            let submitted = self.actor.try_submit(actor::Call::PictureFeatures {
                credential,
                expected_basis: checkpoint.basis(),
                source_hash: evidence.source_hash,
                record: living_scene::PictureFeatures {
                    place: evidence.place,
                    plate_key: key.as_str().into(),
                    seen_in_picture: evidence.visible_features,
                },
                reply,
            });
            if submitted.is_ok() {
                if let Ok(Ok(Ok(_revision))) =
                    tokio::time::timeout(super::timing::active().reply, wait).await
                {
                    committed = true;
                }
            }
        }
        if !committed {
            // A display credential cannot mutate game state. Its connected player stream admits
            // the same cached evidence; the checkpoint wake then republishes this plate.
            scene.live_plate.clear();
            scene.plate_state = rpc::PlateState::Pending as i32;
        }
        committed
    }
}

impl LivingScenes {
    /// Register the committed identity before projecting a view that can request its plate.
    pub(super) fn observe_checkpoint(&self, checkpoint: &df_model::checkpoint::Checkpoint) {
        if let Some(inner) = &self.inner {
            inner.observe_checkpoint(checkpoint);
        }
    }
    /// Queue the committed blow once; the living-scenes worker owns all image I/O.
    pub(super) fn request_kill_shot(
        &self,
        request: kill_shot::KillShotRequest,
        checkpoint: &df_model::checkpoint::Checkpoint,
        outcome: &rpc::CombatOutcome,
    ) {
        let Some(inner) = &self.inner else {
            audit_kill_still(&request, Duration::ZERO, "failed", "imagery_off");
            return;
        };
        inner.observe_checkpoint(checkpoint);
        let scene = super::beat_script_runtime::scene(checkpoint.state()).ok();
        let subject = |id: &[u8], name: &str| {
            if let Some(hero) = checkpoint
                .state()
                .characters
                .iter()
                .find(|hero| hero.entity.as_bytes().as_slice() == id)
            {
                return Some(KillSubject::Hero {
                    entity: hero.entity,
                });
            }
            scene
                .as_ref()?
                .cast
                .iter()
                .find(|member| member.name == name)
                .map(|member| KillSubject::Npc(member.id.clone()))
        };
        let subjects = subject(&outcome.actor_id, &outcome.actor_name)
            .zip(subject(&outcome.target_id, &outcome.target_name));
        let room = scene
            .as_ref()
            .and_then(|scene| scene.setting.as_ref())
            .map(|setting| setting.place_id.clone());
        let mut state = locked(&inner.state);
        if state.kill_stills_seen.was_shown(&request.id, request.epoch) {
            return;
        }
        let (Some((killer, victim)), Some(room)) = (subjects, room) else {
            state.kill_stills_seen.claim(&request.id, request.epoch);
            audit_kill_still(
                &request,
                Duration::ZERO,
                "failed",
                "canonical_subject_missing",
            );
            return;
        };
        if let Some(scene) = &scene {
            state.canonical.observe_room(scene);
            for member in scene.cast.iter().filter(|member| {
                member.name == outcome.actor_name || member.name == outcome.target_name
            }) {
                state.canonical.admit_cast(&Member {
                    id: member.id.clone(),
                    role: role_of(member.role),
                    life_stage: LifeStage::from_age(member.age),
                    gender: member.gender,
                    age: member.age,
                    name: member.name.clone(),
                    sprite: member.sprite.clone(),
                    dossier_id: None,
                    opposes: false,
                    starting_attitude: None,
                });
            }
        }
        inner.queue_kill_still(
            &mut state,
            KillStill {
                request,
                subjects: [killer, victim],
                room,
                references: [None, None, None],
                started: Instant::now(),
            },
        );
        drop(state);
        inner.wake.send_modify(|count| *count += 1);
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
    #[cfg(test)]
    pub(super) fn start(
        imagery: Imagery,
        references: ReferenceStore,
        social: Arc<SocialLedger>,
        rating: ContentRating,
        timing: Timing,
        plates_on: bool,
        checkpoints: watch::Receiver<df_model::checkpoint::Checkpoint>,
    ) -> Self {
        Self::start_with_local_lore(
            imagery,
            references,
            (
                social,
                Arc::new(crate::gameplay::npc_reply::local_lore::LocalLore::in_memory()),
            ),
            rating,
            timing,
            plates_on,
            checkpoints,
        )
    }

    pub(super) fn start_with_local_lore(
        imagery: Imagery,
        references: ReferenceStore,
        social_and_lore: (
            Arc<SocialLedger>,
            Arc<crate::gameplay::npc_reply::local_lore::LocalLore>,
        ),
        rating: ContentRating,
        timing: Timing,
        plates_on: bool,
        checkpoints: watch::Receiver<df_model::checkpoint::Checkpoint>,
    ) -> Self {
        let (social, lore) = social_and_lore;
        let mut state = State::new(timing);
        state.canonical = canonical::References::new(lore);
        let origin = Instant::now();
        let inner = Arc::new(Inner {
            imagery,
            references,
            social,
            rating,
            timing,
            state: Mutex::new(state),
            wake: watch::channel(0).0,
            origin,
        });
        let checkpoint = checkpoints.borrow().clone();
        inner.observe_checkpoint(&checkpoint);
        let plates =
            plates_on.then(|| plates::Plates::start(Arc::clone(&inner), checkpoints.clone()));
        let worker = tokio::spawn(Arc::clone(&inner).run(checkpoints));
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

    pub(super) fn request_committed_item(
        &self,
        hero: &str,
        thing: &crate::gameplay::adventure::scene_model::SceneThing,
        detail: Option<String>,
        shared: bool,
    ) -> Result<(), &'static str> {
        self.plates
            .as_ref()
            .ok_or("scene pictures are unavailable")?
            .request_committed_item(hero, thing, detail, shared)
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
            state.facts = Some(Arc::new(facts_of(&stage, &self.social)));
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
        state.stage = Some(Arc::new(stage));
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
        let mut state = locked(&self.state);
        match message.audience.as_mut() {
            Some(rpc::view_message::Audience::Display(display)) => {
                apply_hero_faces(&state, display.journey.as_mut());
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
                        let overlay_path = overlay
                            .filter(|overlay| overlay.source_sprite == actor.sprite)
                            .map(|overlay| overlay.key.path());
                        if let Some(path) = overlay_path {
                            actor.live_sprite = path;
                        } else {
                            Self::live_photograph(&mut state, actor);
                        }
                    }
                }
                display.scene_graph = state.scene_graph.as_ref().map(graph_to_wire);
                apply_hero_faces(&state, display.journey.as_mut());
                apply_creature_faces(&mut state.canonical, display.journey.as_mut());
            }
            Some(rpc::view_message::Audience::Player(player)) => {
                apply_hero_faces(&state, player.journey.as_mut());
                apply_creature_faces(&mut state.canonical, player.journey.as_mut());
            }
            None => {}
        }
    }

    /// The art contract for a cast member's picture, steps one and three: a live canonical
    /// photograph already made for this person is the cut-out the display shows, even when a
    /// reviewed photograph also answers the cast member; a cast member that only a reviewed
    /// photograph answers is left to the library; otherwise it is requested now, once (the request
    /// is idempotent per subject), and the display keeps its neutral fallback until it is ready.
    /// This is the only place a view first needing a depiction asks for it.
    fn live_photograph(state: &mut State, actor: &mut rpc::SceneActor) {
        let id = actor.id.strip_prefix("npc:").unwrap_or(&actor.id);
        let subject = art_contract::Subject::character(id);
        let reviewed = art_contract::plan(Role::CharacterToken, &actor.sprite).reviewed;
        // Step one of the art contract: a live image already made for this person leads, even when
        // a reviewed photograph also answers the cast member.
        if actor.live_sprite.is_empty()
            && let Some(key) = state.canonical.npc(id)
        {
            actor.live_sprite = key.path();
            state.canonical.note_resolved(&subject, "live");
            return;
        }
        if reviewed.is_some() || !actor.live_sprite.is_empty() {
            let source = if reviewed.is_some() {
                "reviewed"
            } else {
                "live"
            };
            state.canonical.note_resolved(&subject, source);
            return;
        }
        state.canonical.note_resolved(&subject, "neutral");
        let member = state
            .facts
            .as_ref()
            .and_then(|facts| facts.members.iter().find(|member| member.id == id))
            .cloned();
        if let Some(member) = member {
            state.canonical.admit_cast(&member);
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
        let mut kill_requests = self.wake.subscribe();
        let mut tasks: JoinSet<Done> = JoinSet::new();
        let mut pending_renders = std::collections::VecDeque::new();
        loop {
            tokio::select! {
                changed = checkpoints.changed(), if checkpoints_open => {
                    if changed.is_ok() { self.observe_checkpoint(&checkpoints.borrow_and_update().clone()); }
                    else { checkpoints_open = false; }
                }
                _ = ticker.tick() => {}
                _ = kill_requests.changed() => {}
                _ = exchanges.changed() => {}
                Some(done) = tasks.join_next(), if !tasks.is_empty() => {
                    // A render task that panicked or was cancelled has nothing to report.
                    if let Ok(done) = done {
                        self.finish(done);
                    }
                }
            }
            for job in self.take_kill_stills(
                Instant::now(),
                super::imagery::MAXIMUM_IN_FLIGHT.saturating_sub(tasks.len()),
            ) {
                self.spawn(&mut tasks, job);
            }
            let canonical_jobs = locked(&self.state)
                .canonical
                .take_jobs_up_to(super::imagery::MAXIMUM_IN_FLIGHT.saturating_sub(tasks.len()));
            for job in canonical_jobs {
                self.spawn(&mut tasks, job);
            }
            pending_renders.extend(self.evaluate());
            while tasks.len() < super::imagery::MAXIMUM_IN_FLIGHT {
                let Some(job) = pending_renders.pop_front() else {
                    break;
                };
                self.spawn(&mut tasks, job);
            }
            for job in self
                .imagery
                .hub()
                .take_jobs(super::imagery::MAXIMUM_IN_FLIGHT.saturating_sub(tasks.len()))
            {
                tasks.spawn(async move { Done::Resource(job.execute().await) });
            }
        }
    }

    fn spawn(self: &Arc<Self>, tasks: &mut JoinSet<Done>, job: Job) {
        let inner = Arc::clone(self);
        let started = Instant::now();
        match job {
            Job::KillShot { still, references } => {
                tasks.spawn(async move {
                    use futures::FutureExt;
                    let result = std::panic::AssertUnwindSafe(
                        inner.render_kill_still(&still.request, &references),
                    )
                    .catch_unwind()
                    .await
                    .unwrap_or(Err(ImageFailure::Interrupted));
                    Done::KillShot {
                        request: still.request,
                        elapsed: still.started.elapsed(),
                        result,
                    }
                });
            }
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
                    let result = match inner.load_health_reference(&character).await {
                        Ok(sprite) => {
                            let subject =
                                format!("character:health:{}:{tier:?}", &sprite.sha256[..12]);
                            let request = inner.hero_health_request(&character, sprite, tier);
                            match request {
                                Ok(request) => inner.imagery.generate(&subject, request).await,
                                Err(failure) => Err(failure),
                            }
                        }
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
                    if let Ok(sprite) = inner.load_health_reference(&character).await {
                        for tier in GENERATED_TIERS {
                            if let Ok(request) =
                                inner.hero_health_request(&character, sprite.clone(), tier)
                                && let Some(key) = inner.imagery.peek(&request).await
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

    /// The art contract's on-demand request for a cast member the current stage knows and no
    /// reviewed photograph answers. Idempotent per subject (a person already admitted or
    /// requested is left alone); returns whether the person is known to this stage.
    fn request_cast_photograph(&self, id: &str) -> bool {
        let mut state = locked(&self.state);
        let member = state
            .facts
            .as_ref()
            .and_then(|facts| facts.members.iter().find(|member| member.id == id))
            .cloned();
        member.is_some_and(|member| {
            state.canonical.admit_cast(&member);
            true
        })
    }

    /// A reference for an image request: a reviewed photograph only (the art contract). A painted
    /// or unreviewed stem is an invalid request and nothing is read.
    async fn load(&self, name: String) -> Result<ReferenceImage, ImageFailure> {
        let references = self.references.clone();
        tokio::task::spawn_blocking(move || references.load(&name))
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .map_err(|_| ImageFailure::InvalidRequest)
    }

    fn queue_kill_still(&self, state: &mut State, mut still: KillStill) {
        if !state
            .kill_stills_seen
            .claim(&still.request.id, still.request.epoch)
        {
            return;
        }
        if state.kill_stills_pending.len() >= MAXIMUM_PENDING_KILL_STILLS {
            audit_kill_still(&still.request, Duration::ZERO, "failed", "still_queue_full");
            return;
        }
        // Freeze every ready identity immediately, before a subsequent scene can replace it.
        let _ = still.resolve(state);
        audit_kill_still(
            &still.request,
            Duration::ZERO,
            "still_queued",
            "committed_killing_blow",
        );
        state.kill_stills_pending.push(still);
    }

    fn take_kill_stills(&self, now: Instant, available: usize) -> Vec<Job> {
        let mut state = locked(&self.state);
        let pending = std::mem::take(&mut state.kill_stills_pending);
        let mut jobs = Vec::new();
        for mut still in pending {
            let elapsed = now.saturating_duration_since(still.started);
            if elapsed >= KILL_REFERENCE_DEADLINE {
                audit_kill_still(
                    &still.request,
                    elapsed,
                    "failed",
                    "canonical_reference_timeout",
                );
            } else if jobs.len() < available {
                match still.resolve(&state) {
                    Some(references) => jobs.push(Job::KillShot { still, references }),
                    None => state.kill_stills_pending.push(still),
                }
            } else {
                state.kill_stills_pending.push(still);
            }
        }
        jobs
    }

    async fn render_kill_still(
        &self,
        request: &kill_shot::KillShotRequest,
        keys: &[ImageKey; 3],
    ) -> Result<(Vec<u8>, String), ImageFailure> {
        let mut references = Vec::with_capacity(3);
        for key in keys {
            match self.imagery.image(key.as_str()).await {
                Lookup::Image { bytes, format } => {
                    references.push(ReferenceImage::new(format, bytes))
                }
                Lookup::NotReady | Lookup::Unregistered => {
                    return Err(ImageFailure::InvalidRequest);
                }
            }
        }
        let image_request = kill_still_request(request, self.rating, references);
        let key = self.imagery.generate(&request.id, image_request).await?;
        match self.imagery.image(key.as_str()).await {
            Lookup::Image { bytes, .. } => Ok((bytes.to_vec(), key.path())),
            Lookup::NotReady | Lookup::Unregistered => Err(ImageFailure::InvalidRequest),
        }
    }

    /// The art the stage shows for this character: the full-body twin of the cut-out where there
    /// is one (every non-player character stands with their feet on screen), otherwise the
    /// cut-out. The expression is made from that art so it replaces it in place, floor contact
    /// and framing included.
    async fn load_figure(&self, sprite: &str) -> Result<ReferenceImage, ImageFailure> {
        let stem = sprite.rsplit('/').next().unwrap_or(sprite);
        if let Some(name) = stem.strip_prefix("sprite-")
            && let Ok(figure) = self.load(format!("figure-{name}")).await
        {
            return Ok(figure);
        }
        self.load(sprite.to_owned()).await
    }

    /// The source a health variant is made from: a hero's live picture (named by its served
    /// path) is read back from the image cache, so the variant is made from the hero
    /// themselves. Authored figure art remains available only for non-player cast members.
    async fn load_health_reference(
        &self,
        character: &Character,
    ) -> Result<ReferenceImage, ImageFailure> {
        if let Some(key) = character
            .art
            .strip_prefix("/gameplay/imagery/")
            .and_then(ImageKey::parse)
        {
            return match self.imagery.image(key.as_str()).await {
                Lookup::Image { bytes, format } => Ok(ReferenceImage::new(format, bytes)),
                Lookup::Unregistered | Lookup::NotReady => Err(ImageFailure::InvalidRequest),
            };
        }
        self.load_figure(&character.art).await
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
            .generate(
                &format!("character:{}", member.id),
                ImageRequest {
                    kind: ImageKind::CastExpression,
                    references: vec![sprite],
                    prompt,
                    output: OutputSpec::CUTOUT,
                },
            )
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

    /// A canonical hero revision owns its variants even when two revisions render equal bytes.
    fn hero_health_request(
        &self,
        character: &Character,
        sprite: ReferenceImage,
        tier: HealthTier,
    ) -> Result<ImageRequest, ImageFailure> {
        let mut request = self.health_request(sprite, character.facts, tier)?;
        if character.facts.role == CharacterRole::Hero
            && character.art.starts_with("/gameplay/imagery/")
        {
            request.prompt = request.prompt.with_canonical_binding(&character.art);
        }
        Ok(request)
    }

    async fn render_refresh(
        &self,
        place: &PlaceFacts,
        scene_asset: &str,
        look: Look,
    ) -> Result<ImageKey, ImageFailure> {
        let backdrop = self.load(scene_asset.to_owned()).await?;
        self.imagery
            .generate(
                &format!("place:refresh:{scene_asset}"),
                ImageRequest {
                    kind: ImageKind::SceneRefresh,
                    references: vec![backdrop],
                    prompt: scene_refresh(self.rating, place, look),
                    output: OutputSpec::BACKDROP,
                },
            )
            .await
    }

    /// Records a finished render. A scene refresh waits for a quiet moment before it lands.
    fn finish(&self, done: Done) {
        if let Done::Resource(completed) = done {
            self.imagery.hub().complete(completed);
            self.wake.send_modify(|sequence| *sequence += 1);
            return;
        }
        if let Done::KillShot {
            request,
            elapsed,
            result,
        } = done
        {
            match result {
                Ok((bytes, path)) => {
                    audit_kill_still(&request, elapsed, "still_generated", "photo_check_passed");
                    super::home_video::request_kill_shot(request, bytes, path);
                }
                Err(failure) => audit_kill_still(&request, elapsed, "failed", failure.label()),
            }
            return;
        }
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
            Done::Resource(_) | Done::Canonical { .. } | Done::KillShot { .. } => return,
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
            Done::Resource(_) | Done::Canonical { .. } | Done::KillShot { .. } => return,
            Done::Warm { id, tier, result } => {
                match result {
                    Ok(key) => state.book.learn(&id, tier, key),
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
                    Err(_) => {
                        state.failed_faces.insert((npc_id, face));
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
                super::dev_control::timeline::record_structured(
                    "gameplay.living_scene.face_requested",
                    "FACE requested",
                    serde_json::Map::from_iter([
                        ("scene.actor".to_owned(), serde_json::json!(id)),
                        (
                            "scene.expression".to_owned(),
                            serde_json::json!(face.expression.label()),
                        ),
                        (
                            "scene.tier".to_owned(),
                            serde_json::json!(face.tier.label()),
                        ),
                        ("status".to_owned(), serde_json::json!("requested")),
                    ]),
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
    /// The health variants to look for or make now. Looking is always allowed. Making is on
    /// demand: a variant is asked for only when a character is at its tier in this view, once,
    /// in a fight or out of one, and never ahead of time. A character shows the nearest cached
    /// variant meanwhile.
    fn health_jobs(&self, state: &mut State, stage: &Stage, facts: &SceneFacts) -> Vec<Job> {
        let hero_live: Vec<Option<ImageKey>> = stage
            .heroes
            .iter()
            .map(|hero| live_hero_key(state, hero))
            .collect();
        let characters = characters_of(stage, facts, &hero_live);
        let mut jobs = Vec::new();
        for character in &characters {
            if state.book.needs_peek(&character.id) {
                state.book.mark_peeked(&character.id);
                jobs.push(Job::Peek {
                    character: character.clone(),
                });
            }
        }
        // Nothing is asked of a provider that is off.
        if !self.imagery.is_on() {
            return jobs;
        }
        jobs.extend(warm_jobs(
            &mut state.book,
            stage,
            facts,
            &characters,
            &hero_live,
        ));
        jobs
    }
}

impl State {
    fn new(timing: Timing) -> Self {
        Self {
            canonical: canonical::References::default(),
            clip_reads: BTreeSet::new(),
            kill_stills_seen: kill_shot::ShownShots::new(),
            kill_stills_pending: Vec::new(),
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
            plate_covers: false,
            plate_wants: BTreeMap::new(),
            book: Book::default(),
            health_sprites: BTreeMap::new(),
            hero_faces: BTreeMap::new(),
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

fn kill_still_request(
    request: &kill_shot::KillShotRequest,
    rating: ContentRating,
    references: Vec<ReferenceImage>,
) -> ImageRequest {
    let plan = kill_shot::still_plan(request, rating);
    ImageRequest {
        kind: ImageKind::ScenePlate,
        references,
        prompt: super::imagery::AssembledPrompt::kill_shot(request, rating)
            .with_canonical_speaker_reference(1, "killer", &request.killer)
            .with_canonical_speaker_reference(2, "victim", &request.victim)
            .with_canonical_room_reference(3),
        output: OutputSpec {
            width: plan.width,
            height: plan.height,
            transparent: false,
        },
    }
}

fn audit_kill_still(
    request: &kill_shot::KillShotRequest,
    elapsed: Duration,
    status: &str,
    reason: &str,
) {
    actor::record_audit_line(
        "gameplay.kill_shot",
        kill_shot::audit_line(
            status,
            &request.killer,
            &request.victim,
            elapsed.as_millis(),
            reason,
        ),
    );
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

/// The dev tools' view of the on-demand depictions: what a subject resolves to, what is in
/// flight, cached or failed, and how to clear or regenerate one. Every call writes its `ART`
/// audit line through the shared audit path.
impl LivingScenes {
    /// The resource hub that owns every generated image's request, state and audit.
    pub(super) fn resource_hub(&self) -> Option<resources::Hub> {
        self.inner.as_ref().map(|inner| inner.imagery.hub().clone())
    }

    /// Resolves a subject's depiction for a role: the source kind (`live`, `reviewed` or
    /// `neutral`), the asset or live path, the generation status and the last error.
    pub(super) fn describe_depiction(
        &self,
        role: Role,
        request: &str,
        subject: &art_contract::Subject,
    ) -> Option<resources::DepictionReport> {
        let inner = self.inner.as_ref()?;
        let plan = art_contract::plan(role, request);
        let (live, status) = match subject.kind {
            // The guard must drop before the status read takes the same lock: a tuple keeps it
            // alive to the end of the expression and the thread deadlocks against itself.
            art_contract::SubjectKind::Character => {
                let live = locked(&inner.state).canonical.npc(&subject.id);
                (live, inner.canonical_npc_status(&subject.id))
            }
            art_contract::SubjectKind::Place => (
                inner.canonical_room_key(&subject.id),
                inner.canonical_room_status(&subject.id),
            ),
        };
        use canonical::ReferenceStatus;
        let (status, last_error) = match status {
            ReferenceStatus::Missing => ("none", String::new()),
            ReferenceStatus::Pending => ("pending", String::new()),
            ReferenceStatus::Ready => ("ready", String::new()),
            ReferenceStatus::Failed(failure) => ("failed", failure.label().to_owned()),
        };
        let (source, asset) = match (&live, plan.reviewed) {
            (Some(key), _) => ("live", key.path()),
            (None, Some(photo)) => (
                "reviewed",
                format!("{}/{photo}", art_contract::GENERATED_ROUTE),
            ),
            (None, None) => ("neutral", String::new()),
        };
        let report = resources::DepictionReport {
            subject: subject.key(),
            role,
            source,
            asset,
            status,
            last_error,
        };
        actor::record_audit_line(
            "gameplay.art",
            format!(
                "ART resolve subject={} role={role:?} source={} status={} asset={} error={}",
                report.subject, report.source, report.status, report.asset, report.last_error
            ),
        );
        Some(report)
    }

    /// Every generated resource the hub knows, optionally of one kind: in flight, cached and
    /// failed.
    pub(super) fn list_resources(
        &self,
        kind: Option<resources::ResourceKind>,
    ) -> Vec<resources::ResourceRow> {
        self.resource_hub()
            .map(|hub| hub.list(kind))
            .unwrap_or_default()
    }

    /// Clears a subject's cached or failed depiction so it is made again on the next need
    /// (hub entries and the canonical admission together). Returns how many were cleared.
    pub(super) fn clear_depiction(&self, subject: &art_contract::Subject) -> usize {
        let Some(inner) = self.inner.as_ref() else {
            return 0;
        };
        let cleared = inner
            .imagery
            .hub()
            .invalidate(resources::Invalidate::Subject(&subject.key()));
        let canonical = subject.kind == art_contract::SubjectKind::Character
            && locked(&inner.state).canonical.clear_npc(&subject.id);
        actor::record_audit_line(
            "gameplay.art",
            format!(
                "ART clear subject={} entries={cleared} canonical={canonical}",
                subject.key()
            ),
        );
        cleared + usize::from(canonical)
    }

    /// Requests a subject's depiction again now (bounded and idempotent: nothing is requested
    /// while one is already queued or in flight). Returns whether a request was admitted.
    pub(super) fn regenerate_depiction(&self, subject: &art_contract::Subject) -> bool {
        let Some(inner) = self.inner.as_ref() else {
            return false;
        };
        if subject.kind != art_contract::SubjectKind::Character {
            return false;
        }
        self.clear_depiction(subject);
        let mut state = locked(&inner.state);
        let member = state
            .facts
            .as_ref()
            .and_then(|facts| facts.members.iter().find(|member| member.id == subject.id))
            .cloned();
        let admitted = member.is_some_and(|member| {
            state.canonical.admit_cast(&member);
            true
        });
        actor::record_audit_line(
            "gameplay.art",
            format!(
                "ART request subject={} role={:?} source=live admitted={admitted}",
                subject.key(),
                Role::CharacterToken
            ),
        );
        admitted
    }
}

#[cfg(test)]
#[path = "living_scenes_tests.rs"]
mod tests;
