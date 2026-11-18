//! A built hero's live state, the one owner of it: its vitals (current and temporary Hit Points,
//! Hit Point Dice left, limited feature uses, spell slots left), changed only through
//! `apply_vitals` with the rules' checks (`df_rules::hero::HeroVitals::apply`), and its starting
//! equipment put into its inventory once (`give_kit`).
//!
//! A hero's vitals start whole from its finished build and are kept by hero id in
//! `players/hero-states.json`, so they follow the hero across seats and days. Kept vitals of
//! another build of the hero (other maxima or slots) give way to the build's whole ones. Every
//! change writes one `HERO vitals hero= hp= temp= hit_dice= [slots_<level>=] change=` line; the
//! kit one `CREATION kit_given` or `CREATION kit_waits` line. Never does: roll a die (a Hit Point
//! Die's face arrives in the change), or change a hero no seat has built.
use std::sync::Arc;

use df_rules::coins::Denomination;
use df_rules::hero::{HeroVitals, StartingItem, VitalsChange, starting_kit};
use df_rules::srd_equipment_prices::equipment;
use df_table::creation::{CreationTable, Derivation};

use super::request::CreationRouteError;
use super::rules::{HeroRules, sheet_id};
use super::store::{self, HeroState, HeroStates};
use crate::blocking::run_blocking;
use crate::state::AppState;
use crate::world::inventory::{gain_coins, receive};
use crate::world::{InventoryChange, InventoryError, ItemUse, change_own};

/// The vitals of hero `hero` fresh from its finished build; none when no seat built it.
fn fresh(table: Option<&CreationTable>, hero: u64) -> Option<HeroVitals> {
    let build = table?.build_of(hero)?;
    let sheet = HeroRules::sheet(build.hero.as_ref()?, &build.draft).ok()?;
    Some(HeroVitals::fresh(&sheet))
}

fn vitals_line(hero: u64, vitals: &HeroVitals, change: &str) -> String {
    let slots = vitals
        .spell_slots
        .iter()
        .map(|slots| format!(" slots_{}={}/{}", slots.level, slots.left, slots.maximum))
        .collect::<String>();
    format!(
        "HERO vitals hero={hero} hp={}/{} temp={} hit_dice={}/{}{slots} change={change}",
        vitals.current_hp, vitals.max_hp, vitals.temp_hp, vitals.hit_dice_left, vitals.hit_dice_max
    )
}

/// The vitals kept for a hero when they belong to its build as it stands; whole from the build
/// when nothing kept them or they belong to another build of it (other maxima, other slots).
fn current(kept: Option<&HeroVitals>, whole: Option<HeroVitals>) -> Option<HeroVitals> {
    match (kept, whole) {
        (Some(kept), Some(whole)) if !kept.fits(&whole) => Some(whole),
        (Some(kept), _) => Some(kept.clone()),
        (None, whole) => whole,
    }
}

/// The live vitals of hero `hero`: as kept, or whole from its finished build when nothing has
/// changed them yet; none for a hero no seat has built.
pub async fn hero_vitals(
    app: &AppState,
    hero: u64,
) -> Result<Option<HeroVitals>, CreationRouteError> {
    let heroes = store::load_heroes(app).await?;
    let table = store::load(app).await?;
    Ok(current(
        heroes.get(&hero).map(|state| &state.vitals),
        fresh(table.as_ref(), hero),
    ))
}

/// Applies `change` to hero `hero`'s vitals, checked by the rules, and keeps them; answers the
/// vitals after it. Refused, with nothing changed, for a hero no seat has built or a change the
/// rules refuse.
pub async fn apply_vitals(
    app: &AppState,
    hero: u64,
    change: VitalsChange,
) -> Result<HeroVitals, CreationRouteError> {
    let key = change.key();
    let after = store::change_heroes(app, move |heroes, table| {
        let before = current(
            heroes.get(&hero).map(|state| &state.vitals),
            fresh(table, hero),
        )
        .ok_or(CreationRouteError::UnknownHero { hero })?;
        let after = before
            .apply(&change)
            .map_err(|cause| CreationRouteError::InvalidVitals { cause })?;
        heroes
            .entry(hero)
            .and_modify(|state| state.vitals = after.clone())
            .or_insert_with(|| HeroState {
                vitals: after.clone(),
                kit_given: false,
            });
        Ok(after)
    })
    .await?;
    app.record_line(vitals_line(hero, &after, key));
    Ok(after)
}

