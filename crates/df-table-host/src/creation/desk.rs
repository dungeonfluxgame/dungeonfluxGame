//! The host's creation machine: the `CreationTable` kept in its saved record (`store`), each seat's
//! hero read from the hero library the look studio fills, the lobby's Ready tied to the build, and
//! one `CREATION` line per step through the host's event log.
//!
//! It owns the steps between the routes and the record; the machine owns every rule of the
//! states and the rules port every number. A step reads the books first (an await), then changes
//! the record on the blocking pool. A build belongs to its hero: a seat always reads and changes
//! the build of the hero it plays now, a new hero's build begins from its look-studio species and
//! class as a quick hero (every slot random unless chosen), a finished build is kept by its hero
//! when the seat leaves or changes hero, and creation opens again for the next start once the
//! lobby gathers after play. Lines carry seats, hero ids, states, slots and counts, never a name.

use df_playground::pages::creation::{BuildChoose, BuildState, CreationView};
use df_playground::pages::wire::HeroSheetView;
use df_rules::local_journey::SpeciesSize;
use df_table::creation::{
    CreationError, CreationOutcome, CreationTable, CreationTransition, Derivation, HeroStart,
    SeatCreationState,
};
use df_table::identity::SeatId;
use df_table::lobby::LobbyAction;
use df_table::machine::{CarryItem, Entry};

use super::request::{CreationRouteError, PublicSeatView, StartView, StartedHero, TableView};
use super::rules::HeroRules;
use super::store;
use super::view::{creation_view, sheet_view, with_vitals};
use super::vitals;
use crate::adapters::{EntropySource, SystemEntropy};
use crate::seat::request::SeatError;
use crate::seat::table::{self, ActionResult};
use crate::state::AppState;
use std::sync::Arc;

/// One `CREATION transition` line.
pub(crate) fn transition_line(transition: &CreationTransition, slot: Option<&str>) -> String {
    let state = |state: Option<SeatCreationState>| state.map_or("none", SeatCreationState::key);
    let slot = slot.map(|slot| format!(" slot={slot}")).unwrap_or_default();
    format!(
        "CREATION transition seat={} cause={}{slot} from={} to={} issues={} marks={}",
        transition.seat,
        transition.cause.key(),
        state(transition.from),
        state(transition.to),
        transition.issues,
        transition.marks
    )
}

/// One `CREATION derived` line for a seat whose build the rules derived in full: the numbers a
/// player reads first.
fn derived_line(table: &CreationTable, seat: SeatId) -> Option<String> {
    let creation = table.seat(seat)?;
    let hero = creation.hero.as_ref()?;
    let Some(Derivation::Complete { sheet, marks }) = &creation.derivation else {
        return None;
    };
    let sheet = sheet.as_value();
    Some(format!(
        "CREATION derived seat={seat} hero={} hp={} ac={} initiative={} marks={}",
        hero.hero,
        sheet["hit_points"]["maximum"],
        sheet["armor_class"]["value"],
        sheet["initiative_modifier"]["value"],
        marks.len()
    ))
}

/// The hero the seat plays now, as the look studio started it: its id, name, species, class and
/// portrait. A build always resolves through this, so a seat that changed hero never reads the
/// old hero's build.
async fn hero_of(app: &AppState, seat: u64) -> Result<HeroStart, CreationRouteError> {
    Ok(seat_hero(app, seat).await?.0)
}

/// The seat's current hero, and whether the seat builds it in creation: its look is locked in and
/// the seat plays no character (a hero bound to a character skips creation).
async fn seat_hero(app: &AppState, seat: u64) -> Result<(HeroStart, bool), CreationRouteError> {
    let mut open = app.seats().open().await?;
    let books = open.books();
    let seat_record = books
        .record_at(seat)
        .ok_or(CreationRouteError::NoHero { seat })?;
    let plays = seat_record.persona.is_some();
    let hero = seat_record
        .hero
        .ok_or(CreationRouteError::NoHero { seat })?;
    let record = books
        .hero(hero)
        .ok_or(CreationRouteError::NoHero { seat })?;
    let builds = record.locked && !plays;
    let hero = HeroStart {
        hero: record.id,
        name: record.name.clone(),
        species: record.species.clone(),
        class: record.class.clone(),
        portrait: record.portrait.clone(),
        public_story: Vec::new(),
    };
    Ok((hero, builds))
}

