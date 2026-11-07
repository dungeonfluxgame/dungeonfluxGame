//! The world bible: canonical facts (item values, places, stalls) and voice packs that the persona
//! prompts read (`planning/persona-voice-and-world-bible.md`).
//!
//! The bible is data: `<playground>/world/bible.json`, read fresh on every call, so a designer's
//! edit needs no rebuild. A missing file is an empty bible; one that does not read or is not valid
//! is a typed [`WorldError`]. The active place is `<playground>/world/state.json`
//! (`{"place": "<id>"}`).
//!
//! `context_for_viewer` (keeper sees ask, tiers and floor; anyone else only the posted facts) and
//! `context_for` (keeper level, for tools) are the seams between the persona prompts and the
//! bible, and `check_quote` clamps a quoted total. Each answers an
//! empty context when there is nothing to say (no active place, an empty bible), and records one
//! `WORLD context_unavailable error=<key>` line when the bible cannot be had, so persona prompts
//! work without a bible and a broken one is visible in the log.
//!
//! Routes (each answers a `WorldReply`, the shared dev envelope with a typed `WorldError`):
//!
//! - `GET /dev/world`: a `WorldView` (the bible's ids, the active place and its facts).
//! - `POST /dev/world/place`: `{"place": "<id>"}` or `{"place": null}` sets or clears the place.
//! - `GET /dev/world/quote?item=<id>&qty=<n>`: the price quote at the active place.
//! - `GET /dev/world/validate`: reads the bible now and counts what it holds.
//!
//! The dev CLI's `world` verbs are these routes.

pub mod cli;
mod facts;
pub mod request;
pub mod routes;
mod store;

pub use self::cli::{WorldCommand, parse};
pub use self::request::WorldError;
pub use self::routes::router_with;

use df_content::world_bible::clamp_quote;

use crate::state::AppState;

#[cfg(test)]
mod world_tests;

/// What the persona prompts take from the bible: the facts of the active place, and the voice
/// packs a character belongs to. Both are plain JSON the prompt builders embed as data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldContext {
    pub facts: serde_json::Value,
    pub voice_packs: Vec<serde_json::Value>,
}

impl WorldContext {
    pub fn empty() -> Self {
        Self::default()
    }
}

/// The bible's context at keeper level for every stall, for tools: the facts of the active place
/// and the packs named by `pack_ids`. A pack the bible does not hold is left out.
pub fn context_for(app: &AppState, pack_ids: &[String]) -> WorldContext {
    context(app, pack_ids, None)
}

/// The bible's context for a persona's prompt as `viewer_id` sees it: the keeper of a stall sees
/// all
/// of it (ask, tiers, floor, haggle room), anyone else only what the stall posts, so a customer is
/// never told the lowest price.
pub fn context_for_viewer(app: &AppState, pack_ids: &[String], viewer_id: &str) -> WorldContext {
    context(app, pack_ids, Some(viewer_id))
}

fn context(app: &AppState, pack_ids: &[String], viewer: Option<&str>) -> WorldContext {
    let directory = store::world_directory(app);
    let context = (|| {
        let bible = store::read_bible(&directory)?;
        let facts = match store::read_place(&directory)? {
            Some(id) => bible.place(&id).map_or(serde_json::Value::Null, |place| {
                facts::place_facts(&bible, place, viewer)
            }),
            None => serde_json::Value::Null,
        };
        let voice_packs = pack_ids
            .iter()
            .filter_map(|id| bible.voice_pack(id))
            .filter_map(|pack| serde_json::to_value(pack).ok())
            .collect();
        Ok::<_, WorldError>(WorldContext { facts, voice_packs })
    })();
    context.unwrap_or_else(|error| {
        app.record_line(format!("WORLD context_unavailable error={}", error.key()));
        WorldContext::empty()
    })
}

/// The voice packs the bible offers, for a model to choose a character's communities from: the id
/// and a one-line description of each, as JSON.
pub fn available_packs(app: &AppState) -> Vec<serde_json::Value> {
    let directory = store::world_directory(app);
    match store::read_bible(&directory) {
        Ok(bible) => bible
            .voice_packs
            .iter()
            .map(|pack| serde_json::json!({"id": pack.id, "description": pack.description}))
            .collect(),
        Err(error) => {
            app.record_line(format!("WORLD packs_unavailable error={}", error.key()));
            Vec::new()
        }
    }
}

/// The result of checking a quoted total against the stall's rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuoteCheck {
    pub proposed_total_cp: u32,
    pub clamped_total_cp: u32,
    pub floor_total_cp: u32,
    pub ask_total_cp: u32,
}

/// Checks the total a character quoted for `qty` of `item` at the stall `keeper` keeps against the
/// stall's floor and tier total, and clamps it between them. `ask_total_cp` is the tier total, what
/// the keeper asks for this lot. `None` when the bible, read fresh, has no such stall or item.
pub fn check_quote(
    app: &AppState,
    keeper: &str,
    item: &str,
    qty: u32,
    proposed_total_cp: u32,
) -> Option<QuoteCheck> {
    let bible = store::read_bible(&store::world_directory(app)).ok()?;
    let quote = bible.quote_for_keeper(keeper, item, qty)?;
    let narrow = |cp: u64| u32::try_from(cp).unwrap_or(u32::MAX);
    Some(QuoteCheck {
        proposed_total_cp,
        clamped_total_cp: narrow(clamp_quote(u64::from(proposed_total_cp), &quote)),
        floor_total_cp: narrow(quote.floor_total_cp),
        ask_total_cp: narrow(quote.tier_total_cp),
    })
}
