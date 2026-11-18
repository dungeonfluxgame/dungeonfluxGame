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
//! A gain needs a source. A judged line that gains something says where it came from in a closed
//! [`GainSource`], and code checks it: `exchange` (something of the owner's goes down in the same
//! line, listed in `paid_with` and found in the same batch as a decrease of at least that much),
//! `found` (a thing offered at the active place is named), `given` (another character present is
//! named) or `made` (crafted from inputs, listed in `made_from` and consumed in the same batch). A
//! gain with no valid source is refused (`unsourced_gain`), and so are the decreases tied to it, so
//! both sides of an exchange apply or neither. A judged line is whole: when any entry of it is
//! refused, for any reason, none of its changes apply (a use, a sale and a purchase are never
//! applied by halves), and the judge is asked again with the refusals.
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

mod bag;
pub mod cli;
mod operations;
pub mod request;
pub mod routes;
mod screen;
mod store;

use std::collections::BTreeMap;
use std::fmt;

use df_content::world_bible::is_valid_id;
use df_rules::srd_equipment_prices::Weight;
use serde::{Deserialize, Serialize};

pub use self::bag::{
    BagLine, BagView, Bought, CoinLine, ContainerLoad, ShareLoad, ShopView, StockLine,
    standard_coin_of,
};
pub(crate) use self::bag::{
    bag_view, buy, coin_of, coins_in, counting_coin, exact_coins, gain_coins, purse_of, receive,
    shop_view,
};
pub use self::request::InventoryError;
use self::routes::{record_changed, record_refused};
pub use self::screen::follow_judged;
use crate::state::AppState;

#[cfg(test)]
mod bag_tests;
#[cfg(test)]
mod screen_tests;

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
    /// What one costs in copper pieces when its owner sells it (a trader's ware); none for what is
    /// not for sale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_cp: Option<u64>,
    /// The coin one of it is, when the item is money: its value in copper is the coin's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coin: Option<df_rules::coins::Denomination>,
}

impl InventoryItem {
    /// Whether it is a ware its owner sells: no coin, and a price above nothing.
    pub fn is_priced_ware(&self) -> bool {
        self.coin.is_none() && self.price_cp.is_some_and(|price| price > 0)
    }
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
    /// What is put away in a container the owner holds; the rest of each item is carried as it is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stowed: Vec<Stowed>,
}

/// `count` of the owner's `item` inside their container `container` (both item ids they hold).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stowed {
    pub item: String,
    pub container: String,
    pub count: u64,
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
        /// A new item's price in copper pieces, when it has one.
        #[serde(default)]
        price_cp: Option<u64>,
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
/// received) and nothing else; a loss is any of the others, and each needs what it asks for:
/// `used_up` (spent or applied until gone; never a kept item), `sold` (a gain by exchange in the
/// same line names it as paid), `given_away` (`to` names a character present), or `lost`, `broken`
/// or `worn_out` (`what_happened` says what happened, from what the line shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryReason {
    Gained,
    UsedUp,
    Sold,
    WornOut,
    Broken,
    Lost,
    GivenAway,
    /// It leaves the owner's hands but stays at the place (thrown, dropped, set down, left
    /// behind): it lies on the place's ground until it is picked up again there.
    Left,
}

impl InventoryReason {
    pub const ALL: [Self; 8] = [
        Self::Gained,
        Self::UsedUp,
        Self::Sold,
        Self::WornOut,
        Self::Broken,
        Self::Lost,
        Self::GivenAway,
        Self::Left,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Gained => "gained",
            Self::UsedUp => "used_up",
            Self::Sold => "sold",
            Self::WornOut => "worn_out",
            Self::Broken => "broken",
            Self::Lost => "lost",
            Self::GivenAway => "given_away",
            Self::Left => "left",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.key() == key)
    }
}

/// Where a gain came from, a closed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GainSource {
    /// Something of the owner's went down for it in the same line (`paid_with`).
    Exchange,
    /// Picked up from, or made of, a thing at the place (`where`).
    Found,
    /// Another character present gave it (`from`).
    Given,
    /// Crafted from inputs that are used up in the same line (`made_from`).
    Made,
    /// Picked up freely from the place itself, no thing of it named (dry sticks from a hedgerow, a
    /// stone, water from a stream): something worth nothing, or a catalog or world item worth a
    /// copper at most ([`GATHERED_UNIT_CP`]).
    Gathered,
}

impl GainSource {
    pub const ALL: [Self; 5] = [
        Self::Exchange,
        Self::Found,
        Self::Given,
        Self::Made,
        Self::Gathered,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Exchange => "exchange",
            Self::Found => "found",
            Self::Given => "given",
            Self::Made => "made",
            Self::Gathered => "gathered",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.key() == key)
    }
}

/// An item and how many of it, in a gain's `paid_with` or `made_from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemCount {
    pub item: String,
    pub count: u64,
}

/// A model's word on one item a line moves: `change` below zero for what leaves and above for what
/// is gained, the `reason`, and why in a few words. An item not held yet is proposed by a gain
/// with its `name`, `look` and `use`; for one already held they are null. A gain also says its
/// `source`, with the field that source needs: `paid_with` (exchange), `where` (found), `from`
/// (given) or `made_from` (made); the others are empty or null. A loss says its `reason` and what
/// it needs: `to` (given away) or `what_happened` (lost, broken, worn out); a loss with no reason,
/// or one that does not fit the item or lacks what it needs, is refused. Code checks it like any
/// other change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryDelta {
    pub item: String,
    pub change: i64,
    #[serde(default)]
    pub reason: Option<InventoryReason>,
    pub why: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub look: Option<String>,
    #[serde(default, rename = "use")]
    pub usage: Option<ItemUse>,
    #[serde(default)]
    pub source: Option<GainSource>,
    #[serde(default)]
    pub paid_with: Vec<ItemCount>,
    #[serde(default)]
    pub made_from: Vec<ItemCount>,
    #[serde(default, rename = "where")]
    pub place_thing: Option<String>,
    #[serde(default, rename = "from")]
    pub giver: Option<String>,
    #[serde(default)]
    pub what_happened: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

/// The change `delta` comes to: a gain is an add, being used up a consume, and any other loss a
/// remove; a change of zero is no change. The reason must fit the direction, and a loss must have
/// one.
pub fn change_of(delta: &InventoryDelta) -> Result<Option<InventoryChange>, InventoryError> {
    if delta.change == 0 {
        return Ok(None);
    }
    let (item, count, why) = (
        delta.item.clone(),
        delta.change.unsigned_abs(),
        delta.why.clone(),
    );
    let invalid = || InventoryError::InvalidReason {
        item: delta.item.clone(),
    };
    Ok(Some(match (delta.change > 0, delta.reason) {
        (true, None | Some(InventoryReason::Gained)) => InventoryChange::Add {
            name: delta.name.clone(),
            look: delta.look.clone(),
            usage: delta.usage,
            price_cp: None,
            item,
            count,
            why,
        },
        (true, Some(_)) | (false, Some(InventoryReason::Gained)) => return Err(invalid()),
        (false, None) => {
            return Err(InventoryError::UnreasonedLoss { item });
        }
        (false, Some(InventoryReason::UsedUp)) => InventoryChange::Consume { item, count, why },
        (false, Some(_)) => InventoryChange::Remove { item, count, why },
    }))
}

/// What a judged gain's source is checked against: the ids of the things offered at the place (for
/// `found`), the things of it already searched to a find (`searched`: a thing yields one find for
/// the run), the items the world file knows with their value in copper (`known`, its bible's
/// items) and the ids of the characters present (for `given`).
pub struct GainContext<'a> {
    pub things: &'a [String],
    /// What lies on the ground of the place (left there: thrown, dropped), which a hero picks back
    /// up as a find of that very item, whatever it is worth.
    pub lying: &'a [ItemCount],
    pub searched: &'a [String],
    pub known: &'a [(String, u64)],
    pub present: &'a [String],
}

/// The most one found item is worth, in copper: a trinket (SRD 5.2.1 p.89: 100 cp is 1 gp). A
/// place's thing records nothing it holds, so a search finds a small thing, never plate armour
/// to sell.
pub const FOUND_TRINKET_CP: u64 = 100;

/// Whether the found `delta` picks back up what lies on the place's ground: the thing it is found
/// at is the lying item itself, and no more of it than lies there.
fn picks_up_lying(context: &GainContext<'_>, delta: &InventoryDelta) -> bool {
    delta.source == Some(GainSource::Found)
        && delta.place_thing.as_deref() == Some(delta.item.as_str())
        && u64::try_from(delta.change).is_ok_and(|count| {
            context
                .lying
                .iter()
                .any(|lies| lies.item == delta.item && lies.count >= count)
        })
}

/// The most one gathered item may be worth, in copper, when the catalog or the world prices it: a
/// copper (SRD 5.2.1 p.89). A gathered thing the catalog does not know is worth nothing (it has no
/// price, so no shop buys it).
pub const GATHERED_UNIT_CP: u64 = 1;

/// Whether a gathered gain of `delta.item` stands: no coin (a coin item held, or a standard coin),
/// no thing of the place named (that is a find), and, when the catalog or the world file prices
/// the item, at most [`GATHERED_UNIT_CP`] apiece and [`FOUND_TRINKET_CP`] for all of it. Anything
/// else gathered is a new thing worth nothing.
fn gatherable(inventory: &Inventory, delta: &InventoryDelta, context: &GainContext<'_>) -> bool {
    if delta.place_thing.is_some() || bag::coin_of(&inventory.items, &delta.item).is_some() {
        return false;
    }
    let world = context
        .known
        .iter()
        .find(|(known, _)| *known == delta.item)
        .map(|(_, value_cp)| *value_cp);
    let catalog = df_rules::srd_equipment_prices::price_cp(&delta.item).map(|price| price.copper());
    let catalogued = df_rules::srd_equipment_prices::equipment(&delta.item).is_some();
    match world.or(catalog) {
        Some(value_cp) => {
            value_cp <= GATHERED_UNIT_CP
                && u64::try_from(delta.change)
                    .ok()
                    .and_then(|count| count.checked_mul(value_cp))
                    .is_some_and(|total| total <= FOUND_TRINKET_CP)
        }
        // A catalog item with no whole-copper price is not gathered; one it does not know is.
        None => !catalogued,
    }
}

