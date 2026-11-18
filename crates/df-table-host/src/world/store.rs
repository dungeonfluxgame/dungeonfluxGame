//! The bible and the active place, read from the playground's `world` folder.
//!
//! The bible (`bible.json`) is read fresh on every call, so a designer's edit needs no rebuild and
//! no restart. A missing bible is an empty one; a file that does not read or is not a valid bible
//! is a typed error. The active place is `state.json`: `{"place": "<id>"}` for a place of the
//! bible, or `{"place": "<id>", "designed": {<place>}}` for a place designed in play (a move's
//! destination), which the state holds whole. It is written whole or not at all; a missing file
//! is no active place.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use df_content::world_bible::{Place, Thing, WorldBible, is_valid_id};
use serde::{Deserialize, Serialize};

use super::quest::QuestRecord;
use super::region::Region;
use super::request::WorldError;
use super::town::Town;
use super::visited::Visited;
use crate::blocking::WriteLock;
use crate::dev_http::BodyError;
use crate::io_cause::IoCause;
use crate::state::AppState;

/// The folder of the bible under the playground's assets.
pub const WORLD_DIRECTORY: &str = "world";
const BIBLE_FILE: &str = "bible.json";
const STATE_FILE: &str = "state.json";

/// Held around every write of `state.json`.
pub(super) static WORLD_WRITES: WriteLock = WriteLock::new();

/// The stored state of the world.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldState {
    #[serde(default)]
    place: Option<String>,
    /// The active place when it was designed in play rather than written in the bible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    designed: Option<Place>,
    /// The town the party is in, kept while places designed in play follow one another in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    town: Option<Town>,
    /// The places designed in play that the party has been to, oldest first, until a place of the
    /// bible is made the active one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    visited: Vec<Visited>,
    /// Every place designed in play the party has been to, by id, as it stood when it was left
    /// (things added, close-ups kept), so the party can go back to it; until a place of the bible
    /// is made the active one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    kept: Vec<Place>,
    /// The quests offered and taken (`quest`), oldest first; they stay across places and regions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    quests: Vec<QuestRecord>,
    /// The region the party is in (`region`); none before any crossing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    region: Option<Region>,
    /// What play has noted on things of the places designed in play (a door's state now, what a
    /// sign reads, what is inside), by thing id and kind of note: a note of a kind replaces the one
    /// before it, and each is told after the thing's own look (`read_active_place`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    notes: BTreeMap<String, BTreeMap<String, String>>,
}

/// The `world` folder of the host's playground.
pub(super) fn world_directory(app: &AppState) -> PathBuf {
    app.playground().files().assets().join(WORLD_DIRECTORY)
}

