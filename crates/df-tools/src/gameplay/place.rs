//! Places beyond the first room: exits, created places, and where the party stands.
//!
//! A scene model lists its exits (`SceneWayOut`). Leaving through one moves the whole party to the
//! place the exit leads to. A place the run already has (the beat's own place, or one created
//! earlier) is returned to; a place the run does not have yet is created once by the narrative
//! engine (`place_creation`) and kept. Both are plain journal entries of the session checkpoint,
//! so a replay, a reconnect and a return to a place all read the same committed record:
//!
//! * `PlaceRecord` is a created place, stored whole in the entry that created it.
//! * `PlaceMove` is one committed change of place (from, to, the exit used, the beat it happened
//!   in, and the created place on its first use).
//! * `overlay` is the only place the rest of the game learns where the party stands: the beat's
//!   scene with the created place's title, narration, cast and scene model in its stead.
//!
//! The journal entry has its own policy label, so no other reader of the scene journal sees it.
//! A move belongs to the beat it happened in: when the story moves to another beat the beat's own
//! scene is shown again.

use std::collections::BTreeMap;

use df_model::checkpoint::{Checkpoint, GameState};
use df_render::art_contract;
use df_session::submission::RepositoryError;
use df_types::{OperationId, SessionRevision};
use serde::{Deserialize, Serialize};

use super::{
    adventure::{
        scene_model::{
            SceneItemProvenance, SceneItemProvenanceKind, SceneKnowledge, SceneModel, ScenePerson,
            SceneThing, SceneWayOut, StartingAttitude,
        },
        setting::{SceneSetting, SettingKind},
    },
    pack::{self, Age, CastRole, FindingSkill, Gender, Scene, SceneCastMember},
};

/// The policy label and semantic prefix of a place-move journal entry.
pub(in crate::gameplay) const POLICY: &str = "place-move-1";
const PREFIX: &str = "place-move-1:";
/// The `beyond_place_id` of an exit whose destination the content does not name.
pub(in crate::gameplay) const UNNAMED: &str = "none";
/// Places one run may create. A full run refuses a new place and keeps the party where it is.
pub(in crate::gameplay) const MAXIMUM_CREATED_PLACES: usize = 8;
/// The exit every created place has back to the place it was entered from.
pub(in crate::gameplay) const WAY_BACK: &str = "way-back";
/// The exit name of a development move that goes straight to a place the run has.
pub(in crate::gameplay) const DEV_EXIT: &str = "dev";

/// The largest serialized journal entry. The checkpoint codec keeps every stored text to 4,096
/// bytes, so a place is stored as the compact answer that made it and expanded into its scene
/// model and cast each time it is read. The expansion is deterministic.
pub(in crate::gameplay) const MAXIMUM_ENTRY_BYTES: usize = 3_950;
/// The largest serialized place inside an entry.
pub(in crate::gameplay) const MAXIMUM_PLACE_BYTES: usize = 3_150;

/// A created place: durable world state, stored whole in the journal entry that created it.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct PlaceRecord {
    pub(in crate::gameplay) place_id: String,
    /// The place the party came from, and the conditions it was in: the new place shares its
    /// hour and weather and has a way back.
    pub(in crate::gameplay) origin: PlaceOrigin,
    pub(in crate::gameplay) name: String,
    pub(in crate::gameplay) short: String,
    pub(in crate::gameplay) kind: SettingKind,
    /// What the TV and the table are told when the party arrives.
    pub(in crate::gameplay) narration: String,
    /// The person who speaks first on arrival (a name from `people`).
    pub(in crate::gameplay) speaker: String,
    pub(in crate::gameplay) greeting: String,
    pub(in crate::gameplay) exits: Vec<ExitDraft>,
    pub(in crate::gameplay) people: Vec<PersonDraft>,
    pub(in crate::gameplay) things: Vec<ThingDraft>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct PlaceOrigin {
    pub(in crate::gameplay) id: String,
    pub(in crate::gameplay) name: String,
    pub(in crate::gameplay) time_of_day: String,
    pub(in crate::gameplay) weather: String,
}

