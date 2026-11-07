//! Persona storage: the persona files in the playground's `personas` folder, read and written one
//! persona per file, whole or not at all, and the record of the backfills of each persona.

use std::path::{Path, PathBuf};

use df_content::persona::{Persona, PersonaId};

use super::guard::copies_private_detail_into_public_text;
use super::request::{BackfillRecord, PersonaError, PersonaSummary, summary_of};
use crate::blocking::{WriteLock, run_blocking};
use crate::dev_http::BodyError;
use crate::io_cause::IoCause;
use crate::state::AppState;

/// Held around every write or removal of a persona file and of a backfill record, so a removal, a
/// write and a backfill never interleave. One lock for the folder, since persona writes are rare
/// and short.
pub(super) static PERSONA_WRITES: WriteLock = WriteLock::new();

/// The folder of stored personas under the playground's assets.
pub const PERSONA_DIRECTORY: &str = "personas";
/// What a persona file's name adds to its persona's id.
pub const PERSONA_SUFFIX: &str = ".persona.json";
/// What a backfill record file's name adds to its persona's id.
pub const BACKFILLS_SUFFIX: &str = ".backfills.json";

/// The words no persona can be called: the last segments of the dev API's paths and the words of
/// the persona command line, which would be read as a verb or as a route instead of an id.
pub const RESERVED_IDS: [&str; 11] = [
    "remove",
    "edit",
    "generate",
    "generate-cast",
    "say",
    "ask",
    "coverage",
    "backfill",
    "backfills",
    "list",
    "all",
];

/// Whether `id` is one of [`RESERVED_IDS`].
pub(super) fn is_reserved(id: &str) -> bool {
    RESERVED_IDS.contains(&id)
}

/// That `id` is not a reserved word. Every route and command that creates or names a persona
/// checks it here.
pub(super) fn check_reserved(id: &PersonaId) -> Result<(), PersonaError> {
    if is_reserved(id.as_str()) {
        Err(PersonaError::ReservedId { id: id.to_string() })
    } else {
        Ok(())
    }
}

/// `text` as the id of a persona, or the refusal naming why it cannot be one.
pub(super) fn persona_id(text: &str) -> Result<PersonaId, PersonaError> {
    let id = PersonaId::new(text).map_err(|cause| PersonaError::InvalidPersona { cause })?;
    check_reserved(&id)?;
    Ok(id)
}

/// The persona file of `id`.
pub fn persona_file_name(id: &PersonaId) -> String {
    format!("{id}{PERSONA_SUFFIX}")
}

/// The backfill record file of `id`.
pub fn backfills_file_name(id: &PersonaId) -> String {
    format!("{id}{BACKFILLS_SUFFIX}")
}

/// The `personas` folder of the host's playground.
pub(super) fn persona_directory(app: &AppState) -> PathBuf {
    app.playground().files().assets().join(PERSONA_DIRECTORY)
}