/// Whether the table's lobby gathers now (before the first start, or again after play).
async fn lobby_gathers(app: &AppState, seat: u64) -> Result<bool, CreationRouteError> {
    let engine = app.engine().map_err(SeatError::from)?;
    let view = table::quiet_view(engine, seat).await?;
    Ok(table::is_gathering(&view))
}

/// The lines of a step: each transition, then the derived numbers of the seat.
struct Logged<T> {
    lines: Vec<String>,
    answer: T,
}

fn write_lines<T>(app: &AppState, logged: Logged<T>) -> T {
    for line in logged.lines {
        app.record_line(line);
    }
    logged.answer
}

/// The table with `seat` building `hero`: made for the first seat; opened for the next start when
/// the lobby gathers again after play (every finished build kept by its hero); the seat taking up
/// `hero`'s build (kept as it was left, or begun and filled anew), the old hero's build kept.
fn ensure(
    table: Option<CreationTable>,
    seat: SeatId,
    hero: HeroStart,
    gathering: bool,
    lines: &mut Vec<String>,
) -> Result<CreationTable, CreationError> {
    let mut table = match table {
        Some(table) => table,
        None => CreationTable::enter(&Entry::carrying(vec![CarryItem::Seat(seat)]))?,
    };
    if gathering && table.reopen_for_next() {
        lines.push("CREATION reopened for the next start".to_owned());
    }
    if let Some((moved, fresh)) = table.take_up(seat, hero, &HeroRules)? {
        lines.push(transition_line(&moved, None));
        if fresh {
            lines.push(transition_line(
                &table.fill(seat, &HeroRules, &mut picker())?,
                None,
            ));
        }
    }
    Ok(table)
}

/// A choice of index drawn from the host's system entropy.
fn picker() -> impl FnMut(usize) -> usize {
    let mut entropy = SystemEntropy::new();
    move |count: usize| {
        let count = u64::try_from(count).unwrap_or(u64::MAX).max(1);
        usize::try_from(entropy.next_u64() % count).unwrap_or(0)
    }
}

/// Runs `step` on the seat's table (its current hero's build taken up first) and saves it; answers
/// the seat's view.
async fn step(
    app: &AppState,
    seat: u64,
    step: impl FnOnce(&mut CreationTable, SeatId, &mut Vec<String>) -> Result<(), CreationError>
    + Send
    + 'static,
) -> Result<CreationView, CreationRouteError> {
    let hero = hero_of(app, seat).await?;
    let gathering = lobby_gathers(app, seat).await?;
    let logged = store::change(app, move |table| {
        let seat = SeatId::new(seat);
        let mut lines = Vec::new();
        let mut table = ensure(table, seat, hero, gathering, &mut lines)?;
        step(&mut table, seat, &mut lines)?;
        lines.extend(derived_line(&table, seat));
        let answer = creation_view(table.seat(seat));
        Ok((table, Logged { lines, answer }))
    })
    .await?;
    Ok(write_lines(app, logged))
}

/// The seat's build as its phone is shown it: its current hero's, while creation is open for it.
/// A seat whose hero has no build at the seat yet (a new or changed hero, the next start) reads
/// as not begun, and its phone begins it.
pub(crate) async fn view(app: &AppState, seat: u64) -> Result<CreationView, CreationRouteError> {
    let hero = match hero_of(app, seat).await {
        Ok(hero) => hero,
        Err(CreationRouteError::NoHero { .. }) => return Ok(creation_view(None)),
        Err(error) => return Err(error),
    };
    let gathering = lobby_gathers(app, seat).await?;
    let table = store::load(app).await?;
    Ok(creation_view(table.as_ref().and_then(|table| {
        if table.is_started() && gathering {
            return None;
        }
        table.seat(SeatId::new(seat)).filter(|creation| {
            creation
                .hero
                .as_ref()
                .is_some_and(|known| known.hero == hero.hero)
        })
    })))
}