/// The bible in `directory`, read now. A missing file is an empty bible.
pub(super) fn read_bible(directory: &Path) -> Result<WorldBible, WorldError> {
    let text = match std::fs::read_to_string(directory.join(BIBLE_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorldBible::default());
        }
        Err(error) => {
            return Err(WorldError::BibleUnreadable {
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    WorldBible::from_json_str(&text).map_err(|cause| WorldError::BibleInvalid { cause })
}

/// The stored state in `directory`: the default when there is no state file.
fn read_state(directory: &Path) -> Result<WorldState, WorldError> {
    let text = match std::fs::read_to_string(directory.join(STATE_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorldState::default());
        }
        Err(error) => {
            return Err(WorldError::StateUnreadable {
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    serde_json::from_str(&text).map_err(|error| WorldError::StateInvalid {
        cause: BodyError::from_json(&error),
    })
}

/// The id of the active place in `directory`: none when there is no state file, or it names none.
pub(super) fn read_place(directory: &Path) -> Result<Option<String>, WorldError> {
    Ok(read_state(directory)?
        .place
        .filter(|place| is_valid_id(place)))
}

/// The active place in `directory` as `bible` has it, or as it was designed in play: none when
/// there is no active place, or the bible no longer holds it.
pub(super) fn read_active_place(
    directory: &Path,
    bible: &WorldBible,
) -> Result<Option<Place>, WorldError> {
    let state = read_state(directory)?;
    Ok(match state.designed {
        Some(designed) => Some(with_notes(designed, &state.notes)),
        None => state
            .place
            .filter(|place| is_valid_id(place))
            .and_then(|id| bible.place(&id).cloned()),
    })
}

/// `place` with each thing's notes told after its own look (a note its look has already, as a
/// thing carried to a pose plate's place has, is not told twice).
fn with_notes(mut place: Place, notes: &BTreeMap<String, BTreeMap<String, String>>) -> Place {
    for thing in &mut place.things {
        for note in notes.get(&thing.id).into_iter().flat_map(BTreeMap::values) {
            if !thing.look.contains(note.as_str()) {
                thing.look = format!("{} {note}", thing.look.trim_end());
            }
        }
    }
    place
}

/// Notes `text` on the thing `id` of the active place designed in play, as its note of kind
/// `label` (replacing the one before), and names it `name` when given. Answers the place's id.
pub(super) fn note_thing(
    directory: &Path,
    id: &str,
    label: &str,
    (text, name): (&str, Option<&str>),
) -> Result<String, WorldError> {
    let _writing = WORLD_WRITES.lock();
    let mut state = read_state(directory)?;
    let designed = state.designed.as_mut().ok_or(WorldError::NotDesigned)?;
    let thing = designed
        .things
        .iter_mut()
        .find(|thing| thing.id == id)
        .ok_or_else(|| WorldError::UnknownThing { id: id.to_owned() })?;
    if let Some(name) = name {
        name.clone_into(&mut thing.name);
    }
    let place = designed.id.clone();
    state
        .notes
        .entry(id.to_owned())
        .or_default()
        .insert(label.to_owned(), text.to_owned());
    write_state_locked(directory, &state)?;
    Ok(place)
}

/// Makes the bible's `place` the active one (none for no place), whole or not at all. The quests
/// taken stay with the world from one day to the next; the town, the places and the region end.
pub(super) fn write_place(directory: &Path, place: Option<&str>) -> Result<(), WorldError> {
    let _writing = WORLD_WRITES.lock();
    let quests = read_state(directory)?.quests;
    write_state_locked(
        directory,
        &WorldState {
            place: place.map(str::to_owned),
            designed: None,
            town: None,
            visited: Vec::new(),
            kept: Vec::new(),
            quests,
            region: None,
            notes: BTreeMap::new(),
        },
    )
}

/// Makes `designed`, a place designed in play, the active one, whole or not at all. The town the
/// party is in stays, and `feature` (when the design gave one) is kept with the place.
pub(super) fn write_designed(
    directory: &Path,
    designed: Place,
    feature: Option<String>,
) -> Result<(), WorldError> {
    let _writing = WORLD_WRITES.lock();
    let before = read_state(directory)?;
    let mut visited = before.visited;
    if let Some(feature) = feature {
        visited.push(Visited {
            name: designed.name.clone(),
            feature,
        });
    }
    write_state_locked(
        directory,
        &WorldState {
            place: Some(designed.id.clone()),
            designed: Some(designed),
            town: before.town,
            visited,
            kept: before.kept,
            quests: before.quests,
            region: before.region,
            notes: before.notes,
        },
    )
}

/// Makes the kept place designed in play `id` the active one again, as it was left, keeping the
/// town and the places visited; `None` when no such place is kept.
pub(super) fn revisit(directory: &Path, id: &str) -> Result<Option<Place>, WorldError> {
    let _writing = WORLD_WRITES.lock();
    let mut state = read_state(directory)?;
    let Some(place) = state.kept.iter().find(|place| place.id == id).cloned() else {
        return Ok(None);
    };
    state.place = Some(place.id.clone());
    state.designed = Some(place.clone());
    write_state_locked(directory, &state)?;
    Ok(Some(place))
}

/// The places designed in play the party can go back to, oldest first: every kept one but the
/// active place.
pub(super) fn read_kept(directory: &Path) -> Result<Vec<Place>, WorldError> {
    let state = read_state(directory)?;
    Ok(state
        .kept
        .into_iter()
        .filter(|place| state.place.as_deref() != Some(place.id.as_str()))
        .collect())
}

/// The places designed in play that the party has been to, oldest first.
pub(super) fn read_visited(directory: &Path) -> Result<Vec<Visited>, WorldError> {
    Ok(read_state(directory)?.visited)
}

/// The town the party is in, if there is one.
pub(super) fn read_town(directory: &Path) -> Result<Option<Town>, WorldError> {
    Ok(read_state(directory)?.town)
}

/// Keeps `town` as the one the party is in (none: the party is in no town), whole or not at all,
/// and answers the town it replaced.
pub(super) fn write_town(directory: &Path, town: Option<Town>) -> Result<Option<Town>, WorldError> {
    let _writing = WORLD_WRITES.lock();
    let mut state = read_state(directory)?;
    let before = std::mem::replace(&mut state.town, town);
    write_state_locked(directory, &state)?;
    Ok(before)
}

/// The quests of the world state, oldest first.
pub(super) fn read_quests(directory: &Path) -> Result<Vec<QuestRecord>, WorldError> {
    Ok(read_state(directory)?.quests)
}

/// Changes the quests with `change`, whole or not at all: a refusal writes nothing. Answers the
/// quests as written and what `change` answered.
pub(super) fn write_quests<T>(
    directory: &Path,
    change: impl FnOnce(&mut Vec<QuestRecord>) -> Result<T, WorldError>,
) -> Result<(Vec<QuestRecord>, T), WorldError> {
    let _writing = WORLD_WRITES.lock();
    let mut state = read_state(directory)?;
    let answered = change(&mut state.quests)?;
    write_state_locked(directory, &state)?;
    Ok((state.quests, answered))
}

/// The region the party is in, if any.
pub(super) fn read_region(directory: &Path) -> Result<Option<Region>, WorldError> {
    Ok(read_state(directory)?.region)
}

/// The party crosses into `region`: no place is active, and the town, the places visited and the
/// places kept to go back to end with the land left; the quests stay. Answers the region left.
pub(super) fn enter_region(directory: &Path, region: Region) -> Result<Option<Region>, WorldError> {
    let _writing = WORLD_WRITES.lock();
    let before = read_state(directory)?;
    write_state_locked(
        directory,
        &WorldState {
            place: None,
            designed: None,
            town: None,
            visited: Vec::new(),
            kept: Vec::new(),
            quests: before.quests,
            region: Some(region),
            notes: BTreeMap::new(),
        },
    )?;
    Ok(before.region)
}

/// Keeps `picture` as the close-up of the thing `item` at the active place, which must be a place
/// designed in play (a bible place is the designer's file, never written by the host).
pub(super) fn keep_thing_picture(
    directory: &Path,
    item: &str,
    picture: &str,
) -> Result<String, WorldError> {
    let _writing = WORLD_WRITES.lock();
    let mut state = read_state(directory)?;
    let designed = state.designed.as_mut().ok_or(WorldError::NotDesigned)?;
    let thing = designed
        .things
        .iter_mut()
        .find(|thing| thing.id == item)
        .ok_or_else(|| WorldError::UnknownThing {
            id: item.to_owned(),
        })?;
    thing.picture = Some(picture.to_owned());
    let place = designed.id.clone();
    write_state_locked(directory, &state)?;
    Ok(place)
}

/// Adds `things` to the active place, which must be a place designed in play, after checking the
/// whole place as a designed place is checked (`check_place`: ids in the grammar, none twice among
/// the place's things and the bible's items, a name and a look that say something). All of them
/// are added, or none. Answers the place's id.
pub(super) fn add_things(
    directory: &Path,
    bible: &WorldBible,
    things: Vec<Thing>,
    replace: bool,
) -> Result<String, WorldError> {
    let _writing = WORLD_WRITES.lock();
    let mut state = read_state(directory)?;
    let designed = state.designed.as_mut().ok_or(WorldError::NotDesigned)?;
    // Put again under its own id (`replace`), a thing is that thing as it is now (what it reads):
    // it replaces the one kept, never a second thing with the same id. Added, a thing the place
    // has already is refused by the place's check.
    for mut thing in things {
        match designed.things.iter_mut().find(|kept| kept.id == thing.id) {
            Some(kept) if replace => {
                // Its close-up kept stays with it.
                if thing.picture.is_none() {
                    thing.picture = kept.picture.take();
                }
                *kept = thing;
            }
            _ => designed.things.push(thing),
        }
    }
    bible
        .check_place(designed)
        .map_err(|cause| WorldError::PlaceInvalid { cause })?;
    let place = designed.id.clone();
    write_state_locked(directory, &state)?;
    Ok(place)
}

fn write_state_locked(directory: &Path, state: &WorldState) -> Result<(), WorldError> {
    let unwritable = |cause| WorldError::StateUnwritable { cause };
    // The active place designed in play is kept, as it stands now, to go back to.
    let mut state = state.clone();
    if let Some(designed) = &state.designed {
        state.kept.retain(|place| place.id != designed.id);
        if state.kept.len() >= crate::limits::MAXIMUM_KEPT_PLACES {
            state.kept.remove(0);
        }
        state.kept.push(designed.clone());
    }
    let text =
        serde_json::to_string_pretty(&state).map_err(|_| unwritable(IoCause::InvalidData))?;
    crate::atomic_file::write_atomically(directory, STATE_FILE, text.as_bytes())
        .map_err(|error| unwritable(IoCause::from_kind(error.kind())))
}
