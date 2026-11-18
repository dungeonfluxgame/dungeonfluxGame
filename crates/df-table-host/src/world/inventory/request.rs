//! The bodies and replies of the inventory routes, and why they refuse.
//!
//! Every route answers an `InventoryReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`.

use std::fmt;

use axum::http::StatusCode;
use df_playground::kit::ledger::Ledger;
use df_playground::pages::wire::PagesErrorKind;
use serde::{Deserialize, Serialize};

use super::{Inventory, InventoryApplied, InventoryChange, InventoryItem, ItemUse, OwnerKind};
use crate::dev_http::{BodyError, DevReply, QueryError, RouteError};
use crate::io_cause::IoCause;

/// The answer of an inventory route: its value, or why it was refused.
pub type InventoryReply<T> = DevReply<T, InventoryError>;

/// `GET /dev/world/inventory/owner`: whose inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerQuery {
    pub owner: String,
}

/// `POST /dev/world/inventory/set`: the whole inventory of an owner, replacing what it held (and
/// making the owner when it is new).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetRequest {
    pub owner: String,
    pub kind: OwnerKind,
    pub items: Vec<InventoryItem>,
}

/// `POST /dev/world/inventory/add`: `count` more of `item` for `owner`; `name`, `look` and `use`
/// are for an item the owner does not hold yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddRequest {
    pub owner: String,
    pub item: String,
    pub count: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub look: Option<String>,
    #[serde(default, rename = "use")]
    pub usage: Option<ItemUse>,
    /// A new item's price in copper pieces.
    #[serde(default)]
    pub price_cp: Option<u64>,
    #[serde(default)]
    pub why: String,
}

/// `POST /dev/world/inventory/price`: `owner`'s `item` is priced at `price_cp` when it has no
/// price yet; a price it has is left alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceRequest {
    pub owner: String,
    pub item: String,
    pub price_cp: u64,
    #[serde(default)]
    pub why: String,
}

/// What a price request came to: the item's price now, and whether this request set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Priced {
    pub price_cp: u64,
    pub set: bool,
}

/// `POST /dev/world/inventory/remove` and `POST /dev/world/inventory/consume`: `count` of `item`
/// leaves `owner` or is used up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TakeRequest {
    pub owner: String,
    pub item: String,
    pub count: u64,
    #[serde(default)]
    pub why: String,
}

/// `POST /dev/world/inventory/batch`: several changes to one owner, all taken or none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchRequest {
    pub owner: String,
    pub changes: Vec<InventoryChange>,
}

/// `POST /dev/world/inventory/transfer`: `count` of `item` from one owner to another, both sides
/// changed or neither.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferRequest {
    pub from: String,
    pub to: String,
    pub item: String,
    pub count: u64,
    #[serde(default)]
    pub why: String,
}

/// `POST /dev/world/inventory/pay`: `amount_cp` copper's worth of `from`'s coins to `to`, with
/// exact change from `to`'s coins, both sides changed or neither.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayRequest {
    pub from: String,
    pub to: String,
    pub amount_cp: u64,
    #[serde(default)]
    pub why: String,
}

/// `POST /dev/world/inventory/till`: `owner` keeps a till that can make change (a keeper or a
/// fare-taker the scenario will pay).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TillRequest {
    pub owner: String,
}

/// The query of `GET /dev/world/inventory/history`: the changes of one owner (all owners when
/// none), the newest `limit` of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// One owner in the index: its kind and how many items and units it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerSummary {
    pub owner: String,
    pub kind: OwnerKind,
    pub items: usize,
    pub units: u64,
}

/// What `GET /dev/world/inventory` answers: every owner, in id order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerList {
    pub owners: Vec<OwnerSummary>,
}

/// An owner's inventory, and the same as the rows of the kit's list panel for the page: one loot
/// row for each item the owner has at least one of (`id`, `name`, `quantity` as `x4`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryView {
    pub inventory: Inventory,
    pub ledger: Ledger,
}

/// What a change came to: each change as taken, and the inventories it left (one owner, or two for
/// a transfer: the giver first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changed {
    pub applied: Vec<InventoryApplied>,
    pub owners: Vec<InventoryView>,
}

/// One line of the change history: when the host logged it, and the line (ids and counts only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub sequence: u64,
    pub at_ms: u64,
    pub line: String,
}

/// What `GET /dev/world/inventory/history` answers, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

