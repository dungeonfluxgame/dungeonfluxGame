//! The inventory on the shared screen: the rows of one owner's inventory and what the newest
//! application of changes moved, pushed to the TV as the `inventory` overlay slot
//! (`df_playground::inventory::InventoryPanel`), so a supply that is used up is seen going.
//!
//! The panel is hidden until `world inventory show <owner>` shows it for one owner, and
//! `world inventory hide` takes it away. While it is shown, every application of changes to that
//! owner (the judged line, the explicit routes, a trade, a transfer) rebuilds it from the stored
//! inventory and carries that application's moves, so the page can flash them. The host decides
//! everything the page draws: the rows and their order (the numbered families first, then
//! supplies, then kept things), a numbered family of two or more items (ids ending in
//! `_<number>`) collapsed into one row, a row at zero kept for as long as the panel is shown, and
//! the title. Rows past the screen's room fold into one row, the lowest counts first, never a
//! family.
//! Nothing here reads a name or a reason: grouping reads only the id grammar's number suffix, and
//! a move's kind is the change's own.
//!
//! Audit lines: `WORLD inventory_shown owner=.. rows=..`, `WORLD inventory_hidden`,
//! `WORLD inventory_followed owner=.. rows=.. moves=..` and, when the screen refuses a push,
//! `WORLD inventory_screen_refused owner=.. status=..`: ids and counts only.

use df_playground::inventory::{FLASH_MS, InventoryPanel, InventoryRow, MoveKind, RowMove, RowUse};
use df_playground::scene::SceneChange;
use serde::{Deserialize, Serialize};

use super::operations::get;
use super::request::InventoryError;
use super::store::inventory_directory;
use super::{ChangeKind, InventoryApplied, InventoryItem, ItemUse};
use crate::blocking::run_blocking;
use crate::playground::{SceneApplyError, apply_scene_changes};
use crate::state::AppState;

/// The header of a bag whose owner has no persona to name it.
const BARE_TITLE: &str = "Satchel";
/// A numbered family with at least this many live items collapses into one row.
const FAMILY_FROM: usize = 2;
/// The most rows the panel draws: two columns of eight.
const MAXIMUM_ROWS: usize = 16;
/// The row that holds what did not fit in the rows.
const OTHERS_ID: &str = "others_*";
const OTHERS_NAME: &str = "Other items";

/// `POST /dev/world/inventory/show`: whose inventory the shared screen shows from now on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowRequest {
    pub owner: String,
}

/// What `show` and `hide` answer: whose inventory the screen shows now (none after `hide`) and
/// how many rows it draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screen {
    pub owner: Option<String>,
    pub rows: usize,
}

/// The stem of a numbered family: the id without its `_<number>` or `-<number>` end.
fn family_stem(id: &str) -> Option<&str> {
    let (stem, number) = id.rsplit_once(['_', '-'])?;
    (!stem.is_empty() && !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(stem)
}

/// The id of the row of the family `stem`. `*` is not in the id grammar, so it is never an item.
fn family_id(stem: &str) -> String {
    format!("{stem}_*")
}

/// The name of the row of the family `stem`: the stem as words, capitalized, in the plural.
fn family_name(stem: &str) -> String {
    let words = stem.replace(['_', '-'], " ");
    let mut letters = words.chars();
    let head: String = letters
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    let name = format!("{head}{}", letters.as_str());
    if name.ends_with('s') {
        name
    } else {
        format!("{name}s")
    }
}

fn row_use(usage: ItemUse) -> RowUse {
    match usage {
        ItemUse::Consumed => RowUse::Consumed,
        ItemUse::Kept => RowUse::Kept,
    }
}

fn move_kind(kind: ChangeKind) -> MoveKind {
    match kind {
        ChangeKind::Add => MoveKind::Add,
        ChangeKind::Remove => MoveKind::Remove,
        ChangeKind::Consume => MoveKind::Consume,
        ChangeKind::TransferOut => MoveKind::TransferOut,
        ChangeKind::TransferIn => MoveKind::TransferIn,
    }
}

/// Whose panel it is and how it is worded and stamped.
pub(super) struct Basis<'a> {
    pub owner: &'a str,
    pub title: &'a str,
    pub serial: u64,
}

