//! The inventory panel on the shared screen: what one owner holds as small rows with a count, and
//! how the newest change moved those counts, so a supply that is used up is seen going.
//!
//! The host builds the whole panel (the rows, their order, the grouping of a numbered family into
//! one row, the title, which rows stay listed at zero) and the page only draws it with the kit's
//! list (`kit::ledger`); a row's look (struck at zero, a last-one tint, a flash for the change) is
//! read from the row's numbers here and decides nothing about the game. This module holds the data,
//! its range checks and those looks; `view::inventory` draws it.

use serde::{Deserialize, Serialize};

use crate::checks::labelled;
use crate::controls::{FrameMoment, FrameReason, PlateTimer};
use crate::fixture::PlateKind;
use crate::kit::ledger::{Ledger, LedgerItem, LedgerKind};
use crate::scene::SceneError;

/// How long a row's flash and its change tag stay on screen, in milliseconds. The theme's
/// animation (`inv-flash`, `inv-tag`) runs for this long.
pub const FLASH_MS: u64 = 4_000;

/// How a row behaves when it is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowUse {
    /// Using it uses it up: a supply.
    Consumed,
    /// Using it does not use it up: a tool or a possession.
    Kept,
}

/// How a count moved, as the host's inventory worded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveKind {
    Add,
    Remove,
    Consume,
    TransferOut,
    TransferIn,
}

impl MoveKind {
    /// The word that ends the row's `data-kind`, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Consume => "consume",
            Self::TransferOut => "transfer_out",
            Self::TransferIn => "transfer_in",
        }
    }
}

/// One row of the panel: an item, or a numbered family of items the host collapsed into one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryRow {
    pub id: String,
    pub name: String,
    pub count: u64,
    pub usage: RowUse,
}

/// How one row's count moved in the newest application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowMove {
    /// The row (`InventoryRow::id`).
    pub row: String,
    pub before: u64,
    pub after: u64,
    pub kind: MoveKind,
    /// The host's clock in milliseconds when the move was made (the later one, for moves of one
    /// row the host merged), so the page tells a move it has flashed from a new one.
    pub at_ms: u64,
}

/// The panel: whose, what they hold and what just changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryPanel {
    /// The owner's inventory id, which tells the host whose changes this panel follows.
    pub owner: String,
    /// The header: the owner's display name, or a plain word for a bag with no name.
    pub title: String,
    /// The rows in the order they are drawn. A row at zero stays listed, struck.
    pub rows: Vec<InventoryRow>,
    /// The host's clock in milliseconds when the panel was last built; it rises with every
    /// application.
    pub serial: u64,
    /// What the applications of the last `FLASH_MS` changed, merged by row, oldest first; empty
    /// when nothing moved lately. The page flashes each move once, for `FLASH_MS` from when it
    /// first showed it.
    #[serde(default)]
    pub moves: Vec<RowMove>,
}

/// Whether a full-frame picture is up that the panel would cover the point of: an item's close-up
/// insert (the backdrop is a close-up, or the newest frame is an inspection or focus insert that is
/// on screen), a revealed picture, or the day's show plate. A room plate, a declared plate and the
/// plates of a walk keep the panel. The page fades the panel out while this holds and back when the
/// room returns.
pub fn picture_is_up(plate: &PlateTimer, backdrop: Option<PlateKind>) -> bool {
    matches!(backdrop, Some(PlateKind::CloseUp | PlateKind::Show))
        || plate.last.as_ref().is_some_and(|last| {
            last.moment == FrameMoment::Shown
                && matches!(
                    last.reason,
                    FrameReason::Reveal | FrameReason::Inspection | FrameReason::Focus
                )
        })
}

/// How a row's count moved, for its flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flash {
    /// The count fell by this much.
    Spent(u64),
    /// The count rose by this much.
    Gained(u64),
}

impl Flash {
    /// The tag beside the count: "-1" or "+3".
    pub fn tag(self) -> String {
        match self {
            Self::Spent(by) => format!("-{by}"),
            Self::Gained(by) => format!("+{by}"),
        }
    }

    /// The row's class for the flash.
    pub fn class(self) -> &'static str {
        match self {
            Self::Spent(_) => "inv-spent",
            Self::Gained(_) => "inv-gained",
        }
    }
}

/// A move whose flash is on: the row, how it moved and how long ago the page began to show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flashing {
    pub row: String,
    pub flash: Flash,
    pub age_ms: u64,
}

