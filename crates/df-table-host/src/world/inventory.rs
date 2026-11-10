//! The standard inventory of the dev host: what any owner holds, as items with a count, for any
//! number of owners. An owner is a character (player or NPC), a party's stash, a container or a
//! shop, told apart by its [`OwnerKind`] and nothing else. The host keeps one file per owner and an
//! index of owners (`store`), reads them again on every call, and writes them whole or not at all.
//!
//! The contract is typed and keyed by an owner id (`df_content::world_bible::is_valid_id`, the id
//! grammar of the world bible; item ids use it too, so an item may name a bible item). An item is
//! `{id, name, look, count}`: a stable id, its name, how it looks (its reference sheet in words)
//! and a count, never negative. Every operation is one atomic batch of [`InventoryChange`]s for one
//! owner, or a transfer between two: all of it is taken, or none, and a refusal is a typed
//! [`InventoryError`] with a stable key, never a panic.
//!
//! The pure rules are [`apply_changes`] and [`transfer`]: they validate against the owner's
//! inventory and give the new inventories and what each change came to ([`InventoryApplied`]);
//! `store` writes them. A model's judgement of a spoken line comes in as [`InventoryDelta`]s (an
//! item and a signed change), which [`changes_of`] turns into the same changes, so the judged path
//! and the explicit one are one path with one validation.
//!
//! An item's `use` says how it behaves. `consumed` (the default) is used up by use; `kept` is a
//! tool or possession that using does not use up. Code enforces it: a `consume` of a kept item is
//! refused, and a kept item's count falls only through a `remove` (it wore out, broke, was lost or
//! was given away). A judged line says which in a closed [`InventoryReason`].
//!
//! Every item has a reference sheet in the media library, `item-<owner id>-<item id>.png`
//! ([`sheet_name`]), so every picture draws it the same; one that is missing is asked for when a
//! picture shows the item.
//!
//! Where this belongs later: `df_model::checkpoint::InventoryItem` (`item`, `owner`, `quantity`,
//! the rule source, origin and attunement) is the rules-owned record. The dev host keeps the same
//! owner and item ids, and `count` is its `quantity`; the source, origin and attunement are the
//! rules owner's to add when the engine takes this over.

pub mod cli;
mod operations;
pub mod request;
pub mod routes;
mod store;

use std::fmt;

use df_content::world_bible::is_valid_id;
use serde::{Deserialize, Serialize};

pub use self::request::InventoryError;
use self::routes::{record_changed, record_refused};
use crate::state::AppState;

/// The version of the stored shape, written in every owner file and the index.
pub const SCHEMA: u32 = 1;

/// What an owner is. Nothing else differs between a character, a stash, a container and a shop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerKind {
    Character,
    Party,
    Container,
    Shop,
}

impl OwnerKind {
    pub const ALL: [Self; 4] = [Self::Character, Self::Party, Self::Container, Self::Shop];

    pub fn key(self) -> &'static str {
        match self {
            Self::Character => "character",
            Self::Party => "party",
            Self::Container => "container",
            Self::Shop => "shop",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// How an item behaves when it is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemUse {
    /// Using it uses it up (a supply, a coin spent).
    #[default]
    Consumed,
    /// Using it does not use it up (a tool, a possession); its count falls only when it is lost,
    /// broken, worn out or given away.
    Kept,
}

impl ItemUse {
    pub const ALL: [Self; 2] = [Self::Consumed, Self::Kept];

    pub fn key(self) -> &'static str {
        match self {
            Self::Consumed => "consumed",
            Self::Kept => "kept",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|usage| usage.key() == key)
    }
}

/// One item an owner holds: a stable id, a name, how it looks (its reference sheet in words), how
/// it behaves when used and how many there are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryItem {
    pub id: String,
    pub name: String,
    pub look: String,
    #[serde(default, rename = "use")]
    pub usage: ItemUse,
    pub count: u64,
}

/// Everything one owner holds. An item whose count reaches zero stays listed, so its id and words
/// are kept for when it is gained again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub schema: u32,
    pub owner: String,
    pub kind: OwnerKind,
    pub items: Vec<InventoryItem>,
}

/// A change to one item of an owner. `why` is a few words for the history; `Add` of an item the
/// owner does not hold yet needs `name` and `look` (and may say its `use`; an item that is held
/// keeps its own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum InventoryChange {
    /// The owner gains `count` of the item.
    Add {
        item: String,
        count: u64,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        look: Option<String>,
        #[serde(default, rename = "use")]
        usage: Option<ItemUse>,
        #[serde(default)]
        why: String,
    },
    /// `count` of the item leaves the owner (given away, lost, sold, dropped, broken, worn out).
    Remove {
        item: String,
        count: u64,
        #[serde(default)]
        why: String,
    },
    /// `count` of the item is used up. An item whose use is kept cannot be used up.
    Consume {
        item: String,
        count: u64,
        #[serde(default)]
        why: String,
    },
}