/// Why an inventory route refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum InventoryError {
    InvalidBody(BodyError),
    InvalidQuery(QueryError),
    /// An owner id is not an id.
    InvalidOwner {
        owner: String,
    },
    /// An item id is not an id.
    InvalidItem {
        item: String,
    },
    /// A stored or sent inventory has a schema this host does not read.
    InvalidSchema {
        schema: u32,
    },
    /// The owner has no inventory (make it with `set`).
    UnknownOwner {
        owner: String,
    },
    /// The owner does not hold the item.
    UnknownItem {
        owner: String,
        item: String,
    },
    /// A list or batch names an item twice.
    DuplicateItem {
        item: String,
    },
    /// An item has no name or no look: a new item needs both.
    MissingWords {
        item: String,
    },
    /// A change of no items at all.
    EmptyBatch,
    /// A change or transfer of a count of zero.
    ZeroCount {
        item: String,
    },
    /// A judged reason does not fit the direction of the change (a loss that is `gained`, a gain
    /// that is not).
    InvalidReason {
        item: String,
    },
    /// A loss has no reason that stands: none given, a kept item used up, a sale with no payment in
    /// the line, a loss that is lost, broken or worn out with nothing said of what happened, or an
    /// item given to nobody present.
    UnreasonedLoss {
        item: String,
    },
    /// A gain has no valid source: none named, or its payment is not in the same line, its thing is
    /// not offered here, or its giver is not present.
    UnsourcedGain {
        item: String,
    },
    /// An item whose use is kept cannot be used up (`consume`); it leaves only by a `remove`.
    KeptItem {
        owner: String,
        item: String,
    },
    /// More is asked of an item than the owner has: the count would go below zero.
    Insufficient {
        owner: String,
        item: String,
        have: u64,
        asked: u64,
    },
    /// A count would pass the largest there is.
    Overflow {
        owner: String,
        item: String,
    },
    /// A transfer to the owner it is from.
    SameOwner {
        owner: String,
    },
    /// The shared screen refused the inventory panel (no scene is shown, or it could not be read).
    ScreenRefused {
        detail: String,
    },
    /// An inventory file could not be read.
    StoreUnreadable {
        cause: IoCause,
    },
    /// An inventory file is not the stored shape.
    StoreInvalid {
        cause: BodyError,
    },
    /// An inventory file could not be written (nothing was changed).
    StoreUnwritable {
        cause: IoCause,
    },
    Stopped,
    /// A transfer would add a coin to an item of the receiver's that is another coin, or no coin.
    CoinMismatch {
        owner: String,
        item: String,
    },
    /// The SRD equipment catalog does not list `item`, so it has no weight to arrange it by.
    NotInCatalog {
        item: String,
    },
    /// The owner is not a shop: nothing can be bought from them by a tap.
    NotAShop {
        owner: String,
    },
    /// The shop does not sell `item` by a tap: it does not stock it, or the SRD catalog does not
    /// list it (an item off the catalog is bought by talking).
    NotForSale {
        shop: String,
        item: String,
    },
    /// `count` of `item` is not a whole copper price: it is sold `per` at a time (20 sling bullets
    /// for 4 CP).
    UnevenBundle {
        item: String,
        count: u64,
        per: u32,
    },
    /// The buyer's coins come to less than the price.
    InsufficientCoin {
        owner: String,
        cost_cp: u64,
        purse_cp: u64,
    },
    /// The shop's coins cannot make the change owed exactly.
    NoChange {
        shop: String,
        change_cp: u64,
    },
    /// The buyer's containers that could hold `item` have no room for it.
    ContainerFull {
        owner: String,
        container: String,
        item: String,
    },
    /// What the owner would carry passes their carrying capacity (SRD p.178).
    OverCapacity {
        owner: String,
        milli_pounds: u64,
        capacity_milli_pounds: u64,
    },
    /// The owner has no carrying capacity to check against: no hero sheet with a Strength score
    /// and an admitted size.
    NoCarryingCapacity {
        owner: String,
    },
}