/// The stored persona of `id`, or `None` when it has none. A file that does not read, or is not
/// the persona of `id`, is a typed error naming `id`.
pub fn read_persona(directory: &Path, id: &PersonaId) -> Result<Option<Persona>, PersonaError> {
    let path = directory.join(persona_file_name(id));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(PersonaError::Unreadable {
                id: id.to_string(),
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    let persona: Persona =
        serde_json::from_str(&text).map_err(|error| PersonaError::StoredInvalid {
            id: id.to_string(),
            cause: BodyError::from_json(&error),
        })?;
    if persona.id() != id {
        return Err(PersonaError::StoredIdMismatch {
            id: id.to_string(),
            found: persona.id().to_string(),
        });
    }
    Ok(Some(persona))
}

/// `json` with every non-ASCII character written as a `\uXXXX` escape (a surrogate pair above the
/// Basic Multilingual Plane), so the stored files stay ASCII whatever a model wrote into them.
/// Non-ASCII can only be inside a JSON string, where the escape reads back as the same character.
pub(super) fn ascii_only(json: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(json.len());
    for character in json.chars() {
        if character.is_ascii() {
            out.push(character);
        } else {
            let mut units = [0_u16; 2];
            for unit in character.encode_utf16(&mut units) {
                // Writing to a string cannot fail.
                let _ = write!(out, "\\u{unit:04x}");
            }
        }
    }
    out
}

/// Writes `persona` as its file in the `personas` folder (made if missing), whole or not at all.
/// Answers whether it replaced a file of the same id; a file whose presence could not be checked
/// counts as not replaced, since the write itself reports every failure.
pub fn write_persona(directory: &Path, persona: &Persona) -> Result<bool, PersonaError> {
    let unwritable = |cause| PersonaError::Unwritable {
        id: persona.id().to_string(),
        cause,
    };
    // A persona always writes as JSON; the mapping only keeps the path panic-free.
    let text = serde_json::to_string_pretty(persona)
        .map(|json| ascii_only(&json))
        .map_err(|_| unwritable(IoCause::InvalidData))?;
    let file_name = persona_file_name(persona.id());
    let replaced = matches!(directory.join(&file_name).try_exists(), Ok(true));
    crate::atomic_file::write_atomically(directory, &file_name, text.as_bytes())
        .map_err(|error| unwritable(IoCause::from_kind(error.kind())))?;
    Ok(replaced)
}

/// Writes a persona a model just generated, unless one is stored and `overwrite` is not asked:
/// then the stored one is answered and nothing is written. The check and the write are under the
/// write lock, so a persona that appeared while the model was asked is never overwritten.
pub(super) fn store_generated(
    directory: &Path,
    persona: &Persona,
    overwrite: bool,
) -> Result<Option<Persona>, PersonaError> {
    let _writing = PERSONA_WRITES.lock();
    if !overwrite && let Some(existing) = read_persona(directory, persona.id())? {
        return Ok(Some(existing));
    }
    write_persona(directory, persona)?;
    Ok(None)
}

/// Removes the file of `id`; `false` when it had none. The persona's `<id>.backfills.json` stays:
/// the record of what was invented is history (owner decision 2026-11-06), and a persona written
/// later with the same id continues its sequence.
pub fn remove_persona(directory: &Path, id: &PersonaId) -> Result<bool, PersonaError> {
    match std::fs::remove_file(directory.join(persona_file_name(id))) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(PersonaError::Unwritable {
            id: id.to_string(),
            cause: IoCause::from_kind(error.kind()),
        }),
    }
}

/// Every stored persona, by id. A missing folder has none; a file that does not read is the
/// error of its id. A name that is not `<id>.persona.json` is not a persona file and is left out.
pub fn list_personas(directory: &Path) -> Result<Vec<Persona>, PersonaError> {
    let unreadable = |error: std::io::Error| PersonaError::FolderUnreadable {
        cause: IoCause::from_kind(error.kind()),
    };
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(unreadable(error)),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let name = entry.map_err(unreadable)?.file_name();
        let stem = name
            .to_str()
            .and_then(|name| name.strip_suffix(PERSONA_SUFFIX));
        if let Some(Ok(id)) = stem.map(PersonaId::new) {
            ids.push(id);
        }
    }
    ids.sort_by(|first, second| first.as_str().cmp(second.as_str()));
    let mut personas = Vec::new();
    for id in &ids {
        // A persona removed since the listing began is simply no longer there.
        personas.extend(read_persona(directory, id)?);
    }
    Ok(personas)
}

/// The backfills committed to `id`, oldest first; none when it has no record file. A record file
/// that does not read is a typed error and is never replaced.
pub fn read_backfills(
    directory: &Path,
    id: &PersonaId,
) -> Result<Vec<BackfillRecord>, PersonaError> {
    let text = match std::fs::read_to_string(directory.join(backfills_file_name(id))) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(PersonaError::BackfillsUnreadable {
                id: id.to_string(),
                cause: IoCause::from_kind(error.kind()),
            });
        }
    };
    serde_json::from_str(&text).map_err(|error| PersonaError::BackfillsInvalid {
        id: id.to_string(),
        cause: BodyError::from_json(&error),
    })
}