/// Whether a found gain of `item` is something that exists to be found and a search may turn up:
/// an item of the SRD catalog (`srd_equipment_prices::price_cp`) or of the world file, worth at
/// most [`FOUND_TRINKET_CP`] apiece. A place's thing itself is never taken (nothing records it
/// portable: a well, a gate or a wall stays), money is no item of either (the catalog lists no
/// coin, and a world's coins are not its items), and an item with no known value or an id made up
/// for the find is `unknown_item`, whatever the id says.
fn findable(context: &GainContext<'_>, item: &str) -> bool {
    let world = context
        .known
        .iter()
        .find(|(known, _)| known == item)
        .map(|(_, value_cp)| *value_cp);
    let catalog = df_rules::srd_equipment_prices::price_cp(item).map(|price| price.copper());
    world
        .or(catalog)
        .is_some_and(|value_cp| value_cp <= FOUND_TRINKET_CP)
}

/// What a judgement of a line came to: the changes that stand, in order (none when any entry was
/// refused), and why each entry was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    pub changes: Vec<InventoryChange>,
    pub refused: Vec<InventoryError>,
    /// The sold items whose payment the model left out or got wrong and code paired with the one
    /// gain by exchange that fed the deal ([`repair_sales`]).
    pub repaired: Vec<String>,
}

/// Pairs a sale with its payment where the model named the deal's parts but not how they join. A
/// gain by exchange that names a sold item in `paid_with` with a count above what the line sells
/// of it (the price or the goods' count put where the sold item's count belongs) names what the
/// line sells instead, since the sale's own entry is what the counts are checked on. A sold item
/// (`sold`) that no gain by exchange names in `paid_with` is paired with the one gain by exchange
/// of the line whose item pays for another gain by exchange of the same line (the coins that buy
/// the goods) and that does not name the sold item: its `paid_with` becomes the sold item and its
/// count. Nothing else is guessed: two such gains, or none, repair nothing, and the sale is
/// refused as it stands. Answers the corrected deltas and the sold items repaired.
fn repair_sales(deltas: &[InventoryDelta]) -> (Vec<InventoryDelta>, Vec<String>) {
    let mut fixed = deltas.to_vec();
    let mut repaired = Vec::new();
    let by_exchange =
        |delta: &InventoryDelta| delta.change > 0 && delta.source == Some(GainSource::Exchange);
    for gain in fixed.iter_mut().filter(|delta| by_exchange(delta)) {
        for named in &mut gain.paid_with {
            let leaving = |delta: &&InventoryDelta| delta.change < 0 && delta.item == named.item;
            let all_sold = deltas
                .iter()
                .filter(leaving)
                .all(|delta| delta.reason == Some(InventoryReason::Sold));
            let sold: u64 = deltas
                .iter()
                .filter(leaving)
                .map(|delta| delta.change.unsigned_abs())
                .sum();
            if all_sold && sold > 0 && named.count > sold {
                named.count = sold;
                if !repaired.contains(&named.item) {
                    repaired.push(named.item.clone());
                }
            }
        }
    }
    for sale in deltas
        .iter()
        .filter(|delta| delta.change < 0 && delta.reason == Some(InventoryReason::Sold))
    {
        let count = sale.change.unsigned_abs();
        let paid = fixed.iter().any(|gain| {
            by_exchange(gain)
                && gain
                    .paid_with
                    .iter()
                    .any(|named| named.item == sale.item && named.count >= count)
        });
        if paid {
            continue;
        }
        let candidates: Vec<usize> = (0..fixed.len())
            .filter(|&index| {
                let gain = &fixed[index];
                by_exchange(gain)
                    && !gain.paid_with.iter().any(|named| named.item == sale.item)
                    && fixed.iter().enumerate().any(|(other, buys)| {
                        other != index
                            && by_exchange(buys)
                            && buys.paid_with.iter().any(|named| named.item == gain.item)
                    })
            })
            .collect();
        if let [index] = candidates[..] {
            fixed[index].paid_with = vec![ItemCount {
                item: sale.item.clone(),
                count,
            }];
            if !repaired.contains(&sale.item) {
                repaired.push(sale.item.clone());
            }
        }
    }
    (fixed, repaired)
}

/// An id for the log: the id itself when it is one, else `?`, so no free text of the model's gets
/// into a log line.
pub(crate) fn logged_id(text: &str) -> &str {
    if is_valid_id(text) { text } else { "?" }
}

/// `items` as `id*count` joined by `|`.
fn logged_counts(items: &[ItemCount]) -> String {
    items
        .iter()
        .map(|named| format!("{}*{}", logged_id(&named.item), named.count))
        .collect::<Vec<_>>()
        .join("|")
}

/// The entries of a judged line as one token for the log: for each entry
/// `item:change:reason:source` and, when given, `:pay=..`, `:make=..`, `:to=..`, `:from=..`,
/// `:where=..`, joined by `;`. Ids and numbers only (a word that is not an id is `?`, and no `why`
/// or other free text is written), and bounded to the first [`LOGGED_ENTRIES`] entries.
pub fn entries_token(deltas: &[InventoryDelta]) -> String {
    let mut entries: Vec<String> = deltas
        .iter()
        .take(LOGGED_ENTRIES)
        .map(|delta| {
            let mut entry = format!(
                "{}:{:+}:{}:{}",
                logged_id(&delta.item),
                delta.change,
                delta.reason.map_or("-", InventoryReason::key),
                delta.source.map_or("-", GainSource::key)
            );
            if !delta.paid_with.is_empty() {
                entry.push_str(&format!(":pay={}", logged_counts(&delta.paid_with)));
            }
            if !delta.made_from.is_empty() {
                entry.push_str(&format!(":make={}", logged_counts(&delta.made_from)));
            }
            for (key, id) in [
                ("to", &delta.to),
                ("from", &delta.giver),
                ("where", &delta.place_thing),
            ] {
                if let Some(id) = id {
                    entry.push_str(&format!(":{key}={}", logged_id(id)));
                }
            }
            entry
        })
        .collect();
    if deltas.len() > LOGGED_ENTRIES {
        entries.push(format!("+{}", deltas.len() - LOGGED_ENTRIES));
    }
    if entries.is_empty() {
        "-".to_owned()
    } else {
        entries.join(";")
    }
}

/// The most entries of a judged line written to the log.
const LOGGED_ENTRIES: usize = 40;

/// That the loss `delta` has what its reason needs: a kept item is never `used_up`; `lost`,
/// `broken` and `worn_out` say what happened (not blank); `given_away` names a character present
/// who is not the owner. A `sold` loss is checked against its payment with the whole line.
fn check_loss_reason(
    inventory: &Inventory,
    delta: &InventoryDelta,
    context: &GainContext<'_>,
) -> Result<(), InventoryError> {
    let unreasoned = || InventoryError::UnreasonedLoss {
        item: delta.item.clone(),
    };
    let fits = match delta.reason.ok_or_else(unreasoned)? {
        InventoryReason::UsedUp => inventory
            .item(&delta.item)
            .is_none_or(|held| held.usage != ItemUse::Kept),
        InventoryReason::Sold | InventoryReason::Left => true,
        InventoryReason::Lost | InventoryReason::Broken | InventoryReason::WornOut => {
            nonblank(delta.what_happened.as_ref()).is_some()
        }
        InventoryReason::GivenAway => delta
            .to
            .as_ref()
            .is_some_and(|to| *to != inventory.owner && context.present.contains(to)),
        InventoryReason::Gained => false,
    };
    if fits { Ok(()) } else { Err(unreasoned()) }
}

/// `delta` as one entry of the line: none when it changes nothing, refused when its reason does
/// not fit its direction, a loss has no reason that stands, or (for a loss) it lacks what the
/// reason needs. Whether the counts allow it is judged on the net of the line.
fn entry_of(
    inventory: &Inventory,
    delta: &InventoryDelta,
    context: &GainContext<'_>,
) -> Result<Option<InventoryChange>, InventoryError> {
    let Some(change) = change_of(delta)? else {
        return Ok(None);
    };
    if !matches!(change, InventoryChange::Add { .. }) {
        check_loss_reason(inventory, delta, context)?;
    }
    Ok(Some(change))
}

/// One entry of a judged line: the change it comes to, and the delta it came from.
type Entry<'a> = (InventoryChange, &'a InventoryDelta);

/// The one change `entries`, all of one item, net to: nothing when they cancel, an add when they
/// gain (taking the words of the first add that has them), a consume when every loss among them is
/// used up, otherwise a remove.
fn net_change(entries: &[&Entry<'_>]) -> Option<InventoryChange> {
    let item = entries.first()?.0.item().to_owned();
    let signed: i128 = entries
        .iter()
        .map(|(change, _)| match change {
            InventoryChange::Add { count, .. } => i128::from(*count),
            _ => -i128::from(change.count()),
        })
        .sum();
    let count = u64::try_from(signed.unsigned_abs()).unwrap_or(u64::MAX);
    let why = entries
        .iter()
        .map(|(change, _)| change.why().trim())
        .filter(|why| !why.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    match signed {
        0 => None,
        1.. => {
            let words = entries.iter().find_map(|(change, _)| match change {
                InventoryChange::Add {
                    name: Some(name),
                    look: Some(look),
                    usage,
                    ..
                } => Some((name.clone(), look.clone(), *usage)),
                _ => None,
            });
            Some(InventoryChange::Add {
                item,
                count,
                name: words.as_ref().map(|(name, _, _)| name.clone()),
                look: words.as_ref().map(|(_, look, _)| look.clone()),
                usage: words.and_then(|(_, _, usage)| usage),
                price_cp: None,
                why,
            })
        }
        _ => {
            let all_used_up = entries
                .iter()
                .all(|(change, _)| !matches!(change, InventoryChange::Remove { .. }));
            Some(if all_used_up {
                InventoryChange::Consume { item, count, why }
            } else {
                InventoryChange::Remove { item, count, why }
            })
        }
    }
}

/// How many things one line finds at one thing of the place: a place's thing says no count of what
/// is there, so a search finds one thing at it, never a heap.
const FOUND_AT_A_THING: u64 = 1;

/// The things a gain's source asks to have gone down in the same line, when the source stands:
/// none for `found` and `given`, the listed items for `exchange` and `made`. `Err` when the source
/// is missing or its field does not hold up (no list, a count of zero, a thing not offered, a
/// giver who is not present or is the owner themself). A found gain is never money (a coin item
/// of `inventory`'s, or a standard coin: no thing of a place lists coins) and finds at most
/// [`FOUND_AT_A_THING`].
fn payments_of<'a>(
    inventory: &Inventory,
    delta: &'a InventoryDelta,
    context: &GainContext<'_>,
) -> Result<&'a [ItemCount], ()> {
    let owner = inventory.owner.as_str();
    let listed = |list: &'a [ItemCount]| {
        if list.is_empty() || list.iter().any(|paid| paid.count == 0) {
            Err(())
        } else {
            Ok(list)
        }
    };
    match delta.source.ok_or(())? {
        GainSource::Exchange => listed(&delta.paid_with),
        GainSource::Made => listed(&delta.made_from),
        GainSource::Found if picks_up_lying(context, delta) => Ok(&[]),
        GainSource::Found => match &delta.place_thing {
            Some(thing)
                if context.things.contains(thing)
                    && bag::coin_of(&inventory.items, &delta.item).is_none()
                    && delta.change <= FOUND_AT_A_THING.cast_signed() =>
            {
                Ok(&[])
            }
            _ => Err(()),
        },
        GainSource::Given => match &delta.giver {
            Some(giver) if giver != owner && context.present.contains(giver) => Ok(&[]),
            _ => Err(()),
        },
        GainSource::Gathered if gatherable(inventory, delta, context) => Ok(&[]),
        GainSource::Gathered => Err(()),
    }
}