impl InventoryError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidQuery(_) => "invalid_query",
            Self::InvalidOwner { .. } => "invalid_owner",
            Self::InvalidItem { .. } => "invalid_item",
            Self::InvalidSchema { .. } => "invalid_schema",
            Self::UnknownOwner { .. } => "unknown_owner",
            Self::UnknownItem { .. } => "unknown_item",
            Self::DuplicateItem { .. } => "duplicate_item",
            Self::MissingWords { .. } => "missing_words",
            Self::EmptyBatch => "empty_batch",
            Self::ZeroCount { .. } => "zero_count",
            Self::InvalidReason { .. } => "invalid_reason",
            Self::UnsourcedGain { .. } => "unsourced_gain",
            Self::UnreasonedLoss { .. } => "unreasoned_loss",
            Self::KeptItem { .. } => "kept_item",
            Self::Insufficient { .. } => "insufficient",
            Self::Overflow { .. } => "overflow",
            Self::SameOwner { .. } => "same_owner",
            Self::ScreenRefused { .. } => "screen_refused",
            Self::StoreUnreadable { .. } => "store_unreadable",
            Self::StoreInvalid { .. } => "store_invalid",
            Self::StoreUnwritable { .. } => "store_unwritable",
            Self::Stopped => "stopped",
            Self::CoinMismatch { .. } => "coin_mismatch",
            Self::NotInCatalog { .. } => "not_in_catalog",
            Self::NotAShop { .. } => "not_a_shop",
            Self::NotForSale { .. } => "not_for_sale",
            Self::UnevenBundle { .. } => "uneven_bundle",
            Self::InsufficientCoin { .. } => "insufficient_coin",
            Self::NoChange { .. } => "no_change",
            Self::ContainerFull { .. } => "container_full",
            Self::OverCapacity { .. } => "over_capacity",
            Self::NoCarryingCapacity { .. } => "no_carrying_capacity",
        }
    }

    /// The refusal as the phone reads it, one of the shared `PagesErrorKind`s: a purchase's or a
    /// payment's own kind, any other refusal `inventory`.
    pub fn pages_kind(&self) -> PagesErrorKind {
        match self {
            Self::OverCapacity { .. } => PagesErrorKind::OverCapacity,
            Self::ContainerFull { .. } => PagesErrorKind::ContainerFull,
            Self::InsufficientCoin { .. } => PagesErrorKind::InsufficientCoin,
            // The shop has fewer than asked.
            Self::Insufficient { .. } => PagesErrorKind::NotInStock,
            Self::NotForSale { .. } => PagesErrorKind::NotForSale,
            Self::NotAShop { .. } => PagesErrorKind::NotAShop,
            Self::NoChange { .. } => PagesErrorKind::NoChange,
            Self::UnevenBundle { .. } => PagesErrorKind::UnevenBundle,
            Self::NoCarryingCapacity { .. } => PagesErrorKind::NoCarryingCapacity,
            Self::ZeroCount { .. } => PagesErrorKind::ZeroCount,
            Self::CoinMismatch { .. } => PagesErrorKind::CoinMismatch,
            _ => PagesErrorKind::Inventory,
        }
    }
}

