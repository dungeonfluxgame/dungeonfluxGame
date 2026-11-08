//! The world bible: canonical facts a designer edits as data
//! (`planning/persona-voice-and-world-bible.md`).
//!
//! A bible holds `items` (what a thing is worth), `places` (who keeps a stall there and what it
//! sells, with bulk tiers and a haggle floor) and `voice_packs` (how a community talks). The code
//! prices; the model only voices the quoted number. [`WorldBible::from_json_str`] checks a bible
//! whole: ids are lowercase letters, digits, `_` or `-` and unique within their list, every
//! reference resolves, and every range is sane.
//!
//! Money is whole copper pieces (`cp`). A quote is integer math on totals: the bulk discount and
//! the
//! haggle floor are taken off the lot and rounded up, a lot is never below 1 cp a unit, and every
//! product saturates instead of overflowing.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::persona::PersonaId;

/// The longest id any entry of the bible has.
pub const MAXIMUM_ID_CHARS: usize = 64;
/// A permille that means "the whole price".
pub const WHOLE_PERMILLE: u32 = 1000;

/// What a thing is worth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub unit: String,
    /// The price of one unit before any bulk tier, in copper pieces (at least 1).
    pub base_value_cp: u64,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A coin of the world and what it is worth in copper pieces, the unit every amount of the bible
/// is kept in (the 2024 rules' gold piece is 100, the silver piece 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coin {
    pub id: String,
    /// What follows a count when an amount is said ("gold" in "450 gold").
    pub name: String,
    pub value_cp: u64,
}

/// A discount when a customer takes at least `min_qty` units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkTier {
    pub min_qty: u32,
    /// Off the unit price, from 1 to 999 (a hundred is ten percent).
    pub discount_permille: u32,
    /// Whether the discount is posted where a customer can read it; an unposted tier is the
    /// keeper's to offer.
    #[serde(default)]
    pub posted: bool,
}

/// One keeper's stall: what they sell and how far they will bargain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stall {
    /// The persona id of the keeper.
    pub keeper: String,
    pub sells: Vec<String>,
    /// Ascending by `min_qty`; a larger lot never costs more per unit.
    #[serde(default)]
    pub bulk: Vec<BulkTier>,
    /// The lowest total the keeper will take, in permille of the total asked, from 1 to 1000.
    #[serde(default = "whole_permille")]
    pub haggle_floor_permille: u32,
}

fn whole_permille() -> u32 {
    WHOLE_PERMILLE
}

/// A place the active scene can be at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub stalls: Vec<Stall>,
}

/// How a community talks, in word choice, rhythm and idiom, never in spelling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicePack {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub rhythm: String,
    #[serde(default)]
    pub lexicon: Vec<String>,
    #[serde(default)]
    pub idioms: Vec<String>,
    /// Style samples, never facts and never to be repeated verbatim.
    #[serde(default)]
    pub samples: Vec<String>,
    #[serde(default)]
    pub avoid: Vec<String>,
}

/// The whole bible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldBible {
    /// The coins prices are said in; amounts are always kept in copper.
    #[serde(default)]
    pub coins: Vec<Coin>,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub places: Vec<Place>,
    #[serde(default)]
    pub voice_packs: Vec<VoicePack>,
}

/// Where the bible entry that is wrong is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum BibleError {
    /// The text is not the JSON a bible is; `detail` is the reader's own sentence.
    NotABible {
        line: usize,
        column: usize,
        detail: String,
    },
    /// `id` of a `list` entry is empty, too long or has a character outside `a-z 0-9 _ -`.
    BadId {
        list: String,
        id: String,
    },
    DuplicateId {
        list: String,
        id: String,
    },
    /// A text that must say something is empty.
    EmptyText {
        path: String,
    },
    /// A number outside the range its `path` allows.
    OutOfRange {
        path: String,
        allowed: String,
    },
    /// `path` names `id`, which `list` does not hold.
    Dangling {
        path: String,
        list: String,
        id: String,
    },
    /// A stall's keeper is not a valid persona id.
    BadKeeper {
        path: String,
        keeper: String,
    },
    /// A list that must be strictly increasing is not.
    NotAscending {
        path: String,
    },
    /// A list that must not repeat an entry does.
    Repeated {
        path: String,
        entry: String,
    },
}

