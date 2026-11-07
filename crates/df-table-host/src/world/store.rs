//! The bible and the active place, read from the playground's `world` folder.
//!
//! The bible (`bible.json`) is read fresh on every call, so a designer's edit needs no rebuild and
//! no restart. A missing bible is an empty one; a file that does not read or is not a valid bible
//! is a typed error. The active place is `state.json`: `{"place": "<id>"}`, written whole or not
//! at all; a missing file is no active place.

use std::path::{Path, PathBuf};

use df_content::world_bible::{WorldBible, is_valid_id};
use serde::{Deserialize, Serialize};

use super::request::WorldError;
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

/// The active place in `directory`: none when there is no state file, or it names none.
pub(super) fn read_place(directory: &Path) -> Result<Option<String>, WorldError> {
    let text = match std::fs::read_to_string(directory.join(STATE_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(WorldError::StateUnreadable {
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    let state: WorldState =
        serde_json::from_str(&text).map_err(|error| WorldError::StateInvalid {
            cause: BodyError::from_json(&error),
        })?;
    Ok(state.place.filter(|place| is_valid_id(place)))
}

/// Makes `place` the active one (none for no place), whole or not at all.
pub(super) fn write_place(directory: &Path, place: Option<&str>) -> Result<(), WorldError> {
    let state = WorldState {
        place: place.map(str::to_owned),
    };
    let unwritable = |cause| WorldError::StateUnwritable { cause };
    let text = serde_json::to_string_pretty(&state).map_err(|_| unwritable(IoCause::InvalidData))?;
    let _writing = WORLD_WRITES.lock();
    crate::atomic_file::write_atomically(directory, STATE_FILE, text.as_bytes())
        .map_err(|error| unwritable(IoCause::from_kind(error.kind())))
}