/// Sets and clears the picks of `choose`, then fills the rest ("random unless chosen"). With
/// nothing in it, it begins the seat's build.
pub(crate) async fn choose(
    app: &AppState,
    seat: u64,
    choose: BuildChoose,
) -> Result<CreationView, CreationRouteError> {
    step(app, seat, move |table, seat, lines| {
        for pick in &choose.picks {
            let moved = table.choose(seat, &pick.slot, &pick.option, &HeroRules)?;
            lines.push(transition_line(&moved, Some(&pick.slot)));
        }
        for slot in &choose.clear {
            lines.push(transition_line(
                &table.clear(seat, slot, &HeroRules)?,
                Some(slot),
            ));
        }
        if !choose.picks.is_empty() || !choose.clear.is_empty() {
            lines.push(transition_line(
                &table.fill(seat, &HeroRules, &mut picker())?,
                None,
            ));
        }
        Ok(())
    })
    .await
}

/// "Surprise me": a new build from the hero's species and class.
pub(crate) async fn surprise(
    app: &AppState,
    seat: u64,
) -> Result<CreationView, CreationRouteError> {
    step(app, seat, |table, seat, lines| {
        let moved = table.surprise(seat, &HeroRules, &mut picker())?;
        lines.push(transition_line(&moved, None));
        Ok(())
    })
    .await
}

/// The lobby's answer to `action` for `seat`: applied, or why not.
async fn lobby(app: &AppState, seat: u64, action: LobbyAction) -> Result<(), CreationRouteError> {
    match table::act(app.engine().map_err(SeatError::from)?, seat, action).await? {
        ActionResult::Applied => Ok(()),
        ActionResult::Declined(reason) => Err(CreationRouteError::LobbyDeclined { reason }),
        ActionResult::NotApplied => Err(SeatError::NotApplied.into()),
    }
}

/// The reviewed build becomes the hero and the seat is ready in the lobby. A lobby that does not
/// take the ready leaves the build reviewing.
pub(crate) async fn submit(
    app: &Arc<AppState>,
    seat: u64,
) -> Result<CreationView, CreationRouteError> {
    let hero = hero_of(app, seat).await?;
    let submitted = step(app, seat, |table, seat, lines| {
        lines.push(transition_line(&table.submit(seat)?, None));
        Ok(())
    })
    .await?;
    if let Err(error) = lobby(app, seat, LobbyAction::Ready).await {
        step(app, seat, |table, seat, lines| {
            lines.push(transition_line(&table.reopen(seat)?, None));
            Ok(())
        })
        .await?;
        return Err(error);
    }
    // The hero's vitals start whole for a new build, and its starting equipment goes into its
    // inventory once.
    vitals::submitted(app, hero.hero).await?;
    vitals::give_kit(app, hero.hero).await?;
    Ok(submitted)
}

/// A ready seat goes back to reviewing, and is not ready in the lobby.
pub(crate) async fn reopen(app: &AppState, seat: u64) -> Result<CreationView, CreationRouteError> {
    let reopened = step(app, seat, |table, seat, lines| {
        lines.push(transition_line(&table.reopen(seat)?, None));
        Ok(())
    })
    .await?;
    lobby(app, seat, LobbyAction::Unready).await?;
    Ok(reopened)
}

/// Why the lobby's Ready waits for `seat`'s build: its sentence while the seat builds a hero whose
/// build is not submitted; none for a seat that builds no hero in creation (a hero bound to a
/// character, a look not locked in). The seat takes up its current hero's build first, so a
/// changed hero is never ready on the old hero's build.
pub(crate) async fn ready_waits(app: &AppState, seat: u64) -> Result<Option<String>, SeatError> {
    let builds = match seat_hero(app, seat).await {
        Ok((_, builds)) => builds,
        Err(CreationRouteError::NoHero { .. }) => false,
        Err(error) => return Err(seat_error(error)),
    };
    if !builds {
        return Ok(None);
    }
    let view = step(app, seat, |_, _, _| Ok(()))
        .await
        .map_err(seat_error)?;
    Ok((view.state != BuildState::Ready)
        .then(|| "Finish your hero first: choose I'm ready on your hero's build.".to_owned()))
}

