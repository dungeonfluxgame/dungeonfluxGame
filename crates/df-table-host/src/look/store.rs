//! The look studio's files: each seat's studio record (`players/looks/seat-<n>.json`, beside the
//! players books) and the frames' chosen poses (`look/poses.json`, beside the studio file). Each is
//! written whole or not at all. Reading and writing is blocking; callers run it off the async
//! workers.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use df_playground::look::wire::{
    LockState, LookDetail, LookStudioView, PictureFailure, VersionState,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::catalog::LOOK_DIRECTORY;
use super::request::{LookError, PosesView};
use crate::dev_http::BodyError;
use crate::io_cause::IoCause;
use crate::seat::PLAYERS_DIRECTORY;

const LOOKS_DIRECTORY: &str = "looks";
const POSES_FILE: &str = "poses.json";

/// One seat's studio as its file keeps it: the view the phone is shown, and the number the next
/// version takes (never reused, so a version's picture is never overwritten).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StudioRecord {
    pub(crate) next_version: u64,
    /// The hero the versions are for; none in a record written before it was kept.
    #[serde(default)]
    pub(crate) hero: Option<u64>,
    pub(crate) view: LookStudioView,
    /// Every detail Jev read for the hero's look: a draft's details must be among them (or rolled).
    #[serde(default)]
    pub(crate) read_details: Vec<LookDetail>,
}

impl StudioRecord {
    /// The record as `hero`'s: another hero's versions (a seat that changed hands, or a new hero)
    /// are put away, and the version numbers go on.
    pub(crate) fn for_hero(&mut self, hero: u64) {
        if self.hero != Some(hero) {
            self.view = LookStudioView::default();
            self.read_details.clear();
            self.hero = Some(hero);
        }
    }

    /// A record read from its file after a restart: a picture still pending, or a lock-in still
    /// running, was being made by a run that is gone. Answers whether anything was interrupted.
    pub(crate) fn interrupted(&mut self) -> bool {
        let mut interrupted = false;
        for version in &mut self.view.versions {
            if version.state == VersionState::Pending {
                version.state = VersionState::Failed {
                    cause: PictureFailure::Interrupted,
                };
                interrupted = true;
            }
        }
        if self
            .view
            .lock
            .as_ref()
            .is_some_and(|lock| !matches!(lock.state, LockState::Locked { .. }))
        {
            self.view.lock = None;
            interrupted = true;
        }
        interrupted
    }
}

fn storage(error: &std::io::Error) -> LookError {
    LookError::Storage {
        cause: IoCause::from_kind(error.kind()),
    }
}

fn read_json<T: DeserializeOwned + Default>(
    path: &Path,
    invalid: impl FnOnce(BodyError) -> LookError,
    unreadable: impl FnOnce(IoCause) -> LookError,
) -> Result<T, LookError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => return Err(unreadable(IoCause::from_kind(error.kind()))),
    };
    serde_json::from_slice(&bytes).map_err(|error| invalid(BodyError::from_json(&error)))
}

fn write_json<T: Serialize>(directory: &Path, file_name: &str, value: &T) -> Result<(), LookError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| LookError::Storage {
        cause: IoCause::InvalidData,
    })?;
    crate::atomic_file::write_atomically(directory, file_name, &bytes)
        .map_err(|error| storage(&error))
}

fn looks_directory(assets: &Path) -> PathBuf {
    assets.join(PLAYERS_DIRECTORY).join(LOOKS_DIRECTORY)
}

fn record_file(seat: u64) -> String {
    format!("seat-{seat}.json")
}

/// The studio record of `seat`; an empty one when it has none. Blocking.
pub(crate) fn read_record(assets: &Path, seat: u64) -> Result<StudioRecord, LookError> {
    let path = looks_directory(assets).join(record_file(seat));
    read_json(
        &path,
        |_| LookError::Storage {
            cause: IoCause::InvalidData,
        },
        |cause| LookError::Storage { cause },
    )
}

/// Writes the studio record of `seat`. Blocking.
pub(crate) fn write_record(
    assets: &Path,
    seat: u64,
    record: &StudioRecord,
) -> Result<(), LookError> {
    write_json(&looks_directory(assets), &record_file(seat), record)
}

/// The frames' chosen poses; none chosen when there is no file. Blocking.
pub(crate) fn read_poses(assets: &Path) -> Result<PosesView, LookError> {
    let path = assets.join(LOOK_DIRECTORY).join(POSES_FILE);
    read_json(
        &path,
        |cause| LookError::PosesInvalid { cause },
        |cause| LookError::PosesUnreadable { cause },
    )
}

/// Writes the frames' chosen poses. Blocking.
pub(crate) fn write_poses(assets: &Path, poses: &PosesView) -> Result<(), LookError> {
    write_json(&assets.join(LOOK_DIRECTORY), POSES_FILE, poses)
}

/// The bytes of the media library's picture `name` in `media`. Blocking.
pub(crate) fn read_picture(media: &Path, name: &str) -> Result<Vec<u8>, LookError> {
    std::fs::read(media.join(name)).map_err(|error| match error.kind() {
        ErrorKind::NotFound => LookError::UnknownPicture {
            picture: name.to_owned(),
        },
        _ => storage(&error),
    })
}
