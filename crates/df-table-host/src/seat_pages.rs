//! The seat's live pages in play (`planning/phone-and-creation.md`, "The painter's day from the
//! phone"): what the Bag, Talk, the Notebook, the Map and Look closer show, and the Notebook's
//! notes and Bag actions. The read is the phone's `df_playground::path::PLAY_SEAT_PAGES`; the
//! notebook and bag actions come through the one seat action route (`SeatAction::Note`, `Bag`).
//!
//! Each route takes the seat from its key and the character from the seat, never from a body, and
//! reads only that character's own: a thin call into each owner, with no store of its own. The Bag
//! is the inventory API (`world::inventory_of`, owner the character) with each item's reference
//! sheet (`world::sheet_name`) when the library has it; the people are the shown scene's cast with
//! a stored persona (`shown_scene::shown_cast`, `persona::summaries_of`), each by the name the
//! character has learned (its state's `known_names`) or else by what they do; the keepers are the
//! stalls' at the active place (`world::place_items`); the Notebook and the Map are the memory API
//! (`world::memory::recent`, kinds `wire::KeptKind`) with the place the shared screen shows; the
//! things in sight are the active place's (`world::place_items`); the insert and the show are what
//! the shared screen's backdrop is (`fixture::PlateKind`).
//!
//! A note is a memory of the character (`world::memory::keep_next`, numbered by the memory API): a
//! line the player wrote, a line of the story the host heard (named by the id the host gave it,
//! never sent back as text), or a snap the host makes from the backdrop it sent the shared screen.
//! Each is one `SEAT note_kept seat=.. kind=.. id=..` line; no text reaches a log line.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use df_playground::fixture::{ImageRef, PlateKind};
use df_playground::overlay::Edit;
use df_playground::pages::wire::{
    BagAct, BagDone, BagItem, BagLoad, ContainerShown, Kept, KeptKind, MapPlace, NoteAsk, NoteKept,
    PagesErrorKind, Person, SeatPages, ShopShown, Shown, StockShown, Thing, Ware,
};
use df_playground::path::PLAY_SEAT_PAGES;
use df_playground::seat::wire::SeatAsk;
use df_rules::coins::{Denomination, Price};
use serde::{Deserialize, Serialize};

use crate::blocking::run_blocking;
use crate::dev_http::{RouteError, reply};
use crate::media_library::media_directory;
use crate::seat::key::SeatCaller;
use crate::seat::request::SeatError;
use crate::state::AppState;
use crate::world::InventoryItem;
use crate::world::inventory::{
    BagView, InventoryError, ShopView, coins_in, counting_coin, exact_coins, purse_of,
};
use crate::world::memory::{MemoryDraft, MemoryError, keep_next, recent};

/// Why a place the party has been to cannot be gone to by a tap: no card of what the table asks
/// now goes back there.
const NO_WAY_BACK: &str =
    "There is no road back there now: new places are found as you go. Say where you go next.";
/// Who made a note, as the memory records it.
const MADE_BY_PHONE: &str = "phone";

/// Why a seat pages route refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum PagesError {
    Memory {
        cause: MemoryError,
    },
    /// The story has no line of this number.
    UnknownLine {
        line: u64,
    },
    /// The shared screen shows no picture to snap.
    NothingShown,
    /// The bag holds none of `item`.
    NotCarried {
        item: String,
    },
    /// `item` has no price to sell it for.
    NoPrice {
        item: String,
    },
    /// `to` carries no coin to pay with.
    NoCoin {
        to: String,
    },
    /// `to`'s coins cannot make the price exactly (no change is given).
    NoExactCoin {
        to: String,
    },
    /// Terms with `to` are open already: they answer those first.
    OfferOpen {
        to: String,
    },
    /// The terms cannot be offered; `detail` says why (`world::deal_miss`).
    OfferRefused {
        detail: String,
    },
    /// The inventory did not take the change.
    Inventory {
        cause: crate::world::InventoryError,
    },
    /// The seat, its key or its hero refused, in the seat routes' own shape (`{"error":
    /// "unknown_key"}`), so a phone reads a lost key alike on every route.
    #[serde(untagged)]
    Seat(SeatError),
}

