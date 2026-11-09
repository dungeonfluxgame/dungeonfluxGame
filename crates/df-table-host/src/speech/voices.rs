//! The voices file: which supplier voice speaks for which character.
//!
//! `voices.json` in the playground's assets folder holds `{"default": "<voice id>", "voices":
//! {"<character>": "<voice id>"}}`. A missing file is "no voices", never an error; a file that
//! does not read is a typed error. A voice id is an opaque reference to the supplier's voice: not
//! a permission and not an identity, and it is never written to an audit line. A voice is fixed
//! to a character by editing this file (`speech voice`, or a cast: `speech cast` and the first
//! line of a character who has none), written whole or not at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use df_content::persona::PersonaId;
use serde::{Deserialize, Serialize};

use super::limits::{MAXIMUM_VOICE_ID_BYTES, MAXIMUM_VOICES, MAXIMUM_VOICES_FILE_BYTES};
use super::request::{SpeechError, VoiceCause, VoiceChanged, VoiceRow, VoicesCause};
use crate::atomic_file::write_atomically;
use crate::blocking::WriteLock;
use crate::dev_http::BodyError;
use crate::io_cause::IoCause;
use crate::state::AppState;

/// The voices file's name.
pub const VOICES_FILE: &str = "voices.json";

/// Held around every write of the voices file, so two edits never overwrite one another.
pub(super) static VOICE_WRITES: WriteLock = WriteLock::new();

/// Whose voice a line was spoken in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VoiceSource {
    /// A voice id the caller named.
    Override,
    /// The character's own voice.
    Character,
    /// The default voice.
    Default,
}

impl VoiceSource {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::Character => "character",
            Self::Default => "default",
        }
    }
}

/// The voice a line is spoken in and where it came from.
pub(super) struct ResolvedVoice {
    pub(super) id: String,
    pub(super) source: VoiceSource,
}

impl ResolvedVoice {
    /// The label of the profile that spoke: the model and where the voice came from, never the
    /// voice id.
    pub(super) fn profile(&self, model: &str) -> String {
        format!("{model}/{}", self.source.key())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VoicesFile {
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    voices: BTreeMap<String, String>,
}

/// The voices file as it was read.
pub(super) struct VoiceBook {
    file: VoicesFile,
}

/// The voices file of the host's playground.
pub(super) fn voices_path(app: &AppState) -> PathBuf {
    app.playground().files().assets().join(VOICES_FILE)
}

/// That `voice` can be sent to the supplier as a voice id.
pub(super) fn check_voice(voice: &str) -> Result<(), SpeechError> {
    let cause = if voice.trim().is_empty() {
        VoiceCause::Empty
    } else if voice.len() > MAXIMUM_VOICE_ID_BYTES {
        VoiceCause::TooLong
    } else if voice
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        VoiceCause::InvalidCharacters
    } else {
        return Ok(());
    };
    Err(SpeechError::InvalidVoice { cause })
}

/// `text` as a character id, or the refusal naming the rule.
pub(super) fn check_character(text: &str) -> Result<PersonaId, SpeechError> {
    PersonaId::new(text).map_err(|cause| SpeechError::InvalidCharacter { cause })
}

fn invalid(cause: VoicesCause) -> SpeechError {
    SpeechError::VoicesInvalid { cause }
}

impl VoiceBook {
    /// The book at `path`: empty when there is no file.
    pub(super) fn read(path: &Path) -> Result<Self, SpeechError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    file: VoicesFile::default(),
                });
            }
            Err(error) => {
                return Err(SpeechError::VoicesUnreadable {
                    cause: IoCause::from_kind(error.kind()),
                });
            }
        };
        if u64::try_from(bytes.len()).map_or(true, |length| length > MAXIMUM_VOICES_FILE_BYTES) {
            return Err(invalid(VoicesCause::TooLarge));
        }
        let file: VoicesFile = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(VoicesCause::Malformed(BodyError::from_json(&error))))?;
        if file.voices.len() > MAXIMUM_VOICES {
            return Err(invalid(VoicesCause::TooManyVoices));
        }
        for (character, voice) in &file.voices {
            check_character(character).map_err(|_| invalid(VoicesCause::InvalidCharacterId))?;
            check_voice(voice).map_err(|_| invalid(VoicesCause::InvalidVoiceId))?;
        }
        if let Some(voice) = &file.default {
            check_voice(voice).map_err(|_| invalid(VoicesCause::InvalidDefaultVoice))?;
        }
        Ok(Self { file })
    }

    /// The default voice, if one is set.
    pub(super) fn default_voice(&self) -> Option<&str> {
        self.file.default.as_deref()
    }

    /// The voice of `character`'s own, if they have one.
    pub(super) fn voice_of(&self, character: &str) -> Option<&str> {
        self.file.voices.get(character).map(String::as_str)
    }

    /// The voices someone other than `character` speaks in: the other characters' and the
    /// default.
    pub(super) fn voices_in_use(&self, character: &str) -> Vec<&str> {
        self.file
            .voices
            .iter()
            .filter(|(other, _)| other.as_str() != character)
            .map(|(_, voice)| voice.as_str())
            .chain(self.file.default.as_deref())
            .collect()
    }

    /// The characters with a voice of their own, sorted by character.
    pub(super) fn rows(&self) -> Vec<VoiceRow> {
        self.file
            .voices
            .iter()
            .map(|(character, voice)| VoiceRow {
                character: character.clone(),
                voice: voice.clone(),
            })
            .collect()
    }

    /// The voice to speak in: `named` when the caller gives one, else the character's, else the
    /// default.
    pub(super) fn resolve(
        &self,
        character: Option<&str>,
        named: Option<&str>,
    ) -> Result<ResolvedVoice, SpeechError> {
        if let Some(voice) = named {
            check_voice(voice)?;
            return Ok(ResolvedVoice {
                id: voice.to_owned(),
                source: VoiceSource::Override,
            });
        }
        if let Some(voice) = character.and_then(|character| self.file.voices.get(character)) {
            return Ok(ResolvedVoice {
                id: voice.clone(),
                source: VoiceSource::Character,
            });
        }
        match &self.file.default {
            Some(voice) => Ok(ResolvedVoice {
                id: voice.clone(),
                source: VoiceSource::Default,
            }),
            None => Err(SpeechError::NoVoice {
                character: character.map(str::to_owned),
            }),
        }
    }
}

