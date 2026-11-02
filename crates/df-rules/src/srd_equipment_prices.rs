//! SRD 5.2.1 equipment prices in copper pieces, for the ids in `local_journey::SRD_521_BASELINE.equipment`.
//!
//! Owns: the table of those prices with the printed SRD page of each, and `price_cp` to look one up.
//! Never does: price an item the table does not list (magic items, trade goods, other gear), change
//! the baseline, or apply sale rules (see `local_journey::selling_value_cp`).
//! Fails: by returning `None` for an unknown id, and for the two baseline ids that have no single SRD
//! price: `gaming-set` (the price varies by set, SRD p.94) and `gold-piece` (a coin, not a priced
//! item). Callers must treat `None` as "no price", never as a free item.
//!
//! Prices are per single unit as the SRD lists them. Arrows are sold in twenties (SRD p.96: 20 arrows
//! for 1 GP), so one arrow is 5 cp.

use crate::coins::Price;

/// One SRD equipment price: the id used by the baseline, the price of one unit, and the printed SRD
/// page the price comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EquipmentPrice {
    pub id: &'static str,
    pub price: Price,
    pub srd_page: u16,
}

/// Prices for the priced ids of the SRD 5.2.1 baseline. Copper values: 1 gp = 100 cp, 1 sp = 10 cp
/// (SRD p.89).
pub const SRD_521_EQUIPMENT_PRICES: &[EquipmentPrice] = &[
    // Weapons table, SRD p.91.
    EquipmentPrice {
        id: "greatsword",
        price: Price::from_copper(5_000),
        srd_page: 91,
    }, // 50 gp
    EquipmentPrice {
        id: "flail",
        price: Price::from_copper(1_000),
        srd_page: 91,
    }, // 10 gp
    EquipmentPrice {
        id: "javelin",
        price: Price::from_copper(50),
        srd_page: 91,
    }, // 5 sp
    EquipmentPrice {
        id: "spear",
        price: Price::from_copper(100),
        srd_page: 91,
    }, // 1 gp
    EquipmentPrice {
        id: "shortbow",
        price: Price::from_copper(2_500),
        srd_page: 91,
    }, // 25 gp
    // Armor table, SRD p.92.
    EquipmentPrice {
        id: "chain-mail",
        price: Price::from_copper(7_500),
        srd_page: 92,
    }, // 75 gp
    // Adventuring Gear table, SRD p.95.
    EquipmentPrice {
        id: "dungeoneer-pack",
        price: Price::from_copper(1_200),
        srd_page: 95,
    }, // 12 gp
    EquipmentPrice {
        id: "healers-kit",
        price: Price::from_copper(500),
        srd_page: 95,
    }, // 5 gp
    EquipmentPrice {
        id: "quiver",
        price: Price::from_copper(100),
        srd_page: 95,
    }, // 1 gp
    EquipmentPrice {
        id: "travelers-clothes",
        price: Price::from_copper(200),
        srd_page: 95,
    }, // 2 gp
    // Ammunition table, SRD p.96: 20 arrows for 1 gp, so one arrow is 5 cp.
    EquipmentPrice {
        id: "arrow",
        price: Price::from_copper(5),
        srd_page: 96,
    },
];

/// The price in copper of one unit of the baseline equipment item `id`, or `None` when the id is
/// unknown or has no single SRD price (see the module doc).
pub fn price_cp(id: &str) -> Option<Price> {
    SRD_521_EQUIPMENT_PRICES
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.price)
}
