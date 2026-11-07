//! The facts of a place, as the JSON the persona prompts embed.
//!
//! Structural only: the stalls and items of the place, and each item's quote for one unit and for
//! the smallest lot of each bulk tier. The persona voices these numbers and never invents others.
//! What a viewer is told depends on who they are: the keeper of a stall sees all of it (ask, tiers,
//! floor), anyone else only what the stall posts, so a customer's persona cannot repeat the floor.

use df_content::world_bible::{Item, Place, PriceQuote, Stall, WorldBible, quote};
use serde_json::{Value, json};

/// What a customer's facts say about how to get a price.
const CUSTOMER_NOTE: &str = "Ask the keeper for a quote; only the keeper quotes a price.";

/// The lot sizes a keeper's facts quote ready-made (with each tier's smallest lot), so a keeper
/// reads a total instead of working one out.
const QUOTED_LOTS: [u32; 7] = [1, 2, 3, 5, 10, 15, 20];

/// The facts of `place` for `viewer`: `None` is keeper level for every stall (tools), a persona id
/// is keeper level for the stalls it keeps and posted facts for the rest.
pub(super) fn place_facts(bible: &WorldBible, place: &Place, viewer: Option<&str>) -> Value {
    let keeps = |stall: &Stall| viewer.is_none_or(|id| stall.keeper == id);
    let stalls: Vec<Value> = place
        .stalls
        .iter()
        .map(|stall| {
            let items: Vec<Value> = stall
                .sells
                .iter()
                .filter_map(|id| bible.item(id))
                .map(|item| {
                    if keeps(stall) {
                        keeper_item(item, stall)
                    } else {
                        posted_item(item, stall)
                    }
                })
                .collect();
            let mut facts = json!({"keeper": stall.keeper, "items": items});
            if keeps(stall) {
                facts["haggle_floor_permille"] = json!(stall.haggle_floor_permille);
            }
            facts
        })
        .collect();
    let mut facts = json!({
        "audience": "public",
        "currency": "cp",
        "place": {"id": place.id, "name": place.name, "description": place.description},
        "stalls": stalls,
    });
    if place.stalls.iter().any(keeps) {
        facts["viewer_role"] = json!("keeper");
    } else {
        facts["viewer_role"] = json!("customer");
        facts["note"] = json!(CUSTOMER_NOTE);
    }
    facts
}

/// An item as its keeper sees it: the ask, tier and floor totals for the common lot sizes and each
/// tier's smallest lot, and the rule for any other lot, in words built from the bible's numbers.
fn keeper_item(item: &Item, stall: &Stall) -> Value {
    let mut lots: Vec<u32> = QUOTED_LOTS
        .into_iter()
        .chain(stall.bulk.iter().map(|tier| tier.min_qty))
        .collect();
    lots.sort_unstable();
    lots.dedup();
    let quotes: Vec<PriceQuote> = lots
        .into_iter()
        .filter_map(|qty| quote(item, qty, stall).ok())
        .collect();
    json!({
        "id": item.id,
        "name": item.name,
        "unit": item.unit,
        "tags": item.tags,
        "quotes": quotes,
        "rule": price_rule(item, stall),
    })
}

/// How the keeper prices a lot of `item` that the quotes do not list.
fn price_rule(item: &Item, stall: &Stall) -> String {
    let mut rule = format!("{} cp each", item.base_value_cp);
    for tier in &stall.bulk {
        rule.push_str(&format!(
            "; from {} on, {} permille off the lot's total, rounded up to a whole copper",
            tier.min_qty, tier.discount_permille
        ));
    }
    rule.push_str(&format!(
        "; the lowest you take is {} permille of the lot's undiscounted total, rounded up",
        stall.haggle_floor_permille
    ));
    rule
}

/// An item as the stall posts it: the asking price of one unit, and for the smallest lot of each
/// posted bulk tier the asking total beside the tier's total. Never the floor.
fn posted_item(item: &Item, stall: &Stall) -> Value {
    let one = quote(item, 1, stall)
        .ok()
        .map(|one| json!({"qty": one.qty, "ask_total_cp": one.ask_total_cp}));
    let posted = stall
        .bulk
        .iter()
        .filter(|tier| tier.posted)
        .filter_map(|tier| quote(item, tier.min_qty, stall).ok())
        .map(|lot| {
            json!({
                "qty": lot.qty,
                "ask_total_cp": lot.ask_total_cp,
                "discount_permille": lot.discount_permille,
                "tier_total_cp": lot.tier_total_cp,
            })
        });
    let quotes: Vec<Value> = one.into_iter().chain(posted).collect();
    json!({
        "id": item.id,
        "name": item.name,
        "unit": item.unit,
        "tags": item.tags,
        "ask_each_cp": item.base_value_cp.max(1),
        "quotes": quotes,
    })
}