/// A way on from a created place. Its destination is not named, so the first move through it
/// creates the place beyond.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct ExitDraft {
    pub(in crate::gameplay) name: String,
    pub(in crate::gameplay) position: String,
    pub(in crate::gameplay) beyond: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct PersonDraft {
    pub(in crate::gameplay) name: String,
    pub(in crate::gameplay) role: String,
    pub(in crate::gameplay) description: String,
    pub(in crate::gameplay) position: String,
    pub(in crate::gameplay) gender: String,
    pub(in crate::gameplay) wants: String,
    pub(in crate::gameplay) attitude: String,
    pub(in crate::gameplay) knowledge: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct ThingDraft {
    pub(in crate::gameplay) name: String,
    pub(in crate::gameplay) detail: String,
    pub(in crate::gameplay) position: String,
    pub(in crate::gameplay) portable: bool,
}

/// A place expanded into what the rest of the game reads: its scene model and its cast.
pub(in crate::gameplay) struct Expanded {
    pub(in crate::gameplay) model: SceneModel,
    pub(in crate::gameplay) cast: Vec<SceneCastMember>,
}

fn within(value: &str, maximum: usize) -> bool {
    let length = value.trim().chars().count();
    length > 0 && length <= maximum && !value.chars().any(char::is_control)
}

/// `text` as an id token: ASCII letters and digits in lowercase, other runs made one `-`.
pub(in crate::gameplay) fn slug(text: &str) -> String {
    let mut slug = String::new();
    let mut gap = false;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            gap = true;
        }
    }
    slug.chars().take(32).collect()
}

