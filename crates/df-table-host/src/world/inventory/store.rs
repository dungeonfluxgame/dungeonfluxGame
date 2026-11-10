//! Where the inventories live: the playground's `world/inventory` folder holds one file per owner
//! (`<owner id>.json`, an [`Inventory`]) and `index.json`, the list of owners with their kind and
//! how many items and units each holds. Files are read again on every call and written whole or
//! not at all, so one owner's change never rewrites another's file and the cost of a call does not
//! grow with the number of owners (only the index, a line a owner, is read for the list).
//!
//! A change of one or two owners is one transaction under the inventory write lock: the owner
//! files are written one by one, and when a later one cannot be written the earlier ones are put
//! back as they were, so both sides of a transfer change or neither. The index is written last; a
//! crash between an owner file and the index leaves the owner out of the list until its next
//! change, never a half-written file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::request::{InventoryError, OwnerSummary};
use super::{Inventory, SCHEMA, check_owner};
use crate::blocking::WriteLock;
use crate::dev_http::BodyError;
use crate::io_cause::IoCause;
use crate::state::AppState;

/// The folder of the inventories under the world folder.
const DIRECTORY: &str = "inventory";
const INDEX_FILE: &str = "index.json";

/// Held around every transaction, so two changes never lose one another.
static INVENTORY_WRITES: WriteLock = WriteLock::new();

/// The stored index: every owner, in id order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: u32,
    owners: Vec<OwnerSummary>,
}

/// The `world/inventory` folder of the host's playground.
pub(super) fn inventory_directory(app: &AppState) -> PathBuf {
    crate::world::store::world_directory(app).join(DIRECTORY)
}

fn owner_file(owner: &str) -> String {
    format!("{owner}.json")
}

fn unreadable(error: &std::io::Error) -> InventoryError {
    InventoryError::StoreUnreadable {
        cause: IoCause::from_kind(error.kind()),
    }
}

fn unwritable(error: &std::io::Error) -> InventoryError {
    InventoryError::StoreUnwritable {
        cause: IoCause::from_kind(error.kind()),
    }
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, InventoryError> {
    serde_json::from_str(text).map_err(|error| InventoryError::StoreInvalid {
        cause: BodyError::from_json(&error),
    })
}

/// The text of `file` in `directory`, or none when there is no such file.
fn read_text(directory: &Path, file: &str) -> Result<Option<String>, InventoryError> {
    match std::fs::read_to_string(directory.join(file)) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(unreadable(&error)),
    }
}

/// The owners in the index, in id order; none when there is no index.
pub(super) fn read_index(directory: &Path) -> Result<Vec<OwnerSummary>, InventoryError> {
    let Some(text) = read_text(directory, INDEX_FILE)? else {
        return Ok(Vec::new());
    };
    let index: Index = parse(&text)?;
    if index.schema != SCHEMA {
        return Err(InventoryError::InvalidSchema {
            schema: index.schema,
        });
    }
    Ok(index.owners)
}

/// The inventory of `owner`, or none when it has none.
fn read_optional(directory: &Path, owner: &str) -> Result<Option<Inventory>, InventoryError> {
    check_owner(owner)?;
    let Some(text) = read_text(directory, &owner_file(owner))? else {
        return Ok(None);
    };
    let inventory: Inventory = parse(&text)?;
    inventory.check()?;
    if inventory.owner != owner {
        return Err(InventoryError::InvalidOwner {
            owner: inventory.owner,
        });
    }
    Ok(Some(inventory))
}

/// The inventory of `owner`.
pub(super) fn read_inventory(directory: &Path, owner: &str) -> Result<Inventory, InventoryError> {
    read_optional(directory, owner)?.ok_or_else(|| InventoryError::UnknownOwner {
        owner: owner.to_owned(),
    })
}

fn summary_of(inventory: &Inventory) -> OwnerSummary {
    OwnerSummary {
        owner: inventory.owner.clone(),
        kind: inventory.kind,
        items: inventory.items.len(),
        units: inventory
            .items
            .iter()
            .fold(0, |units, item| units.saturating_add(item.count)),
    }
}

fn write_file<T: Serialize>(directory: &Path, file: &str, value: &T) -> Result<(), InventoryError> {
    let text =
        serde_json::to_string_pretty(value).map_err(|_| InventoryError::StoreUnwritable {
            cause: IoCause::InvalidData,
        })?;
    crate::atomic_file::write_atomically(directory, file, text.as_bytes())
        .map_err(|error| unwritable(&error))
}

/// Puts `file` back as `before` had it: its old bytes, or no file when it had none. A failure to
/// put back leaves the write's own error to be reported.
fn restore(directory: &Path, file: &str, before: Option<&str>) {
    let _ = match before {
        Some(text) => crate::atomic_file::write_atomically(directory, file, text.as_bytes()),
        None => std::fs::remove_file(directory.join(file)),
    };
}

/// One transaction over `owners` (one or two, distinct): their inventories as stored (none for an
/// owner without one) go to `change`, which answers the inventories to store and a value; they are
/// written, all or none, and the index follows. Nothing is written when `change` refuses.
pub(super) fn transact<T>(
    directory: &Path,
    owners: &[&str],
    change: impl FnOnce(Vec<Option<Inventory>>) -> Result<(Vec<Inventory>, T), InventoryError>,
) -> Result<T, InventoryError> {
    let _writing = INVENTORY_WRITES.lock();
    let stored = owners
        .iter()
        .map(|owner| read_optional(directory, owner))
        .collect::<Result<Vec<_>, _>>()?;
    let (updated, value) = change(stored)?;
    let mut index = read_index(directory)?;
    let files: Vec<(String, Option<String>)> = updated
        .iter()
        .map(|inventory| {
            let file = owner_file(&inventory.owner);
            read_text(directory, &file).map(|before| (file, before))
        })
        .collect::<Result<_, _>>()?;
    for (written, (inventory, (file, _))) in updated.iter().zip(&files).enumerate() {
        if let Err(error) = write_file(directory, file, inventory) {
            put_back(directory, &files[..written]);
            return Err(error);
        }
        let summary = summary_of(inventory);
        match index.iter_mut().find(|seen| seen.owner == summary.owner) {
            Some(seen) => *seen = summary,
            None => index.push(summary),
        }
    }
    index.sort_by(|a, b| a.owner.cmp(&b.owner));
    let index = Index {
        schema: SCHEMA,
        owners: index,
    };
    if let Err(error) = write_file(directory, INDEX_FILE, &index) {
        put_back(directory, &files);
        return Err(error);
    }
    Ok(value)
}

/// Puts the files written so far back as they were, newest first.
fn put_back(directory: &Path, files: &[(String, Option<String>)]) {
    for (file, before) in files.iter().rev() {
        restore(directory, file, before.as_deref());
    }
}
