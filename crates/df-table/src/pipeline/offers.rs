//! The buttons each seat holds now, published to the views (C.4, C.5, G.4, K.5).
//!
//! PROJECT calls `sync_offers` after every turn, and a table loaded from a checkpoint calls it
//! once, so the offers are a pure function of what the table holds: every choice of every open
//! pending that still waits on the seat (by pending id, so a seat that dropped and came back finds
//! the same buttons) and the options of the seat's own clarification. A seat that has nothing to
//! answer has no offers, and the shared screen has none at all: it shows only the plain waiting
//! status. When the pipeline is asked for standing offers (`Pipeline::with_machine_surface`, H3),
//! a seat with no pending and no clarification of its own also holds the buttons the running
//! machine states for it (`Machine::standing_offers`), so a phone in the lobby can tap ready and
//! start; a seat that has a question to answer holds only that question's buttons, so a tap cannot
//! abandon it by accident. Each button's id is the name the seat's tap sends back, and its label
//! is that name in words; the first button with a name wins when two pendings offer the same one,
//! because a tap names a choice and the seat answers the pending that is oldest. The standing
//! offers are derived like the others and are not saved, but a table loaded from a checkpoint
//! derives them again at once (`restore_offers`), so a phone has its buttons before any input.
//!
//! One sync resolves the running machine and takes one snapshot, and asks the machine once for
//! every seat that may hold standing buttons, so a table of any size costs one reading of the
//! machine's state. A machine that cannot be read, or that the registry does not hold, is a typed
//! `MachineSurfaceError`: the buttons of the pendings are still synced, the standing ones are
//! left out, and the caller records the failure.
//!
//! It changes the attachments only when the offers a seat holds are not the ones it should hold,
//! so a turn that changed no buttons does not raise the seat's attachment version. It never
//! decides what an answer means.

use std::sync::Arc;

use crate::identity::SeatId;
use crate::machine::{Machine, MachineFailureCause, StandingOffers};
use crate::ports::ActionId;
use crate::projection::NonEmptyText;
use crate::projection::{AttachmentError, OfferEntry, OfferId};
use crate::registry::Registry;
use crate::table_state::TableSnapshot;

use super::record::separated_words;
use super::table_access::{PipelineTable, Running};

/// Why what the running machine says for itself (its standing buttons, its line) could not be
/// read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineSurfaceError {
    /// The machine that runs is not in the registry.
    NotRegistered,
    /// The machine failed to read its own saved state or to project it.
    Failed(MachineFailureCause),
}

impl std::fmt::Display for MachineSurfaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRegistered => formatter.write_str("the running machine is not registered"),
            Self::Failed(cause) => write!(
                formatter,
                "the running machine could not say what it offers ({})",
                cause.key()
            ),
        }
    }
}

impl std::error::Error for MachineSurfaceError {}

/// Why a sync did not finish cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OffersError {
    /// The attachments refused a seat's buttons.
    Attachments(AttachmentError),
    /// The standing buttons could not be derived; the others were published.
    Machine(MachineSurfaceError),
}

/// The machine that runs now with the registry's entry for it, read from one snapshot.
pub(super) struct Surface<'r, 's> {
    pub(super) machine: &'r Arc<dyn Machine>,
    pub(super) running: Running<'s>,
}

/// The running machine and its saved state, or `None` when no machine runs. A machine that runs
/// and is not registered is the typed error, never "nothing to show".
pub(super) fn running_surface<'r, 's>(
    table: &PipelineTable,
    snapshot: &'s TableSnapshot,
    registry: &'r Registry,
) -> Result<Option<Surface<'r, 's>>, MachineSurfaceError> {
    let Some(running) = table.running(snapshot) else {
        return Ok(None);
    };
    let machine = registry
        .machine(running.id)
        .ok_or(MachineSurfaceError::NotRegistered)?;
    Ok(Some(Surface { machine, running }))
}

fn entry(choice: &ActionId) -> Option<OfferEntry> {
    let id = OfferId::new(choice.as_str()).ok()?;
    Some(OfferEntry::new(id, words(choice.as_str())))
}

/// A name in words, with a capital: `attack_innkeeper` is "Attack innkeeper".
fn words(name: &str) -> NonEmptyText {
    let spaced = separated_words(name);
    let mut letters = spaced.chars();
    let text = match letters.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), letters.as_str()),
        None => spaced,
    };
    NonEmptyText::or_fallback(&text)
}