/// Judges the `deltas` of one line against `inventory`. The entries of one item are netted into
/// one signed change (a coin that rises by a sale and falls by a purchase is the difference; an
/// item sold and listed as paid is one decrease), and the net must be possible
/// (`insufficient`, `kept_item`, `missing_words`, ...). Each entry keeps its own reason and source
/// for the checks: a gain whose source does not stand is refused (`unsourced_gain`), a sale with no
/// payment, or a loss with no reason, is refused (`unreasoned_loss`), and so is whatever leaned on
/// it, so an exchange applies both sides or neither. A line applies whole or not at all: when any
/// entry is refused, for any reason, no change stands (`changes` is empty), so a use, a sale or a
/// purchase is never applied by halves. The decreases a gain is paid with count gross
/// (what went down, not the net), and a decrease named by gains is covered once, in order, so two
/// gains cannot be paid for with the same coin. The changes that stand are one per item.
pub fn judge_deltas(
    inventory: &Inventory,
    deltas: &[InventoryDelta],
    context: &GainContext<'_>,
) -> Judged {
    let (fixed, repaired) = repair_sales(deltas);
    let deltas = &fixed[..];
    let mut refused = Vec::new();
    let mut standing: Vec<Entry<'_>> = Vec::new();
    for delta in deltas {
        match entry_of(inventory, delta, context) {
            Ok(Some(change)) => standing.push((change, delta)),
            Ok(None) => {}
            Err(error) => refused.push(error),
        }
    }
    let is_gain = |change: &InventoryChange| matches!(change, InventoryChange::Add { .. });
    // What counts as paying: a coin spent, an input used up, a thing sold.
    let pays = |delta: &InventoryDelta| {
        matches!(
            delta.reason,
            Some(InventoryReason::UsedUp | InventoryReason::Sold)
        )
    };
    let mut items: Vec<&str> = Vec::new();
    for (change, _) in &standing {
        if !items.contains(&change.item()) {
            items.push(change.item());
        }
    }
    let mut alive = vec![true; standing.len()];
    loop {
        let mut changed = false;
        // The net of each item must be possible; if it is not, every entry of the item goes.
        for item in &items {
            let entries: Vec<&Entry<'_>> = standing
                .iter()
                .zip(&alive)
                .filter(|((change, _), live)| **live && change.item() == *item)
                .map(|(entry, _)| entry)
                .collect();
            let Some(net) = net_change(&entries) else {
                continue;
            };
            if let Err(error) = apply_changes(inventory, std::slice::from_ref(&net)) {
                for (index, (change, _)) in standing.iter().enumerate() {
                    if change.item() == *item {
                        alive[index] = false;
                    }
                }
                refused.push(error);
                changed = true;
            }
        }
        let decreases: BTreeMap<&str, u64> = standing
            .iter()
            .zip(&alive)
            .filter(|((change, delta), live)| **live && !is_gain(change) && pays(delta))
            .fold(BTreeMap::new(), |mut gone, ((change, _), _)| {
                let total = gone.entry(change.item()).or_default();
                *total = u64::saturating_add(*total, change.count());
                gone
            });
        let mut owed: BTreeMap<&str, u64> = BTreeMap::new();
        // What the line finds at each thing of the place, all its found gains together.
        let mut found: BTreeMap<&str, u64> = BTreeMap::new();
        for (index, (change, delta)) in standing.iter().enumerate() {
            if !alive[index] || !is_gain(change) {
                continue;
            }
            // Picking back up what lies here is the item's own way back: no search, no bound.
            let found_at = delta
                .place_thing
                .as_deref()
                .filter(|_| delta.source == Some(GainSource::Found))
                .filter(|_| !picks_up_lying(context, delta));
            let heap = found_at.is_some_and(|thing| {
                found
                    .get(thing)
                    .copied()
                    .unwrap_or_default()
                    .saturating_add(change.count())
                    > FOUND_AT_A_THING
            });
            // A thing already searched to a find yields nothing more.
            if found_at.is_some_and(|thing| context.searched.iter().any(|done| done == thing)) {
                alive[index] = false;
                changed = true;
                refused.push(InventoryError::UnsourcedGain {
                    item: delta.item.clone(),
                });
                continue;
            }
            if found_at.is_some() && !findable(context, &delta.item) {
                alive[index] = false;
                changed = true;
                refused.push(InventoryError::UnknownItem {
                    owner: inventory.owner.clone(),
                    item: delta.item.clone(),
                });
                continue;
            }
            let covered = !heap
                && payments_of(inventory, delta, context).is_ok_and(|payments| {
                    payments.iter().all(|paid| {
                        let total = owed
                            .get(paid.item.as_str())
                            .copied()
                            .unwrap_or_default()
                            .saturating_add(paid.count);
                        decreases
                            .get(paid.item.as_str())
                            .is_some_and(|gone| *gone >= total)
                    })
                });
            if covered {
                if let Some(thing) = found_at {
                    let at = found.entry(thing).or_default();
                    *at = at.saturating_add(change.count());
                }
                for paid in delta.paid_with.iter().chain(&delta.made_from) {
                    let owed_now = owed.entry(paid.item.as_str()).or_default();
                    *owed_now = owed_now.saturating_add(paid.count);
                }
            } else {
                alive[index] = false;
                changed = true;
                refused.push(InventoryError::UnsourcedGain {
                    item: delta.item.clone(),
                });
            }
        }
        // A sale goes down only together with a gain by exchange that names it as paid.
        for (index, (change, delta)) in standing.iter().enumerate() {
            if !alive[index] || is_gain(change) || delta.reason != Some(InventoryReason::Sold) {
                continue;
            }
            let paid = standing.iter().zip(&alive).any(|((gain, by), live)| {
                *live
                    && is_gain(gain)
                    && by.source == Some(GainSource::Exchange)
                    && by
                        .paid_with
                        .iter()
                        .any(|named| named.item == change.item() && named.count >= change.count())
            });
            if !paid {
                alive[index] = false;
                changed = true;
                refused.push(InventoryError::UnreasonedLoss {
                    item: delta.item.clone(),
                });
            }
        }
        if !changed {
            break;
        }
    }
    let changes = if refused.is_empty() {
        items
            .iter()
            .filter_map(|item| {
                let entries: Vec<&Entry<'_>> = standing
                    .iter()
                    .filter(|(change, _)| change.item() == *item)
                    .collect();
                net_change(&entries)
            })
            .collect()
    } else {
        Vec::new()
    };
    Judged {
        changes,
        refused,
        repaired,
    }
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

/// [`judge_deltas`] of `deltas` against `inventory`, with the things offered at the place read
/// when a gain is found there.
fn judge_line(
    app: &AppState,
    inventory: &Inventory,
    deltas: &[InventoryDelta],
    present: &[String],
) -> Judged {
    let finds = deltas
        .iter()
        .any(|delta| matches!(delta.source, Some(GainSource::Found | GainSource::Gathered)));
    let things: Vec<String> = if finds {
        crate::world::place_items(app)
            .into_iter()
            .map(|offered| offered.id)
            .collect()
    } else {
        Vec::new()
    };
    // A world with no bible knows no items of its own; the catalog still counts.
    let known: Vec<(String, u64)> = if finds {
        super::store::read_bible(&super::store::world_directory(app))
            .map(|bible| {
                bible
                    .items
                    .into_iter()
                    .map(|item| (item.id, item.base_value_cp))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let searched = if finds {
        active_place_id(app).map_or_else(Vec::new, |place| app.deals().searched_at(&place))
    } else {
        Vec::new()
    };
    let lying = if finds { lying_here(app) } else { Vec::new() };
    let mut things = things;
    things.extend(lying.iter().map(|lies| lies.item.clone()));
    judge_deltas(
        inventory,
        deltas,
        &GainContext {
            things: &things,
            lying: &lying,
            searched: &searched,
            known: &known,
            present,
        },
    )
}

/// The owner id of the ground of `place`: what is left there lies in it.
fn ground_of(place: &str) -> String {
    format!("ground-{place}")
}

/// What lies on the ground of the active place (left there by a line: thrown, dropped, set down),
/// by id and count; nothing when there is no active place or nothing lies there.
pub fn lying_here(app: &AppState) -> Vec<ItemCount> {
    lying_items_here(app)
        .into_iter()
        .map(|item| ItemCount {
            item: item.id,
            count: item.count,
        })
        .collect()
}

/// The items lying on the ground of the active place, with their words, for a judge's facts.
pub fn lying_items_here(app: &AppState) -> Vec<InventoryItem> {
    let Some(place) = active_place_id(app) else {
        return Vec::new();
    };
    store::read_inventory(&store::inventory_directory(app), &ground_of(&place))
        .map(|ground| ground.held().cloned().collect())
        .unwrap_or_default()
}

/// The id of the active place, when there is one.
fn active_place_id(app: &AppState) -> Option<String> {
    let directory = super::store::world_directory(app);
    let bible = super::store::read_bible(&directory).ok()?;
    super::store::read_active_place(&directory, &bible)
        .ok()
        .flatten()
        .map(|place| place.id)
}

/// Each found or gathered gain of `deltas`, a line of `owner` just taken: one `INVENTORY found
/// owner= item= from_thing=` line (`-` for what was gathered from the place itself), and a thing
/// found at marked searched at the active place, so it yields nothing more for the run.
fn record_found(app: &AppState, owner: &str, deltas: &[InventoryDelta]) {
    let place = active_place_id(app);
    for delta in deltas.iter().filter(|delta| {
        matches!(delta.source, Some(GainSource::Found | GainSource::Gathered)) && delta.change > 0
    }) {
        let thing = delta.place_thing.as_deref().unwrap_or("-");
        app.record_line(format!(
            "INVENTORY found owner={} item={} from_thing={}",
            logged_id(owner),
            logged_id(&delta.item),
            logged_id(thing)
        ));
        // A thing searched to a find yields nothing more; what was picked back up off the
        // ground is no thing searched.
        let picked_up = delta.place_thing.as_deref() == Some(delta.item.as_str());
        if let (Some(place), Some(GainSource::Found), false) = (&place, delta.source, picked_up) {
            app.deals().mark_searched(place, thing);
        }
    }
}

/// The `WORLD judged_entries` line of a judged line: the model's structured answer as ids and
/// numbers, what was refused and what code repaired.
fn record_entries(
    app: &AppState,
    owner: &str,
    deltas: &[InventoryDelta],
    (refused, repaired): (&[InventoryError], &[String]),
) {
    let keys = |keys: Vec<&str>| {
        if keys.is_empty() {
            "-".to_owned()
        } else {
            keys.join(",")
        }
    };
    app.record_line(format!(
        "WORLD judged_entries owner={owner} entries={} refused={} repaired={}",
        entries_token(deltas),
        keys(refused.iter().map(InventoryError::key).collect()),
        keys(repaired.iter().map(|item| logged_id(item)).collect())
    ));
}

/// Why a judged line was refused, for the judge to answer again: the typed `keys` of the
/// refusals, joined by `,`, and a `note` of ids and counts only (no model text).
pub struct InventoryMiss {
    pub keys: String,
    pub note: String,
}

/// What the judge is told of a refused answer: each refusal by its key and item (with the count
/// held and asked when the count was short), and for every item the gains name as paid or used,
/// the count they name together against the count that goes down in the line.
fn refusal_note(deltas: &[InventoryDelta], refused: &[InventoryError]) -> String {
    let refusals: Vec<String> = refused
        .iter()
        .map(|error| match error {
            InventoryError::Insufficient {
                item, have, asked, ..
            } => format!(
                "insufficient {} (holds {have}, asked {asked})",
                logged_id(item)
            ),
            InventoryError::UnsourcedGain { item } | InventoryError::UnreasonedLoss { item } => {
                format!("{} {}", error.key(), logged_id(item))
            }
            other => other.key().to_owned(),
        })
        .collect();
    let mut named: BTreeMap<&str, u64> = BTreeMap::new();
    for gain in deltas.iter().filter(|delta| {
        delta.change > 0 && matches!(delta.source, Some(GainSource::Exchange | GainSource::Made))
    }) {
        for paid in gain.paid_with.iter().chain(&gain.made_from) {
            let total = named.entry(paid.item.as_str()).or_default();
            *total = u64::saturating_add(*total, paid.count);
        }
    }
    let payments: Vec<String> = named
        .into_iter()
        .map(|(item, claimed)| {
            let gone: u64 = deltas
                .iter()
                .filter(|delta| delta.change < 0 && delta.item == item)
                .map(|delta| delta.change.unsigned_abs())
                .sum();
            format!("{} named {claimed}, goes down {gone}", logged_id(item))
        })
        .collect();
    let mut note = format!(
        "inventory_changes was refused and nothing of it was applied: {}.",
        refusals.join("; ")
    );
    if !payments.is_empty() {
        note.push_str(&format!(
            " Payments the gains name against what goes down in the line: {}.",
            payments.join("; ")
        ));
    }
    note
}

/// Judges `deltas` as [`take_judged_changes`] would, without changing anything: `None` when every
/// entry stands (or the character has no inventory), else why it was refused, for the judge to
/// answer again. A refused answer is one `WORLD judged_entries` line.
pub fn judged_miss(
    app: &AppState,
    owner: &str,
    deltas: &[InventoryDelta],
    present: &[String],
) -> Option<InventoryMiss> {
    if deltas.iter().all(|delta| delta.change == 0) {
        return None;
    }
    let inventory = store::read_inventory(&store::inventory_directory(app), owner).ok()?;
    let Judged {
        refused, repaired, ..
    } = judge_line(app, &inventory, deltas, present);
    if refused.is_empty() {
        return None;
    }
    record_entries(app, owner, deltas, (&refused, &repaired));
    Some(InventoryMiss {
        keys: refused
            .iter()
            .map(InventoryError::key)
            .collect::<Vec<_>>()
            .join(","),
        note: refusal_note(deltas, &refused),
    })
}

/// Takes `deltas`, a model's word on what a line of `owner` moved, into their inventory, and
/// answers each change as taken. Each delta is judged on its own, in order, and each gain against
/// its source ([`judge_deltas`]; `present` are the ids of the characters there): one code refuses
/// (an item named twice, a reason that does not fit, a kept item used up, a count below zero, a new
/// item without its words, a gain with no valid source) is one `WORLD refused
/// verb=inventory_judged error=<key>` line and refuses the whole line: nothing of it is taken.
/// Otherwise the changes are one atomic batch, and each change taken is one `WORLD
/// inventory_changed` line. A character
/// with no inventory has nothing to change.
pub fn take_judged_changes(
    app: &AppState,
    owner: &str,
    deltas: &[InventoryDelta],
    present: &[String],
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
    let Judged {
        changes,
        refused,
        repaired,
    } = judge_line(app, &inventory, deltas, present);
    for error in &refused {
        record_refused(app, "judged", error);
    }
    if !refused.is_empty() || !repaired.is_empty() {
        record_entries(app, owner, deltas, (&refused, &repaired));
    }
    if changes.is_empty() {
        return Vec::new();
    }
    // A hero carries what a line gives them only within their capacity (as weighed for the line,
    // `OpenDeals::weigh`), as a purchase or a deal: `over_capacity` takes none of it.
    let capacity = app.deals().capacity(owner);
    // What the line leaves at the place, and what it picks back up from its ground, move between
    // the bag and the ground in the same transaction.
    let left: Vec<ItemCount> = deltas
        .iter()
        .filter(|delta| delta.reason == Some(InventoryReason::Left) && delta.change < 0)
        .map(|delta| ItemCount {
            item: delta.item.clone(),
            count: delta.change.unsigned_abs(),
        })
        .collect();
    let lying = lying_here(app);
    let picked: Vec<ItemCount> = deltas
        .iter()
        .filter(|delta| {
            delta.source == Some(GainSource::Found)
                && delta.place_thing.as_deref() == Some(delta.item.as_str())
                && lying.iter().any(|lies| lies.item == delta.item)
        })
        .map(|delta| ItemCount {
            item: delta.item.clone(),
            count: delta.change.unsigned_abs(),
        })
        .collect();
    let place = active_place_id(app);
    let ground = place.as_deref().map(ground_of);
    let moves = ground
        .as_deref()
        .filter(|_| !left.is_empty() || !picked.is_empty())
        .map(|ground| operations::GroundMove {
            owner: ground,
            left: &left,
            picked: &picked,
        });
    match operations::change(&directory, owner, &changes, capacity, moves) {
        Ok(changed) => {
            record_changed(app, &changed);
            record_left(app, owner, &left, place.as_deref());
            record_found(app, owner, deltas);
            changed.applied
        }
        Err(error) => {
            record_refused(app, "judged", &error);
            Vec::new()
        }
    }
}

/// One change `owner`'s own hands make (a seat's Bag: using or dropping a thing), taken or refused
/// whole: each change one `WORLD inventory_changed` line, a refusal one `WORLD refused
/// verb=inventory_bag` line.
pub fn change_own(
    app: &AppState,
    owner: &str,
    change: InventoryChange,
) -> Result<Vec<InventoryApplied>, InventoryError> {
    let directory = store::inventory_directory(app);
    match operations::change(&directory, owner, &[change], None, None) {
        Ok(changed) => {
            record_changed(app, &changed);
            Ok(changed.applied)
        }
        Err(error) => {
            record_refused(app, "bag", &error);
            Err(error)
        }
    }
}

/// `count` of `item` that `owner`'s own hands put down (a seat's Bag: dropping it), onto the ground
/// of the place they are at, as a line that leaves it there does: it lies there to be picked back
/// up. With no active place it is only gone from the bag. Taken or refused whole, as
/// [`change_own`]; each thing left is one `INVENTORY left` line.
pub fn drop_own(
    app: &AppState,
    owner: &str,
    item: &str,
    count: u64,
) -> Result<Vec<InventoryApplied>, InventoryError> {
    let directory = store::inventory_directory(app);
    let change = InventoryChange::Remove {
        item: item.to_owned(),
        count,
        why: "dropped".to_owned(),
    };
    let left = [ItemCount {
        item: item.to_owned(),
        count,
    }];
    let place = active_place_id(app);
    let ground = place.as_deref().map(ground_of);
    let moves = ground.as_deref().map(|ground| operations::GroundMove {
        owner: ground,
        left: &left,
        picked: &[],
    });
    match operations::change(&directory, owner, &[change], None, moves) {
        Ok(changed) => {
            record_changed(app, &changed);
            if place.is_some() {
                record_left(app, owner, &left, place.as_deref());
            }
            Ok(changed.applied)
        }
        Err(error) => {
            record_refused(app, "bag", &error);
            Err(error)
        }
    }
}

/// One `INVENTORY left owner= item= qty= at=` line for each thing `owner` left at `place`.
fn record_left(app: &AppState, owner: &str, left: &[ItemCount], place: Option<&str>) {
    for left in left {
        app.record_line(format!(
            "INVENTORY left owner={} item={} qty={} at={}",
            logged_id(owner),
            logged_id(&left.item),
            left.count,
            place.map_or("-", logged_id)
        ));
    }
}

/// Where a deal between a line's speaker and one other character stands after the line: proposed,
/// answered with other terms, agreed, turned down, or a gift given freely. Only an agreed deal and
/// a gift move anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DealState {
    Offer,
    Counter,
    Accept,
    Refuse,
    /// The line gives what `gives` names to the other person, asking nothing back (a present, an
    /// apology): it moves one way at once and is never an accept of open terms.
    Gift,
}

impl DealState {
    pub const ALL: [Self; 5] = [
        Self::Offer,
        Self::Counter,
        Self::Accept,
        Self::Refuse,
        Self::Gift,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Offer => "offer",
            Self::Counter => "counter",
            Self::Accept => "accept",
            Self::Refuse => "refuse",
            Self::Gift => "gift",
        }
    }
}

/// An amount of money as it was said: how many of which coin (SRD 5.2.1 p.89: cp, sp, ep, gp,
/// pp), whatever coins the payer holds. In a deal's terms it is the standard coin item of that coin
/// ([`CoinAmount::terms`]), paid from the payer's purse with change when it settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoinAmount {
    pub amount: u64,
    pub coin: df_rules::coins::Denomination,
}

impl CoinAmount {
    /// The amount as an entry of a deal's terms: `amount` of the coin's standard coin item.
    pub fn terms(self) -> ItemCount {
        ItemCount {
            item: bag::standard_coin_id(self.coin).to_owned(),
            count: self.amount,
        }
    }
}

/// A model's word on the deal a line makes with `with` (an id of the characters present): where
/// it stands, what the speaker hands over (`gives`, from their own inventory) and what they receive
/// (`gets`, from `with`'s). Coins in the terms are any coin item, held or standard
/// (`copper-piece`): they are worth their coin and paid from the purse with change, never needed
/// in the coin named. Code checks the ids, counts and the purse when it settles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DealRecord {
    pub with: String,
    pub state: DealState,
    pub gives: Vec<ItemCount>,
    pub gets: Vec<ItemCount>,
}

