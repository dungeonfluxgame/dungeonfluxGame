//! The facts of a place, as the JSON the persona prompts embed.
//!
//! Structural only: the stalls and items of the place, each item's quote for one unit and for the
//! smallest lot of each bulk tier. The persona voices these numbers and never invents others.

use df_content::world_bible::{Place, PriceQuote, WorldBible, quote};
use serde_json::{Value, json};

/// The facts of `place`: its identity, and for each stall the keeper's priced items.
pub(super) fn place_facts(bible: &WorldBible, place: &Place) -> Value {
    let stalls: Vec<Value> = place
        .stalls
        .iter()
        .map(|stall| {
            let items: Vec<Value> = stall
                .sells
                .iter()
                .filter_map(|id| bible.item(id))
                .map(|item| {
                    let quotes: Vec<PriceQuote> = std::iter::once(1)
                        .chain(stall.bulk.iter().map(|tier| tier.min_qty))
                        .filter_map(|qty| quote(item, qty, stall).ok())
                        .collect();
                    json!({
                        "id": item.id,
                        "name": item.name,
                        "unit": item.unit,
                        "tags": item.tags,
                        "quotes": quotes,
                    })
                })
                .collect();
            json!({
                "keeper": stall.keeper,
                "haggle_floor_permille": stall.haggle_floor_permille,
                "items": items,
            })
        })
        .collect();
    json!({
        "audience": "public",
        "currency": "cp",
        "place": {"id": place.id, "name": place.name, "description": place.description},
        "stalls": stalls,
    })
}
