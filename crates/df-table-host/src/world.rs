//! The world bible: canonical facts (item values, places, stalls) and voice packs that the persona
//! prompts read (`planning/persona-voice-and-world-bible.md`).
//!
//! The bible is data: `<playground>/world/bible.json`, read fresh on every call, so a designer's
//! edit needs no rebuild. A missing file is an empty bible; one that does not read or is not valid
//! is a typed [`WorldError`]. The active place is `<playground>/world/state.json`
//! (`{"place": "<id>"}`, or a place designed in play held whole, see `store`).
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
//! - `POST /dev/world/arrive`: `{"place": {<place>}}` makes a place designed in play (a move's
//!   destination) the active one.
//! - `GET /dev/world/quote?item=<id>&qty=<n>`: the price quote at the active place.
//! - `GET /dev/world/validate`: reads the bible now and counts what it holds.
//! - `POST /dev/world/picture`: `{"item", "picture"}` keeps a close-up as the reference sheet of a
//!   thing at a place designed in play.
//! - `GET /dev/world/town`: the town the party is in, if any (see `town`).
//! - `POST /dev/world/walk`: notes one stage of a move's walk in the audit log (see `walk_note`).
//! - `/dev/world/inventory/..`: the standard inventory of any number of owners (see `inventory`).
//! - `/dev/world/memory/..`: the standard memory of any number of owners (see `memory`).
//! - `/dev/world/quests..`: the quests offered and taken (see `quest`).
//! - `/dev/world/region`: the region the party is in (see `region`).
//!
//! The dev CLI's `world` verbs are these routes.

pub mod cli;
mod facts;
pub mod inventory;
pub mod memory;
pub mod quest;
pub mod region;
pub mod request;
pub mod routes;
mod store;
mod town;
mod visited;
mod walk_note;

pub use self::cli::{WorldCommand, parse};
pub(crate) use self::inventory::logged_id;
pub use self::inventory::{
    CoinPrice, DealRecord, DealState, GainSource, Inventory, InventoryApplied, InventoryChange,
    InventoryDelta, InventoryError, InventoryItem, InventoryMiss, InventoryReason, ItemCount,
    ItemUse, OpenDeal, OpenDeals, ShopStock, change_own, deal_miss, drop_own, follow_judged,
    inventory_of, judged_miss, prices_in_coin, settle_deal, sheet_name, stock_shop,
    take_judged_changes, without_deal, worth_cp,
};
pub(crate) use self::quest::clear_quests;
pub use self::quest::{Quest, QuestRecord, QuestStanding, quests_of};
pub use self::region::{Region, region_of};
pub use self::request::WorldError;
pub use self::routes::router_with;
pub use self::town::{Town, TownView, end_town, seed_town, town_of};
pub use self::visited::{Visited, visited_of};
pub use self::walk_note::{WalkNoted, WalkOutcome, WalkStageNote};

use df_content::world_bible::clamp_quote;

use crate::state::AppState;

#[cfg(test)]
mod inventory_tests;
#[cfg(test)]
mod memory_tests;
#[cfg(test)]
mod quest_tests;
#[cfg(test)]
mod town_tests;
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
        let facts = store::read_active_place(&directory, &bible)?
            .map_or(serde_json::Value::Null, |place| {
                facts::place_facts(&bible, &place, viewer)
            });
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

/// `amount_cp` as a character says it, in the bible's coins ("608 gold and 5 silver"), or in
/// copper pieces ("60850 cp") when the bible has no coins to say it in.
pub fn spoken_amount(app: &AppState, amount_cp: u64) -> String {
    store::read_bible(&store::world_directory(app))
        .ok()
        .and_then(|bible| bible.spoken_amount(amount_cp))
        .unwrap_or_else(|| format!("{amount_cp} cp"))
}

/// A thing at the active place worth a close look: an item a stall sells, with its tags and the
/// keeper of the first stall that sells it (who knows how it looks up close), or a thing in sight
/// there, with its look and no keeper.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlaceItem {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeper: Option<String>,
    /// The close-up kept as its reference sheet, for a thing in sight that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// How many of it lie on the ground here, left there by a line (thrown, dropped, set down) and
    /// there to be picked back up; none for what belongs to the place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lying: Option<u64>,
}

/// Adds `things`, noticed by `via` (a feature's name), to the active place, which must be a place
/// designed in play. The place is checked as a designed place is (`arrive`), all of them are added
/// or none, and each is one `WORLD thing_added place=.. id=.. via=..` line; a refusal is one `WORLD
/// refused verb=add_things error=<key>` line. Answers the place's id.
pub async fn add_things(
    app: &AppState,
    via: &str,
    things: Vec<df_content::world_bible::Thing>,
) -> Result<String, WorldError> {
    keep_things(app, via, things, false).await
}

/// Puts `thing`, as it is now, at the active place ([`add_things`]): a thing the place has under
/// its id already is replaced by it (what its sign reads, kept on it), its close-up kept.
pub async fn put_thing(
    app: &AppState,
    via: &str,
    thing: df_content::world_bible::Thing,
) -> Result<String, WorldError> {
    keep_things(app, via, vec![thing], true).await
}