/// `deltas` split into what stands and what passes between owners, which moves only by a deal
/// both sides agree to ([`settle_deal`]), never by one line: a sale, a gain by exchange, a gift
/// given to someone or received from someone. Using up, making, finding, losing or breaking
/// always stands. The entries taken out are answered as `item:reason` for the log.
pub fn without_deal(deltas: Vec<InventoryDelta>) -> (Vec<InventoryDelta>, Vec<String>) {
    // What a gain by exchange is paid with leaves by the trade, whatever reason its loss gives.
    let paid: Vec<String> = deltas
        .iter()
        .filter(|delta| delta.source == Some(GainSource::Exchange))
        .flat_map(|delta| delta.paid_with.iter().map(|named| named.item.clone()))
        .collect();
    let (dealt, kept): (Vec<InventoryDelta>, Vec<InventoryDelta>) =
        deltas.into_iter().partition(|delta| {
            (delta.change < 0 && paid.contains(&delta.item))
                || delta.reason == Some(InventoryReason::Sold)
                || delta.reason == Some(InventoryReason::GivenAway)
                || delta.source == Some(GainSource::Exchange)
                || delta.source == Some(GainSource::Given)
                || delta.to.is_some()
                || delta.giver.is_some()
        });
    let dealt = dealt
        .iter()
        .map(|delta| {
            let reason = match (delta.reason, delta.source) {
                (Some(reason), _) => reason.key(),
                (None, Some(source)) => source.key(),
                (None, None) => "-",
            };
            format!("{}:{reason}", logged_id(&delta.item))
        })
        .collect();
    (kept, dealt)
}