impl BibleError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::NotABible { .. } => "not_a_bible",
            Self::BadId { .. } => "bad_id",
            Self::DuplicateId { .. } => "duplicate_id",
            Self::EmptyText { .. } => "empty_text",
            Self::OutOfRange { .. } => "out_of_range",
            Self::Dangling { .. } => "dangling",
            Self::BadKeeper { .. } => "bad_keeper",
            Self::NotAscending { .. } => "not_ascending",
            Self::Repeated { .. } => "repeated",
        }
    }
}

impl fmt::Display for BibleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABible {
                line,
                column,
                detail,
            } => write!(
                formatter,
                "the world bible is not valid JSON (line {line}, column {column}): {detail}"
            ),
            Self::BadId { list, id } => write!(
                formatter,
                "{id:?} in {list} is not an id (1 to {MAXIMUM_ID_CHARS} of a-z, 0-9, '_' or '-')"
            ),
            Self::DuplicateId { list, id } => write!(formatter, "{list} holds {id:?} twice"),
            Self::EmptyText { path } => write!(formatter, "{path} is empty"),
            Self::OutOfRange { path, allowed } => {
                write!(formatter, "{path} must be {allowed}")
            }
            Self::Dangling { path, list, id } => {
                write!(formatter, "{path} names {id:?}, which {list} does not hold")
            }
            Self::BadKeeper { path, keeper } => {
                write!(formatter, "{path} {keeper:?} is not a persona id")
            }
            Self::NotAscending { path } => write!(formatter, "{path} must be in ascending order"),
            Self::Repeated { path, entry } => write!(formatter, "{path} holds {entry:?} twice"),
        }
    }
}

impl std::error::Error for BibleError {}

/// Whether `text` is an id of the bible.
pub fn is_valid_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAXIMUM_ID_CHARS
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn check_id(list: &str, id: &str, seen: &mut BTreeSet<String>) -> Result<(), BibleError> {
    if !is_valid_id(id) {
        return Err(BibleError::BadId {
            list: list.to_owned(),
            id: id.to_owned(),
        });
    }
    if !seen.insert(id.to_owned()) {
        return Err(BibleError::DuplicateId {
            list: list.to_owned(),
            id: id.to_owned(),
        });
    }
    Ok(())
}

fn check_text(path: String, text: &str) -> Result<(), BibleError> {
    if text.trim().is_empty() {
        Err(BibleError::EmptyText { path })
    } else {
        Ok(())
    }
}

fn check_range(path: String, value: u64, low: u64, high: u64) -> Result<(), BibleError> {
    if (low..=high).contains(&value) {
        Ok(())
    } else {
        Err(BibleError::OutOfRange {
            path,
            allowed: format!("from {low} to {high}"),
        })
    }
}

impl WorldBible {
    /// The bible in `text`, checked whole.
    pub fn from_json_str(text: &str) -> Result<Self, BibleError> {
        let bible: Self = serde_json::from_str(text).map_err(|error| BibleError::NotABible {
            line: error.line(),
            column: error.column(),
            detail: error.to_string(),
        })?;
        bible.validate()?;
        Ok(bible)
    }