/// Writes `records` as the record file of `id`, whole or not at all.
fn write_backfills(
    directory: &Path,
    id: &PersonaId,
    records: &[BackfillRecord],
) -> Result<(), IoCause> {
    let text = serde_json::to_string_pretty(records)
        .map(|json| ascii_only(&json))
        .map_err(|_| IoCause::InvalidData)?;
    crate::atomic_file::write_atomically(directory, &backfills_file_name(id), text.as_bytes())
        .map_err(|error| IoCause::from_kind(error.kind()))
}

/// Applies `record`'s edits to the stored persona of `id` and records the backfill.
pub(super) fn commit_backfill(
    directory: &Path,
    id: &PersonaId,
    record: BackfillRecord,
) -> Result<(Persona, u64), PersonaError> {
    commit_backfill_with(directory, id, record, write_backfills)
}

/// [`commit_backfill`] with the writer of the record file given, so a test can make it fail.
///
/// All of it is under the persona write lock, against the persona as it is now: a change made
/// while the question was asked is kept, an id that was taken meanwhile is refused, and the
/// privacy guard sees the persona being committed. `record.sequence` is assigned here: one more
/// than the highest in the record file. The record file is read before anything is written, so
/// one that does not read stops the backfill. The persona is written first and its record after;
/// if the record cannot be written the persona file is put back as it was (best effort) and the
/// error says whether that worked.
pub(super) fn commit_backfill_with(
    directory: &Path,
    id: &PersonaId,
    mut record: BackfillRecord,
    write_record: impl FnOnce(&Path, &PersonaId, &[BackfillRecord]) -> Result<(), IoCause>,
) -> Result<(Persona, u64), PersonaError> {
    let _writing = PERSONA_WRITES.lock();
    let current = read_persona(directory, id)?
        .ok_or_else(|| PersonaError::UnknownPersona { id: id.to_string() })?;
    let mut records = read_backfills(directory, id)?;
    if copies_private_detail_into_public_text(&current, &record.edits) {
        return Err(PersonaError::PrivateDetailInPublicField);
    }
    let updated = current
        .edit(&record.edits)
        .map_err(|cause| PersonaError::InvalidEdit { cause })?;
    let sequence = records
        .iter()
        .map(|seen| seen.sequence)
        .max()
        .map_or(Some(1), |highest| highest.checked_add(1))
        .ok_or_else(|| PersonaError::BackfillSequenceExhausted { id: id.to_string() })?;
    record.sequence = sequence;
    write_persona(directory, &updated)?;
    records.push(record);
    if let Err(cause) = write_record(directory, id, &records) {
        let persona_restored = write_persona(directory, &current).is_ok();
        return Err(PersonaError::BackfillsUnwritable {
            id: id.to_string(),
            cause,
            persona_restored,
        });
    }
    Ok((updated, sequence))
}

/// The summary of each of `names` that has a stored persona, in the order given. A name that is
/// not a persona id has none: a cast name is a layout label, which has the persona id grammar (a
/// test keeps the two in step). A persona file that does not read is left out and recorded as
/// `PERSONA unreadable id=<name> error=<key>`, so a damaged file never stops a cast.
pub(crate) async fn summaries_of(app: &AppState, names: Vec<String>) -> Vec<PersonaSummary> {
    let directory = persona_directory(app);
    let read = run_blocking(
        move || {
            let mut summaries = Vec::new();
            let mut damaged = Vec::new();
            for name in names {
                let Ok(id) = PersonaId::new(&name) else {
                    continue;
                };
                match read_persona(&directory, &id) {
                    Ok(Some(persona)) => summaries.push(summary_of(&persona)),
                    Ok(None) => {}
                    Err(error) => damaged.push((name, error.key())),
                }
            }
            Ok((summaries, damaged))
        },
        PersonaError::Stopped,
    )
    .await;
    match read {
        Ok((summaries, damaged)) => {
            for (name, key) in damaged {
                app.record_line(format!("PERSONA unreadable id={name} error={key}"));
            }
            summaries
        }
        Err(error) => {
            app.record_line(format!("PERSONA unreadable error={}", error.key()));
            Vec::new()
        }
    }
}
