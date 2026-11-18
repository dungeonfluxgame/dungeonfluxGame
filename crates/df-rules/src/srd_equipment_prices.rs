//! SRD 5.2.1 equipment: every weapon, armour, tool and adventuring gear item of the Equipment
//! chapter (SRD pp.89-100) with its cost, weight, category and printed page, what a container
//! holds, what a pack contains, the weight of a coin, and carrying capacity (SRD p.178). The
//! Wizard's Spellbook (SRD p.78) is here too: its weight is printed, its cost is not.
//!
//! Owns: the catalog ([`SRD_521_EQUIPMENT`], in `catalog`), [`equipment`] and [`price_cp`] to look
//! an item up, [`Weight`] and [`carrying_capacity`]. Never does: price an item the SRD does not
//! list (magic items beyond the gear table's two, trade goods), apply sale rules (see
//! `local_journey::selling_value_cp`), or move items (the host's inventory does that).
//! Fails: by returning `None` for an unknown id and for a unit price that is not a whole copper
//! piece (20 sling bullets cost 4 CP) or no cost is printed (the Spellbook). Callers must treat
//! `None` as "no price", never as free.
//!
//! An entry's cost and weight are the printed row: `cost` buys `per` units (SRD p.96: 20 arrows for
//! 1 GP), and `weight` is the weight of those `per` units. A unit's weight is always a whole number
//! of thousandths of a pound. Weights print as `—` for a negligible weight, which is zero here.

mod catalog;
#[cfg(test)]
mod equipment_tests;

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::coins::Price;
use crate::local_journey::SpeciesSize;

pub use self::catalog::SRD_521_EQUIPMENT;

/// A weight, in thousandths of a pound, so the SRD's fractions (1/4 lb, 1/2 lb, 1½ lb, a coin's
/// 1/50 lb) are exact.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
pub struct Weight {
    milli_pounds: u64,
}

impl Weight {
    pub const ZERO: Weight = Weight { milli_pounds: 0 };

    /// A weight of `milli_pounds` thousandths of a pound.
    pub const fn from_milli_pounds(milli_pounds: u64) -> Self {
        Self { milli_pounds }
    }

    /// A weight of `pounds` whole pounds.
    pub const fn pounds(pounds: u64) -> Self {
        Self {
            milli_pounds: pounds * 1000,
        }
    }

    /// The weight in thousandths of a pound.
    pub const fn milli_pounds(self) -> u64 {
        self.milli_pounds
    }

    /// The sum, or `None` when it does not fit.
    pub fn checked_add(self, other: Weight) -> Option<Weight> {
        self.milli_pounds
            .checked_add(other.milli_pounds)
            .map(Weight::from_milli_pounds)
    }

    /// `count` of this weight, or `None` when it does not fit.
    pub fn checked_times(self, count: u64) -> Option<Weight> {
        self.milli_pounds
            .checked_mul(count)
            .map(Weight::from_milli_pounds)
    }
}

/// Pounds with the fraction as decimals, trailing zeros dropped: `2 lb`, `0.25 lb`, `58.5 lb`.
impl fmt::Display for Weight {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, rest) = (self.milli_pounds / 1000, self.milli_pounds % 1000);
        if rest == 0 {
            return write!(formatter, "{whole} lb");
        }
        let decimals = format!("{rest:03}");
        write!(formatter, "{whole}.{} lb", decimals.trim_end_matches('0'))
    }
}

/// SRD p.89: "fifty coins weigh a pound", so one coin of any kind weighs 1/50 lb.
pub const COIN_WEIGHT: Weight = Weight::from_milli_pounds(20);

/// The section of the Equipment chapter an item is listed in.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentCategory {
    /// Weapons table, SRD p.91.
    SimpleWeapon,
    MartialWeapon,
    /// Armor table, SRD p.92.
    LightArmor,
    MediumArmor,
    HeavyArmor,
    Shield,
    /// Tools, SRD pp.93-94: Artisan's Tools and Other Tools.
    Tool,
    /// Adventuring Gear table, SRD p.95, with the focus and holy symbol tables (pp.96-97).
    AdventuringGear,
    /// Ammunition table, SRD p.96.
    Ammunition,
    /// A pack of the Adventuring Gear table, sold whole and carried as its contents.
    Pack,
    /// An item a class feature gives (the Wizard's Spellbook, SRD p.78).
    ClassGear,
}

impl EquipmentCategory {
    pub fn key(self) -> &'static str {
        match self {
            Self::SimpleWeapon => "simple_weapon",
            Self::MartialWeapon => "martial_weapon",
            Self::LightArmor => "light_armor",
            Self::MediumArmor => "medium_armor",
            Self::HeavyArmor => "heavy_armor",
            Self::Shield => "shield",
            Self::Tool => "tool",
            Self::AdventuringGear => "adventuring_gear",
            Self::Ammunition => "ammunition",
            Self::Pack => "pack",
            Self::ClassGear => "class_gear",
        }
    }
}