/// The panel of `items`: the rows (items the owner has at least one of, those the panel already
/// listed, and those that ran out in `applied`), grouped, ordered and cut to the screen, and the
/// moves `applied` made to them.
pub(super) fn panel_of(
    items: &[InventoryItem],
    basis: &Basis<'_>,
    previous: Option<&InventoryPanel>,
    applied: &[InventoryApplied],
) -> InventoryPanel {
    let listed =
        |id: &str| previous.is_some_and(|before| before.rows.iter().any(|row| row.id == id));
    let mut families: Vec<&str> = Vec::new();
    for stem in items.iter().filter_map(|item| family_stem(&item.id)) {
        let live = items
            .iter()
            .filter(|item| item.count > 0 && family_stem(&item.id) == Some(stem))
            .count();
        if !families.contains(&stem) && (live >= FAMILY_FROM || listed(&family_id(stem))) {
            families.push(stem);
        }
    }
    let grouped = |id: &str| family_stem(id).is_some_and(|stem| families.contains(&stem));
    let ran_out = |id: &str| {
        applied
            .iter()
            .any(|taken| taken.owner == basis.owner && taken.item == id && taken.before > 0)
    };
    let mut rows: Vec<InventoryRow> = items
        .iter()
        .filter(|item| !grouped(&item.id))
        .filter(|item| item.count > 0 || listed(&item.id) || ran_out(&item.id))
        .map(|item| InventoryRow {
            id: item.id.clone(),
            name: item.name.clone(),
            count: item.count,
            usage: row_use(item.usage),
        })
        .collect();
    // Supplies first, then the things that are kept; each keeps the order the owner holds them in.
    rows.sort_by_key(|row| row.usage == RowUse::Kept);
    let mut shown: Vec<InventoryRow> = families
        .iter()
        .map(|stem| {
            let members = items
                .iter()
                .filter(|item| family_stem(&item.id) == Some(*stem));
            InventoryRow {
                id: family_id(stem),
                name: family_name(stem),
                count: members.clone().map(|item| item.count).sum(),
                usage: members
                    .map(|item| row_use(item.usage))
                    .next()
                    .unwrap_or(RowUse::Kept),
            }
        })
        .collect();
    let others = fold_to_fit(&mut rows, MAXIMUM_ROWS.saturating_sub(shown.len()));
    shown.append(&mut rows);
    shown.extend(others);
    let rows = shown;
    let fresh = moves_of(&rows, (basis.owner, basis.serial), applied, &families);
    let moves = carried(previous, &rows, basis.serial, fresh);
    InventoryPanel {
        owner: basis.owner.to_owned(),
        title: basis.title.to_owned(),
        rows,
        serial: basis.serial,
        moves,
    }
}

/// Cuts `rows` to `room` by folding the rows with the lowest counts into one "other items" row,
/// which it returns (and which takes the last of the room). A row at zero folds last: it is what
/// ran out. Of equal counts the later row folds first. Nothing folds when the rows fit.
fn fold_to_fit(rows: &mut Vec<InventoryRow>, room: usize) -> Option<InventoryRow> {
    if rows.len() <= room {
        return None;
    }
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&index| {
        (
            rows[index].count == 0,
            rows[index].count,
            usize::MAX - index,
        )
    });
    let mut folded: Vec<usize> = order
        .into_iter()
        .take(rows.len() - room.saturating_sub(1))
        .collect();
    folded.sort_unstable_by(|first, second| second.cmp(first));
    let count = folded.iter().map(|&index| rows.remove(index).count).sum();
    Some(InventoryRow {
        id: OTHERS_ID.to_owned(),
        name: OTHERS_NAME.to_owned(),
        count,
        usage: RowUse::Kept,
    })
}

/// The moves of the panel before that are still flashing at `now` (its page shows a move for
/// `FLASH_MS`), joined with the `fresh` ones: two applications close together both flash. A row
/// moved by both keeps the count it had before the earlier and the one it has now, stamped with
/// the later; a row that is back where it began, or is gone, does not flash.
fn carried(
    previous: Option<&InventoryPanel>,
    rows: &[InventoryRow],
    now: u64,
    fresh: Vec<RowMove>,
) -> Vec<RowMove> {
    let count_of = |row: &str| {
        rows.iter()
            .find(|shown| shown.id == row)
            .map(|shown| shown.count)
    };
    let mut merged: Vec<RowMove> = previous
        .into_iter()
        .flat_map(|before| before.moves.iter())
        .filter(|moved| moved.at_ms + FLASH_MS > now)
        .filter_map(|moved| {
            Some(RowMove {
                after: count_of(&moved.row)?,
                ..moved.clone()
            })
        })
        .collect();
    for moved in fresh {
        match merged.iter().position(|older| older.row == moved.row) {
            Some(index) => {
                let before = merged[index].before;
                merged[index] = RowMove { before, ..moved };
            }
            None => merged.push(moved),
        }
    }
    merged.retain(|moved| moved.before != moved.after);
    merged
}