/// The terms open between two characters: what `proposer` gives and gets as they last put them
/// (an offer or a counter-offer), until a line accepts or turns them down or the party leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenDeal {
    pub proposer: String,
    pub other: String,
    pub gives: Vec<ItemCount>,
    pub gets: Vec<ItemCount>,
}

impl OpenDeal {
    /// The terms as `owner`, one side of them, sees them: what they give and what they get.
    pub fn terms_of(&self, owner: &str) -> (&[ItemCount], &[ItemCount]) {
        if owner == self.proposer {
            (&self.gives, &self.gets)
        } else {
            (&self.gets, &self.gives)
        }
    }

    fn other_side(&self, owner: &str) -> &str {
        if owner == self.proposer {
            &self.other
        } else {
            &self.proposer
        }
    }
}

/// The deals open now, one for each pair of characters, and the last one each pair settled, kept
/// while the host runs; and the carrying capacity of each hero who may trade, as last weighed
/// ([`OpenDeals::weigh`]) for a deal's settling and its facts.
#[derive(Debug, Default)]
pub struct OpenDeals {
    open: std::sync::Mutex<BTreeMap<(String, String), OpenDeal>>,
    settled: std::sync::Mutex<BTreeMap<(String, String), OpenDeal>>,
    carrying: std::sync::Mutex<BTreeMap<String, Weight>>,
    /// The things searched to a find, by place and thing id, kept while the host runs (a run's
    /// places have their own ids): each yields one find.
    searched: std::sync::Mutex<std::collections::BTreeSet<(String, String)>>,
}

/// What `qty` of `item` cost at `owner`'s stock price (the ware's `price_cp`, as
/// [`prices_in_coin`] tells it): none for a ware they do not hold priced, or a coin. A seller's
/// plain quote is this total; only a counter to open terms names another.
pub fn stock_total_cp(app: &AppState, owner: &str, item: &str, qty: u32) -> Option<u64> {
    inventory_of(app, owner)
        .iter()
        .find(|held| held.id == item && held.coin.is_none())?
        .price_cp?
        .checked_mul(u64::from(qty))
}

/// What one ware costs ([`prices_in_coin`]): the price in copper, and the same price exactly as an
/// amount of a coin, as the SRD's tables write it (`Price::quoted`) and as a deal names money.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoinPrice {
    pub item: String,
    pub price_cp: u64,
    pub price: CoinAmount,
}

/// What each priced ware of `inventory` costs, each in its own value (3 cp stays 3 cp, never a
/// gold piece rounded up); none when it prices nothing. A coin is no ware.
pub fn prices_in_coin(inventory: &[InventoryItem]) -> Vec<CoinPrice> {
    inventory
        .iter()
        .filter(|item| item.coin.is_none())
        .filter_map(|item| {
            let price_cp = item.price_cp?;
            let (amount, coin) = df_rules::coins::Price::from_copper(price_cp).quoted();
            Some(CoinPrice {
                item: item.id.clone(),
                price_cp,
                price: CoinAmount { amount, coin },
            })
        })
        .collect()
}

/// The key of the pair `a` and `b`, the same whichever speaks.
fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

impl OpenDeals {
    fn open(&self) -> std::sync::MutexGuard<'_, BTreeMap<(String, String), OpenDeal>> {
        self.open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The terms open between `a` and `b`.
    pub fn between(&self, a: &str, b: &str) -> Option<OpenDeal> {
        self.open().get(&pair(a, b)).cloned()
    }

    /// Every deal `owner` is one side of, with the other side.
    pub fn of(&self, owner: &str) -> Vec<OpenDeal> {
        self.open()
            .values()
            .filter(|deal| deal.proposer == owner || deal.other == owner)
            .cloned()
            .collect()
    }

    fn put(&self, deal: OpenDeal) {
        self.open().insert(pair(&deal.proposer, &deal.other), deal);
    }

    fn settled(&self) -> std::sync::MutexGuard<'_, BTreeMap<(String, String), OpenDeal>> {
        self.settled
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The deals `owner` settled last with each other side.
    pub fn settled_of(&self, owner: &str) -> Vec<OpenDeal> {
        self.settled()
            .values()
            .filter(|deal| deal.proposer == owner || deal.other == owner)
            .cloned()
            .collect()
    }

    fn take(&self, a: &str, b: &str) -> Option<OpenDeal> {
        self.open().remove(&pair(a, b))
    }