/// A hero's build was submitted: its vitals start whole when the build's numbers changed (a new
/// or rebuilt hero); vitals the same build already had are kept.
pub(super) fn settle_vitals(heroes: &mut HeroStates, table: &CreationTable, hero: u64) -> bool {
    let Some(whole) = fresh(Some(table), hero) else {
        return false;
    };
    match heroes.get_mut(&hero) {
        Some(state) if state.vitals.fits(&whole) => false,
        Some(state) => {
            state.vitals = whole;
            true
        }
        None => {
            heroes.insert(
                hero,
                HeroState {
                    vitals: whole,
                    kit_given: false,
                },
            );
            true
        }
    }
}

/// The vitals of the submitted build, kept: whole for a new or rebuilt hero.
pub(super) async fn submitted(app: &AppState, hero: u64) -> Result<(), CreationRouteError> {
    let reset = store::change_heroes(app, move |heroes, table| {
        let Some(table) = table else {
            return Ok(None);
        };
        Ok(settle_vitals(heroes, table, hero)
            .then(|| heroes.get(&hero).map(|state| state.vitals.clone()))
            .flatten())
    })
    .await?;
    if let Some(vitals) = reset {
        app.record_line(vitals_line(hero, &vitals, "whole"));
    }
    Ok(())
}

/// What handing a hero its starting kit came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KitOutcome {
    /// The kit and its gold went into the hero's inventory now.
    Given,
    /// The kit was given before: nothing is given twice.
    AlreadyGiven,
    /// Nothing was given yet, for this reason key (`no_inventory`, `no_build`, `incomplete`,
    /// `no_kit`, or the inventory's refusal); it is given at the next submit or start.
    Waits(String),
}

/// Puts hero `hero`'s starting equipment into its inventory (`hero-<id>`), once: each item through
/// the world inventory's `receive` (`put_kit`), priced from the SRD catalog. A hero whose
/// inventory is not made yet waits (`CREATION kit_waits`) and is given it at the next submit or
/// start. Its gold arrives as gold pieces (`gain_coins`); gold the inventory refuses (a load past
/// the carrying capacity, a species with no admitted size) is named on the line (`gold_refused=`).
pub(crate) async fn give_kit(
    app: &Arc<AppState>,
    hero: u64,
) -> Result<KitOutcome, CreationRouteError> {
    let heroes = store::load_heroes(app).await?;
    if heroes.get(&hero).is_some_and(|state| state.kit_given) {
        return Ok(KitOutcome::AlreadyGiven);
    }
    hand_out_kit(app, hero).await
}

/// Puts hero `hero`'s starting equipment and gold into its bag whether or not it was given before:
/// a Start, which has just emptied the bag (`world::inventory::routes::reset_owner`), so the same
/// run started twice gives the same bag (`planning/town-to-tomb.md` part C, a Start is a fresh
/// journey). As [`give_kit`] otherwise.
pub(crate) async fn give_kit_again(
    app: &Arc<AppState>,
    hero: u64,
) -> Result<KitOutcome, CreationRouteError> {
    hand_out_kit(app, hero).await
}

/// The hand-out of [`give_kit`] and [`give_kit_again`].
async fn hand_out_kit(app: &Arc<AppState>, hero: u64) -> Result<KitOutcome, CreationRouteError> {
    let table = store::load(app).await?;
    let waits = |reason: &str| Ok(KitOutcome::Waits(reason.to_owned()));
    let Some(build) = table.as_ref().and_then(|table| table.build_of(hero)) else {
        return waits("no_build");
    };
    if !matches!(build.derivation, Some(Derivation::Complete { .. })) {
        return waits("incomplete");
    }
    let Ok(kit) = starting_kit(build.draft.picks()) else {
        return waits("no_kit");
    };
    let owner = sheet_id(hero);
    let given = put_kit(app, &owner, kit.items.clone()).await;
    match given {
        Ok(Some(count)) => {
            let gold = give_gold(app, &owner, kit.gold_gp).await;
            store::change_heroes(app, move |heroes, table| {
                if let Some(table) = table
                    && !heroes.contains_key(&hero)
                {
                    settle_vitals(heroes, table, hero);
                }
                if let Some(state) = heroes.get_mut(&hero) {
                    state.kit_given = true;
                }
                Ok(())
            })
            .await?;
            let gold = match gold {
                Ok(()) => format!("gold_gp={}", kit.gold_gp),
                Err(error) => format!("gold_gp=0 gold_refused={}", error.key()),
            };
            app.record_line(format!(
                "CREATION kit_given hero={hero} items={count} {gold}"
            ));
            Ok(KitOutcome::Given)
        }
        Ok(None) => {
            app.record_line(format!(
                "CREATION kit_waits hero={hero} reason=no_inventory"
            ));
            waits("no_inventory")
        }
        Err(error) => {
            app.record_line(format!(
                "CREATION kit_waits hero={hero} reason={}",
                error.key()
            ));
            waits(error.key())
        }
    }
}

