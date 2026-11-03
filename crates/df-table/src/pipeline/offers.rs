//! The buttons each seat holds now, published to the views (C.4, C.5, G.4, K.5).
//!
//! PROJECT calls `sync_offers` after every turn, and a table loaded from a checkpoint calls it
//! once, so the offers are a pure function of what the table holds: every choice of every open
//! pending that still waits on the seat (by pending id, so a seat that dropped and came back finds
//! the same buttons) and the options of the seat's own clarification. A seat that has nothing to
//! answer has no offers, and the shared screen has none at all: it shows only the plain waiting
//! status. Each button's id is the name the seat's tap sends back, and its label is that name in
//! words; the first button with a name wins when two pendings offer the same one, because a tap
//! names a choice and the seat answers the pending that is oldest.
//!
//! It changes the attachments only when the offers a seat holds are not the ones it should hold,
//! so a turn that changed no buttons does not raise the seat's attachment version. It never
//! decides what an answer means.

use crate::identity::SeatId;
use crate::ports::ActionId;
use crate::projection::NonEmptyText;
use crate::projection::{AttachmentError, OfferEntry, OfferId};

use super::table_access::PipelineTable;

/// A name in words, with a capital: `attack_innkeeper` is "Attack innkeeper".
fn words(name: &str) -> NonEmptyText {
    let spaced = name.replace(['_', '-'], " ");
    let mut letters = spaced.chars();
    let text = match letters.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), letters.as_str()),
        None => spaced,
    };
    NonEmptyText::or_fallback(&text)
}

fn entry(choice: &ActionId) -> Option<OfferEntry> {
    let id = OfferId::new(choice.as_str()).ok()?;
    Some(OfferEntry::new(id, words(choice.as_str())))
}

/// The buttons `seat` should hold, oldest pending first, then the clarification's.
fn wanted(table: &PipelineTable, seat: SeatId) -> Vec<OfferEntry> {
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
    offers
}

/// Makes the offers of every seat at the table the ones the table's pendings and clarifications
/// call for.
pub(super) fn sync_offers(table: &mut PipelineTable) -> Result<(), AttachmentError> {
    sync(table, false)
}

/// The same for a table just loaded from a checkpoint, whose saved versions already count the
/// publishes that made the offers: nothing is counted again.
pub(super) fn restore_offers(table: &mut PipelineTable) -> Result<(), AttachmentError> {
    sync(table, true)
}

fn sync(table: &mut PipelineTable, restoring: bool) -> Result<(), AttachmentError> {
    let seats: Vec<SeatId> = table.facts().seats().iter().copied().collect();
    for seat in seats {
        let offers = wanted(table, seat);
        if table.attachments().offers_of(seat) == offers.as_slice() {
            continue;
        }
        let attachments = table.attachments_mut();
        match (offers.is_empty(), restoring) {
            (true, _) => attachments.clear_seat_offers(seat)?,
            (false, false) => attachments.set_seat_offers(seat, offers)?,
            (false, true) => attachments.restore_seat_offers(seat, offers)?,
        }
    }
    Ok(())
}