    fn carrying(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Weight>> {
        self.carrying
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reads the carrying capacity of each of `owners` from their hero sheet (Strength and size,
    /// SRD p.178), as a purchase does, for the deals they may settle: an owner with no sheet or
    /// admitted size (a resident) has none, and a deal loads them without a limit. A line that may
    /// trade weighs its speaker and the others present before its facts are made.
    pub async fn weigh(&self, app: &AppState, owners: &[String]) {
        for owner in owners {
            match bag::capacity_of(app, owner).await {
                Ok(capacity) => self.carrying().insert(owner.clone(), capacity),
                Err(_) => self.carrying().remove(owner),
            };
        }
    }

    fn searched(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeSet<(String, String)>> {
        self.searched
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The things of `place` already searched to a find.
    fn searched_at(&self, place: &str) -> Vec<String> {
        self.searched()
            .iter()
            .filter(|(at, _)| at == place)
            .map(|(_, thing)| thing.clone())
            .collect()
    }

    /// Marks `thing` of `place` searched to a find.
    fn mark_searched(&self, place: &str, thing: &str) {
        self.searched().insert((place.to_owned(), thing.to_owned()));
    }

    /// The carrying capacity of `owner` as last weighed, if they have one.
    fn capacity(&self, owner: &str) -> Option<Weight> {
        self.carrying().get(owner).copied()
    }

    /// Drops every open deal: the party left the place (`WORLD deal_dropped .. reason=left`).
    pub fn leave(&self, app: &AppState) {
        self.settled().clear();
        let dropped = std::mem::take(&mut *self.open());
        for deal in dropped.values() {
            app.record_line(format!(
                "WORLD deal_dropped owner={} with={} reason=left",
                logged_id(&deal.proposer),
                logged_id(&deal.other)
            ));
        }
    }
}

/// Why the side `owner` of a deal cannot hand over `items`: an item named twice or a count of none;
/// goods they hold none of or fewer than named; coins worth more than their purse (a coin is paid
/// from the whole purse with change, so it need not be held in the coin named). None when they
/// can.
fn side_miss(app: &AppState, owner: &str, items: &[ItemCount], field: &str) -> Option<String> {
    let held = inventory_of(app, owner);
    for (index, named) in items.iter().enumerate() {
        let holds = held
            .iter()
            .find(|item| item.id == named.item)
            .map_or(0, |item| item.count);
        let twice = items[..index]
            .iter()
            .any(|before| before.item == named.item);
        let coin = bag::coin_of(&held, &named.item).is_some();
        if named.count == 0 || (!coin && named.count > holds) || twice {
            return Some(format!(
                "deal.{field} names {}*{}, and {} holds {holds} of it",
                logged_id(&named.item),
                named.count,
                logged_id(owner)
            ));
        }
    }
    let short = |coins_cp: u64| {
        format!(
            "deal.{field} names coins worth {coins_cp} cp, and {} holds coins worth {} cp",
            logged_id(owner),
            bag::purse_cp(owner, &held).unwrap_or(u64::MAX)
        )
    };
    match bag::goods_and_coins(owner, &held, items) {
        Ok((_, coins_cp)) if bag::purse_cp(owner, &held).is_ok_and(|purse| coins_cp <= purse) => {
            None
        }
        Ok((_, coins_cp)) => Some(short(coins_cp)),
        Err(_) => Some(short(u64::MAX)),
    }
}

/// What `items` of `owner` are worth in copper: the wares at their prices, and the coins at their
/// coins' value (a coin they do not hold at its standard coin's, `bag::coin_of`). An item that is
/// neither priced nor money counts in neither.
pub fn worth_cp(app: &AppState, owner: &str, items: &[ItemCount]) -> (u64, u64) {
    let held = inventory_of(app, owner);
    worth_of(owner, &held, items)
}

/// [`worth_cp`] of `items` of `owner`, who holds `held`.
fn worth_of(owner: &str, held: &[InventoryItem], items: &[ItemCount]) -> (u64, u64) {
    let found = |named: &ItemCount| held.iter().find(|item| item.id == named.item);
    let wares = items.iter().fold(0_u64, |wares, named| match found(named) {
        Some(InventoryItem {
            coin: None,
            price_cp: Some(price),
            ..
        }) => wares.saturating_add(named.count.saturating_mul(*price)),
        _ => wares,
    });
    // A purse past u64 is worth the most a count can say.
    let coins = bag::goods_and_coins(owner, held, items).map_or(u64::MAX, |(_, coins_cp)| coins_cp);
    (wares, coins)
}

/// One side of terms as a line's facts show it ([`terms_facts`]): the goods by id and count, the
/// coins as they were named (an amount of a coin, `CoinAmount`), and what each is worth in copper
/// (`wares_cp` at the wares' prices, `coins_cp` at the coins' value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TermsFacts {
    pub goods: Vec<ItemCount>,
    pub coins: Vec<CoinAmount>,
    pub wares_cp: u64,
    pub coins_cp: u64,
}

/// `items`, one side of terms that `owner` hands over, as a line's facts show it: goods apart from
/// coins, each coin as the amount and coin it names, whatever coins the purse holds.
pub fn terms_facts(app: &AppState, owner: &str, items: &[ItemCount]) -> TermsFacts {
    let held = inventory_of(app, owner);
    let (wares_cp, coins_cp) = worth_of(owner, &held, items);
    let mut goods = Vec::new();
    let mut coins = Vec::new();
    for named in items {
        match bag::coin_of(&held, &named.item) {
            Some(coin) => coins.push(CoinAmount {
                amount: named.count,
                coin,
            }),
            None => goods.push(named.clone()),
        }
    }
    TermsFacts {
        goods,
        coins,
        wares_cp,
        coins_cp,
    }
}

/// Why the terms `open` would be refused if they were accepted now (the typed refusal's key:
/// `insufficient_coin`, `no_change`, `insufficient`, `over_capacity` for a hero weighed by
/// [`OpenDeals::weigh`], ...), worked out on both sides' inventories
/// with nothing written; none when they would settle. A line's facts carry it, so no line says a
/// trade went through that the books would refuse.
pub fn settle_refusal(app: &AppState, open: &OpenDeal) -> Option<&'static str> {
    settle_preview(app, open).err().map(|error| error.key())
}

/// The change in copper the payee would hand back if the terms `open` were accepted now (none when
/// no coins would move, or the books would refuse them), worked out with nothing written: the one
/// figure a line may say for the change (`{change}`), never the speaker's own arithmetic.
pub fn settle_change_cp(app: &AppState, open: &OpenDeal) -> Option<u64> {
    settle_preview(app, open)
        .ok()
        .flatten()
        .map(|paid| paid.change_cp)
}

/// What settling the terms `open` would come to now, on both sides' inventories with nothing
/// written: the payment, when coins would move, or the typed refusal.
fn settle_preview(
    app: &AppState,
    open: &OpenDeal,
) -> Result<Option<bag::DealPaid>, InventoryError> {
    let directory = store::inventory_directory(app);
    let proposer = store::read_inventory(&directory, &open.proposer)?;
    let other = store::read_inventory(&directory, &open.other)?;
    if let Some(unpaid) = unpaid_wares((&proposer, &open.gives), (&other, &open.gets)) {
        return Err(unpaid);
    }
    let side = |owner, gives| operations::DealSide {
        owner,
        gives,
        capacity: app.deals().capacity(owner),
    };
    operations::settled(
        (&proposer, side(&open.proposer, &open.gives)),
        (&other, side(&open.other, &open.gets)),
        "terms",
    )
    .map(|(_, _, _, paid)| paid)
}

/// Whether two terms between `owner` and `other` are the same deal: the same goods and counts each
/// way, in any order, and coins of the same worth each way, whatever coin they are named in (six
/// copper pieces are the open six copper, however the purse will pay them). With `Money::Ignored`
/// only the goods are compared, and a side that names no goods keeps the open terms' own: an
/// accept's money is never its terms (it may name the coin handed to pay, or the change it gives
/// back), so the open price stands and the change is the books'.
fn same_deal(
    app: &AppState,
    (owner, other): (&str, &str),
    (gives, gets): (&[ItemCount], &[ItemCount]),
    (named_gives, named_gets): (&[ItemCount], &[ItemCount]),
    money: Money,
) -> bool {
    let side = |side_owner: &str, open: &[ItemCount], named: &[ItemCount]| {
        let held = inventory_of(app, side_owner);
        match (
            bag::goods_and_coins(side_owner, &held, open),
            bag::goods_and_coins(side_owner, &held, named),
        ) {
            // An accept that names no goods on a side (it speaks only of the coin handed or the
            // change) leaves that side's goods as the open terms have them.
            (Ok((open_goods, open_cp)), Ok((named_goods, named_cp))) => match money {
                Money::Compared => named_cp == open_cp && same_terms(&open_goods, &named_goods),
                Money::Ignored => named_goods.is_empty() || same_terms(&open_goods, &named_goods),
            },
            _ => false,
        }
    };
    side(owner, gives, named_gives) && side(other, gets, named_gets)
}

/// Whether [`same_deal`] compares the money of two terms or only their goods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Money {
    Compared,
    Ignored,
}

/// Whether two lists of terms name the same items and counts, in any order.
fn same_terms(left: &[ItemCount], right: &[ItemCount]) -> bool {
    let sorted = |items: &[ItemCount]| {
        let mut pairs: Vec<(String, u64)> = items
            .iter()
            .map(|named| (named.item.clone(), named.count))
            .collect();
        pairs.sort();
        pairs
    };
    sorted(left) == sorted(right)
}

/// Why a shop would give `items` it has no price for: a ware of a shop is sold only at its price.
fn unpriced_miss(app: &AppState, owner: &str, items: &[ItemCount]) -> Option<String> {
    let directory = store::inventory_directory(app);
    let shop = store::read_inventory(&directory, owner).ok()?;
    if shop.kind != OwnerKind::Shop {
        return None;
    }
    items
        .iter()
        .find(|named| {
            shop.item(&named.item)
                .is_some_and(|item| item.price_cp.is_none() && item.coin.is_none())
        })
        .map(|named| {
            format!(
                "{} has no price for {}, so it is not for sale",
                logged_id(owner),
                logged_id(&named.item)
            )
        })
}

/// Why the deal a line of `owner` makes cannot stand, in words the judge is asked again with: the
/// other side is not someone present; an offer or counter-offer names nothing, or names what its
/// side does not hold (`gives` from `owner`'s holdings, `gets` from the other side's); an accept or
/// a refusal has no open terms to answer. None when it stands.
pub fn deal_miss(
    app: &AppState,
    owner: &str,
    deal: &DealRecord,
    present: &[String],
) -> Option<String> {
    let with = logged_id(&deal.with);
    if !present.contains(&deal.with) {
        return Some(format!("deal.with {with} is not someone present"));
    }
    match deal.state {
        DealState::Accept => match app.deals().between(owner, &deal.with) {
            None => Some(format!("there are no open terms with {with} to accept")),
            // Terms stand when the other side agrees to them, never by their own proposer.
            Some(open) if open.proposer == owner => Some(format!(
                "the open terms are this character's own: only {with} can accept them"
            )),
            Some(_) => None,
        },
        DealState::Refuse => app
            .deals()
            .between(owner, &deal.with)
            .is_none()
            .then(|| format!("there are no open terms with {with} to refuse")),
        DealState::Gift => {
            if deal.gives.is_empty() || !deal.gets.is_empty() {
                return Some(
                    "a gift names what is given in gives and asks nothing back: gets is empty"
                        .to_owned(),
                );
            }
            side_miss(app, owner, &deal.gives, "gives")
        }
        DealState::Offer | DealState::Counter => {
            if deal.gives.is_empty() && deal.gets.is_empty() {
                return Some("an offer or counter-offer names what changes hands".to_owned());
            }
            if deal.state == DealState::Counter && turned_around(app, owner, deal) {
                return Some(format!(
                    "the counter turns the open terms with {with} around: what each side hands \
                     over stays on its side (the one giving the goods still gives them, the one \
                     paying still pays)"
                ));
            }
            // Terms just settled are done: thanks or a hand-over of them is no new deal.
            let resettled = app.deals().settled_of(owner).iter().any(|settled| {
                settled.other_side(owner) == deal.with
                    && same_deal(
                        app,
                        (owner, &deal.with),
                        settled.terms_of(owner),
                        (&deal.gives, &deal.gets),
                        Money::Compared,
                    )
            });
            if resettled {
                return Some(format!(
                    "these terms were just settled with {with}: a new deal needs new terms"
                ));
            }
            side_miss(app, owner, &deal.gives, "gives")
                .or_else(|| side_miss(app, &deal.with, &deal.gets, "gets"))
                .or_else(|| unpriced_miss(app, owner, &deal.gives))
                .or_else(|| unpriced_miss(app, &deal.with, &deal.gets))
        }
    }
}

/// Whether the counter `deal` of `owner` turns the open terms around: goods the open terms have one
/// side hand over named on the other side, or the money flowing the other way. A counter changes
/// what is given, never who gives it.
fn turned_around(app: &AppState, owner: &str, deal: &DealRecord) -> bool {
    let Some(open) = app.deals().between(owner, &deal.with) else {
        return false;
    };
    let (gives, gets) = open.terms_of(owner);
    let own = inventory_of(app, owner);
    let theirs = inventory_of(app, &deal.with);
    let split = |owner: &str, held: &[InventoryItem], items: &[ItemCount]| {
        bag::goods_and_coins(owner, held, items).unwrap_or_default()
    };
    let (open_gives, open_paid) = split(owner, &own, gives);
    let (open_gets, open_received) = split(&deal.with, &theirs, gets);
    let (named_gives, named_paid) = split(owner, &own, &deal.gives);
    let (named_gets, named_received) = split(&deal.with, &theirs, &deal.gets);
    let crossed = |left: &[ItemCount], right: &[ItemCount]| {
        left.iter()
            .any(|named| right.iter().any(|other| other.item == named.item))
    };
    let goods = crossed(&open_gives, &named_gets) || crossed(&open_gets, &named_gives);
    let open_flow = open_paid.cmp(&open_received);
    let named_flow = named_paid.cmp(&named_received);
    let money = open_flow != std::cmp::Ordering::Equal
        && named_flow != std::cmp::Ordering::Equal
        && open_flow != named_flow;
    goods || money
}

/// Whether the accept `deal` of `owner` names other goods than the open terms: it does not settle
/// them and is a counter at what it names. The money an accept names is never its terms (the coin
/// handed to pay, or change given back: Jev reads an accept at another price as a counter), so the
/// open price stands; an accept that names nothing takes the open terms as they stand.
fn accept_differs(app: &AppState, owner: &str, deal: &DealRecord) -> bool {
    let named = !deal.gives.is_empty() || !deal.gets.is_empty();
    app.deals().between(owner, &deal.with).is_some_and(|open| {
        named
            && !same_deal(
                app,
                (owner, &deal.with),
                open.terms_of(owner),
                (&deal.gives, &deal.gets),
                Money::Ignored,
            )
    })
}

/// Whether `owner` keeps a shop (a keeper whose wares have prices), from their stored inventory.
pub fn is_shop(app: &AppState, owner: &str) -> bool {
    store::read_inventory(&store::inventory_directory(app), owner)
        .is_ok_and(|inventory| inventory.kind == OwnerKind::Shop)
}

/// Whether a line of `owner` that says goods or coins change hands, yet settles nothing itself,
/// can only be speaking of a trade the books already recorded: no terms are open between `owner`
/// and anyone `present`, and terms with one of them were settled (a hand-over or thanks after the
/// sale). Otherwise the exchange it claims is not recorded.
pub fn speaks_of_settled(app: &AppState, owner: &str, present: &[String]) -> bool {
    let with_present = |deal: &OpenDeal| present.iter().any(|id| *id == deal.other_side(owner));
    let deals = app.deals();
    !deals.of(owner).iter().any(with_present) && deals.settled_of(owner).iter().any(with_present)
}

/// Why the accept `deal` of `owner` would not settle anything, as a key with no ids or names in it
/// (it reaches the speaker's own words): `does_not_stand` ([`deal_miss`]), `other_terms` (it
/// names other terms than the open ones: a counter, which moves nothing), or the typed refusal's
/// key when the books refuse the open terms ([`settle_refusal`]: `insufficient_coin`,
/// `no_change`, `over_capacity`, ...). None when it would settle, and for every other state.
pub fn unsettled_accept(
    app: &AppState,
    owner: &str,
    deal: &DealRecord,
    present: &[String],
) -> Option<&'static str> {
    if deal.state != DealState::Accept {
        return None;
    }
    if deal_miss(app, owner, deal, present).is_some() {
        return Some("does_not_stand");
    }
    if accept_differs(app, owner, deal) {
        return Some("other_terms");
    }
    settle_refusal(app, &app.deals().between(owner, &deal.with)?)
}

/// The offer `deal` of `owner`, a buyer, at the price the shop it buys from asks, when it pays
/// only money for goods all priced in that shop's stock and names more than that price ("one
/// ration, here's a gold piece" for a 5 sp ration): the coin named is what is handed, not the
/// price, so the terms are the price ([`stock_total_cp`], as `Price::quoted` says it) and the
/// change comes back when they settle. Only the side paying coin to a shop is repriced: a shop's
/// own offer is never turned into a payment of its own. Answers the offer at that price, the copper
/// named and the price; none otherwise (an offer at or under the price stands as it was put).
fn tendered_offer(
    app: &AppState,
    owner: &str,
    deal: &DealRecord,
) -> Option<(DealRecord, u64, u64)> {
    if !is_shop(app, &deal.with) || is_shop(app, owner) {
        return None;
    }
    let held = inventory_of(app, owner);
    let (goods_given, named_cp) = bag::goods_and_coins(owner, &held, &deal.gives).ok()?;
    if !goods_given.is_empty() || deal.gets.is_empty() {
        return None;
    }
    let price_cp = deal.gets.iter().try_fold(0_u64, |total, named| {
        let qty = u32::try_from(named.count).ok()?;
        total.checked_add(stock_total_cp(app, &deal.with, &named.item, qty)?)
    })?;
    if price_cp == 0 || named_cp <= price_cp {
        return None;
    }
    let (amount, coin) = df_rules::coins::Price::from_copper(price_cp).quoted();
    let priced = DealRecord {
        gives: vec![CoinAmount { amount, coin }.terms()],
        ..deal.clone()
    };
    Some((priced, named_cp, price_cp))
}

/// Takes the deal a line of `owner` makes (checked by [`deal_miss`]; a deal that does not stand is
/// one `WORLD deal_invalid` line and changes nothing). An offer or a counter-offer opens its terms
/// between the two, replacing any open ones (`WORLD deal_open`); a refusal drops them (`WORLD
/// deal_dropped .. reason=refused`); an accept of the open terms (or of terms of the same worth,
/// [`same_deal`]; other terms are a counter, `WORLD deal_accept_differs`) settles them as one
/// atomic transaction between the two owners through [`transfer`], the coins paid from the
/// payer's purse with change (`WORLD deal_settled`, each change one `WORLD inventory_changed`
/// line, then one `INVENTORY deal` line; a transaction refused is `WORLD refused
/// verb=inventory_deal` and drops the terms). Answers the changes taken.
pub fn settle_deal(
    app: &AppState,
    owner: &str,
    deal: &DealRecord,
    present: &[String],
) -> Vec<InventoryApplied> {
    let with = logged_id(&deal.with);
    if deal_miss(app, owner, deal, present).is_some() {
        app.record_line(format!(
            "WORLD deal_invalid owner={owner} with={with} state={}",
            deal.state.key()
        ));
        return Vec::new();
    }
    match deal.state {
        DealState::Offer | DealState::Counter => {
            let tendered = (deal.state == DealState::Offer)
                .then(|| tendered_offer(app, owner, deal))
                .flatten();
            let deal = match &tendered {
                Some((priced, named_cp, price_cp)) => {
                    app.record_line(format!(
                        "WORLD deal_tendered owner={owner} with={with} named_cp={named_cp} \
                         price_cp={price_cp}"
                    ));
                    priced
                }
                None => deal,
            };
            app.record_line(format!(
                "WORLD deal_open state={} owner={owner} with={with} gives={} gets={}",
                deal.state.key(),
                logged_counts(&deal.gives),
                logged_counts(&deal.gets)
            ));
            app.deals().put(OpenDeal {
                proposer: owner.to_owned(),
                other: deal.with.clone(),
                gives: deal.gives.clone(),
                gets: deal.gets.clone(),
            });
            Vec::new()
        }
        DealState::Refuse => {
            app.deals().take(owner, &deal.with);
            app.record_line(format!(
                "WORLD deal_dropped owner={owner} with={with} reason=refused"
            ));
            Vec::new()
        }
        DealState::Gift => give(app, owner, deal),
        DealState::Accept => {
            if accept_differs(app, owner, deal) {
                let counter = DealRecord {
                    state: DealState::Counter,
                    ..deal.clone()
                };
                app.record_line(format!(
                    "WORLD deal_accept_differs owner={owner} with={with}: taken as a counter"
                ));
                return settle_deal(app, owner, &counter, present);
            }
            let Some(open) = app.deals().take(owner, &deal.with) else {
                return Vec::new();
            };
            let directory = store::inventory_directory(app);
            let why = format!("deal between {} and {}", open.proposer, open.other);
            let side = |owner, gives| operations::DealSide {
                owner,
                gives,
                capacity: app.deals().capacity(owner),
            };
            // The terms are worked out first (`settle_preview`: a shop's wares never for nothing),
            // then written in one transaction.
            let settled = settle_preview(app, &open).and_then(|_| {
                operations::settle(
                    &directory,
                    side(&open.proposer, &open.gives),
                    side(&open.other, &open.gets),
                    &why,
                )
            });
            let (gives, gets) = open.terms_of(owner);
            // What the wares cost and what the coins are worth, each side's, before they moved.
            let (gives_wares, gives_coins) = worth_cp(app, owner, gives);
            let (gets_wares, gets_coins) = worth_cp(app, open.other_side(owner), gets);
            match settled {
                Ok((changed, paid)) => {
                    app.record_line(format!(
                        "WORLD deal_settled owner={owner} with={} gives={} gets={} \
                         wares_cp={} coins_cp={}",
                        logged_id(open.other_side(owner)),
                        logged_counts(gives),
                        logged_counts(gets),
                        gives_wares + gets_wares,
                        gives_coins.saturating_add(gets_coins)
                    ));
                    record_changed(app, &changed);
                    record_deal_paid(app, &changed, paid.as_ref());
                    app.deals()
                        .settled()
                        .insert(pair(&open.proposer, &open.other), open.clone());
                    changed.applied
                }
                Err(error) => {
                    record_refused(app, "deal", &error);
                    app.record_line(format!(
                        "WORLD deal_dropped owner={owner} with={with} reason={}",
                        error.key()
                    ));
                    Vec::new()
                }
            }
        }
    }
}

/// Why terms between `a` and `b` (each with what it gives) would hand a shop's priced wares over
/// for nothing: a side that is a shop gives wares of its own priced stock, and the other side gives
/// nothing at all. A shop sells its wares only for a price; a gift is its own state
/// ([`DealState::Gift`], given at once), never terms. `not_for_sale`, naming the ware.
fn unpaid_wares(
    (a, a_gives): (&Inventory, &[ItemCount]),
    (b, b_gives): (&Inventory, &[ItemCount]),
) -> Option<InventoryError> {
    [(a, a_gives, b_gives), (b, b_gives, a_gives)]
        .into_iter()
        .find_map(|(shop, gives, paid)| {
            if shop.kind != OwnerKind::Shop || !paid.is_empty() {
                return None;
            }
            gives
                .iter()
                .find(|named| {
                    shop.item(&named.item)
                        .is_some_and(InventoryItem::is_priced_ware)
                })
                .map(|named| InventoryError::NotForSale {
                    shop: shop.owner.clone(),
                    item: named.item.clone(),
                })
        })
}

/// The gift `deal` of `owner`, given at once: what `gives` names moves to `with` through the deal's
/// one transaction, and nothing comes back (`WORLD deal_gift owner= with= gives=`, each change one
/// `WORLD inventory_changed` line, then `INVENTORY deal`). Giving freely ends any bargaining
/// between the two: terms open between them are dropped (`WORLD deal_dropped .. reason=gift`), so
/// a stale offer is never settled by a later line. A refused gift is `WORLD refused
/// verb=inventory_deal` and moves nothing.
fn give(app: &AppState, owner: &str, deal: &DealRecord) -> Vec<InventoryApplied> {
    let with = logged_id(&deal.with);
    if app.deals().take(owner, &deal.with).is_some() {
        app.record_line(format!(
            "WORLD deal_dropped owner={owner} with={with} reason=gift"
        ));
    }
    let side = |owner, gives| operations::DealSide {
        owner,
        gives,
        capacity: app.deals().capacity(owner),
    };
    let settled = operations::settle(
        &store::inventory_directory(app),
        side(owner, &deal.gives),
        side(&deal.with, &[]),
        &format!("gift from {owner} to {}", deal.with),
    );
    match settled {
        Ok((changed, paid)) => {
            app.record_line(format!(
                "WORLD deal_gift owner={owner} with={with} gives={}",
                logged_counts(&deal.gives)
            ));
            record_changed(app, &changed);
            record_deal_paid(app, &changed, paid.as_ref());
            changed.applied
        }
        Err(error) => {
            record_refused(app, "deal", &error);
            Vec::new()
        }
    }
}

/// The `INVENTORY deal owner= with= payer= cp= handed_cp= change_cp= coins_before= coins_after=
/// weight= with_weight=` line of a settled deal: its two sides (`changed.owners`, the proposer
/// first), who paid the coins' net worth in copper (`-` and zeros when none moved), the coins
/// handed and the change back, the payer's coins before and after, and what each side carries
/// after it, in pounds.
fn record_deal_paid(app: &AppState, changed: &request::Changed, paid: Option<&bag::DealPaid>) {
    let [first, second] = changed.owners.as_slice() else {
        return;
    };
    let weight = |view: &request::InventoryView| {
        bag::carried_pounds(&view.inventory).unwrap_or_else(|error| error.key().to_owned())
    };
    let (payer, cp, handed_cp, change_cp, before, after) = match paid {
        Some(paid) => {
            let after = changed
                .owners
                .iter()
                .find(|view| view.inventory.owner == paid.payer)
                .and_then(|view| bag::coins_cp(&view.inventory).ok())
                .unwrap_or(u64::MAX);
            (
                logged_id(&paid.payer),
                paid.cp,
                paid.handed_cp,
                paid.change_cp,
                paid.coins_before_cp,
                after,
            )
        }
        None => ("-", 0, 0, 0, 0, 0),
    };
    app.record_line(format!(
        "INVENTORY deal owner={} with={} payer={payer} cp={cp} handed_cp={handed_cp} \
         change_cp={change_cp} coins_before={before} coins_after={after} weight={} with_weight={}",
        logged_id(&first.inventory.owner),
        logged_id(&second.inventory.owner),
        weight(first),
        weight(second)
    ));
}

/// What a trader of a designed place has in stock: their wares and the coin they pay with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShopStock {
    pub owner: String,
    pub wares: Vec<InventoryItem>,
    #[serde(default)]
    pub coin: Option<InventoryItem>,
}

