//! The switches the host remembers from one run to the next: the overlays, the voice switches and
//! the sound, kept in `settings.json` beside the playground's other files.
//!
//! A change of one of them is saved whole or not at all (`atomic_file`), so a stop in the middle
//! of one never leaves half a file. A run reads the file once, as it starts. No file, or one that
//! does not read, leaves every switch at its default, and every default that can spend money or
//! open a microphone is off (owner, 2026-11-10: a restart must never turn a paid switch back on).
//! An unreadable file is named in one audit line and never changes a switch by guessing.

use std::fmt;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use df_playground::controls::Controls;
use df_playground::sound_plan::AudioPrefs;
use serde::{Deserialize, Serialize};

use super::audit::playground_line;
use super::files::FileError;
use crate::blocking::{WriteLock, run_blocking};
use crate::io_cause::IoCause;
use crate::state::AppState;

const SETTINGS_FILE: &str = "settings.json";

/// Held while the file is written, so two changes at once leave the newer one's switches.
static SETTINGS_WRITE: WriteLock = WriteLock::new();

/// The controls that outlive a run of the host. A key the file lacks stays at its default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StoredSettings {
    show_unavailable: bool,
    show_grid: bool,
    show_sam: bool,
    tts_on: bool,
    stt_on: bool,
    audio: AudioPrefs,
}

impl StoredSettings {
    pub(super) fn of(controls: &Controls) -> Self {
        Self {
            show_unavailable: controls.show_unavailable,
            show_grid: controls.show_grid,
            show_sam: controls.show_sam,
            tts_on: controls.tts_on,
            stt_on: controls.stt_on,
            audio: controls.audio,
        }
    }

    /// `controls` with these switches in place of its own.
    pub(super) fn applied_to(&self, controls: Controls) -> Controls {
        Controls {
            show_unavailable: self.show_unavailable,
            show_grid: self.show_grid,
            show_sam: self.show_sam,
            tts_on: self.tts_on,
            stt_on: self.stt_on,
            audio: self.audio,
            ..controls
        }
    }

    /// The switches as the audit line of a start or a save names them.
    fn pairs(&self) -> Vec<(&'static str, String)> {
        vec![
            ("unavailable", self.show_unavailable.to_string()),
            ("grid", self.show_grid.to_string()),
            ("sam", self.show_sam.to_string()),
            ("tts", self.tts_on.to_string()),
            ("stt", self.stt_on.to_string()),
            ("sound", self.audio.sound_on.to_string()),
            ("music", self.audio.music_on.to_string()),
            ("volume", self.audio.volume_percent.to_string()),
        ]
    }
}

/// Why the saved switches were not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsError {
    /// The file exists and could not be read.
    Unreadable { cause: IoCause },
    /// The file is not the switches' JSON.
    Malformed,
}

impl SettingsError {
    /// A closed word for the audit line.
    fn key(self) -> &'static str {
        match self {
            Self::Unreadable { .. } => "unreadable",
            Self::Malformed => "malformed",
        }
    }
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { cause } => {
                write!(formatter, "the saved settings could not be read: {cause}")
            }
            Self::Malformed => formatter.write_str("the saved settings are not readable JSON"),
        }
    }
}

impl std::error::Error for SettingsError {}

/// How the switches of this run came to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Restore {
    /// There was no file: the defaults.
    Defaults,
    /// The file was read.
    Saved(StoredSettings),
    /// The file was there and could not be taken: the defaults.
    Unusable(SettingsError),
}

impl Restore {
    /// The switches this run starts with.
    pub(super) fn settings(&self) -> StoredSettings {
        match self {
            Self::Saved(settings) => settings.clone(),
            Self::Defaults | Self::Unusable(_) => StoredSettings::default(),
        }
    }

    /// The audit line of the start: where the switches came from and what they are.
    pub(crate) fn line(&self) -> String {
        let mut pairs = vec![(
            "source",
            match self {
                Self::Defaults => "defaults",
                Self::Saved(_) => "file",
                Self::Unusable(_) => "defaults_after_error",
            }
            .to_owned(),
        )];
        if let Self::Unusable(error) = self {
            pairs.push(("error", error.key().to_owned()));
        }
        pairs.extend(self.settings().pairs());
        playground_line("settings_restored", &pairs)
    }
}

/// Reads the saved switches in `directory`. A start-up read, before the host listens.
pub(super) fn restore(directory: &Path) -> Restore {
    let bytes = match std::fs::read(directory.join(SETTINGS_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Restore::Defaults,
        Err(error) => {
            return Restore::Unusable(SettingsError::Unreadable {
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(settings) => Restore::Saved(settings),
        Err(_) => Restore::Unusable(SettingsError::Malformed),
    }
}

fn write(directory: &Path, settings: &StoredSettings) -> Result<(), FileError> {
    let text =
        serde_json::to_vec_pretty(settings).map_err(|_| FileError::Io(ErrorKind::InvalidData))?;
    crate::atomic_file::write_atomically(directory, SETTINGS_FILE, &text).map_err(FileError::from)
}

/// Whether the change from `before` to `after` altered a switch the host remembers.
pub(super) fn is_remembered_change(before: &Controls, after: &Controls) -> bool {
    StoredSettings::of(before) != StoredSettings::of(after)
}

/// Saves the switches as they stand now, on the blocking pool, and writes the audit line of it:
/// `PLAYGROUND settings_saved ..` or `PLAYGROUND settings_unsaved error=<key>`. The switches are
/// read inside the write turn, so of two changes at once the file ends with the newer.
pub(super) async fn save(state: &Arc<AppState>) -> Result<(), FileError> {
    let worker = Arc::clone(state);
    let saved = run_blocking(
        move || {
            let _turn = SETTINGS_WRITE.lock();
            let settings = StoredSettings::of(&worker.playground().snapshot().controls);
            write(worker.playground().files().assets(), &settings).map(|()| settings)
        },
        FileError::Stopped,
    )
    .await;
    match &saved {
        Ok(settings) => state.record_line(playground_line("settings_saved", &settings.pairs())),
        Err(error) => state.record_line(playground_line(
            "settings_unsaved",
            &[("error", error.key().to_owned())],
        )),
    }
    saved.map(|_| ())
}

#[cfg(test)]
mod settings_tests;