fn sentences(text: &str) -> Vec<&str> {
    text.split(['.', '!', '?'])
        .map(str::trim)
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

/// `text` cut at a word to at most `maximum` characters.
fn cut(text: &str, maximum: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= maximum {
        return text.to_owned();
    }
    let kept: String = text.chars().take(maximum).collect();
    kept.rsplit_once(' ')
        .map_or_else(|| kept.clone(), |(head, _)| head.to_owned())
}

impl PlaceRecord {
    /// The id prefix of everything in this place.
    fn prefix(&self) -> String {
        slug(self.place_id.trim_start_matches("place-"))
    }

    /// The serialized size of the place.
    pub(in crate::gameplay) fn stored_bytes(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len())
    }

    /// The scene model and the cast this place stands for. Deterministic: the same record always
    /// gives the same ids, aliases and descriptions.
    pub(in crate::gameplay) fn expand(&self) -> Result<Expanded, PlaceRefusal> {
        let prefix = self.prefix();
        let mut people = Vec::with_capacity(self.people.len());
        let mut cast = Vec::with_capacity(self.people.len());
        for person in &self.people {
            let id = format!("{prefix}-{}", slug(&person.name));
            let attitude = match person.attitude.as_str() {
                "friendly" => StartingAttitude::Friendly,
                "indifferent" => StartingAttitude::Indifferent,
                "hostile" => StartingAttitude::Hostile,
                _ => return Err(PlaceRefusal::InvalidPlace),
            };
            let gender = Gender::from_table(&person.gender).ok_or(PlaceRefusal::InvalidPlace)?;
            let name = person.name.trim().to_owned();
            let role = person.role.trim().to_owned();
            people.push(ScenePerson {
                id: id.clone(),
                name: name.clone(),
                role: role.clone(),
                description: person.description.trim().to_owned(),
                relation_to_speaker: cut(&format!("a local at {}", self.name.trim()), 100),
                position: person.position.trim().to_owned(),
                aliases: if role == name {
                    vec![name.clone()]
                } else {
                    vec![name.clone(), role]
                },
                is_background: false,
                // Rewritten to the beat scene's key when the place is shown (`scene_of`).
                present_in: vec!["opening".to_owned()],
                attitude,
                attitudes: Default::default(),
                knowledge: person
                    .knowledge
                    .iter()
                    .enumerate()
                    .map(|(number, statement)| SceneKnowledge {
                        fact_id: format!("{id}:public:{number}"),
                        statement: statement.trim().to_owned(),
                    })
                    .collect(),
                wants: person.wants.trim().to_owned(),
                coin_cp: 0,
                passive_perception: 10,
            });
            cast.push(SceneCastMember {
                id,
                name,
                role: CastRole::Bystander,
                sprite: art_contract::CANONICAL_PHOTO.to_owned(),
                portrait: art_contract::CANONICAL_PHOTO.to_owned(),
                voice: "an even, plain adult voice".to_owned(),
                gender: Some(gender),
                age: Some(Age::Adult),
                stat_block: None,
            });
        }
        let things = self
            .things
            .iter()
            .map(|thing| {
                let id = format!("{prefix}-{}", slug(&thing.name));
                let lines = sentences(&thing.detail);
                SceneThing {
                    id: id.clone(),
                    name: thing.name.trim().to_owned(),
                    short: thing.name.trim().to_owned(),
                    detail: thing.detail.trim().to_owned(),
                    picture_description: cut(lines.first().copied().unwrap_or_default(), 120),
                    closer_look: cut(lines.last().copied().unwrap_or_default(), 180),
                    skill: FindingSkill::Investigation,
                    dc: 10,
                    finding_id: None,
                    clue_fact_id: None,
                    portable: thing.portable,
                    value_cp: None,
                    use_effect: "It can be handled and examined.".to_owned(),
                    position: thing.position.trim().to_owned(),
                    owner_id: None,
                    in_use: false,
                    in_scene: true,
                    publicly_visible: true,
                    holder_id: None,
                    open: false,
                    lit: false,
                    broken: false,
                    searched_by: Vec::new(),
                    aliases: vec![thing.name.trim().to_owned()],
                    provenance: SceneItemProvenance {
                        kind: SceneItemProvenanceKind::AuthoredLocation,
                        source_id: id,
                        seller_id: None,
                    },
                }
            })
            .collect();
        let mut ways_out = vec![way_back(&self.origin)];
        ways_out.extend(self.exits.iter().map(|exit| SceneWayOut {
            id: slug(&exit.name),
            name: exit.name.trim().to_owned(),
            aliases: vec![exit.name.trim().to_lowercase()],
            position: exit.position.trim().to_owned(),
            beyond_place_id: UNNAMED.to_owned(),
            beyond: exit.beyond.trim().to_owned(),
            open: true,
        }));
        Ok(Expanded {
            model: SceneModel {
                inventory_version: 1,
                time_of_day: self.origin.time_of_day.clone(),
                weather: self.origin.weather.clone(),
                people,
                things,
                ways_out,
                offers: Vec::new(),
                services: Vec::new(),
                witnessed_thefts: Vec::new(),
            },
            cast,
        })
    }

    /// Checks everything a place must satisfy: bounded fields, unique ids, a speaker who is in
    /// the place, a scene model the game accepts and a size the journal can hold.
    pub(in crate::gameplay) fn validate(&self) -> Result<(), PlaceRefusal> {
        let invalid = PlaceRefusal::InvalidPlace;
        if !self.place_id.starts_with("place-")
            || !within(&self.place_id, 96)
            || !within(&self.name, 40)
            || !within(&self.short, 80)
            || !within(&self.narration, 260)
            || !within(&self.greeting, 120)
            || !within(&self.origin.id, 96)
            || !within(&self.origin.name, 64)
            || !within(&self.origin.time_of_day, 32)
            || !within(&self.origin.weather, 72)
            || self.exits.len() > 2
            || !(2..=3).contains(&self.people.len())
            || !(4..=5).contains(&self.things.len())
        {
            return Err(invalid);
        }
        let prefix = self.prefix();
        let mut ids: Vec<String> = Vec::new();
        for person in &self.people {
            if !within(&person.name, 32)
                || !within(&person.role, 32)
                || !within(&person.description, 110)
                || !within(&person.position, 32)
                || !within(&person.wants, 60)
                || person.knowledge.len() != 3
                || person.knowledge.iter().any(|line| !within(line, 80))
                || slug(&person.name).is_empty()
            {
                return Err(invalid);
            }
            let id = format!("{prefix}-{}", slug(&person.name));
            if ids.contains(&id) {
                return Err(invalid);
            }
            ids.push(id);
        }
        for thing in &self.things {
            if !within(&thing.name, 32)
                || !within(&thing.detail, 220)
                || !(2..=4).contains(&sentences(&thing.detail).len())
                || !within(&thing.position, 32)
                || slug(&thing.name).is_empty()
            {
                return Err(invalid);
            }
            let id = format!("{prefix}-{}", slug(&thing.name));
            if ids.contains(&id) {
                return Err(invalid);
            }
            ids.push(id);
        }
        let mut exits: Vec<String> = vec![WAY_BACK.to_owned()];
        for exit in &self.exits {
            let id = slug(&exit.name);
            if !within(&exit.name, 32)
                || !within(&exit.position, 32)
                || !within(&exit.beyond, 100)
                || id.is_empty()
                || exits.contains(&id)
                || ids.contains(&id)
            {
                return Err(invalid);
            }
            exits.push(id);
        }
        if !self
            .people
            .iter()
            .any(|person| person.name.trim() == self.speaker.trim())
        {
            return Err(invalid);
        }
        self.expand()?
            .model
            .validate_structure()
            .map_err(|_| PlaceRefusal::InvalidPlace)?;
        if self.stored_bytes() > MAXIMUM_PLACE_BYTES {
            return Err(PlaceRefusal::TooLarge);
        }
        Ok(())
    }
}