/// What becomes of a trader's kept stock when they are met again.
enum Kept {
    /// Every ware has its price and the place shows no ware they lack: the stock stands.
    AsItIs,
    /// Wares without a price were given one (the rules' price, else the current design's), and
    /// wares the place shows them selling that they did not hold were added: the items, how many
    /// were priced and how many added.
    Changed {
        items: Vec<InventoryItem>,
        repriced: usize,
        added: usize,
    },
    /// A ware no price is known for: the stock is not kept.
    Stale,
}

/// `kept`, a trader's stored stock, with a price for each ware that has none (stock kept from
/// before wares had prices): the rules' price of the item, else the price `shop` (the design of
/// the place now) names for it. The coin, and every coin of its till, is no ware and has no price.
/// A ware `shop` shows that the stock has never held is added as the design stocks it, so what the
/// plate shows them selling can be sold; a ware held (even sold out) keeps its count.
fn repriced(kept: Inventory, shop: &ShopStock) -> Kept {
    let coin = shop.coin.as_ref().map(|coin| coin.id.as_str());
    let mut count = 0;
    let mut items = Vec::with_capacity(kept.items.len());
    for item in kept.items {
        if item.price_cp.is_some() || item.coin.is_some() || Some(item.id.as_str()) == coin {
            items.push(item);
            continue;
        }
        let ruled = df_rules::srd_equipment_prices::price_cp(&item.id).map(|price| price.copper());
        let designed = shop
            .wares
            .iter()
            .find(|ware| ware.id == item.id)
            .and_then(|ware| ware.price_cp);
        let Some(price) = ruled.or(designed) else {
            return Kept::Stale;
        };
        count += 1;
        items.push(InventoryItem {
            price_cp: Some(price),
            ..item
        });
    }
    let missing: Vec<InventoryItem> = shop
        .wares
        .iter()
        .filter(|ware| !items.iter().any(|held| held.id == ware.id))
        .cloned()
        .collect();
    let added = missing.len();
    items.extend(missing);
    if count == 0 && added == 0 {
        Kept::AsItIs
    } else {
        Kept::Changed {
            items,
            repriced: count,
            added,
        }
    }
}