/// What `applied` did to the rows: for each row it touched, the count it had before and the count
/// it has now (a family's changes are added together), in the order they were taken.
fn moves_of(
    rows: &[InventoryRow],
    (owner, at_ms): (&str, u64),
    applied: &[InventoryApplied],
    families: &[&str],
) -> Vec<RowMove> {
    let mut nets: Vec<(String, i128, ChangeKind)> = Vec::new();
    for taken in applied.iter().filter(|taken| taken.owner == owner) {
        let row = family_stem(&taken.item)
            .filter(|stem| families.contains(stem))
            .map_or_else(|| taken.item.clone(), family_id);
        let delta = i128::from(taken.after) - i128::from(taken.before);
        match nets.iter_mut().find(|(id, _, _)| *id == row) {
            Some((_, net, _)) => *net += delta,
            None => nets.push((row, delta, taken.kind)),
        }
    }
    nets.into_iter()
        .filter_map(|(row, net, kind)| {
            let after = rows.iter().find(|shown| shown.id == row)?.count;
            let before = u64::try_from((i128::from(after) - net).max(0)).ok()?;
            (before != after).then(|| RowMove {
                row,
                before,
                after,
                kind: move_kind(kind),
                at_ms,
            })
        })
        .collect()
}

/// The panel the screen shows now, when it shows one.
fn shown(app: &AppState) -> Option<InventoryPanel> {
    app.playground()
        .snapshot()
        .scene
        .inventory
        .set_value()
        .cloned()
}

/// A serial after the panel before it and after the host's clock.
fn next_serial(app: &AppState, previous: Option<&InventoryPanel>) -> u64 {
    previous
        .map_or(0, |before| before.serial + 1)
        .max(app.playground().clock_ms())
}

async fn title_of(app: &AppState, owner: &str) -> String {
    crate::persona::summaries_of(app, vec![owner.to_owned()])
        .await
        .into_iter()
        .next()
        .map_or_else(|| BARE_TITLE.to_owned(), |summary| summary.display_name)
}

impl InventoryError {
    fn screen(error: &SceneApplyError) -> Self {
        Self::ScreenRefused {
            detail: error.message.clone(),
        }
    }
}

/// Shows the inventory of `owner` on the shared screen, with no moves.
pub(super) async fn show(app: &AppState, owner: &str) -> Result<Screen, InventoryError> {
    let directory = inventory_directory(app);
    let name = owner.to_owned();
    let view = run_blocking(move || get(&directory, &name), InventoryError::Stopped).await?;
    let title = title_of(app, owner).await;
    let basis = Basis {
        owner,
        title: &title,
        serial: next_serial(app, shown(app).as_ref()),
    };
    let panel = panel_of(&view.inventory.items, &basis, None, &[]);
    let rows = panel.rows.len();
    apply_scene_changes(app, vec![SceneChange::Inventory { panel }])
        .await
        .map_err(|error| InventoryError::screen(&error))?;
    app.record_line(format!("WORLD inventory_shown owner={owner} rows={rows}"));
    Ok(Screen {
        owner: Some(owner.to_owned()),
        rows,
    })
}

/// Takes the inventory panel off the shared screen.
pub(super) async fn hide(app: &AppState) -> Result<Screen, InventoryError> {
    apply_scene_changes(app, vec![SceneChange::ClearInventory {}])
        .await
        .map_err(|error| InventoryError::screen(&error))?;
    app.record_line("WORLD inventory_hidden".to_owned());
    Ok(Screen {
        owner: None,
        rows: 0,
    })
}

/// Brings the panel up to date after `applied` changed `owner`'s inventory, which now holds
/// `items`, when the screen shows that owner. A screen that refuses it loses nothing the store
/// keeps: the next application builds the panel again.
pub async fn follow(
    app: &AppState,
    owner: &str,
    items: &[InventoryItem],
    applied: &[InventoryApplied],
) {
    let Some(previous) = shown(app).filter(|panel| panel.owner == owner) else {
        return;
    };
    let basis = Basis {
        owner,
        title: &previous.title,
        serial: next_serial(app, Some(&previous)),
    };
    let panel = panel_of(items, &basis, Some(&previous), applied);
    let line = format!(
        "WORLD inventory_followed owner={owner} rows={} moves={}",
        panel.rows.len(),
        panel.moves.len()
    );
    match apply_scene_changes(app, vec![SceneChange::Inventory { panel }]).await {
        Ok(_) => app.record_line(line),
        Err(error) => app.record_line(format!(
            "WORLD inventory_screen_refused owner={owner} status={}",
            error.status.as_u16()
        )),
    }
}

/// `follow` for the changes a judged line made to `owner`, whose inventory is read from the store.
pub async fn follow_judged(app: &AppState, owner: &str, applied: &[InventoryApplied]) {
    if applied.is_empty() || shown(app).is_none_or(|panel| panel.owner != owner) {
        return;
    }
    follow(app, owner, &crate::world::inventory_of(app, owner), applied).await;
}
