//! What each inventory operation does to the store, with no HTTP and no logging: the routes and the
//! judged path call these, so there is one validation and one way to change an inventory.

use std::path::Path;

use df_playground::kit::ledger::{Ledger, LedgerItem, LedgerKind};

use super::request::{
    Changed, History, HistoryEntry, HistoryQuery, InventoryError, InventoryView, OwnerList,
    SetRequest, TransferRequest,
};
use super::store::{read_index, read_inventory, transact};
use super::{Inventory, InventoryChange, apply_changes, check_owner, transfer};
use crate::event_log::EventLog;

/// `inventory` with the list panel's rows of it for the page: a loot row for each item the owner
/// has at least one of, in the order they are held.
pub(super) fn view_of(inventory: Inventory) -> InventoryView {
    let ledger = Ledger {
        title: None,
        icon: None,
        kind: LedgerKind::Loot,
        items: inventory
            .held()
            .map(|item| LedgerItem {
                id: item.id.clone(),
                name: item.name.clone(),
                quantity: Some(format!("x{}", item.count)),
                price: None,
                picture: None,
                icon: None,
                rarity: None,
                taken: false,
                highlight: false,
            })
            .collect(),
        framed: true,
    };
    InventoryView { inventory, ledger }
}

/// Every owner, from the index.
pub(super) fn list(directory: &Path) -> Result<OwnerList, InventoryError> {
    Ok(OwnerList {
        owners: read_index(directory)?,
    })
}

/// The inventory of `owner`.
pub(super) fn get(directory: &Path, owner: &str) -> Result<InventoryView, InventoryError> {
    check_owner(owner)?;
    read_inventory(directory, owner).map(view_of)
}

/// Makes `request.items` the whole inventory of `request.owner`, which is made when it is new.
pub(super) fn set(directory: &Path, request: SetRequest) -> Result<Changed, InventoryError> {
    let inventory = Inventory::new(&request.owner, request.kind, request.items)?;
    transact(directory, &[request.owner.as_str()], |_| {
        Ok((
            vec![inventory.clone()],
            Changed {
                applied: Vec::new(),
                owners: vec![view_of(inventory)],
            },
        ))
    })
}

/// `changes` to the inventory of `owner`, all taken or none.
pub(super) fn change(
    directory: &Path,
    owner: &str,
    changes: &[InventoryChange],
) -> Result<Changed, InventoryError> {
    check_owner(owner)?;
    transact(directory, &[owner], |stored| {
        let Some(Some(inventory)) = stored.into_iter().next() else {
            return Err(InventoryError::UnknownOwner {
                owner: owner.to_owned(),
            });
        };
        let (after, applied) = apply_changes(&inventory, changes)?;
        Ok((
            vec![after.clone()],
            Changed {
                applied,
                owners: vec![view_of(after)],
            },
        ))
    })
}

/// `request.count` of `request.item` from one owner to another, both sides changed or neither.
pub(super) fn move_between(
    directory: &Path,
    request: &TransferRequest,
) -> Result<Changed, InventoryError> {
    check_owner(&request.from)?;
    check_owner(&request.to)?;
    if request.from == request.to {
        return Err(InventoryError::SameOwner {
            owner: request.from.clone(),
        });
    }
    transact(directory, &[&request.from, &request.to], |stored| {
        let mut stored = stored.into_iter();
        let (from, to) = (stored.next().flatten(), stored.next().flatten());
        let from = from.ok_or_else(|| InventoryError::UnknownOwner {
            owner: request.from.clone(),
        })?;
        let to = to.ok_or_else(|| InventoryError::UnknownOwner {
            owner: request.to.clone(),
        })?;
        let (from_after, to_after, applied) = transfer(
            &from,
            &to,
            (request.item.as_str(), request.count),
            &request.why,
        )?;
        Ok((
            vec![from_after.clone(), to_after.clone()],
            Changed {
                applied,
                owners: vec![view_of(from_after), view_of(to_after)],
            },
        ))
    })
}

/// The change lines the host's log still holds (`WORLD inventory_changed` and `WORLD
/// inventory_set`), oldest first: those of `query.owner` when given, the newest `query.limit`.
pub(super) fn history(log: &EventLog, query: &HistoryQuery) -> Result<History, InventoryError> {
    if let Some(owner) = &query.owner {
        check_owner(owner)?;
    }
    let mut entries: Vec<HistoryEntry> = log
        .after_sequence_in(0, usize::MAX, "WORLD")
        .into_iter()
        .filter(|line| {
            let mut words = line.text.split(' ').skip(1);
            matches!(words.next(), Some("inventory_changed" | "inventory_set"))
                && query.owner.as_ref().is_none_or(|owner| {
                    line.text
                        .split(' ')
                        .any(|word| word.strip_prefix("owner=") == Some(owner.as_str()))
                })
        })
        .map(|line| HistoryEntry {
            sequence: line.sequence,
            at_ms: line.at_ms,
            line: line.text,
        })
        .collect();
    if let Some(limit) = query.limit {
        entries.drain(..entries.len().saturating_sub(limit));
    }
    Ok(History { entries })
}