impl InventoryChange {
    /// The item the change is to.
    pub fn item(&self) -> &str {
        match self {
            Self::Add { item, .. } | Self::Remove { item, .. } | Self::Consume { item, .. } => item,
        }
    }

    fn count(&self) -> u64 {
        match self {
            Self::Add { count, .. } | Self::Remove { count, .. } | Self::Consume { count, .. } => {
                *count
            }
        }
    }

    fn why(&self) -> &str {
        match self {
            Self::Add { why, .. } | Self::Remove { why, .. } | Self::Consume { why, .. } => why,
        }
    }

    fn kind(&self) -> ChangeKind {
        match self {
            Self::Add { .. } => ChangeKind::Add,
            Self::Remove { .. } => ChangeKind::Remove,
            Self::Consume { .. } => ChangeKind::Consume,
        }
    }
}

/// How an item's count moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Add,
    Remove,
    Consume,
    TransferOut,
    TransferIn,
}

impl ChangeKind {
    pub const ALL: [Self; 5] = [
        Self::Add,
        Self::Remove,
        Self::Consume,
        Self::TransferOut,
        Self::TransferIn,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Consume => "consume",
            Self::TransferOut => "transfer_out",
            Self::TransferIn => "transfer_in",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// One change as it was taken: whose item, how, and the count before and after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryApplied {
    pub owner: String,
    pub item: String,
    pub kind: ChangeKind,
    pub before: u64,
    pub after: u64,
    pub why: String,
}

/// Why a judged line moved an item, a closed list. A gain is `gained` (bought, found, given,
/// received) and nothing else; a loss is any of the others: `used_up` (spent or applied until
/// gone), `worn_out` (it wore out with use), `broken`, `lost` or `given_away`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryReason {
    Gained,
    UsedUp,
    WornOut,
    Broken,
    Lost,
    GivenAway,
}

impl InventoryReason {
    pub const ALL: [Self; 6] = [
        Self::Gained,
        Self::UsedUp,
        Self::WornOut,
        Self::Broken,
        Self::Lost,
        Self::GivenAway,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Gained => "gained",
            Self::UsedUp => "used_up",
            Self::WornOut => "worn_out",
            Self::Broken => "broken",
            Self::Lost => "lost",
            Self::GivenAway => "given_away",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.key() == key)
    }
}

/// A model's word on one item a line moves: `change` below zero for what leaves and above for what
/// is gained, the `reason`, and why in a few words. An item not held yet is proposed by a gain
/// with its `name`, `look` and `use`; for one already held they are null. Code checks it like any
/// other change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryDelta {
    pub item: String,
    pub change: i64,
    pub reason: InventoryReason,
    pub why: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub look: Option<String>,
    #[serde(default, rename = "use")]
    pub usage: Option<ItemUse>,
}

/// The change `delta` comes to: a gain is an add, being used up a consume, and any other loss a
/// remove; a change of zero is no change. The reason must fit the direction.
pub fn change_of(delta: &InventoryDelta) -> Result<Option<InventoryChange>, InventoryError> {
    if delta.change == 0 {
        return Ok(None);
    }
    let gain = delta.change > 0;
    if gain != (delta.reason == InventoryReason::Gained) {
        return Err(InventoryError::InvalidReason {
            item: delta.item.clone(),
        });
    }
    let (item, count, why) = (
        delta.item.clone(),
        delta.change.unsigned_abs(),
        delta.why.clone(),
    );
    Ok(Some(match delta.reason {
        InventoryReason::Gained => InventoryChange::Add {
            item,
            count,
            name: delta.name.clone(),
            look: delta.look.clone(),
            usage: delta.usage,
            why,
        },
        InventoryReason::UsedUp => InventoryChange::Consume { item, count, why },
        InventoryReason::WornOut
        | InventoryReason::Broken
        | InventoryReason::Lost
        | InventoryReason::GivenAway => InventoryChange::Remove { item, count, why },
    }))
}

/// `delta` as a change to `inventory`, after the changes already `accepted` in this judgement:
/// none when it changes nothing, refused when it names an item twice, does not fit its direction,
/// or would not apply.
fn judged_change(
    inventory: &Inventory,
    accepted: &[InventoryChange],
    delta: &InventoryDelta,
) -> Result<Option<InventoryChange>, InventoryError> {
    let Some(change) = change_of(delta)? else {
        return Ok(None);
    };
    if accepted.iter().any(|seen| seen.item() == change.item()) {
        return Err(InventoryError::DuplicateItem {
            item: delta.item.clone(),
        });
    }
    apply_changes(inventory, std::slice::from_ref(&change))?;
    Ok(Some(change))
}