/// Gives `character` the voice `voice`, or takes theirs away when `voice` is `None`, and writes
/// the file whole. Blocking; the caller holds no lock.
pub(super) fn change_voice(
    path: &Path,
    character: &str,
    voice: Option<&str>,
) -> Result<VoiceChanged, SpeechError> {
    let character = check_character(character)?;
    if let Some(voice) = voice {
        check_voice(voice)?;
    }
    let _writing = VOICE_WRITES.lock();
    let mut book = VoiceBook::read(path)?;
    let before = book.file.clone();
    match voice {
        Some(voice) => {
            book.file
                .voices
                .insert(character.as_str().to_owned(), voice.to_owned());
        }
        None => {
            book.file.voices.remove(character.as_str());
        }
    }
    let changed = book.file != before;
    if changed {
        write_book(path, &book.file)?;
    }
    Ok(VoiceChanged {
        character: character.to_string(),
        has_voice: voice.is_some(),
        changed,
    })
}

/// The voice `character` speaks in after a claim: `voice` when they had none, or their own when
/// someone gave them one first, which a claim never replaces.
pub(super) struct Claimed {
    pub(super) voice: String,
    pub(super) given: bool,
}

/// Gives `character` the voice `voice` unless they have one already, and writes the file whole.
/// The check and the write are under the write lock, so a voice set while `voice` was chosen is
/// kept. Blocking; the caller holds no lock.
pub(super) fn claim_voice(
    path: &Path,
    character: &str,
    voice: &str,
) -> Result<Claimed, SpeechError> {
    let character = check_character(character)?;
    check_voice(voice)?;
    let _writing = VOICE_WRITES.lock();
    let mut book = VoiceBook::read(path)?;
    if let Some(own) = book.voice_of(character.as_str()) {
        return Ok(Claimed {
            voice: own.to_owned(),
            given: false,
        });
    }
    book.file
        .voices
        .insert(character.as_str().to_owned(), voice.to_owned());
    write_book(path, &book.file)?;
    Ok(Claimed {
        voice: voice.to_owned(),
        given: true,
    })
}

/// Writes `file` at `path` whole or not at all. The caller holds the write lock.
fn write_book(path: &Path, file: &VoicesFile) -> Result<(), SpeechError> {
    let mut text = serde_json::to_string_pretty(file)
        .map_err(|error| invalid(VoicesCause::Malformed(BodyError::from_json(&error))))?;
    text.push('\n');
    let (directory, name) = split(path)?;
    write_atomically(directory, name, text.as_bytes()).map_err(|error| {
        SpeechError::VoicesUnwritable {
            cause: IoCause::from_kind(error.kind()),
        }
    })
}

/// The folder and file name of `path`.
fn split(path: &Path) -> Result<(&Path, &str), SpeechError> {
    let name = path.file_name().and_then(|name| name.to_str());
    match (path.parent(), name) {
        (Some(directory), Some(name)) => Ok((directory, name)),
        _ => Err(SpeechError::VoicesUnwritable {
            cause: IoCause::InvalidData,
        }),
    }
}