/// One committed change of place.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct PlaceMove {
    /// The place the party left.
    pub(in crate::gameplay) from: String,
    /// The place the party entered.
    pub(in crate::gameplay) to: String,
    /// The exit of `from` the party went through; `dev` for a development move.
    pub(in crate::gameplay) exit: String,
    /// The durable id of the beat the move happened in.
    pub(in crate::gameplay) beat: String,
    /// The place, on the first move into it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::gameplay) created: Option<PlaceRecord>,
}

/// A journal entry: the move, the hero whose line caused it and the line the table is told.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct PlaceEntry {
    pub(in crate::gameplay) hero: String,
    pub(in crate::gameplay) line: String,
    #[serde(rename = "move")]
    pub(in crate::gameplay) change: PlaceMove,
}

impl PlaceEntry {
    pub(in crate::gameplay) fn encode(&self) -> Result<String, RepositoryError> {
        let text = serde_json::to_string(self).map_err(|_| RepositoryError::InvalidCandidate)?;
        Ok(format!("{PREFIX}{text}"))
    }
}

/// Every place move in the journal, oldest first, with the revision that committed it.
pub(in crate::gameplay) fn entries(
    state: &GameState,
) -> Result<Vec<(SessionRevision, PlaceEntry)>, RepositoryError> {
    let mut found = Vec::new();
    for decision in &state.decisions {
        if decision.source_policy.as_str() != POLICY {
            continue;
        }
        let entry = decision
            .semantic_output
            .as_deref()
            .and_then(|text| text.strip_prefix(PREFIX))
            .and_then(|text| serde_json::from_str::<PlaceEntry>(text).ok())
            .ok_or(RepositoryError::InvalidCandidate)?;
        found.push((decision.revision, entry));
    }
    Ok(found)
}

/// The places this run has created, by id, in creation order.
pub(in crate::gameplay) fn created(
    state: &GameState,
) -> Result<BTreeMap<String, PlaceRecord>, RepositoryError> {
    let mut places = BTreeMap::new();
    for (_, entry) in entries(state)? {
        if let Some(place) = entry.change.created {
            places.insert(place.place_id.clone(), place);
        }
    }
    Ok(places)
}

/// The ids of every place the story itself owns: the settings of the pack's scenes.
pub(in crate::gameplay) fn story_places() -> Vec<(String, String)> {
    pack::with_content(|content| {
        let mut places: Vec<(String, String)> = Vec::new();
        for scene in content.scenes.iter().chain(
            content
                .beat_script
                .as_ref()
                .into_iter()
                .flat_map(|script| script.scenes.values()),
        ) {
            if let Some(setting) = &scene.setting
                && !places.iter().any(|(id, _)| *id == setting.place_id)
            {
                places.push((setting.place_id.clone(), setting.name.clone()));
            }
        }
        places
    })
}

/// Where an exit leads, as far as this run's world knows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Destination {
    /// The place the beat is set in: the beat's own scene, with no overlay.
    Home,
    /// A place this run created earlier.
    Created(Box<PlaceRecord>),
    /// A place this run does not have yet. `named` is the id the content gave the exit.
    Create { named: Option<String> },
    /// A place the story itself owns; the story's beats move the party there, not an exit.
    Story { place_id: String },
}

/// The destination of `exit` taken from `here`. An exit names its place in the content, or has been
/// bound to one by an earlier move through it; either way the same exit always leads to the same
/// place.
pub(in crate::gameplay) fn destination(
    state: &GameState,
    home: &str,
    here: &str,
    exit: &SceneWayOut,
) -> Result<Destination, RepositoryError> {
    let target = if exit.beyond_place_id != UNNAMED {
        Some(exit.beyond_place_id.clone())
    } else {
        entries(state)?
            .into_iter()
            .rev()
            .find(|(_, entry)| entry.change.from == here && entry.change.exit == exit.id)
            .map(|(_, entry)| entry.change.to)
    };
    resolve(state, home, target)
}