/// Why a starting item is received, as its look and its `INVENTORY` line name it.
const KIT_WHY: &str = "from the hero's starting equipment";

/// Puts each starting item into `owner`'s inventory through the inventory's `receive`, containers
/// first (a quiver takes the arrows after it): a pack arrives as its contents in its backpack,
/// the rest carried; goods go in the containers that hold them. An item the SRD catalog does not
/// list (a generic Holy Symbol, a chosen tool), or one `receive` turns away for its load or an
/// unknown carrying capacity, is added as it is and carried: the rules give a hero its
/// starting equipment whole. Answers how many items went in; none when the inventory is not made.
async fn put_kit(
    app: &Arc<AppState>,
    owner: &str,
    mut items: Vec<StartingItem>,
) -> Result<Option<usize>, InventoryError> {
    items.sort_by_key(|item| !equipment(&item.id).is_some_and(|entry| entry.capacity.is_some()));
    for item in &items {
        match receive(app, owner, &item.id, u64::from(item.count), KIT_WHY).await {
            Ok(_) => {}
            Err(InventoryError::UnknownOwner { .. }) => return Ok(None),
            Err(
                InventoryError::NotInCatalog { .. }
                | InventoryError::OverCapacity { .. }
                | InventoryError::NoCarryingCapacity { .. },
            ) => add_carried(app, owner, item).await?,
            Err(error) => return Err(error),
        }
    }
    Ok(Some(items.len()))
}

/// Gives `owner` the kit's `gold_gp` as gold pieces (SRD 5.2.1 p. 89) through the inventory's
/// one coin gain; nothing for a kit with no gold.
async fn give_gold(app: &Arc<AppState>, owner: &str, gold_gp: u32) -> Result<(), InventoryError> {
    if gold_gp == 0 {
        return Ok(());
    }
    gain_coins(app, owner, Denomination::Gp, u64::from(gold_gp), KIT_WHY)
        .await
        .map(|_| ())
}

/// Adds `item` to `owner`'s inventory as it is, carried in no container.
async fn add_carried(
    app: &Arc<AppState>,
    owner: &str,
    item: &StartingItem,
) -> Result<(), InventoryError> {
    let change = InventoryChange::Add {
        item: item.id.clone(),
        count: u64::from(item.count),
        name: Some(item.name.clone()),
        look: Some(format!("{}, {KIT_WHY}.", item.name)),
        usage: Some(if item.kept {
            ItemUse::Kept
        } else {
            ItemUse::Consumed
        }),
        price_cp: item.price_cp,
        why: "starting equipment".to_owned(),
    };
    let reader = Arc::clone(app);
    let owner = owner.to_owned();
    run_blocking(
        move || change_own(&reader, &owner, change).map(|_| ()),
        InventoryError::Stopped,
    )
    .await
}

/// What hero `hero`'s ready pose shows it carrying, in words, from its finished build's starting
/// equipment ("wearing Chain Mail, wielding a Greatsword"); none for a hero no seat has built, or
/// whose kit holds no armour, shield or weapon. Read-only: for the look studio's ready pose.
pub async fn hero_gear(app: &AppState, hero: u64) -> Result<Option<String>, CreationRouteError> {
    let table = store::load(app).await?;
    Ok(table
        .as_ref()
        .and_then(|table| table.build_of(hero))
        .filter(|build| matches!(build.derivation, Some(Derivation::Complete { .. })))
        .and_then(|build| starting_kit(build.draft.picks()).ok())
        .and_then(|kit| kit.gear_words()))
}