/// What `owner` holds now, for a line's or a picture's facts; nothing when they have no inventory
/// (most characters have none), or when it cannot be had (one `WORLD refused
/// verb=inventory_read error=<key>` line).
pub fn inventory_of(app: &AppState, owner: &str) -> Vec<InventoryItem> {
    match operations::get(&store::inventory_directory(app), owner) {
        Ok(view) => view.inventory.items,
        Err(InventoryError::UnknownOwner { .. }) => Vec::new(),
        Err(error) => {
            record_refused(app, "read", &error);
            Vec::new()
        }
    }
}

/// Takes `deltas`, a model's word on what a line of `owner` moved, into their inventory, and
/// answers each change as taken. Each delta is judged on its own, in order: one code refuses (an
/// item named twice, a reason that does not fit, a kept item used up, a count below zero, a new
/// item without its words or past the bound) is one `WORLD refused verb=inventory_judged
/// error=<key>` line and does not hold back the others. The ones that stand are one atomic batch,
/// and each change taken is one `WORLD inventory_changed` line. A character with no inventory has
/// nothing to change.
pub fn take_judged_changes(
    app: &AppState,
    owner: &str,
    deltas: &[InventoryDelta],
) -> Vec<InventoryApplied> {
    if deltas.iter().all(|delta| delta.change == 0) {
        return Vec::new();
    }
    let directory = store::inventory_directory(app);
    let inventory = match store::read_inventory(&directory, owner) {
        Ok(inventory) => inventory,
        Err(InventoryError::UnknownOwner { .. }) => return Vec::new(),
        Err(error) => {
            record_refused(app, "judged", &error);
            return Vec::new();
        }
    };
    let mut changes: Vec<InventoryChange> = Vec::new();
    for delta in deltas {
        match judged_change(&inventory, &changes, delta) {
            Ok(Some(change)) => changes.push(change),
            Ok(None) => {}
            Err(error) => record_refused(app, "judged", &error),
        }
    }
    if changes.is_empty() {
        return Vec::new();
    }
    match operations::change(&directory, owner, &changes) {
        Ok(changed) => {
            record_changed(app, &changed);
            changed.applied
        }
        Err(error) => {
            record_refused(app, "judged", &error);
            Vec::new()
        }
    }
}

/// The media library's name of the reference sheet of `item`, held by `owner`.
pub fn sheet_name(owner: &str, item: &str) -> String {
    format!("item-{owner}-{item}.png")
}

impl fmt::Display for OwnerKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.key())
    }
}

/// An owner id or item id that is not one: the bible's id grammar.
fn check_id(
    id: &str,
    invalid: impl FnOnce(String) -> InventoryError,
) -> Result<(), InventoryError> {
    if is_valid_id(id) {
        Ok(())
    } else {
        Err(invalid(id.to_owned()))
    }
}

pub(super) fn check_owner(owner: &str) -> Result<(), InventoryError> {
    check_id(owner, |owner| InventoryError::InvalidOwner { owner })
}

fn check_item_id(item: &str) -> Result<(), InventoryError> {
    check_id(item, |item| InventoryError::InvalidItem { item })
}

impl Inventory {
    /// The inventory of `owner`, checked: ids in the grammar, no item twice, words not blank.
    pub fn new(
        owner: &str,
        kind: OwnerKind,
        items: Vec<InventoryItem>,
    ) -> Result<Self, InventoryError> {
        let inventory = Self {
            schema: SCHEMA,
            owner: owner.to_owned(),
            kind,
            items,
        };
        inventory.check()?;
        Ok(inventory)
    }

    /// Checks what a stored or sent inventory must hold.
    pub fn check(&self) -> Result<(), InventoryError> {
        if self.schema != SCHEMA {
            return Err(InventoryError::InvalidSchema {
                schema: self.schema,
            });
        }
        check_owner(&self.owner)?;
        for (index, item) in self.items.iter().enumerate() {
            check_item_id(&item.id)?;
            if self.items[..index].iter().any(|seen| seen.id == item.id) {
                return Err(InventoryError::DuplicateItem {
                    item: item.id.clone(),
                });
            }
            if item.name.trim().is_empty() || item.look.trim().is_empty() {
                return Err(InventoryError::MissingWords {
                    item: item.id.clone(),
                });
            }
        }
        Ok(())
    }

    /// The item `id`, if the owner holds it (a count of zero included).
    pub fn item(&self, id: &str) -> Option<&InventoryItem> {
        self.items.iter().find(|item| item.id == id)
    }