/// The destination for a place id the caller already holds (a development move by place).
pub(in crate::gameplay) fn destination_of(
    state: &GameState,
    home: &str,
    place_id: &str,
) -> Result<Destination, RepositoryError> {
    resolve(state, home, Some(place_id.to_owned()))
}

fn resolve(
    state: &GameState,
    home: &str,
    target: Option<String>,
) -> Result<Destination, RepositoryError> {
    let Some(target) = target else {
        return Ok(Destination::Create { named: None });
    };
    if target == home {
        return Ok(Destination::Home);
    }
    if let Some(place) = created(state)?.remove(&target) {
        return Ok(Destination::Created(Box::new(place)));
    }
    if story_places().iter().any(|(id, _)| *id == target) {
        return Ok(Destination::Story { place_id: target });
    }
    Ok(Destination::Create {
        named: Some(target),
    })
}

/// A place id from a name: `place-` and the name's ASCII letters and digits, joined by `-`, made
/// unique against the ids in use.
pub(in crate::gameplay) fn place_id(name: &str, taken: &[String]) -> String {
    let mut slug = String::new();
    let mut gap = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            gap = true;
        }
    }
    let slug: String = slug.chars().take(48).collect();
    let base = if slug.is_empty() {
        "place-created".to_owned()
    } else {
        format!("place-{slug}")
    };
    let mut candidate = base.clone();
    let mut number = 2;
    while taken.contains(&candidate) {
        candidate = format!("{base}-{number}");
        number += 1;
    }
    candidate
}

/// The place the party stands in when it has left the beat's own place: the created place of the
/// last move made in this beat. `None` means the beat's own scene is shown.
pub(in crate::gameplay) fn standing(
    state: &GameState,
    beat: &str,
    home: &str,
) -> Result<Option<PlaceRecord>, RepositoryError> {
    let mut last = None;
    for (_, entry) in entries(state)? {
        if entry.change.beat == beat {
            last = Some(entry.change.to);
        }
    }
    let Some(to) = last else {
        return Ok(None);
    };
    if to == home {
        return Ok(None);
    }
    created(state)?
        .remove(&to)
        .map(Some)
        .ok_or(RepositoryError::InvalidCandidate)
}

/// Where the party stands: the beat, the place it is in, and the place the beat's own scene is
/// set in (the place of the first move made in this beat, or the place the party is in).
pub(in crate::gameplay) struct Whereabouts {
    pub(in crate::gameplay) beat: String,
    pub(in crate::gameplay) here: String,
    pub(in crate::gameplay) home: String,
}

/// Where the party stands, given the scene it is shown (the beat's scene with any overlay).
pub(in crate::gameplay) fn whereabouts(
    state: &GameState,
    scene: &Scene,
) -> Result<Whereabouts, RepositoryError> {
    let beat = super::beat_script::with_script(|script| {
        script.current(state).map(|beat| beat.id.clone())
    })?;
    let here = home_id(scene)?;
    let home = entries(state)?
        .into_iter()
        .find(|(_, entry)| entry.change.beat == beat)
        .map_or_else(|| here.clone(), |(_, entry)| entry.change.from);
    Ok(Whereabouts { beat, here, home })
}

/// The name of a place the run knows: one the story owns, or one it created.
pub(in crate::gameplay) fn name_of(
    state: &GameState,
    place_id: &str,
) -> Result<String, RepositoryError> {
    if let Some(place) = created(state)?.remove(place_id) {
        return Ok(place.name);
    }
    story_places()
        .into_iter()
        .find(|(id, _)| id == place_id)
        .map(|(_, name)| name)
        .ok_or(RepositoryError::InvalidCandidate)
}

/// The id of the place a beat's own scene is set in.
pub(in crate::gameplay) fn home_id(base: &Scene) -> Result<String, RepositoryError> {
    base.setting
        .as_ref()
        .map(|setting| setting.place_id.clone())
        .ok_or(RepositoryError::InvalidCandidate)
}

/// The beat's scene as the party sees it now: the created place's own scene when the party stands
/// in one, and the beat's scene unchanged otherwise.
pub(in crate::gameplay) fn overlay(
    state: &GameState,
    beat: &str,
    base: Scene,
) -> Result<Scene, RepositoryError> {
    let Ok(home) = home_id(&base) else {
        return Ok(base);
    };
    let Some(place) = standing(state, beat, &home)? else {
        return Ok(base);
    };
    scene_of(&place, &base)
}