impl From<BodyError> for InventoryError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl fmt::Display for InventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidQuery(error) => write!(formatter, "{error}"),
            Self::InvalidOwner { owner } => write!(
                formatter,
                "{owner:?} is not an owner id (lowercase letters, digits, '_' or '-')"
            ),
            Self::InvalidItem { item } => write!(
                formatter,
                "{item:?} is not an item id (lowercase letters, digits, '_' or '-')"
            ),
            Self::InvalidSchema { schema } => {
                write!(
                    formatter,
                    "inventory schema {schema} is not one this host reads"
                )
            }
            Self::UnknownOwner { owner } => {
                write!(
                    formatter,
                    "{owner} has no inventory (see: world inventory set)"
                )
            }
            Self::UnknownItem { owner, item } => write!(formatter, "{owner} holds no {item}"),
            Self::DuplicateItem { item } => write!(formatter, "{item} is named twice"),
            Self::MissingWords { item } => {
                write!(
                    formatter,
                    "{item} needs a name and a look that say something"
                )
            }
            Self::EmptyBatch => formatter.write_str("the batch changes nothing"),
            Self::ZeroCount { item } => write!(formatter, "a count of zero of {item}"),
            Self::InvalidReason { item } => {
                write!(
                    formatter,
                    "the reason for the change of {item} does not fit its direction"
                )
            }
            Self::UnreasonedLoss { item } => write!(
                formatter,
                "the loss of {item} has no reason that stands: it is used up (never a kept item), \
                 sold with its payment in the same line, given to someone present, or lost, \
                 broken or worn out with what happened said"
            ),
            Self::UnsourcedGain { item } => write!(
                formatter,
                "the gain of {item} has no source that stands: something of theirs must go down \
                 for it, or it is found at a thing here, or given by someone present, or made"
            ),
            Self::KeptItem { owner, item } => {
                write!(
                    formatter,
                    "{owner}'s {item} is kept: using it does not use it up"
                )
            }
            Self::Insufficient {
                owner,
                item,
                have,
                asked,
            } => write!(formatter, "{owner} has {have} of {item}, not {asked}"),
            Self::Overflow { owner, item } => {
                write!(
                    formatter,
                    "{owner}'s count of {item} would pass the largest there is"
                )
            }
            Self::SameOwner { owner } => write!(formatter, "{owner} cannot give to themself"),
            Self::ScreenRefused { detail } => {
                write!(
                    formatter,
                    "the shared screen refused the inventory: {detail}"
                )
            }
            Self::StoreUnreadable { cause } => {
                write!(formatter, "an inventory file could not be read: {cause}")
            }
            Self::StoreInvalid { cause } => write!(formatter, "an inventory file: {cause}"),
            Self::StoreUnwritable { cause } => {
                write!(formatter, "an inventory file could not be written: {cause}")
            }
            Self::Stopped => formatter.write_str("the host stopped before the work finished"),
            Self::CoinMismatch { owner, item } => {
                write!(
                    formatter,
                    "{owner}'s {item} is another coin than the one given"
                )
            }
            Self::NotInCatalog { item } => {
                write!(formatter, "{item} is not in the SRD equipment catalog")
            }
            Self::NotAShop { owner } => write!(formatter, "{owner} keeps no shop"),
            Self::NotForSale { shop, item } => write!(formatter, "{shop} does not sell {item}"),
            Self::UnevenBundle { item, count, per } => write!(
                formatter,
                "{count} {item} is no whole price: it is sold {per} at a time"
            ),
            Self::InsufficientCoin {
                owner,
                cost_cp,
                purse_cp,
            } => write!(
                formatter,
                "{owner} has {purse_cp} cp in coins, and the price is {cost_cp} cp"
            ),
            Self::NoChange { shop, change_cp } => {
                write!(formatter, "{shop} cannot make {change_cp} cp of change")
            }
            Self::ContainerFull {
                owner,
                container,
                item,
            } => write!(formatter, "{owner}'s {container} has no room for {item}"),
            Self::OverCapacity {
                owner,
                milli_pounds,
                capacity_milli_pounds,
            } => write!(
                formatter,
                "{owner} would carry {}, over their carrying capacity of {}",
                df_rules::srd_equipment_prices::Weight::from_milli_pounds(*milli_pounds),
                df_rules::srd_equipment_prices::Weight::from_milli_pounds(*capacity_milli_pounds)
            ),
            Self::NoCarryingCapacity { owner } => write!(
                formatter,
                "{owner} has no Strength and size to weigh a load against"
            ),
        }
    }
}

impl std::error::Error for InventoryError {}

impl RouteError for InventoryError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::InvalidQuery(_)
            | Self::InvalidOwner { .. }
            | Self::InvalidItem { .. }
            | Self::InvalidSchema { .. }
            | Self::DuplicateItem { .. }
            | Self::MissingWords { .. }
            | Self::EmptyBatch
            | Self::ZeroCount { .. }
            | Self::InvalidReason { .. }
            | Self::UnsourcedGain { .. }
            | Self::UnreasonedLoss { .. }
            | Self::SameOwner { .. } => StatusCode::BAD_REQUEST,
            Self::ScreenRefused { .. } => StatusCode::CONFLICT,
            Self::UnknownOwner { .. } | Self::UnknownItem { .. } => StatusCode::NOT_FOUND,
            Self::NotAShop { .. } | Self::NotForSale { .. } | Self::NotInCatalog { .. } => {
                StatusCode::NOT_FOUND
            }
            Self::KeptItem { .. }
            | Self::Insufficient { .. }
            | Self::Overflow { .. }
            | Self::CoinMismatch { .. }
            | Self::UnevenBundle { .. }
            | Self::InsufficientCoin { .. }
            | Self::NoChange { .. }
            | Self::ContainerFull { .. }
            | Self::OverCapacity { .. }
            | Self::NoCarryingCapacity { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::StoreUnreadable { .. }
            | Self::StoreInvalid { .. }
            | Self::StoreUnwritable { .. }
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