/// What one container holds, as its SRD description prints it. Each part is absent when the
/// description does not give it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capacity {
    /// The most weight it holds ("holds up to 30 pounds").
    pub weight: Option<Weight>,
    /// Its space in thousandths of a cubic foot ("within 1 cubic foot" is 1000).
    pub cubic_foot_thousandths: Option<u32>,
    /// The most liquid it holds, in fluid ounces (1 pint is 16, 1 gallon 128).
    pub fluid_ounces: Option<u32>,
    /// The only things it holds and the most of each it takes alone (a Quiver, 20 arrows; a Map or
    /// Scroll Case, 10 sheets of paper or 5 of parchment). Empty when it is not limited to some.
    pub only: &'static [(&'static str, u32)],
}

/// One item of the Equipment chapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EquipmentItem {
    /// The inventory id: the SRD name in lower case, words joined by `-`, with no apostrophe,
    /// comma or bracket (the baseline's `arrow` and `dungeoneer-pack` keep their own ids).
    pub id: &'static str,
    pub name: &'static str,
    pub category: EquipmentCategory,
    /// What `per` units cost; none when the SRD prints no cost.
    pub cost: Option<Price>,
    /// How many units the cost and the weight are for (1 for all but ammunition).
    pub per: u32,
    /// What `per` units weigh.
    pub weight: Weight,
    /// The printed page of the table row.
    pub srd_page: u16,
    /// What it holds, for a container.
    pub capacity: Option<Capacity>,
    /// What a pack contains, by id and count (SRD pp.96-97).
    pub contents: &'static [(&'static str, u32)],
    /// Using it spends it: ammunition, a day's Rations, a flask of Oil, a Torch or Candle burnt
    /// down, a sheet written on, a vial thrown or drunk (each by its SRD description).
    pub used_up: bool,
}

impl EquipmentItem {
    /// The weight of one unit.
    pub fn unit_weight(&self) -> Weight {
        Weight::from_milli_pounds(self.weight.milli_pounds() / u64::from(self.per.max(1)))
    }

    /// What `count` units cost, or `None` when no cost is printed, that is not a whole copper
    /// piece, or it does not fit.
    pub fn price_for(&self, count: u64) -> Option<Price> {
        let per = u64::from(self.per.max(1));
        let total = self.cost?.copper().checked_mul(count)?;
        (total % per == 0).then(|| Price::from_copper(total / per))
    }
}

/// The ids creation's kits name an item by (its kit name's id) where the catalog id differs: a
/// kit's "20 Arrows" is `arrows`, the catalog's `arrow`; a kit's "Book (history)" is a Book.
const KIT_IDS: [(&str, &str); 7] = [
    ("arrows", "arrow"),
    ("bolts", "bolt"),
    ("dungeoneers-pack", "dungeoneer-pack"),
    ("book-history", "book"),
    ("book-occult-lore", "book"),
    ("book-prayers", "book"),
    ("parchment-sheets", "parchment"),
];

/// The id of what `name` names: its name in lower case, words joined by `-`, with no apostrophe,
/// bracket or comma ("Dungeoneer's Pack" is `dungeoneers-pack`). A kit's item ids are made so, and
/// the host's log id of a reason is made from it.
pub fn item_id(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|character| !matches!(character, '\'' | '(' | ')' | ','))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

/// The catalog entry of `id`, by its own id or the id a creation kit names it by.
pub fn equipment(id: &str) -> Option<&'static EquipmentItem> {
    let id = KIT_IDS
        .iter()
        .find(|(kit, _)| *kit == id)
        .map_or(id, |(_, own)| own);
    SRD_521_EQUIPMENT.iter().find(|item| item.id == id)
}

/// The price in copper of one unit of `id`, or `None` when the id is unknown or one unit has no
/// whole-copper price (see the module doc).
pub fn price_cp(id: &str) -> Option<Price> {
    equipment(id)?.price_for(1)
}

/// The most weight a creature of `size` and Strength score `strength` can carry: SRD p.178,
/// Carrying Capacity table, Small/Medium "Str. × 15 lb.". A hero is Small or Medium (SRD
/// pp.84-86, each species' Size); the table's Tiny (× 7.5), Large (× 30), Huge (× 60) and
/// Gargantuan (× 120) rows are not hero sizes. Over it, a creature can drag, lift or push, but its
/// Speed is at most 5 feet.
pub fn carrying_capacity(strength: u8, size: SpeciesSize) -> Weight {
    match size {
        SpeciesSize::Small | SpeciesSize::Medium => Weight::pounds(u64::from(strength) * 15),
    }
}
