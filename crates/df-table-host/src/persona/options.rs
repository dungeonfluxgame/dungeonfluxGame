//! The options the persona commands that ask the text service share: which model, and the
//! narrative guidance. Key=value options come before a command's free text.

use std::path::PathBuf;

use df_content::persona::backfill::{Latitude, NarrativeGuidance};

use crate::command::{CommandError, bad_choice};

/// The keys of the guidance options.
pub const GUIDANCE_KEYS: [&str; 11] = [
    "guidance",
    "arc",
    "phase",
    "beat",
    "hook",
    "thread",
    "establish",
    "avoid",
    "tone",
    "latitude",
    "note",
];

/// The guidance a command line gives: a file of `NarrativeGuidance` JSON and/or keys. A key given
/// on the command line replaces the file's value for it (a repeatable key replaces the file's
/// whole list).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuidanceArgs {
    pub file: Option<PathBuf>,
    pub arc: Option<String>,
    pub phase: Option<String>,
    pub beat: Option<String>,
    pub hooks: Vec<String>,
    pub threads: Vec<String>,
    pub establish: Vec<String>,
    pub avoid: Vec<String>,
    pub tone: Option<String>,
    pub latitude: Option<Latitude>,
    pub note: Option<String>,
}

impl GuidanceArgs {
    /// Whether any key (not the file) was given.
    fn has_keys(&self) -> bool {
        self.arc.is_some()
            || self.phase.is_some()
            || self.beat.is_some()
            || !self.hooks.is_empty()
            || !self.threads.is_empty()
            || !self.establish.is_empty()
            || !self.avoid.is_empty()
            || self.tone.is_some()
            || self.latitude.is_some()
            || self.note.is_some()
    }

    /// The guidance these options make over `file`'s (read from `self.file` by the caller); none
    /// when neither a file nor a key was given.
    pub fn merge(self, file: Option<NarrativeGuidance>) -> Option<NarrativeGuidance> {
        if file.is_none() && !self.has_keys() {
            return None;
        }
        let mut guidance = file.unwrap_or_default();
        if self.arc.is_some() {
            guidance.arc = self.arc;
        }
        if self.phase.is_some() {
            guidance.phase = self.phase;
        }
        if self.beat.is_some() {
            guidance.beat = self.beat;
        }
        if !self.hooks.is_empty() {
            guidance.hooks = self.hooks;
        }
        if !self.threads.is_empty() {
            guidance.threads = self.threads;
        }
        if !self.establish.is_empty() {
            guidance.must_establish = self.establish;
        }
        if !self.avoid.is_empty() {
            guidance.must_avoid = self.avoid;
        }
        if self.tone.is_some() {
            guidance.tone = self.tone;
        }
        if let Some(latitude) = self.latitude {
            guidance.latitude = latitude;
        }
        if self.note.is_some() {
            guidance.note = self.note;
        }
        Some(guidance)
    }
}

/// What every question to the model takes: the model to ask and the guidance.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Asking {
    pub model: Option<String>,
    pub guidance: GuidanceArgs,
}

/// The `key=value` words taken from the front of `words` whose key is in `keys`, and the words
/// left. A word that is not `key=` of one of `keys` ends the options.
pub fn take_options<'a>(
    words: &'a [String],
    keys: &[&str],
) -> (Vec<(&'a str, &'a str)>, &'a [String]) {
    let mut taken = Vec::new();
    let mut rest = words;
    while let Some((word, after)) = rest.split_first() {
        let Some((key, value)) = word.split_once('=').filter(|(key, _)| keys.contains(key)) else {
            break;
        };
        taken.push((key, value));
        rest = after;
    }
    (taken, rest)
}

/// Sets `slot` to `value`, refusing a key given twice.
fn once(slot: &mut Option<String>, key: &str, value: &str) -> Result<(), CommandError> {
    if slot.is_some() {
        return Err(CommandError::DuplicateKey(key.to_owned()));
    }
    *slot = Some(value.to_owned());
    Ok(())
}

/// Reads the `model` and guidance keys among `pairs`; any other key is the caller's.
pub fn asking_from(pairs: &[(&str, &str)]) -> Result<Asking, CommandError> {
    let mut asking = Asking::default();
    let guidance = &mut asking.guidance;
    let mut file = None;
    let mut latitude = None;
    for &(key, value) in pairs {
        match key {
            "model" => once(&mut asking.model, key, value)?,
            "guidance" => once(&mut file, key, value)?,
            "arc" => once(&mut guidance.arc, key, value)?,
            "phase" => once(&mut guidance.phase, key, value)?,
            "beat" => once(&mut guidance.beat, key, value)?,
            "tone" => once(&mut guidance.tone, key, value)?,
            "note" => once(&mut guidance.note, key, value)?,
            "latitude" => {
                once(&mut latitude, key, value)?;
                guidance.latitude = Some(Latitude::from_key(value).ok_or_else(|| {
                    let keys: Vec<&str> = Latitude::ALL.iter().map(|level| level.key()).collect();
                    bad_choice("latitude", value, keys.join(", "))
                })?);
            }
            "hook" => guidance.hooks.push(value.to_owned()),
            "thread" => guidance.threads.push(value.to_owned()),
            "establish" => guidance.establish.push(value.to_owned()),
            "avoid" => guidance.avoid.push(value.to_owned()),
            _ => {}
        }
    }
    guidance.file = file.map(PathBuf::from);
    Ok(asking)
}
