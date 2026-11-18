//! The creation machine's saved record: the whole `CreationTable` in `players/creation.json`,
//! beside the players books, so the phone's listener and the loopback listener (each with its own
//! routes) read one table, and a build survives a restart.
//!
//! A step reads the record, changes it and writes it back whole under one lock, on the blocking
//! pool. No record is no creation yet.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use df_table::creation::CreationTable;

use super::request::CreationRouteError;
use crate::blocking::{WriteLock, run_blocking};
use crate::io_cause::IoCause;
use crate::seat::PLAYERS_DIRECTORY;
use crate::state::AppState;

const CREATION_FILE: &str = "creation.json";

/// Held around each read-change-write of the record, so two steps never overwrite each other.
static RECORD_LOCK: WriteLock = WriteLock::new();

fn storage(kind: ErrorKind) -> CreationRouteError {
    CreationRouteError::Storage {
        cause: IoCause::from_kind(kind),
    }
}

/// The folder the record is kept in.
pub(super) fn directory(app: &AppState) -> PathBuf {
    app.playground().files().assets().join(PLAYERS_DIRECTORY)
}

fn read(directory: &Path) -> Result<Option<CreationTable>, CreationRouteError> {
    let bytes = match std::fs::read(directory.join(CREATION_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(storage(error.kind())),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| storage(ErrorKind::InvalidData))
}

fn write(directory: &Path, table: &CreationTable) -> Result<(), CreationRouteError> {
    let bytes = serde_json::to_vec_pretty(table).map_err(|_| storage(ErrorKind::InvalidData))?;
    crate::atomic_file::write_atomically(directory, CREATION_FILE, &bytes)
        .map_err(|error| storage(error.kind()))
}

/// The record as it stands; none when creation has not begun.
pub(super) async fn load(app: &AppState) -> Result<Option<CreationTable>, CreationRouteError> {
    let directory = directory(app);
    run_blocking(
        move || {
            let _held = RECORD_LOCK.lock();
            read(&directory)
        },
        CreationRouteError::Stopped,
    )
    .await
}

/// Reads the record, lets `change` change it (a table is made for a record not there yet when
/// `change` asks for one) and writes it back when `change` succeeds; answers what `change`
/// answered.
pub(super) async fn change<T: Send + 'static>(
    app: &AppState,
    change: impl FnOnce(Option<CreationTable>) -> Result<(CreationTable, T), CreationRouteError>
    + Send
    + 'static,
) -> Result<T, CreationRouteError> {
    let directory = directory(app);
    run_blocking(
        move || {
            let _held = RECORD_LOCK.lock();
            let (table, answer) = change(read(&directory)?)?;
            write(&directory, &table)?;
            Ok(answer)
        },
        CreationRouteError::Stopped,
    )
    .await
}

/// Each built hero's live state between sessions, by hero id: its vitals, and whether its starting
/// equipment went into its inventory.
pub(super) type HeroStates = std::collections::BTreeMap<u64, HeroState>;

/// One hero's live state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HeroState {
    pub(super) vitals: df_rules::hero::HeroVitals,
    /// The starting equipment is in the hero's inventory (given once).
    #[serde(default)]
    pub(super) kit_given: bool,
}

const HEROES_FILE: &str = "hero-states.json";

fn read_heroes(directory: &Path) -> Result<HeroStates, CreationRouteError> {
    let bytes = match std::fs::read(directory.join(HEROES_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(HeroStates::new()),
        Err(error) => return Err(storage(error.kind())),
    };
    serde_json::from_slice(&bytes).map_err(|_| storage(ErrorKind::InvalidData))
}

/// Every built hero's live state; none recorded yet is an empty record.
pub(super) async fn load_heroes(app: &AppState) -> Result<HeroStates, CreationRouteError> {
    let directory = directory(app);
    run_blocking(
        move || {
            let _held = RECORD_LOCK.lock();
            read_heroes(&directory)
        },
        CreationRouteError::Stopped,
    )
    .await
}

/// Reads the heroes' live state and the creation record, lets `change` change the live state and
/// writes it back when `change` succeeds; answers what `change` answered.
pub(super) async fn change_heroes<T: Send + 'static>(
    app: &AppState,
    change: impl FnOnce(&mut HeroStates, Option<&CreationTable>) -> Result<T, CreationRouteError>
    + Send
    + 'static,
) -> Result<T, CreationRouteError> {
    let directory = directory(app);
    run_blocking(
        move || {
            let _held = RECORD_LOCK.lock();
            let table = read(&directory)?;
            let mut heroes = read_heroes(&directory)?;
            let answer = change(&mut heroes, table.as_ref())?;
            let bytes =
                serde_json::to_vec_pretty(&heroes).map_err(|_| storage(ErrorKind::InvalidData))?;
            crate::atomic_file::write_atomically(&directory, HEROES_FILE, &bytes)
                .map_err(|error| storage(error.kind()))?;
            Ok(answer)
        },
        CreationRouteError::Stopped,
    )
    .await
}