impl std::fmt::Display for PagesError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Seat(cause) => cause.fmt(formatter),
            Self::Memory { cause } => cause.fmt(formatter),
            Self::UnknownLine { line } => write!(formatter, "the story has no line {line}"),
            Self::NothingShown => formatter.write_str("the main screen shows no picture to snap"),
            Self::NotCarried { item } => write!(formatter, "the bag holds no {item}"),
            Self::NoPrice { item } => write!(formatter, "{item} has no price to sell it for"),
            Self::NoCoin { to } => write!(formatter, "{to} carries no coin to pay with"),
            Self::NoExactCoin { to } => {
                write!(formatter, "{to}'s coins cannot make the price exactly")
            }
            Self::OfferOpen { to } => write!(formatter, "terms with {to} are open already"),
            Self::OfferRefused { detail } => {
                write!(formatter, "the terms cannot be offered: {detail}")
            }
            Self::Inventory { cause } => cause.fmt(formatter),
        }
    }
}

impl PagesError {
    /// The refusal as the phone reads it: one of the shared `PagesErrorKind`s.
    pub fn kind(&self) -> PagesErrorKind {
        match self {
            Self::Memory { .. } => PagesErrorKind::Memory,
            Self::UnknownLine { .. } => PagesErrorKind::UnknownLine,
            Self::NothingShown => PagesErrorKind::NothingShown,
            Self::NotCarried { .. } => PagesErrorKind::NotCarried,
            Self::NoPrice { .. } => PagesErrorKind::NoPrice,
            Self::NoCoin { .. } => PagesErrorKind::NoCoin,
            Self::NoExactCoin { .. } => PagesErrorKind::NoExactCoin,
            Self::OfferOpen { .. } => PagesErrorKind::OfferOpen,
            Self::OfferRefused { .. } => PagesErrorKind::OfferRefused,
            Self::Inventory { .. } => PagesErrorKind::Inventory,
            Self::Seat(cause) => PagesErrorKind::Seat(cause.kind()),
        }
    }
}

impl std::error::Error for PagesError {}

impl RouteError for PagesError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Seat(cause) => cause.status(),
            Self::Memory { cause } => cause.status(),
            Self::UnknownLine { .. } | Self::NotCarried { .. } => StatusCode::NOT_FOUND,
            Self::NothingShown
            | Self::NoPrice { .. }
            | Self::NoCoin { .. }
            | Self::NoExactCoin { .. }
            | Self::OfferOpen { .. }
            | Self::OfferRefused { .. } => StatusCode::CONFLICT,
            Self::Inventory { cause } => cause.status(),
        }
    }
}

impl From<SeatError> for PagesError {
    fn from(cause: SeatError) -> Self {
        Self::Seat(cause)
    }
}

impl From<MemoryError> for PagesError {
    fn from(cause: MemoryError) -> Self {
        Self::Memory { cause }
    }
}

/// The seat the key holds and the character it plays.
async fn seat_character(state: &AppState, headers: &HeaderMap) -> Result<(u64, String), SeatError> {
    let caller = SeatCaller::of(state, headers).await?;
    Ok((caller.seat(), caller.character()?.to_owned()))
}

/// The backdrop the shared screen shows now, when it shows one.
fn backdrop(state: &AppState) -> Option<ImageRef> {
    match state.playground().scene_backdrop() {
        Edit::Set(image) => Some(image),
        Edit::Keep | Edit::Clear => None,
    }
}

/// `name` when the media library holds it.
fn in_library(state: &AppState, name: String) -> Option<String> {
    media_directory(state.playground().files())
        .join(&name)
        .is_file()
        .then_some(name)
}

/// What the character `hero` knows `ids` by: the learned name with what they do under it, or what
/// they do; each with their reference sheet as the portrait when the library has it.
pub(crate) async fn people_of(state: &Arc<AppState>, hero: &str, ids: Vec<String>) -> Vec<Person> {
    let summaries = crate::persona::summaries_of(state, ids).await;
    let known = crate::seat::routes::known_to(state, hero).await;
    let (reader, hero) = (Arc::clone(state), hero.to_owned());
    run_blocking(
        move || {
            Ok::<_, ()>(
                summaries
                    .into_iter()
                    .filter(|summary| summary.id != hero)
                    .map(|summary| Person {
                        portrait: in_library(
                            &reader,
                            crate::persona::reference_sheet_name(&summary.id),
                        ),
                        name: crate::seat::routes::label_for(&summary, &known),
                        about: known.contains(&summary.id).then(|| summary.job.clone()),
                        id: summary.id,
                    })
                    .collect(),
            )
        },
        (),
    )
    .await
    .unwrap_or_default()
}

