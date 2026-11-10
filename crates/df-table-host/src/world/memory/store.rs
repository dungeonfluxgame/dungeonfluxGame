//! Where the memories live: the playground's `world/memory` folder holds `owners/<owner id>.json`
//! (one [`MemoryBook`] for each owner), `index.json` (the list of owners with how many memories
//! each keeps, of which kinds, and the next `seq`) and `galleries/<name>.json` (the members of each
//! gallery sheet composed). The books are in a folder of their own so that no owner id can be the
//! index. Files are read again on every call and written whole or not at all, so one owner's change
//! rewrites only its book and the index, and the cost of a call does not grow with the number of
//! owners (only the index, a line an owner, is read for the list).
//!
//! A change of one owner is one transaction under the memory write lock: the book is written, then
//! the index; when the index cannot be written the book is put back as it was. A crash between the
//! two leaves the owner out of the list until its next change, never a half-written file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::request::{KindCount, MemoryError, OwnerSummary};
use super::{MemoryBook, SCHEMA, check_owner};
use crate::blocking::WriteLock;
use crate::dev_http::BodyError;
use crate::io_cause::IoCause;
use crate::state::AppState;

/// The folder of the memories under the world folder.
const DIRECTORY: &str = "memory";
const BOOKS: &str = "owners";
const GALLERIES: &str = "galleries";
const INDEX_FILE: &str = "index.json";

/// Held around every transaction, so two changes never lose one another.
static MEMORY_WRITES: WriteLock = WriteLock::new();

/// The stored index: every owner, in id order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: u32,
    owners: Vec<OwnerSummary>,
}

/// The `world/memory` folder of the host's playground.
pub(super) fn memory_directory(app: &AppState) -> PathBuf {
    crate::world::store::world_directory(app).join(DIRECTORY)
}

fn book_file(owner: &str) -> String {
    format!("{owner}.json")
}

fn books_directory(directory: &Path) -> PathBuf {
    directory.join(BOOKS)
}

fn unreadable(error: &std::io::Error) -> MemoryError {
    MemoryError::StoreUnreadable {
        cause: IoCause::from_kind(error.kind()),
    }
}

fn unwritable(error: &std::io::Error) -> MemoryError {
    MemoryError::StoreUnwritable {
        cause: IoCause::from_kind(error.kind()),
    }
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, MemoryError> {
    serde_json::from_str(text).map_err(|error| MemoryError::StoreInvalid {
        cause: BodyError::from_json(&error),
    })
}

/// The text of `file` in `directory`, or none when there is no such file.
fn read_text(directory: &Path, file: &str) -> Result<Option<String>, MemoryError> {
    match std::fs::read_to_string(directory.join(file)) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(unreadable(&error)),
    }
}

/// The owners in the index, in id order; none when there is no index.
pub(super) fn read_index(directory: &Path) -> Result<Vec<OwnerSummary>, MemoryError> {
    let Some(text) = read_text(directory, INDEX_FILE)? else {
        return Ok(Vec::new());
    };
    let index: Index = parse(&text)?;
    if index.schema != SCHEMA {
        return Err(MemoryError::InvalidSchema {
            schema: index.schema,
        });
    }
    Ok(index.owners)
}

/// The book of `owner`, or none when it keeps none.
fn read_optional(directory: &Path, owner: &str) -> Result<Option<MemoryBook>, MemoryError> {
    check_owner(owner)?;
    let Some(text) = read_text(&books_directory(directory), &book_file(owner))? else {
        return Ok(None);
    };
    let book: MemoryBook = parse(&text)?;
    book.check()?;
    if book.owner != owner {
        return Err(MemoryError::InvalidOwner { owner: book.owner });
    }
    Ok(Some(book))
}

/// The book of `owner`.
pub(super) fn read_book(directory: &Path, owner: &str) -> Result<MemoryBook, MemoryError> {
    read_optional(directory, owner)?.ok_or_else(|| MemoryError::UnknownOwner {
        owner: owner.to_owned(),
    })
}

fn summary_of(book: &MemoryBook) -> OwnerSummary {
    OwnerSummary {
        owner: book.owner.clone(),
        memories: book.memories.len(),
        next_seq: book.next_seq,
        kinds: book
            .kind_counts()
            .into_iter()
            .map(|(kind, count)| KindCount { kind, count })
            .collect(),
    }
}

fn write_file<T: Serialize>(directory: &Path, file: &str, value: &T) -> Result<(), MemoryError> {
    let text = serde_json::to_string_pretty(value).map_err(|_| MemoryError::StoreUnwritable {
        cause: IoCause::InvalidData,
    })?;
    crate::atomic_file::write_atomically(directory, file, text.as_bytes())
        .map_err(|error| unwritable(&error))
}

/// One transaction over the book of `owner` (none when it keeps none yet): it goes to `change`,
/// which answers the book to store and a value; the book is written and the index follows. Nothing
/// is written when `change` refuses.
pub(super) fn transact<T>(
    directory: &Path,
    owner: &str,
    change: impl FnOnce(Option<MemoryBook>) -> Result<(MemoryBook, T), MemoryError>,
) -> Result<T, MemoryError> {
    let _writing = MEMORY_WRITES.lock();
    let stored = read_optional(directory, owner)?;
    let (book, value) = change(stored)?;
    let mut index = read_index(directory)?;
    let books = books_directory(directory);
    let file = book_file(&book.owner);
    let before = read_text(&books, &file)?;
    write_file(&books, &file, &book)?;
    let summary = summary_of(&book);
    match index.iter_mut().find(|seen| seen.owner == summary.owner) {
        Some(seen) => *seen = summary,
        None => index.push(summary),
    }
    index.sort_by(|a, b| a.owner.cmp(&b.owner));
    let index = Index {
        schema: SCHEMA,
        owners: index,
    };
    if let Err(error) = write_file(directory, INDEX_FILE, &index) {
        // A failure to put back leaves the index's own error to be reported.
        let _ = match before {
            Some(text) => crate::atomic_file::write_atomically(&books, &file, text.as_bytes()),
            None => std::fs::remove_file(books.join(&file)),
        };
        return Err(error);
    }
    Ok(value)
}

/// The members of one gallery sheet, kept beside the memories (the sheet itself is a picture of
/// the media library).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GallerySidecar {
    pub schema: u32,
    pub owner: String,
    pub picture: String,
    pub members: Vec<String>,
    pub pictures: Vec<String>,
    pub columns: u32,
    pub rows: u32,
}

/// Writes the sidecar of the gallery sheet `picture` (`<name>.json` under `galleries`).
pub(super) fn write_sidecar(
    directory: &Path,
    name: &str,
    sidecar: &GallerySidecar,
) -> Result<(), MemoryError> {
    write_file(&directory.join(GALLERIES), &format!("{name}.json"), sidecar)
}