/// The seat said it is not ready in the lobby: its current hero's build reopens, when it was
/// submitted.
pub(crate) async fn reopen_after_unready(app: &AppState, seat: u64) -> Result<(), SeatError> {
    let builds = match seat_hero(app, seat).await {
        Ok((_, builds)) => builds,
        Err(CreationRouteError::NoHero { .. }) => false,
        Err(error) => return Err(seat_error(error)),
    };
    if !builds {
        return Ok(());
    }
    step(app, seat, |table, seat, lines| {
        if table
            .seat(seat)
            .is_some_and(|creation| creation.state == SeatCreationState::Ready)
        {
            lines.push(transition_line(&table.reopen(seat)?, None));
        }
        Ok(())
    })
    .await
    .map(|_| ())
    .map_err(seat_error)
}

/// The seat left the table: it is out of creation, and its hero's finished build is kept for the
/// hero.
pub(crate) async fn release_seat(app: &AppState, seat: u64) -> Result<(), SeatError> {
    let line = store::change(app, move |table| {
        let Some(mut table) = table else {
            return Err(CreationRouteError::NotOpen);
        };
        let left = table.leave(SeatId::new(seat));
        Ok((table, transition_line(&left, None)))
    })
    .await;
    match line {
        Ok(line) => {
            app.record_line(line);
            Ok(())
        }
        Err(CreationRouteError::NotOpen) => Ok(()),
        Err(error) => Err(seat_error(error)),
    }
}

/// The lobby started the game: creation hands every hero on, one sheet each.
pub(crate) async fn start(app: &AppState) -> Result<StartView, CreationRouteError> {
    let outcome = store::change(app, |table| {
        let mut table = table.ok_or(CreationRouteError::NotOpen)?;
        let outcome = table.start()?;
        Ok((table, outcome))
    })
    .await?;
    let CreationOutcome { hand_off, heroes } = outcome;
    let ids: Vec<String> = heroes
        .iter()
        .map(|output| output.hero.hero.to_string())
        .collect();
    let next = hand_off
        .machine_target()
        .map_or("none", |id| id.as_str())
        .to_owned();
    app.record_line(format!(
        "CREATION started seats={} heroes={} next={next}",
        heroes.len(),
        ids.join(",")
    ));
    Ok(StartView {
        next_machine: next,
        heroes: heroes
            .into_iter()
            .map(|output| StartedHero {
                seat: output.seat.get(),
                hero: output.hero,
                sheet: output.sheet.as_value().clone(),
                marks: output.marks,
            })
            .collect(),
    })
}

/// The lobby started the game: creation, when any seat built a hero, hands them on. A refusal
/// (a seat not ready) is written as a line; the lobby has already decided.
pub(crate) async fn start_play(app: &Arc<AppState>) {
    match start(app).await {
        Ok(started) => {
            // A hero whose inventory was not made at its submit is given its equipment now.
            for hero in started.heroes {
                if let Err(error) = vitals::give_kit(app, hero.hero.hero).await {
                    app.record_line(format!(
                        "CREATION refused route=start_play error={}",
                        error.key()
                    ));
                }
            }
        }
        Err(CreationRouteError::NotOpen) => {}
        Err(error) => app.record_line(format!(
            "CREATION refused route=start_play error={}",
            error.key()
        )),
    }
}

/// Every seat as the TV sees it.
pub(crate) async fn table(app: &AppState) -> Result<TableView, CreationRouteError> {
    let table = store::load(app).await?.ok_or(CreationRouteError::NotOpen)?;
    Ok(TableView {
        started: table.is_started(),
        seats: table
            .public_seats()
            .into_iter()
            .map(|seat| PublicSeatView {
                seat: seat.seat.get(),
                name: seat.name,
                state: seat.state,
            })
            .collect(),
    })
}