/// When the page first showed each move, so a flash lasts `FLASH_MS` from then: it neither starts
/// again when the panel is drawn again nor ends early when a newer push replaces the panel. A move
/// that arrives while the panel is not visible (a full-frame picture is up) starts when the panel
/// returns, so it is seen for the whole time.
#[derive(Debug, Default)]
pub struct FlashClock {
    first_shown: Vec<((String, u64), u64)>,
}

impl FlashClock {
    /// The moves of `panel` that are flashing at `now_ms` (the page's clock); `visible` is whether
    /// the panel can be seen now. The moves the panel no longer carries are forgotten.
    pub fn flashing(
        &mut self,
        panel: &InventoryPanel,
        now_ms: u64,
        visible: bool,
    ) -> Vec<Flashing> {
        let keys: Vec<(String, u64)> = panel
            .moves
            .iter()
            .map(|moved| (moved.row.clone(), moved.at_ms))
            .collect();
        self.first_shown.retain(|(key, _)| keys.contains(key));
        let mut on = Vec::new();
        for (moved, key) in panel.moves.iter().zip(keys) {
            let first = match self.first_shown.iter().find(|(seen, _)| *seen == key) {
                Some((_, first)) => *first,
                None if visible => {
                    self.first_shown.push((key, now_ms));
                    now_ms
                }
                None => continue,
            };
            let age_ms = now_ms.saturating_sub(first);
            let flash = match moved.after.cmp(&moved.before) {
                std::cmp::Ordering::Less => Flash::Spent(moved.before - moved.after),
                std::cmp::Ordering::Greater => Flash::Gained(moved.after - moved.before),
                std::cmp::Ordering::Equal => continue,
            };
            if age_ms < FLASH_MS {
                on.push(Flashing {
                    row: moved.row.clone(),
                    flash,
                    age_ms,
                });
            }
        }
        on
    }
}

/// How a row is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowLook {
    /// The count is zero: the row stays listed, struck and dimmed.
    pub out: bool,
    /// One of a supply is left: an amber tint.
    pub last: bool,
    /// The count moved lately and the flash is still on: how it moved.
    pub flash: Option<Flash>,
    /// How long the flash has been on, so a row drawn again carries on from there.
    pub age_ms: u64,
}

impl RowLook {
    /// The classes the row carries on top of the kit's, space separated and empty for a plain row.
    pub fn classes(self) -> String {
        let mut classes = Vec::new();
        if self.out {
            classes.push("inv-out");
        }
        if self.last {
            classes.push("inv-last");
        }
        if let Some(flash) = self.flash {
            classes.push(flash.class());
        }
        classes.join(" ")
    }
}

impl InventoryPanel {
    /// The kit's list of the rows: a framed loot list under the title, each row `x<count>`.
    pub fn ledger(&self) -> Ledger {
        Ledger {
            title: Some(self.title.clone()),
            icon: None,
            kind: LedgerKind::Loot,
            items: self
                .rows
                .iter()
                .map(|row| LedgerItem {
                    id: row.id.clone(),
                    name: row.name.clone(),
                    quantity: Some(format!("x{}", row.count)),
                    price: None,
                    picture: None,
                    icon: None,
                    rarity: None,
                    taken: false,
                    highlight: false,
                })
                .collect(),
            framed: true,
        }
    }

    /// How the row `id` is drawn; `flashing` are the moves whose flash is on (`FlashClock`).
    pub fn look_of(&self, id: &str, flashing: &[Flashing]) -> RowLook {
        let Some(row) = self.rows.iter().find(|row| row.id == id) else {
            return RowLook::default();
        };
        let on = flashing.iter().find(|on| on.row == id);
        RowLook {
            out: row.count == 0,
            last: row.count == 1 && row.usage == RowUse::Consumed,
            flash: on.map(|on| on.flash),
            age_ms: on.map_or(0, |on| on.age_ms),
        }
    }

    /// Checks the panel the way a live change is checked.
    pub fn check(&self) -> Result<(), SceneError> {
        labelled("the inventory's owner", &self.owner)?;
        labelled("the inventory's title", &self.title)?;
        for (index, row) in self.rows.iter().enumerate() {
            labelled("an inventory row's id", &row.id)?;
            labelled("an inventory row's name", &row.name)?;
            if self.rows[..index].iter().any(|other| other.id == row.id) {
                return Err(SceneError::DuplicateRow(row.id.clone()));
            }
        }
        if let Some(moved) = self
            .moves
            .iter()
            .find(|moved| self.rows.iter().all(|row| row.id != moved.row))
        {
            return Err(SceneError::UnknownRow(moved.row.clone()));
        }
        Ok(())
    }
}