/// Whether the seat has a question of its own to answer: an open pending that waits on it, or a
/// clarification.
fn has_question(table: &PipelineTable, seat: SeatId) -> bool {
    !table.pending().offers_awaiting(seat).is_empty() || table.clarification_of(seat).is_some()
}

/// The buttons `seat` should hold, oldest pending first, then the clarification's, then the
/// running machine's own `standing` ones for a seat with no question (the caller asked the
/// machine only about such seats).
fn wanted(table: &PipelineTable, seat: SeatId, standing: &[ActionId]) -> Vec<OfferEntry> {
    let mut offers: Vec<OfferEntry> = Vec::new();
    let mut add = |choice: &ActionId| {
        if let Some(offer) = entry(choice)
            && !offers.iter().any(|held| held.id() == offer.id())
        {
            offers.push(offer);
        }
    };
    for (_, offer) in table.pending().offers_awaiting(seat) {
        for choice in offer.choices() {
            add(choice);
        }
    }
    if let Some(options) = table.clarification_of(seat) {
        for choice in options {
            add(choice);
        }
    }
    if !has_question(table, seat) {
        for choice in standing {
            add(choice);
        }
    }
    offers
}

/// What the machine that runs now says each seat without a question may do now: one call for all
/// of them, from the one snapshot.
fn standing_for(
    table: &PipelineTable,
    snapshot: &TableSnapshot,
    registry: &Registry,
    seats: &[SeatId],
) -> Result<StandingOffers, MachineSurfaceError> {
    let free: Vec<SeatId> = seats
        .iter()
        .copied()
        .filter(|seat| !has_question(table, *seat))
        .collect();
    if free.is_empty() {
        return Ok(Vec::new());
    }
    let Some(surface) = running_surface(table, snapshot, registry)? else {
        return Ok(Vec::new());
    };
    surface
        .machine
        .standing_offers(table.facts(), surface.running.slot, &free)
        .map_err(|error| MachineSurfaceError::Failed(error.cause()))
}

/// Makes the offers of every seat at the table the ones the table's pendings and clarifications
/// call for, with the standing ones of the running machine when `standing` names the registry.
/// `snapshot` is the table as the turn left it.
pub(super) fn sync_offers(
    table: &mut PipelineTable,
    snapshot: &TableSnapshot,
    standing: Option<&Registry>,
) -> Result<(), OffersError> {
    sync(table, snapshot, false, standing)
}

/// The same for a table just loaded from a checkpoint, whose saved versions already count the
/// publishes that made the offers: nothing is counted again. `standing` is the registry when the
/// pipeline that will run the table publishes the machine's buttons, so a restored phone holds the
/// buttons it held when the checkpoint was taken.
pub(super) fn restore_offers(
    table: &mut PipelineTable,
    standing: Option<&Registry>,
) -> Result<(), OffersError> {
    let snapshot = table.facts().snapshot();
    sync(table, &snapshot, true, standing)
}

fn sync(
    table: &mut PipelineTable,
    snapshot: &TableSnapshot,
    restoring: bool,
    standing: Option<&Registry>,
) -> Result<(), OffersError> {
    let seats: Vec<SeatId> = table.facts().seats().iter().copied().collect();
    let (standing, fault) =
        match standing.map(|registry| standing_for(table, snapshot, registry, &seats)) {
            Some(Ok(offers)) => (offers, None),
            Some(Err(fault)) => (Vec::new(), Some(fault)),
            None => (Vec::new(), None),
        };
    for seat in seats {
        let own = standing
            .iter()
            .find_map(|(named, actions)| (*named == seat).then_some(actions.as_slice()))
            .unwrap_or_default();
        let offers = wanted(table, seat, own);
        if table.attachments().offers_of(seat) == offers.as_slice() {
            continue;
        }
        let attachments = table.attachments_mut();
        let done = match (offers.is_empty(), restoring) {
            (true, _) => attachments.clear_seat_offers(seat),
            (false, false) => attachments.set_seat_offers(seat, offers),
            (false, true) => attachments.restore_seat_offers(seat, offers),
        };
        done.map_err(OffersError::Attachments)?;
    }
    fault.map_or(Ok(()), |fault| Err(OffersError::Machine(fault)))
}