    /// The items of which the owner has at least one.
    pub fn held(&self) -> impl Iterator<Item = &InventoryItem> {
        self.items.iter().filter(|item| item.count > 0)
    }
}

fn nonblank(text: Option<&String>) -> Option<&str> {
    text.map(|text| text.trim()).filter(|text| !text.is_empty())
}

/// `inventory` after `changes`, and each change as it was taken. Refused whole, with nothing
/// changed, when the batch is empty, names an item twice, has an item that is not an id, a count of
/// zero or one that overflows, takes more of an item than the owner has or an item they do not
/// hold, or adds an item they do not hold without its words.
pub fn apply_changes(
    inventory: &Inventory,
    changes: &[InventoryChange],
) -> Result<(Inventory, Vec<InventoryApplied>), InventoryError> {
    if changes.is_empty() {
        return Err(InventoryError::EmptyBatch);
    }
    let mut after = inventory.clone();
    let mut applied = Vec::with_capacity(changes.len());
    for (index, change) in changes.iter().enumerate() {
        let id = change.item();
        check_item_id(id)?;
        if changes[..index].iter().any(|seen| seen.item() == id) {
            return Err(InventoryError::DuplicateItem {
                item: id.to_owned(),
            });
        }
        let count = change.count();
        if count == 0 {
            return Err(InventoryError::ZeroCount {
                item: id.to_owned(),
            });
        }
        let held = after.items.iter_mut().find(|item| item.id == id);
        let (before, now) = match (change, held) {
            (InventoryChange::Add { .. }, Some(item)) => {
                let before = item.count;
                item.count = before
                    .checked_add(count)
                    .ok_or_else(|| InventoryError::Overflow {
                        owner: inventory.owner.clone(),
                        item: id.to_owned(),
                    })?;
                (before, item.count)
            }
            (
                InventoryChange::Add {
                    name, look, usage, ..
                },
                None,
            ) => {
                let (Some(name), Some(look)) = (nonblank(name.as_ref()), nonblank(look.as_ref()))
                else {
                    return Err(InventoryError::MissingWords {
                        item: id.to_owned(),
                    });
                };
                after.items.push(InventoryItem {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    look: look.to_owned(),
                    usage: usage.unwrap_or_default(),
                    count,
                });
                (0, count)
            }
            (_, None) => {
                return Err(InventoryError::UnknownItem {
                    owner: inventory.owner.clone(),
                    item: id.to_owned(),
                });
            }
            (_, Some(item)) => {
                if matches!(change, InventoryChange::Consume { .. }) && item.usage == ItemUse::Kept
                {
                    return Err(InventoryError::KeptItem {
                        owner: inventory.owner.clone(),
                        item: id.to_owned(),
                    });
                }
                let before = item.count;
                item.count =
                    before
                        .checked_sub(count)
                        .ok_or_else(|| InventoryError::Insufficient {
                            owner: inventory.owner.clone(),
                            item: id.to_owned(),
                            have: before,
                            asked: count,
                        })?;
                (before, item.count)
            }
        };
        applied.push(InventoryApplied {
            owner: inventory.owner.clone(),
            item: id.to_owned(),
            kind: change.kind(),
            before,
            after: now,
            why: change.why().trim().to_owned(),
        });
    }
    Ok((after, applied))
}

/// `count` of `item` moved from `from` to `to`: both inventories after it, and what each side came
/// to. The receiver gains the item with the giver's words when it did not hold it. Refused whole
/// when the owners are the same, or the giver cannot give that much.
pub fn transfer(
    from: &Inventory,
    to: &Inventory,
    (item, count): (&str, u64),
    why: &str,
) -> Result<(Inventory, Inventory, Vec<InventoryApplied>), InventoryError> {
    if from.owner == to.owner {
        return Err(InventoryError::SameOwner {
            owner: from.owner.clone(),
        });
    }
    let taken = InventoryChange::Remove {
        item: item.to_owned(),
        count,
        why: why.to_owned(),
    };
    let (from_after, mut out) = apply_changes(from, &[taken])?;
    let words = from.item(item).ok_or_else(|| InventoryError::UnknownItem {
        owner: from.owner.clone(),
        item: item.to_owned(),
    })?;
    let given = InventoryChange::Add {
        item: item.to_owned(),
        count,
        name: Some(words.name.clone()),
        look: Some(words.look.clone()),
        usage: Some(words.usage),
        why: why.to_owned(),
    };
    let (to_after, mut received) = apply_changes(to, &[given])?;
    out[0].kind = ChangeKind::TransferOut;
    received[0].kind = ChangeKind::TransferIn;
    out.extend(received);
    Ok((from_after, to_after, out))
}