/// A created place as a scene: it keeps the beat scene's key (the beat is unchanged) and takes
/// everything else from the place.
pub(in crate::gameplay) fn scene_of(
    place: &PlaceRecord,
    base: &Scene,
) -> Result<Scene, RepositoryError> {
    let Expanded { mut model, cast } = place
        .expand()
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let key_label = base.key.source_label().replace('-', "_");
    for person in &mut model.people {
        person.present_in = vec![key_label.clone()];
    }
    let mut scene = base.clone();
    scene.title = place.name.clone();
    scene.narration = place.narration.clone();
    scene.rules_note = String::new();
    scene.cast = cast;
    scene.narration_rested = None;
    scene.speaker = place.speaker.clone();
    scene.dialogue = place.greeting.clone();
    scene.asset = art_contract::ROOM_PHOTO.to_owned();
    scene.speaker_portrait = String::new();
    scene.findings = Vec::new();
    scene.source_variant_id = None;
    scene.setting = Some(SceneSetting {
        place_id: place.place_id.clone(),
        name: place.name.clone(),
        short: place.short.clone(),
        kind: place.kind,
        exterior: place.kind.is_exterior(),
        art_key: place.place_id.trim_start_matches("place-").to_owned(),
        model: Some(model),
    });
    Ok(scene)
}

/// The way back every created place has to the place it was entered from.
fn way_back(origin: &PlaceOrigin) -> SceneWayOut {
    SceneWayOut {
        id: WAY_BACK.to_owned(),
        name: format!("way back to {}", origin.name),
        aliases: vec![
            origin.name.to_lowercase(),
            "way back".to_owned(),
            "back".to_owned(),
        ],
        position: "behind the party".to_owned(),
        beyond_place_id: origin.id.clone(),
        beyond: cut(&format!("back to {}", origin.name), 180),
        open: true,
    }
}

/// What the table is told when the party moves, from the committed move alone.
pub(in crate::gameplay) fn arrival_line(hero: &str, exit_name: &str, place_name: &str) -> String {
    format!("{hero} leads the party through the {exit_name} to {place_name}.")
}

/// A move the owner has checked: the name of the exit used and of the place entered.
pub(in crate::gameplay) struct Admitted {
    pub(in crate::gameplay) exit_name: String,
    pub(in crate::gameplay) to_name: String,
}

/// Why a move could not be admitted: a rule refused it, or the checkpoint could not be read.
pub(in crate::gameplay) enum PlaceFailure {
    Refused(PlaceRefusal),
    State(RepositoryError),
}

impl From<PlaceRefusal> for PlaceFailure {
    fn from(reason: PlaceRefusal) -> Self {
        Self::Refused(reason)
    }
}

impl From<RepositoryError> for PlaceFailure {
    fn from(error: RepositoryError) -> Self {
        Self::State(error)
    }
}