    /// Checks ids, references and ranges.
    pub fn validate(&self) -> Result<(), BibleError> {
        let mut coin_ids = BTreeSet::new();
        for (index, coin) in self.coins.iter().enumerate() {
            check_id("coins", &coin.id, &mut coin_ids)?;
            check_text(format!("coins[{index}].name"), &coin.name)?;
            check_range(
                format!("coins[{index}].value_cp"),
                coin.value_cp,
                1,
                u64::MAX,
            )?;
        }
        let mut item_ids = BTreeSet::new();
        for (index, item) in self.items.iter().enumerate() {
            check_id("items", &item.id, &mut item_ids)?;
            check_text(format!("items[{index}].name"), &item.name)?;
            check_text(format!("items[{index}].unit"), &item.unit)?;
            check_range(
                format!("items[{index}].base_value_cp"),
                item.base_value_cp,
                1,
                u64::MAX,
            )?;
        }
        let mut place_ids = BTreeSet::new();
        for (index, place) in self.places.iter().enumerate() {
            check_id("places", &place.id, &mut place_ids)?;
            check_text(format!("places[{index}].name"), &place.name)?;
            for (stall_index, stall) in place.stalls.iter().enumerate() {
                let path = format!("places[{index}].stalls[{stall_index}]");
                check_stall(&path, stall, &item_ids)?;
            }
        }
        let mut pack_ids = BTreeSet::new();
        for (index, pack) in self.voice_packs.iter().enumerate() {
            check_id("voice_packs", &pack.id, &mut pack_ids)?;
            check_text(
                format!("voice_packs[{index}].description"),
                &pack.description,
            )?;
        }
        Ok(())
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn place(&self, id: &str) -> Option<&Place> {
        self.places.iter().find(|place| place.id == id)
    }

    pub fn voice_pack(&self, id: &str) -> Option<&VoicePack> {
        self.voice_packs.iter().find(|pack| pack.id == id)
    }

    /// The quote for `qty` of `item_id` at the first stall of `place_id` that sells it.
    pub fn quote_at(
        &self,
        place_id: &str,
        item_id: &str,
        qty: u32,
    ) -> Result<PriceQuote, QuoteError> {
        let place = self
            .place(place_id)
            .ok_or_else(|| QuoteError::UnknownPlace {
                id: place_id.to_owned(),
            })?;
        let item = self.item(item_id).ok_or_else(|| QuoteError::UnknownItem {
            id: item_id.to_owned(),
        })?;
        let stall = place
            .stalls
            .iter()
            .find(|stall| stall.sells.contains(&item.id))
            .ok_or_else(|| QuoteError::NotSold {
                item: item.id.clone(),
                place: place.id.clone(),
            })?;
        quote(item, qty, stall)
    }

    /// `amount_cp` as it is said, in the largest coins that fit ("608 gold and 5 silver"); `None`
    /// when the bible has no coins, or none small enough to make up the amount exactly.
    pub fn spoken_amount(&self, amount_cp: u64) -> Option<String> {
        let mut coins: Vec<&Coin> = self.coins.iter().collect();
        coins.sort_by_key(|coin| std::cmp::Reverse(coin.value_cp));
        let mut left = amount_cp;
        let mut parts = Vec::new();
        for coin in coins {
            let count = left / coin.value_cp;
            if count > 0 {
                parts.push(format!("{count} {}", coin.name));
                left -= count * coin.value_cp;
            }
        }
        (left == 0 && !parts.is_empty()).then(|| parts.join(" and "))
    }

    /// The quote for `qty` of `item_id` at the stall `keeper` keeps; `None` when the bible has no
    /// such stall, item or a zero quantity.
    pub fn quote_for_keeper(&self, keeper: &str, item_id: &str, qty: u32) -> Option<PriceQuote> {
        let item = self.item(item_id)?;
        let stall = self
            .places
            .iter()
            .flat_map(|place| &place.stalls)
            .find(|stall| stall.keeper == keeper && stall.sells.contains(&item.id))?;
        quote(item, qty, stall).ok()
    }
}

fn check_stall(path: &str, stall: &Stall, item_ids: &BTreeSet<String>) -> Result<(), BibleError> {
    if PersonaId::new(&stall.keeper).is_err() {
        return Err(BibleError::BadKeeper {
            path: format!("{path}.keeper"),
            keeper: stall.keeper.clone(),
        });
    }
    if stall.sells.is_empty() {
        return Err(BibleError::EmptyText {
            path: format!("{path}.sells"),
        });
    }
    let mut sold = BTreeSet::new();
    for id in &stall.sells {
        if !item_ids.contains(id) {
            return Err(BibleError::Dangling {
                path: format!("{path}.sells"),
                list: "items".to_owned(),
                id: id.clone(),
            });
        }
        if !sold.insert(id) {
            return Err(BibleError::Repeated {
                path: format!("{path}.sells"),
                entry: id.clone(),
            });
        }
    }
    let mut previous: Option<BulkTier> = None;
    for (index, tier) in stall.bulk.iter().enumerate() {
        let tier_path = format!("{path}.bulk[{index}]");
        check_range(
            format!("{tier_path}.min_qty"),
            tier.min_qty.into(),
            2,
            u32::MAX.into(),
        )?;
        check_range(
            format!("{tier_path}.discount_permille"),
            tier.discount_permille.into(),
            1,
            (WHOLE_PERMILLE - 1).into(),
        )?;
        if let Some(before) = previous
            && (tier.min_qty <= before.min_qty || tier.discount_permille < before.discount_permille)
        {
            return Err(BibleError::NotAscending {
                path: format!("{path}.bulk"),
            });
        }
        previous = Some(*tier);
    }
    check_range(
        format!("{path}.haggle_floor_permille"),
        stall.haggle_floor_permille.into(),
        1,
        WHOLE_PERMILLE.into(),
    )
}

/// Why a quote cannot be made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum QuoteError {
    ZeroQuantity,
    UnknownPlace {
        id: String,
    },
    UnknownItem {
        id: String,
    },
    NotSold {
        item: String,
        place: String,
    },
    /// The stall given to `quote` does not sell the item.
    StallDoesNotSell {
        item: String,
    },
}

