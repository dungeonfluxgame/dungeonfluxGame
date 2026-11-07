//! The live emotional state of each persona (`planning/persona-emotion.md`): what it feels, what it
//! attends to and how it stands toward the others. The host keeps it, per persona, as
//! `<id>.json` in the playground's `persona-state` folder, whole or not at all, and reads it again
//! on every call; a persona with no state starts at its temperament's rest.
//!
//! Seam of the line pipeline: `load_state` puts the state in a line's facts and `apply_appraisal`
//! takes the model's appraisal of what was said into it. Neither blocks a line: a state or an
//! emotion file that does not read is recorded as `PERSONA state_unavailable id=.. error=<key>`
//! and the line goes on with the built-in list and a state at rest. The rules are
//! `df_content::persona::emotion_logic`; this file only keeps the files.

use std::path::{Path, PathBuf};

use df_content::persona::{
    Appraisal, EmotionConfig, Persona, PersonaId, PersonaState, Temperament, fresh_state,
};

use super::emotions::{EmotionError, load_emotions};
use super::storage::{ascii_only, persona_directory, read_persona};
use crate::blocking::WriteLock;
use crate::dev_http::BodyError;
use crate::io_cause::IoCause;
use crate::state::AppState;

pub(crate) use df_content::persona::state_triggers;

/// The folder of live states under the playground's assets.
pub const STATE_DIRECTORY: &str = "persona-state";

/// Held around every read-then-write of a state file, so two appraisals never lose one another.
static STATE_WRITES: WriteLock = WriteLock::new();

/// The `persona-state` folder of the host's playground.
pub(super) fn state_directory(app: &AppState) -> PathBuf {
    app.playground().files().assets().join(STATE_DIRECTORY)
}

fn state_file_name(id: &str) -> String {
    format!("{id}.json")
}

/// The stored state of `id`, or none when it has no state file.
pub(super) fn read_state(directory: &Path, id: &str) -> Result<Option<PersonaState>, EmotionError> {
    let text = match std::fs::read_to_string(directory.join(state_file_name(id))) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(EmotionError::StateUnreadable {
                id: id.to_owned(),
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    let state: PersonaState =
        serde_json::from_str(&text).map_err(|error| EmotionError::StateInvalid {
            id: id.to_owned(),
            cause: BodyError::from_json(&error),
        })?;
    if state.id != id {
        return Err(EmotionError::StateIdMismatch {
            id: id.to_owned(),
            found: state.id,
        });
    }
    Ok(Some(state))
}

/// Writes `state` as its file (the folder is made if missing), whole or not at all.
pub(super) fn write_state(directory: &Path, state: &PersonaState) -> Result<(), EmotionError> {
    let unwritable = |cause| EmotionError::StateUnwritable {
        id: state.id.clone(),
        cause,
    };
    let text = serde_json::to_string_pretty(state)
        .map(|json| ascii_only(&json))
        .map_err(|_| unwritable(IoCause::InvalidData))?;
    crate::atomic_file::write_atomically(directory, &state_file_name(&state.id), text.as_bytes())
        .map_err(|error| unwritable(IoCause::from_kind(error.kind())))
}

/// Removes the state file of `id`; the state is then the persona's rest again.
pub(super) fn remove_state(directory: &Path, id: &str) -> Result<(), EmotionError> {
    match std::fs::remove_file(directory.join(state_file_name(id))) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(EmotionError::StateUnwritable {
            id: id.to_owned(),
            cause: IoCause::from_kind(error.kind()),
        }),
    }
}

/// The temperament of the stored persona `id`; a persona with none, or none stored, rests flat.
/// A persona file that does not read is the error of its id.
pub(super) fn temperament_of(app: &AppState, id: &str) -> Result<Temperament, EmotionError> {
    let Ok(id) = PersonaId::new(id) else {
        return Ok(Temperament::default());
    };
    Ok(read_persona(&persona_directory(app), &id)?
        .map(|persona| persona.temperament().clone())
        .unwrap_or_default())
}

/// The state of `id` now: the stored one, or a fresh one at the temperament's rest.
pub(super) fn current_state(
    directory: &Path,
    id: &str,
    config: &EmotionConfig,
    temperament: &Temperament,
) -> Result<PersonaState, EmotionError> {
    Ok(read_state(directory, id)?.unwrap_or_else(|| fresh_state(id, config, temperament)))
}

/// Records that the emotional layer fell back for `id`.
fn record_unavailable(app: &AppState, id: &str, error: &EmotionError) {
    app.record_line(format!(
        "PERSONA state_unavailable id={id} error={}",
        error.key()
    ));
}

/// The state of `id`, or a fresh one at the persona's rest when it has none.
pub(crate) fn load_state(app: &AppState, id: &str) -> PersonaState {
    let loaded = load_emotions(app).and_then(|config| {
        let temperament = temperament_of(app, id)?;
        current_state(&state_directory(app), id, &config, &temperament)
    });
    loaded.unwrap_or_else(|error| {
        record_unavailable(app, id, &error);
        fresh_state(id, &EmotionConfig::default(), &Temperament::default())
    })
}

/// Takes `appraisal` into the state of `persona`: validates the emotions against the emotion list,
/// scales each change by the temperament, fades the rest toward the baseline, derives the mood and
/// keeps the result. Answers the new state.
pub(crate) fn apply_appraisal(
    app: &AppState,
    persona: &Persona,
    appraisal: &Appraisal,
) -> PersonaState {
    let id = persona.id().as_str();
    let config = load_emotions(app).unwrap_or_else(|error| {
        record_unavailable(app, id, &error);
        EmotionConfig::default()
    });
    let directory = state_directory(app);
    let _writing = STATE_WRITES.lock();
    let before =
        current_state(&directory, id, &config, persona.temperament()).unwrap_or_else(|error| {
            record_unavailable(app, id, &error);
            fresh_state(id, &config, persona.temperament())
        });
    let (after, dropped) =
        df_content::persona::apply_appraisal(&config, persona.temperament(), &before, appraisal);
    if let Err(error) = write_state(&directory, &after) {
        record_unavailable(app, id, &error);
    }
    app.record_line(format!(
        concat!(
            "PERSONA state_applied id={} turn={} mood_valence={} mood_arousal={} ",
            "dropped_emotions={} dropped_relationships={} dropped_noticed={}"
        ),
        id,
        after.turn,
        after.mood.valence,
        after.mood.arousal,
        dropped.emotions,
        dropped.relationships,
        dropped.noticed
    ));
    after
}

/// Replaces the state of `id` with the one `change` makes of it, under the write lock, and answers
/// it. The one place the dev routes change a state by hand.
pub(super) fn change_state(
    directory: &Path,
    id: &str,
    config: &EmotionConfig,
    temperament: &Temperament,
    change: impl FnOnce(&mut PersonaState) -> Result<(), EmotionError>,
) -> Result<PersonaState, EmotionError> {
    let _writing = STATE_WRITES.lock();
    let mut state = current_state(directory, id, config, temperament)?;
    change(&mut state)?;
    write_state(directory, &state)?;
    Ok(state)
}

/// Removes the state file of `id` under the write lock.
pub(super) fn reset_state(directory: &Path, id: &str) -> Result<(), EmotionError> {
    let _writing = STATE_WRITES.lock();
    remove_state(directory, id)
}