/// The owner's checks of a proposed move against its own current state: the exit exists and is
/// open, the destination is exactly where the exit leads (a place the run has, or one new place
/// that is not yet in the journal and fits the run), and nothing but the story's own beats own
/// the destination. `exit` is the id of an exit of the place `party` stands in, or `dev` for a
/// development move straight to a place the run has.
pub(in crate::gameplay) fn admit(
    state: &GameState,
    model: &SceneModel,
    party: &Whereabouts,
    exit: &str,
    to: &str,
    created_place: Option<&PlaceRecord>,
) -> Result<Admitted, PlaceFailure> {
    let exit_name = if exit == DEV_EXIT {
        // A development move goes straight to a place the run already has.
        match destination_of(state, &party.home, to)? {
            Destination::Home if to == party.home && created_place.is_none() => {}
            Destination::Created(known) if known.place_id == to && created_place.is_none() => {}
            Destination::Home | Destination::Created(_) => {
                return Err(PlaceRefusal::InvalidPlace.into());
            }
            Destination::Create { .. } => return Err(PlaceRefusal::NoExit.into()),
            Destination::Story { .. } => return Err(PlaceRefusal::StoryPlace.into()),
        }
        "a development order".to_owned()
    } else {
        let way = model
            .ways_out
            .iter()
            .find(|way| way.id == exit)
            .ok_or(PlaceRefusal::NoExit)?;
        if !way.open {
            return Err(PlaceRefusal::Closed.into());
        }
        match destination(state, &party.home, &party.here, way)? {
            Destination::Home if to == party.home && created_place.is_none() => {}
            Destination::Created(known) if known.place_id == to && created_place.is_none() => {}
            Destination::Home | Destination::Created(_) => {
                return Err(PlaceRefusal::InvalidPlace.into());
            }
            Destination::Story { .. } => return Err(PlaceRefusal::StoryPlace.into()),
            Destination::Create { named } => {
                let made = created_place.ok_or(PlaceRefusal::InvalidPlace)?;
                let known = created(state)?;
                if made.place_id != to
                    || named.as_ref().is_some_and(|id| *id != made.place_id)
                    || known.contains_key(&made.place_id)
                    || made.place_id == party.home
                    || story_places().iter().any(|(id, _)| *id == made.place_id)
                {
                    return Err(PlaceRefusal::AlreadyKnown.into());
                }
                if known.len() >= MAXIMUM_CREATED_PLACES {
                    return Err(PlaceRefusal::RunFull.into());
                }
                made.validate()?;
            }
        }
        way.name.clone()
    };
    let to_name = match created_place {
        Some(made) => made.name.clone(),
        None => name_of(state, to)?,
    };
    Ok(Admitted { exit_name, to_name })
}

/// A move that passed every check, as a candidate checkpoint the owner can commit.
pub(in crate::gameplay) struct Planned {
    pub(in crate::gameplay) candidate: Checkpoint,
    pub(in crate::gameplay) entry: PlaceEntry,
    pub(in crate::gameplay) to_name: String,
    /// What the screens show on arrival, read back from the candidate itself.
    pub(in crate::gameplay) arrival: String,
}

/// Checks a proposed move against `current` and builds the candidate that commits it. The hero is
/// the one whose line (or order) it is. Nothing is committed here; a refusal changes nothing.
pub(in crate::gameplay) fn plan(
    current: &Checkpoint,
    hero: &str,
    operation: OperationId,
    from: &str,
    exit: &str,
    to: &str,
    created_place: Option<&PlaceRecord>,
) -> Result<Planned, PlaceFailure> {
    let state = current.state();
    if state
        .decisions
        .iter()
        .any(|decision| decision.operation == operation)
    {
        return Err(PlaceRefusal::Duplicate.into());
    }
    let scene = super::beat_script_runtime::scene(state)?;
    let snapshot = super::journey::living_scene::snapshot(state, &scene)?
        .ok_or(RepositoryError::InvalidCandidate)?;
    let party = whereabouts(state, &scene)?;
    if from != party.here {
        return Err(PlaceRefusal::NotHere.into());
    }
    let admitted = admit(state, &snapshot.model, &party, exit, to, created_place)?;
    let entry = PlaceEntry {
        hero: hero.to_owned(),
        line: arrival_line(hero, &admitted.exit_name, &admitted.to_name),
        change: PlaceMove {
            from: from.to_owned(),
            to: to.to_owned(),
            exit: exit.to_owned(),
            beat: party.beat,
            created: created_place.cloned(),
        },
    };
    // The checkpoint codec keeps every stored text to 4,096 bytes.
    if entry.encode()?.len() > MAXIMUM_ENTRY_BYTES {
        return Err(PlaceRefusal::TooLarge.into());
    }
    let next = current
        .basis()
        .revision
        .next_sequence()
        .map_err(|_| PlaceRefusal::Refused)?;
    let candidate = super::journey::living_scene::commit_place_move(current, operation, &entry)
        .map_err(|_| PlaceRefusal::Refused)?;
    if candidate.basis().revision != next {
        return Err(PlaceRefusal::Refused.into());
    }
    // Honest by construction: the arrival the table hears is read back from the candidate, and a
    // candidate that would not stand in the destination is not a move.
    let arrived = super::beat_script_runtime::scene(candidate.state())?;
    if arrived
        .setting
        .as_ref()
        .map(|setting| setting.place_id.as_str())
        != Some(to)
    {
        return Err(PlaceRefusal::Refused.into());
    }
    Ok(Planned {
        candidate,
        entry,
        to_name: admitted.to_name,
        arrival: arrived.narration,
    })
}