impl QuoteError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::ZeroQuantity => "zero_quantity",
            Self::UnknownPlace { .. } => "unknown_place",
            Self::UnknownItem { .. } => "unknown_item",
            Self::NotSold { .. } => "not_sold",
            Self::StallDoesNotSell { .. } => "stall_does_not_sell",
        }
    }
}

impl fmt::Display for QuoteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroQuantity => formatter.write_str("a quote needs at least one unit"),
            Self::UnknownPlace { id } => write!(formatter, "the bible has no place {id:?}"),
            Self::UnknownItem { id } => write!(formatter, "the bible has no item {id:?}"),
            Self::NotSold { item, place } => {
                write!(formatter, "no stall at {place:?} sells {item:?}")
            }
            Self::StallDoesNotSell { item } => {
                write!(formatter, "the stall does not sell {item:?}")
            }
        }
    }
}

impl std::error::Error for QuoteError {}

/// What `qty` units cost at a stall. All amounts are copper pieces. The three totals are the
/// truth; the `*_each_cp` fields are display only (a total over `qty`, rounded up).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceQuote {
    pub qty: u32,
    /// The unit price before any bulk tier: the item's base value.
    pub ask_each_cp: u64,
    /// The discount applied, in permille; 0 when no tier is reached.
    pub discount_permille: u32,
    /// `ask_each_cp * qty`: the lot with no bulk discount.
    pub ask_total_cp: u64,
    /// The lot after the bulk tier, rounded up on the total, at least `qty` and never above
    /// `ask_total_cp`: what the keeper asks.
    pub tier_total_cp: u64,
    /// The lowest total the keeper takes for the lot: the haggle floor of `ask_total_cp`, rounded
    /// up, at least 1 and never above `tier_total_cp`.
    pub floor_total_cp: u64,
    /// `tier_total_cp` over `qty`, rounded up (display only).
    pub tier_each_cp: u64,
    /// `floor_total_cp` over `qty`, rounded up (display only).
    pub floor_each_cp: u64,
}

/// `value * permille / 1000`, rounded up and saturating.
fn permille_of(value: u64, permille: u32) -> u64 {
    let scaled = u128::from(value) * u128::from(permille);
    let rounded = scaled.div_ceil(u128::from(WHOLE_PERMILLE));
    u64::try_from(rounded).unwrap_or(u64::MAX)
}

/// The price of `qty` of `item` at `stall`. The item must be one the stall sells.
pub fn quote(item: &Item, qty: u32, stall: &Stall) -> Result<PriceQuote, QuoteError> {
    if qty == 0 {
        return Err(QuoteError::ZeroQuantity);
    }
    if !stall.sells.contains(&item.id) {
        return Err(QuoteError::StallDoesNotSell {
            item: item.id.clone(),
        });
    }
    let ask_each_cp = item.base_value_cp.max(1);
    let discount_permille = stall
        .bulk
        .iter()
        .filter(|tier| tier.min_qty <= qty)
        .map(|tier| tier.discount_permille)
        .max()
        .unwrap_or(0);
    let ask_total_cp = ask_each_cp.saturating_mul(u64::from(qty));
    let tier_total_cp = permille_of(ask_total_cp, WHOLE_PERMILLE - discount_permille)
        .max(u64::from(qty))
        .min(ask_total_cp);
    let floor_total_cp = permille_of(
        ask_total_cp,
        stall.haggle_floor_permille.min(WHOLE_PERMILLE),
    )
    .clamp(1, tier_total_cp);
    let each = |total: u64| total.div_ceil(u64::from(qty));
    Ok(PriceQuote {
        qty,
        ask_each_cp,
        discount_permille,
        ask_total_cp,
        tier_total_cp,
        floor_total_cp,
        tier_each_cp: each(tier_total_cp),
        floor_each_cp: each(floor_total_cp),
    })
}

/// `proposed_cp`, a total the model chose for the lot, held inside what the keeper allows: never
/// below `floor_total_cp` and never above `tier_total_cp`.
pub fn clamp_quote(proposed_cp: u64, quote: &PriceQuote) -> u64 {
    proposed_cp.clamp(quote.floor_total_cp, quote.tier_total_cp)
}