/// Gives `owner` a till that can make change (`operations::till`: its coins topped up to the
/// till's, an inventory made when it has none), so a keeper or a fare-taker who takes money gives
/// change from their own purse (`planning/town-to-tomb.md` part C, Change). One `INVENTORY till
/// owner= added_cp=` line, or `INVENTORY till_refused owner= reason=`. Blocking.
pub(super) fn give_till(app: &AppState, owner: &str) -> Result<request::Changed, InventoryError> {
    match operations::till(&store::inventory_directory(app), owner) {
        Ok((changed, added_cp)) => {
            app.record_line(format!(
                "INVENTORY till owner={} added_cp={added_cp}",
                logged_id(owner)
            ));
            Ok(changed)
        }
        Err(error) => {
            app.record_line(format!(
                "INVENTORY till_refused owner={} reason={}",
                logged_id(owner),
                error.key()
            ));
            Err(error)
        }
    }
}

/// Makes `shop`'s owner a shop holding its wares and coin, through the inventory's set, once: an
/// owner who already has an inventory (a trader met again) keeps it (`WORLD shop_kept owner=`),
/// its wares without a price priced (`WORLD shop_repriced owner= items=`) and the wares the place
/// now shows them selling that they never held added (`WORLD shop_wares_added owner= items=`), or,
/// when a ware has no price to give, stocked afresh. Either way the trader keeps a till that can
/// make change ([`give_till`]).
/// One `WORLD shop_stocked owner= items= coins=` line, or a `WORLD refused verb=inventory_stock`
/// line when the stock is refused. Blocking.
pub fn stock_shop(app: &AppState, shop: ShopStock) {
    let owner = shop.owner.clone();
    if stock_wares(app, shop) {
        // A refused till is its own line; the stock stands.
        let _ = give_till(app, &owner);
    }
}

/// The stock of [`stock_shop`]; whether the trader has one now.
fn stock_wares(app: &AppState, shop: ShopStock) -> bool {
    let directory = store::inventory_directory(app);
    match store::read_inventory(&directory, &shop.owner) {
        Ok(kept) => match repriced(kept, &shop) {
            Kept::AsItIs => {
                app.record_line(format!("WORLD shop_kept owner={}", logged_id(&shop.owner)));
                return true;
            }
            Kept::Changed {
                items,
                repriced: count,
                added,
            } => {
                let request = request::SetRequest {
                    owner: shop.owner.clone(),
                    kind: OwnerKind::Shop,
                    items,
                };
                return match operations::set(&directory, request) {
                    Ok(_) => {
                        if count > 0 {
                            app.record_line(format!(
                                "WORLD shop_repriced owner={} items={count}",
                                logged_id(&shop.owner)
                            ));
                        }
                        if added > 0 {
                            app.record_line(format!(
                                "WORLD shop_wares_added owner={} items={added}",
                                logged_id(&shop.owner)
                            ));
                        }
                        true
                    }
                    Err(error) => {
                        record_refused(app, "stock", &error);
                        false
                    }
                };
            }
            // Stock that cannot be priced is not kept: the trader is stocked afresh below.
            Kept::Stale => {}
        },
        Err(InventoryError::UnknownOwner { .. }) => {}
        Err(error) => {
            record_refused(app, "stock", &error);
            return false;
        }
    }
    let coins = shop.coin.as_ref().map_or(0, |coin| coin.count);
    let items: Vec<InventoryItem> = shop.wares.into_iter().chain(shop.coin).collect();
    let stocked = items.len();
    let request = request::SetRequest {
        owner: shop.owner,
        kind: OwnerKind::Shop,
        items,
    };
    let owner = request.owner.clone();
    match operations::set(&directory, request) {
        Ok(_) => {
            app.record_line(format!(
                "WORLD shop_stocked owner={} items={stocked} coins={coins}",
                logged_id(&owner)
            ));
            true
        }
        Err(error) => {
            record_refused(app, "stock", &error);
            false
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
            stowed: Vec::new(),
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
        for stowed in &self.stowed {
            let held = |id: &str| self.item(id).map_or(0, |item| item.count);
            let put_away: u64 = self
                .stowed
                .iter()
                .filter(|other| other.item == stowed.item)
                .map(|other| other.count)
                .sum();
            if stowed.count == 0 || held(&stowed.container) == 0 || put_away > held(&stowed.item) {
                return Err(InventoryError::InvalidItem {
                    item: stowed.item.clone(),
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
                    name,
                    look,
                    usage,
                    price_cp,
                    ..
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
                    price_cp: *price_cp,
                    coin: None,
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
    bag::settle_stowed(&mut after)?;
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
        // The price goes with the thing.
        price_cp: words.price_cp,
        why: why.to_owned(),
    };
    if let Some(held) = to.item(item)
        && held.coin != words.coin
    {
        return Err(InventoryError::CoinMismatch {
            owner: to.owner.clone(),
            item: item.to_owned(),
        });
    }
    let (mut to_after, mut received) = apply_changes(to, &[given])?;
    // Money stays money: a coin the receiver did not hold arrives as that coin.
    if let Some(arrived) = to_after.items.iter_mut().find(|held| held.id == item) {
        arrived.coin = words.coin;
    }
    out[0].kind = ChangeKind::TransferOut;
    received[0].kind = ChangeKind::TransferIn;
    out.extend(received);
    Ok((from_after, to_after, out))
}

/// A deal between two inventories, both sides or neither: each item and count of `a_gives` moves
/// from `a` to `b` and each of `b_gives` from `b` to `a`, one [`transfer`] each, in order. An
/// agreed deal and a purchase both settle through it.
pub fn exchange(
    (a, a_gives): (&Inventory, &[ItemCount]),
    (b, b_gives): (&Inventory, &[ItemCount]),
    why: &str,
) -> Result<(Inventory, Inventory, Vec<InventoryApplied>), InventoryError> {
    let (mut first, mut second) = (a.clone(), b.clone());
    let mut applied = Vec::new();
    for named in a_gives {
        let (from, to, moved) = transfer(&first, &second, (named.item.as_str(), named.count), why)?;
        (first, second) = (from, to);
        applied.extend(moved);
    }
    for named in b_gives {
        let (from, to, moved) = transfer(&second, &first, (named.item.as_str(), named.count), why)?;
        (second, first) = (from, to);
        applied.extend(moved);
    }
    Ok((first, second, applied))
}