/// Why a move was not made. The label goes in the `PLACE refused` audit line; it never carries
/// player words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PlaceRefusal {
    /// No exit of this place was chosen.
    NoExit,
    /// The exit is shut.
    Closed,
    /// The party is not in the place the move leaves (the table moved on meanwhile).
    NotHere,
    /// The destination belongs to the story's own beats.
    StoryPlace,
    /// The run has made as many places as it may.
    RunFull,
    /// The narrative engine could not make the place (provider off, late or invalid).
    CreationUnavailable,
    /// The created place broke a rule of a place.
    InvalidPlace,
    /// The created place does not fit the journal.
    TooLarge,
    /// The place is already known and may not be created again.
    AlreadyKnown,
    /// The checkpoint refused the candidate.
    Refused,
    /// This operation already changed the place.
    Duplicate,
}

impl PlaceRefusal {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::NoExit => "no_exit",
            Self::Closed => "exit_closed",
            Self::NotHere => "not_here",
            Self::StoryPlace => "story_place",
            Self::RunFull => "run_full",
            Self::CreationUnavailable => "creation_unavailable",
            Self::InvalidPlace => "invalid_place",
            Self::TooLarge => "too_large",
            Self::AlreadyKnown => "already_known",
            Self::Refused => "refused",
            Self::Duplicate => "duplicate",
        }
    }
}

// ---- Presentation copies --------------------------------------------------------------------
//
// The living scenes, the plates and the place music look a scene up by what the display shows
// (its title, picture and key). A created place is not in the pack, so the session owner
// publishes the scene it just projected, the same way it publishes scene models.

#[cfg(not(test))]
fn registry<T>(use_it: impl FnOnce(&mut BTreeMap<String, Scene>) -> T) -> T {
    static SCENES: std::sync::Mutex<BTreeMap<String, Scene>> =
        std::sync::Mutex::new(BTreeMap::new());
    use_it(
        &mut SCENES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

#[cfg(test)]
fn registry<T>(use_it: impl FnOnce(&mut BTreeMap<String, Scene>) -> T) -> T {
    thread_local! { static SCENES: std::cell::RefCell<BTreeMap<String, Scene>> = const { std::cell::RefCell::new(BTreeMap::new()) }; }
    SCENES.with(|scenes| use_it(&mut scenes.borrow_mut()))
}

/// Remembers the scene of a created place the party now stands in.
pub(in crate::gameplay) fn publish(scene: &Scene) {
    if let Some(setting) = &scene.setting {
        registry(|scenes| {
            scenes.insert(setting.place_id.clone(), scene.clone());
        });
    }
}

/// Forgets every published scene (a new adventure).
pub(in crate::gameplay) fn clear() {
    registry(BTreeMap::clear);
}

/// The published scene the display shows by this title, picture and key.
pub(in crate::gameplay) fn registered(
    title: &str,
    asset: &str,
    key: pack::SceneKey,
) -> Option<Scene> {
    let wanted = asset.trim_start_matches('/');
    registry(|scenes| {
        scenes
            .values()
            .find(|scene| {
                scene.key == key
                    && scene.title == title
                    && scene.asset.trim_start_matches('/') == wanted
            })
            .cloned()
    })
}

/// The setting of a published place.
pub(in crate::gameplay) fn registered_setting(place_id: &str) -> Option<SceneSetting> {
    registry(|scenes| scenes.get(place_id).and_then(|scene| scene.setting.clone()))
}

// ---- Audit lines ----------------------------------------------------------------------------

/// The one audit line of a committed change of place.
pub(in crate::gameplay) fn audit_changed(from: &str, to: &str, exit: &str, created: bool) {
    super::actor::record_audit_line(
        "gameplay.place",
        format!("PLACE change from={from} to={to} exit={exit} created={created}"),
    );
}

/// The audit line of a move that was not made. `to` is `none` when no destination was reached.
pub(in crate::gameplay) fn audit_refused(from: &str, to: &str, exit: &str, reason: &str) {
    super::actor::record_audit_line(
        "gameplay.place",
        format!("PLACE refused from={from} to={to} exit={exit} reason={reason}"),
    );
}

/// The audit line of Jev's choice among the exits for a hero's line.
pub(in crate::gameplay) fn audit_judged(from: &str, exit: &str, detail: &str) {
    super::actor::record_audit_line(
        "gameplay.place",
        format!("PLACE judged from={from} source=jev exit={exit}{detail}"),
    );
}
