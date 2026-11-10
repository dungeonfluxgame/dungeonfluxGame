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

pub mod cli;
mod operations;
pub mod request;
pub mod routes;
mod screen;
mod store;

use std::collections::BTreeMap;
use std::fmt;

use df_content::world_bible::is_valid_id;
use serde::{Deserialize, Serialize};

pub use self::request::InventoryError;
use self::routes::{record_changed, record_refused};
pub use self::screen::follow_judged;
use crate::state::AppState;

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
}

impl InventoryReason {
    pub const ALL: [Self; 7] = [
        Self::Gained,
        Self::UsedUp,
        Self::Sold,
        Self::WornOut,
        Self::Broken,
        Self::Lost,
        Self::GivenAway,
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
}

impl GainSource {
    pub const ALL: [Self; 4] = [Self::Exchange, Self::Found, Self::Given, Self::Made];

    pub fn key(self) -> &'static str {
        match self {
            Self::Exchange => "exchange",
            Self::Found => "found",
            Self::Given => "given",
            Self::Made => "made",
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
/// `found`) and of the characters present (for `given`).
pub struct GainContext<'a> {
    pub things: &'a [String],
    pub present: &'a [String],
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
fn logged_id(text: &str) -> &str {
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
        InventoryReason::Sold => true,
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

/// The things a gain's source asks to have gone down in the same line, when the source stands:
/// none for `found` and `given`, the listed items for `exchange` and `made`. `Err` when the source
/// is missing or its field does not hold up (no list, a count of zero, a thing not offered, a
/// giver who is not present or is the owner themself).
fn payments_of<'a>(
    owner: &str,
    delta: &'a InventoryDelta,
    context: &GainContext<'_>,
) -> Result<&'a [ItemCount], ()> {
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
        GainSource::Found => match &delta.place_thing {
            Some(thing) if context.things.contains(thing) => Ok(&[]),
            _ => Err(()),
        },
        GainSource::Given => match &delta.giver {
            Some(giver) if giver != owner && context.present.contains(giver) => Ok(&[]),
            _ => Err(()),
        },
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
        for (index, (change, delta)) in standing.iter().enumerate() {
            if !alive[index] || !is_gain(change) {
                continue;
            }
            let covered = payments_of(&inventory.owner, delta, context).is_ok_and(|payments| {
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
    let things: Vec<String> = if deltas
        .iter()
        .any(|delta| delta.source == Some(GainSource::Found))
    {
        crate::world::place_items(app)
            .into_iter()
            .map(|offered| offered.id)
            .collect()
    } else {
        Vec::new()
    };
    judge_deltas(
        inventory,
        deltas,
        &GainContext {
            things: &things,
            present,
        },
    )
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