/// The keepers of the active place's stalls, by id, each once.
fn keeper_ids(offered: &[crate::world::PlaceItem]) -> Vec<String> {
    let mut keepers: Vec<String> = Vec::new();
    for keeper in offered.iter().filter_map(|item| item.keeper.as_ref()) {
        if !keepers.contains(keeper) {
            keepers.push(keeper.clone());
        }
    }
    keepers
}

/// Everyone a seat's character may speak to now: the people of the shown scene and the keepers of
/// the place's stalls (who stay there when the picture cuts to a close-up), by id. The Talk page
/// lists them, and a line said to anyone else is refused.
pub(crate) async fn listener_ids(state: &Arc<AppState>) -> Vec<String> {
    let reader = Arc::clone(state);
    let offered = run_blocking(move || Ok::<_, ()>(crate::world::place_items(&reader)), ())
        .await
        .unwrap_or_default();
    let mut ids = scene_ids(state).await;
    for keeper in keeper_ids(&offered) {
        if !ids.contains(&keeper) {
            ids.push(keeper);
        }
    }
    ids
}

/// The people of the shown scene, by id; none when the scene has no scanned backdrop.
async fn scene_ids(state: &AppState) -> Vec<String> {
    match crate::shown_scene::shown_cast(state).await {
        Ok(shown) => shown
            .layout
            .names()
            .into_iter()
            .map(str::to_owned)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Everything `hero` keeps of the Notebook's kinds, newest first; and, when the party is in the
/// region `region` (its id), the ids of the places kept there: a scene memory kept in a region
/// carries the region's id as a tag (`world::region`).
async fn kept_of(
    state: &AppState,
    hero: &str,
    region: Option<&str>,
) -> Result<(Vec<Kept>, Option<Vec<String>>), MemoryError> {
    let mut kept = Vec::new();
    let mut in_region = region.map(|_| Vec::new());
    for kind in KeptKind::ALL {
        for memory in recent(state, hero, kind.key(), usize::MAX).await? {
            if kind == KeptKind::Place
                && let (Some(region), Some(ids)) = (region, in_region.as_mut())
                && memory.tags.iter().any(|tag| tag == region)
            {
                ids.push(memory.id.clone());
            }
            kept.push(Kept {
                picture: memory.primary().map(str::to_owned),
                id: memory.id,
                kind,
                seq: memory.seq,
                title: memory.title,
                text: memory.description,
            });
        }
    }
    kept.sort_by_key(|kept| std::cmp::Reverse(kept.seq));
    Ok((kept, in_region))
}

/// The places of `kept`, oldest first, each once with its newest plate; only those of
/// `in_region` (the ids of the places kept in the region the party is in) when it is in one, so
/// the map starts again after a crossing. The party is at the one the shared screen shows, or else
/// at the newest. A place a card of `ask` goes back to (its `thing` is the place's id) can be gone
/// to: its tap sends that card.
pub(crate) fn places_of(
    kept: &[Kept],
    in_region: Option<&[String]>,
    shown: Option<&str>,
    ask: Option<&SeatAsk>,
) -> Vec<MapPlace> {
    let mut places: Vec<MapPlace> = Vec::new();
    for memory in kept.iter().rev().filter(|kept| {
        kept.kind == KeptKind::Place && in_region.is_none_or(|ids| ids.contains(&kept.id))
    }) {
        match places.iter_mut().find(|place| place.id == memory.id) {
            Some(place) => place.plate.clone_from(&memory.picture),
            None => places.push(MapPlace {
                id: memory.id.clone(),
                name: memory.title.clone(),
                plate: memory.picture.clone(),
                here: false,
                refused: Some(NO_WAY_BACK.to_owned()),
                card: None,
            }),
        }
    }
    for place in &mut places {
        place.card = ask.and_then(|ask| {
            ask.cards
                .iter()
                .position(|card| card.thing.as_deref() == Some(place.id.as_str()))
        });
        if place.card.is_some() {
            place.refused = None;
        }
    }
    let here = places
        .iter()
        .position(|place| shown.is_some() && place.plate.as_deref() == shown)
        .or_else(|| places.len().checked_sub(1));
    if let Some(place) = here.and_then(|index| places.get_mut(index)) {
        place.here = true;
        place.refused = None;
    }
    places
}

/// `total_cp` copper pieces in `coin`'s terms: "32 sp", with the copper left over when it does
/// not divide ("32 sp 4 cp").
fn in_coin(total_cp: u64, coin: Denomination) -> String {
    let each = coin.copper().max(1);
    let (whole, rest) = (total_cp / each, total_cp % each);
    match (whole, rest) {
        (_, 0) => format!("{whole} {}", coin.key()),
        (0, _) => format!("{rest} cp"),
        _ => format!("{whole} {} {rest} cp", coin.key()),
    }
}

/// What the coins of `owner`'s `inventory` come to, in its counting coin; none when it holds no
/// coin.
fn purse_words(owner: &str, inventory: &[InventoryItem]) -> Option<String> {
    let coin = counting_coin(inventory)?;
    let total_cp = purse_of(owner, coins_in(inventory)).ok()?.total_copper()?;
    Some(in_coin(total_cp, coin))
}

/// What a carried item is worth in words: a coin by its own denomination, a priced ware by its
/// price in the owner's coin.
fn worth_of(item: &InventoryItem, coin: Option<Denomination>) -> Option<String> {
    match (item.coin, item.price_cp) {
        (Some(held), _) => Some(format!("{} {}", item.count, held.key())),
        (None, Some(price)) => Some(format!(
            "{} each",
            in_coin(price, coin.unwrap_or(Denomination::Cp))
        )),
        (None, None) => None,
    }
}

/// What the character carries, from the inventory API, and what their coins come to.
fn items_of(state: &AppState, hero: &str) -> (Vec<BagItem>, Option<String>) {
    let inventory = crate::world::inventory_of(state, hero);
    let coin = counting_coin(&inventory);
    let purse = purse_words(hero, &inventory);
    let items = inventory
        .into_iter()
        .filter(|item| item.count > 0)
        .map(|item| BagItem {
            picture: in_library(state, crate::world::sheet_name(hero, &item.id)),
            kept: item.usage == crate::world::ItemUse::Kept,
            worth: worth_of(&item, coin),
            weight: None,
            stowed: None,
            id: item.id,
            name: item.name,
            look: item.look,
            count: item.count,
        })
        .collect();
    (items, purse)
}

/// The words of a price: in the largest of gold, silver and copper that makes it exactly ("1 gp",
/// "5 sp", "4 cp"; SRD p.89). The Shop page and a purchase's answer both word prices so.
pub(crate) fn price_words(copper: u64) -> String {
    [Denomination::Gp, Denomination::Sp]
        .into_iter()
        .find(|coin| copper >= coin.copper() && copper.is_multiple_of(coin.copper()))
        .map_or_else(|| format!("{copper} cp"), |coin| in_coin(copper, coin))
}

/// What each carried thing of `items` weighs and where it is packed, from the bag `bag`.
pub(crate) fn weigh_items(items: &mut [BagItem], bag: &BagView) {
    for item in items {
        let lines: Vec<_> = bag
            .lines
            .iter()
            .filter(|line| line.item == item.id)
            .collect();
        let unit = lines.iter().find_map(|line| line.unit_weight);
        let total = lines.iter().filter_map(|line| line.weight).try_fold(
            df_rules::srd_equipment_prices::Weight::ZERO,
            |sum, weight| sum.checked_add(weight),
        );
        item.weight = match (unit, total) {
            (Some(unit), Some(total)) if item.count > 1 => Some(format!("{unit} each, {total}")),
            (Some(unit), _) => Some(unit.to_string()),
            _ => None,
        };
        let packed: Vec<&str> = lines
            .iter()
            .filter_map(|line| line.container.as_deref())
            .filter_map(|container| {
                bag.containers
                    .iter()
                    .find(|known| known.container == container)
                    .map(|known| known.name.as_str())
            })
            .collect();
        item.stowed = (!packed.is_empty()).then(|| format!("In the {}", packed.join(" and the ")));
    }
}

/// The load of the bag `bag`: the total against the capacity and each container against its room.
pub(crate) fn load_of(bag: &BagView) -> BagLoad {
    BagLoad {
        weight: bag.weight.to_string(),
        capacity: bag.capacity.map(|capacity| capacity.to_string()),
        over: bag.capacity.is_some_and(|capacity| bag.weight > capacity),
        containers: bag
            .containers
            .iter()
            .map(|container| ContainerShown {
                id: container.container.clone(),
                name: container.name.clone(),
                count: container.count,
                load: container.load.to_string(),
                capacity: container
                    .weight_capacity
                    .map(|capacity| capacity.to_string()),
                full: container
                    .weight_capacity
                    .is_some_and(|capacity| container.load >= capacity)
                    || container
                        .shares
                        .as_ref()
                        .is_some_and(|shares| shares.used >= shares.most),
                shares: container
                    .shares
                    .as_ref()
                    .map(|shares| format!("{} of {}", shares.used, shares.most)),
            })
            .collect(),
        unweighed: bag
            .unweighed
            .iter()
            .map(|id| {
                bag.lines
                    .iter()
                    .find(|line| line.item == *id)
                    .map_or_else(|| id.clone(), |line| line.name.clone())
            })
            .collect(),
    }
}

/// The hero's bag (`world::inventory::bag_view`), or none before they have an inventory. Another
/// refusal is one `SEAT pages_bag_refused` line and no load.
async fn bag_of(state: &AppState, seat: u64, hero: &str) -> Option<BagView> {
    match crate::world::inventory::bag_view(state, hero).await {
        Ok(bag) => Some(bag),
        Err(InventoryError::UnknownOwner { .. }) => None,
        Err(error) => {
            state.record_line(format!(
                "SEAT pages_bag_refused seat={seat} reason={}",
                error.key()
            ));
            None
        }
    }
}

/// The shop `view` as the phone shows it, its keeper named as the hero knows them.
pub(crate) fn shop_shown(view: ShopView, keeper: String) -> ShopShown {
    ShopShown {
        id: view.keeper,
        keeper,
        lines: view
            .lines
            .into_iter()
            .map(|line| StockShown {
                price: if line.per > 1 {
                    format!("{} for {}", price_words(line.price.copper()), line.per)
                } else {
                    price_words(line.price.copper())
                },
                weight: line.unit_weight.map(|weight| weight.to_string()),
                item: line.item,
                name: line.name,
                in_stock: line.in_stock,
                per: line.per,
                buyable: line.buyable,
            })
            .collect(),
    }
}

/// The shops among the people `here` (`world::inventory::shop_view`: a keeper whose inventory is
/// a shop), each named as the hero knows its keeper.
async fn shops_of(state: &Arc<AppState>, hero: &str, here: Vec<String>) -> Vec<ShopShown> {
    let mut views = Vec::new();
    for id in here {
        if let Ok(view) = crate::world::inventory::shop_view(state, &id).await {
            views.push(view);
        }
    }
    let ids = views.iter().map(|view| view.keeper.clone()).collect();
    let named = people_of(state, hero, ids).await;
    views
        .into_iter()
        .map(|view| {
            let keeper = named
                .iter()
                .find(|person| person.id == view.keeper)
                .map_or_else(|| view.keeper.clone(), |person| person.name.clone());
            shop_shown(view, keeper)
        })
        .collect()
}

/// The wares the keepers `keepers` sell, each with its price in the keeper's coin.
fn wares_of(state: &AppState, keepers: &[String]) -> Vec<Ware> {
    keepers
        .iter()
        .flat_map(|keeper| {
            let stock = crate::world::inventory_of(state, keeper);
            let coin = counting_coin(&stock).unwrap_or(Denomination::Cp);
            stock
                .into_iter()
                .filter(|item| item.count > 0 && item.coin.is_none())
                .filter_map(move |item| {
                    let price = item.price_cp?;
                    Some(Ware {
                        keeper: keeper.clone(),
                        id: item.id,
                        name: item.name,
                        price: format!("{} each", in_coin(price, coin)),
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `GET PLAY_SEAT_PAGES`.
async fn pages(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let result = async {
        let (seat, hero) = seat_character(&state, &headers).await?;
        let ask = state.seats().talk().ask_of(seat).cloned();
        let (reader, owner) = (Arc::clone(&state), hero.clone());
        let (place, (items, purse), offered, wares) = run_blocking(
            move || {
                let offered = crate::world::place_items(&reader);
                let wares = wares_of(&reader, &keeper_ids(&offered));
                Ok::<_, ()>((
                    crate::world::active_place_name(&reader),
                    items_of(&reader, &owner),
                    offered,
                    wares,
                ))
            },
            (),
        )
        .await
        .unwrap_or_default();
        let mut items = items;
        let bag = bag_of(&state, seat, &hero).await;
        if let Some(bag) = &bag {
            weigh_items(&mut items, bag);
        }
        let mut here = keeper_ids(&offered);
        for id in listener_ids(&state).await {
            if !here.contains(&id) {
                here.push(id);
            }
        }
        let shops = shops_of(&state, &hero, here).await;
        let keepers = people_of(&state, &hero, keeper_ids(&offered)).await;
        let people = people_of(&state, &hero, scene_ids(&state).await).await;
        // After a crossing the map shows only the places of the region the party is in.
        let reader = Arc::clone(&state);
        let region = run_blocking(move || Ok::<_, ()>(crate::world::region_of(&reader)), ())
            .await
            .unwrap_or_default();
        let (kept, in_region) = kept_of(
            &state,
            &hero,
            region.as_ref().map(|region| region.id.as_str()),
        )
        .await?;
        // The Hero and Roll pages: the hero's sheet when it was built in creation. A record that
        // could not be read leaves the pages without it, and says so in the log.
        let sheet = match crate::creation::hero_sheet(&state, seat).await {
            Ok(sheet) => sheet,
            Err(error) => {
                state.record_line(format!(
                    "CREATION refused route=pages error={}",
                    error.key()
                ));
                None
            }
        };
        let shown = backdrop(&state);
        let places = places_of(
            &kept,
            in_region.as_deref(),
            shown
                .as_ref()
                .filter(|image| image.plate != PlateKind::CloseUp)
                .map(|image| image.media.as_str()),
            ask.as_ref(),
        );
        let things = offered
            .into_iter()
            .map(|item| Thing {
                id: item.id,
                name: item.name,
                look: item.look,
                picture: item.picture,
            })
            .collect();
        Ok::<_, PagesError>(SeatPages {
            place,
            items,
            purse,
            wares,
            keepers,
            people,
            kept,
            places,
            things,
            insert: shown
                .as_ref()
                .filter(|image| image.plate == PlateKind::CloseUp)
                .map(|image| Shown {
                    picture: image.media.clone(),
                    caption: image.caption.clone(),
                }),
            show: shown.is_some_and(|image| image.plate == PlateKind::Show),
            sheet,
            load: bag.as_ref().map(load_of),
            shops,
        })
    }
    .await;
    reply(result)
}

/// What a note keeps: its kind, title, words and pictures.
type NoteParts = (KeptKind, String, String, Vec<String>);

/// What the note `ask` keeps, as `hero` knows it.
async fn parts_of(
    state: &Arc<AppState>,
    hero: &str,
    ask: NoteAsk,
) -> Result<NoteParts, PagesError> {
    Ok(match ask {
        NoteAsk::Write { text } => {
            let text = text.trim().to_owned();
            if text.is_empty() {
                return Err(SeatError::EmptyLine.into());
            }
            (KeptKind::Note, "My note".to_owned(), text, Vec::new())
        }
        NoteAsk::Keep { line } => {
            let heard = state.seats().talk().story_lines(hero);
            let said = heard
                .into_iter()
                .find(|said| said.id == line)
                .ok_or(PagesError::UnknownLine { line })?;
            let title = crate::seat::routes::speaker_name(state, hero, &said).await;
            (KeptKind::Note, title, said.text, Vec::new())
        }
        NoteAsk::Snap => {
            let image = backdrop(state).ok_or(PagesError::NothingShown)?;
            let reader = Arc::clone(state);
            let place = run_blocking(
                move || Ok::<_, ()>(crate::world::active_place_name(&reader)),
                (),
            )
            .await
            .ok()
            .flatten();
            let present = people_of(state, hero, scene_ids(state).await).await;
            let mut description = image
                .caption
                .clone()
                .unwrap_or_else(|| "What the main screen showed.".to_owned());
            if !present.is_empty() {
                let names: Vec<String> = present.into_iter().map(|person| person.name).collect();
                description.push_str(&format!(" With: {}.", names.join(", ")));
            }
            let title = place.unwrap_or_else(|| "The main screen".to_owned());
            (KeptKind::Snap, title, description, vec![image.media])
        }
    })
}

/// Keeps a note in the seat's notebook (`SeatAction::Note`): the memory API's own id. One `SEAT
/// note_kept seat= kind= id=` line, or `SEAT note_refused error=`.
pub(crate) async fn keep_note(
    state: &Arc<AppState>,
    caller: &SeatCaller,
    ask: NoteAsk,
) -> Result<NoteKept, PagesError> {
    let result = async {
        let (seat, hero) = (caller.seat(), caller.character()?.to_owned());
        let note = ask.key();
        let (kind, title, description, pictures) = parts_of(state, &hero, ask).await?;
        let owner = hero.clone();
        // The id is the memory API's own: the kind and the next `seq`, given in its store.
        let put = keep_next(state, &hero, kind.key(), move |id| MemoryDraft {
            id,
            owner,
            kind: kind.key().to_owned(),
            title,
            description,
            place: None,
            pictures,
            links: Vec::new(),
            tags: Vec::new(),
            created_by: Some(MADE_BY_PHONE.to_owned()),
        })
        .await?;
        state.record_line(format!(
            "SEAT note_kept seat={seat} kind={note} id={}",
            put.memory.id
        ));
        Ok::<_, PagesError>(NoteKept { id: put.memory.id })
    }
    .await;
    if let Err(error) = &result {
        state.record_line(format!("SEAT note_refused error={}", error_key(error)));
    }
    result
}

fn error_key(error: &PagesError) -> &'static str {
    match error {
        PagesError::Memory { cause } => cause.key(),
        other => other.kind().key(),
    }
}

/// What a seat's hands did with a thing, as its audit word.
fn bag_word(act: &BagAct) -> &'static str {
    match act {
        BagAct::Use { .. } => "use",
        BagAct::Drop { .. } => "drop",
        BagAct::Give { .. } => "give",
        BagAct::Sell { .. } => "sell",
    }
}

/// The terms `hero` offers `to` for one of `item`: nothing back for a gift; for a sale, the
/// buyer's coins that make its price exactly (`world::inventory::exact_coins`: no change is given
/// in a deal).
fn offer_terms(
    state: &AppState,
    (item, to): (&InventoryItem, &str),
    sell: bool,
) -> Result<Vec<crate::world::ItemCount>, PagesError> {
    if !sell {
        return Ok(Vec::new());
    }
    let price = item.price_cp.ok_or_else(|| PagesError::NoPrice {
        item: item.id.clone(),
    })?;
    let purse = crate::world::inventory_of(state, to);
    if counting_coin(&purse).is_none() {
        return Err(PagesError::NoCoin { to: to.to_owned() });
    }
    exact_coins(to, &purse, Price::from_copper(price))
        .map_err(|cause| PagesError::Inventory { cause })?
        .ok_or_else(|| PagesError::NoExactCoin { to: to.to_owned() })
}

/// What the seat's hands do (`SeatAction::Bag`) with a thing its character carries. Using one
/// (used up unless it is kept) or dropping one changes the bag at once, with a line in the story.
/// Giving or selling one opens terms with someone present (`world::settle_deal`, the deal's one
/// transfer once they take it in their own line) and puts the offer in the seat's inbox as its
/// talk to them, so the table hands them the turn. One `SEAT bag seat= act= item=` line.
pub(crate) async fn bag_act(
    state: &Arc<AppState>,
    caller: &SeatCaller,
    act: BagAct,
) -> Result<BagDone, PagesError> {
    let result = async {
        let (seat, hero) = (caller.seat(), caller.character()?.to_owned());
        let (reader, owner) = (Arc::clone(state), hero.clone());
        let held = run_blocking(
            move || Ok::<_, ()>(crate::world::inventory_of(&reader, &owner)),
            (),
        )
        .await
        .unwrap_or_default();
        let wanted = match &act {
            BagAct::Use { item }
            | BagAct::Drop { item }
            | BagAct::Give { item, .. }
            | BagAct::Sell { item, .. } => item.clone(),
        };
        let item = held
            .into_iter()
            .find(|held| held.id == wanted && held.count > 0)
            .ok_or_else(|| PagesError::NotCarried {
                item: wanted.clone(),
            })?;
        let name = crate::seat::routes::hero_name(state, &hero).await;
        state.record_line(format!(
            "SEAT bag seat={seat} act={} item={}",
            bag_word(&act),
            item.id
        ));
        let done = match act {
            BagAct::Use { .. } | BagAct::Drop { .. } => {
                let using = matches!(act, BagAct::Use { .. });
                // A dropped thing goes onto the ground of the place, where it can be picked back
                // up; a used one is used up, unless it is kept (a tool).
                let applied = match (using, item.usage == crate::world::ItemUse::Kept) {
                    (true, true) => None,
                    (true, false) => Some(crate::world::change_own(
                        state,
                        &hero,
                        crate::world::InventoryChange::Consume {
                            item: item.id.clone(),
                            count: 1,
                            why: "used".to_owned(),
                        },
                    )),
                    (false, _) => Some(crate::world::drop_own(state, &hero, &item.id, 1)),
                };
                let count = match applied {
                    Some(applied) => applied
                        .map_err(|cause| PagesError::Inventory { cause })?
                        .first()
                        .map_or(item.count, |applied| applied.after),
                    None => item.count,
                };
                let verb = if using { "uses" } else { "drops" };
                state
                    .seats()
                    .hear(None, &format!("{name} {verb} the {}.", item.name));
                BagDone {
                    count,
                    offered: false,
                }
            }
            BagAct::Give { ref to, .. } | BagAct::Sell { ref to, .. } => {
                let present = listener_ids(state).await;
                if !present.contains(to) {
                    return Err(SeatError::UnknownListener { to: to.clone() }.into());
                }
                // Terms already open with them are answered first (or countered in a line).
                if state.deals().between(&hero, to).is_some() {
                    return Err(PagesError::OfferOpen { to: to.clone() });
                }
                let selling = matches!(act, BagAct::Sell { .. });
                let deal = crate::world::DealRecord {
                    with: to.clone(),
                    state: crate::world::DealState::Offer,
                    gives: vec![crate::world::ItemCount {
                        item: item.id.clone(),
                        count: 1,
                    }],
                    gets: offer_terms(state, (&item, to), selling)?,
                };
                if let Some(detail) = crate::world::deal_miss(state, &hero, &deal, &present) {
                    return Err(PagesError::OfferRefused { detail });
                }
                // The terms are opened here, in code, exactly as the Bag gave them; the story says
                // so, and no line goes to a judge that could read other terms into it. They take
                // or turn down the open terms in their own next line.
                crate::world::settle_deal(state, &hero, &deal, &present);
                let other = crate::seat::routes::hero_name(state, to).await;
                let words = if selling {
                    format!("{name} offers {other} the {} for its price.", item.name)
                } else {
                    format!("{name} offers {other} the {}.", item.name)
                };
                state.seats().hear(None, &words);
                BagDone {
                    count: item.count,
                    offered: true,
                }
            }
        };
        Ok::<_, PagesError>(done)
    }
    .await;
    if let Err(error) = &result {
        state.record_line(format!("SEAT bag_refused error={}", error_key(error)));
    }
    result
}

/// The seat pages routes, for the player listener and the loopback listener.
pub(crate) fn player_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(PLAY_SEAT_PAGES, get(pages))
        .with_state(state)
}