/// Notes `text` on the thing `id` of the active place as its note of kind `label` (its state, what
/// it reads, what is inside), replacing that kind's note before and never the thing's own look,
/// and names it `name` when given (an open gate shut is the gate). One `WORLD thing_noted place=
/// id= label=` line; a refusal is one `WORLD refused verb=note_thing error=` line.
pub async fn note_thing(
    app: &AppState,
    id: &str,
    label: &str,
    (text, name): (&str, Option<&str>),
) -> Result<String, WorldError> {
    let directory = store::world_directory(app);
    let (id_owned, label_owned) = (id.to_owned(), label.to_owned());
    let (text, name) = (text.to_owned(), name.map(str::to_owned));
    let noted = crate::blocking::run_blocking(
        move || {
            store::note_thing(
                &directory,
                &id_owned,
                &label_owned,
                (&text, name.as_deref()),
            )
        },
        WorldError::Stopped,
    )
    .await;
    match &noted {
        Ok(place) => app.record_line(format!(
            "WORLD thing_noted place={place} id={} label={}",
            logged_id(id),
            logged_id(label)
        )),
        Err(error) => app.record_line(format!(
            "WORLD refused verb=note_thing error={}",
            error.key()
        )),
    }
    noted
}

async fn keep_things(
    app: &AppState,
    via: &str,
    things: Vec<df_content::world_bible::Thing>,
    replace: bool,
) -> Result<String, WorldError> {
    let directory = store::world_directory(app);
    let ids: Vec<String> = things.iter().map(|thing| thing.id.clone()).collect();
    let added = crate::blocking::run_blocking(
        move || {
            let bible = store::read_bible(&directory)?;
            store::add_things(&directory, &bible, things, replace)
        },
        WorldError::Stopped,
    )
    .await;
    match &added {
        Ok(place) => {
            for id in &ids {
                app.record_line(format!("WORLD thing_added place={place} id={id} via={via}"));
            }
        }
        Err(error) => app.record_line(format!(
            "WORLD refused verb=add_things error={}",
            error.key()
        )),
    }
    added
}

/// The things worth a close look at the active place, for a judge that names one by id: the items
/// its stalls sell (each once, in the bible's order), then the things in sight there; empty with no
/// place, or when the bible cannot be had.
pub fn place_items(app: &AppState) -> Vec<PlaceItem> {
    let directory = store::world_directory(app);
    let items = (|| {
        let bible = store::read_bible(&directory)?;
        let Some(place) = store::read_active_place(&directory, &bible)? else {
            return Ok(Vec::new());
        };
        let mut items: Vec<PlaceItem> = Vec::new();
        for stall in &place.stalls {
            for id in &stall.sells {
                if let Some(item) = bible.item(id)
                    && !items.iter().any(|seen| seen.id == item.id)
                {
                    items.push(PlaceItem {
                        id: item.id.clone(),
                        name: item.name.clone(),
                        tags: item.tags.clone(),
                        look: None,
                        keeper: Some(stall.keeper.clone()),
                        picture: None,
                        lying: None,
                    });
                }
            }
        }
        items.extend(place.things.iter().map(|thing| PlaceItem {
            id: thing.id.clone(),
            name: thing.name.clone(),
            tags: Vec::new(),
            look: Some(thing.look.clone()),
            keeper: None,
            picture: thing.picture.clone(),
            lying: None,
        }));
        // What a line left on the ground here is in sight too, for as long as it lies there.
        for lies in inventory::lying_items_here(app) {
            let lies_here = if lies.count == 1 {
                "It lies on the ground here.".to_owned()
            } else {
                format!("{} of them lie on the ground here.", lies.count)
            };
            let look = match lies.look.trim() {
                "" => lies_here,
                own if own.ends_with('.') => format!("{own} {lies_here}"),
                own => format!("{own}. {lies_here}"),
            };
            match items.iter_mut().find(|seen| seen.id == lies.id) {
                Some(seen) => seen.lying = Some(lies.count),
                None => items.push(PlaceItem {
                    id: lies.id,
                    name: lies.name,
                    tags: Vec::new(),
                    look: Some(look),
                    keeper: None,
                    picture: None,
                    lying: Some(lies.count),
                }),
            }
        }
        Ok::<_, WorldError>(items)
    })();
    items.unwrap_or_else(|error| {
        app.record_line(format!("WORLD items_unavailable error={}", error.key()));
        Vec::new()
    })
}

/// A place designed in play the party has been to and can go back to (`world place <id>`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KeptPlace {
    pub id: String,
    pub name: String,
}

/// The places designed in play the party can go back to, oldest first, the active one left out;
/// none when the state cannot be read (written as `WORLD place_unavailable error=..`). Blocking.
pub fn kept_places(app: &AppState) -> Vec<KeptPlace> {
    match store::read_kept(&store::world_directory(app)) {
        Ok(places) => places
            .into_iter()
            .map(|place| KeptPlace {
                id: place.id,
                name: place.name,
            })
            .collect(),
        Err(error) => {
            app.record_line(format!("WORLD place_unavailable error={}", error.key()));
            Vec::new()
        }
    }
}

/// Makes the kept place designed in play `id` the active one again, as it was left (`world place`
/// for a kept place: the town and the places visited stay); `None` when no such place is kept.
/// Blocking.
pub fn revisit_place(app: &AppState, id: &str) -> Result<Option<KeptPlace>, WorldError> {
    Ok(
        store::revisit(&store::world_directory(app), id)?.map(|place| KeptPlace {
            id: place.id,
            name: place.name,
        }),
    )
}

/// The name of the active place (designed on arrival, or the bible's), or none with no place or
/// when the bible cannot be had (written as `WORLD place_unavailable error=..`). Blocking.
pub fn active_place_name(app: &AppState) -> Option<String> {
    let directory = store::world_directory(app);
    let place = store::read_bible(&directory)
        .and_then(|bible| store::read_active_place(&directory, &bible));
    match place {
        Ok(place) => place.map(|place| place.name),
        Err(error) => {
            app.record_line(format!("WORLD place_unavailable error={}", error.key()));
            None
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