/// The derived sheet of the hero the seat plays now, in the shape the check judge reads, for the
/// sheet file: that hero's build, never another hero's that sat there before.
pub(crate) async fn sheet_file(
    app: &AppState,
    seat: u64,
) -> Result<serde_json::Value, CreationRouteError> {
    let hero = hero_of(app, seat).await?;
    let table = store::load(app).await?.ok_or(CreationRouteError::NotOpen)?;
    match table
        .build_of(hero.hero)
        .and_then(|build| build.derivation.as_ref())
    {
        Some(Derivation::Complete { sheet, .. }) => Ok(sheet.as_value().clone()),
        Some(Derivation::Incomplete { issues }) => Err(CreationRouteError::BuildIncomplete {
            seat,
            issues: issues.len(),
        }),
        None => Err(CreationRouteError::BuildIncomplete { seat, issues: 0 }),
    }
}

/// The sheet of the hero the seat plays now, as its phone's Hero and Roll pages show it; none for
/// a seat with no hero, or whose hero has no legal build.
pub(crate) async fn hero_sheet(
    app: &AppState,
    seat: u64,
) -> Result<Option<HeroSheetView>, CreationRouteError> {
    let hero = match hero_of(app, seat).await {
        Ok(hero) => hero,
        Err(CreationRouteError::NoHero { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let table = store::load(app).await?;
    let live = vitals::hero_vitals(app, hero.hero).await?;
    Ok(table
        .as_ref()
        .and_then(|table| table.build_of(hero.hero))
        .and_then(|build| HeroRules::sheet(build.hero.as_ref()?, &build.draft).ok())
        .map(|sheet| with_vitals(sheet_view(&sheet), live.as_ref())))
}

/// A creation refusal as a seat route's: a record that could not be kept is the seat books'
/// storage refusal; anything else did not apply.
pub(crate) fn seat_error(error: CreationRouteError) -> SeatError {
    match error {
        CreationRouteError::Storage { cause } => SeatError::Storage { cause },
        CreationRouteError::Seat(error) => error,
        _ => SeatError::NotApplied,
    }
}

/// What the TV's party card shows of a hero built in creation: its live Hit Points and its Armor
/// Class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeroSheetSummary {
    pub hero_id: u64,
    pub max_hp: u32,
    /// The hero's live Hit Points (`hero_vitals`).
    pub current_hp: u32,
    pub temp_hp: u32,
    pub armor_class: i16,
    /// The final Strength score, for carrying capacity (SRD p. 178, Carrying Capacity).
    pub strength: u8,
    /// The hero's size; none for a species with no admitted source.
    pub size: Option<SpeciesSize>,
}

/// The summary of the hero whose character id is `character` (`hero-<id>`, the id its sheet
/// carries); none when no seat built that hero, or its build is not legal yet. Read-only.
pub async fn hero_summary(
    app: &AppState,
    character: &str,
) -> Result<Option<HeroSheetSummary>, CreationRouteError> {
    let table = store::load(app).await?;
    let Some(mut summary) = table.and_then(|table| summary_of(&table, character)) else {
        return Ok(None);
    };
    if let Some(live) = vitals::hero_vitals(app, summary.hero_id).await? {
        summary.current_hp = live.current_hp;
        summary.temp_hp = live.temp_hp;
    }
    Ok(Some(summary))
}

/// The summary of the hero of `table` whose character id is `character`, whole (its live vitals
/// are `hero_vitals`').
pub(super) fn summary_of(table: &CreationTable, character: &str) -> Option<HeroSheetSummary> {
    table.builds().find_map(|creation| {
        let hero = creation.hero.as_ref()?;
        if super::rules::sheet_id(hero.hero) != character {
            return None;
        }
        let sheet = HeroRules::sheet(hero, &creation.draft).ok()?;
        Some(HeroSheetSummary {
            hero_id: hero.hero,
            max_hp: sheet.hit_points.maximum,
            current_hp: sheet.hit_points.maximum,
            temp_hp: 0,
            armor_class: sheet.armor_class.value,
            strength: sheet.ability_scores.strength,
            size: sheet.size.and_then(SpeciesSize::from_key),
        })
    })
}
