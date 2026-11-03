//! How an operation ended, as the result book keeps it, and the answer a duplicate delivery gets
//! (H.2).
//!
//! `stored_outcome` turns the end of an input into what the books keep: how it ended, the line the
//! seat was shown, the decline, the buttons and the routed kind. `replayed_outcome` is the way
//! back: a duplicate delivery of an operation id finds that in the result book (it committed) or
//! in the session's memory of inputs that committed nothing (it was declined, clarified or routed),
//! so no port is asked, no die is drawn, and the seat is told what it was told the first time. It
//! never decides anything about the table.

use crate::envelope::PendingId;
use crate::outcome::{Decline, DeclineCode, Outcome};

use super::Clarification;
use super::books::{StoredClarification, StoredEnding, StoredOutcome};
use super::table_access::PipelineTable;
use super::turn::Turn;

/// The outcome of an operation that was seen before: found in the result book (committed) or in
/// the session's memory of inputs that committed nothing, so no port is asked, no die is drawn and
/// the seat is told what it was told the first time.
pub(super) fn replayed_outcome(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    stored: StoredOutcome,
) -> Outcome {
    turn.replayed = true;
    turn.replayed_readout = Some(stored.readout.clone());
    turn.routed = stored.routed;
    turn.clarification = stored.clarification.map(|saved| Clarification {
        options: saved.options,
        cause: saved.cause,
        keeps_draft: saved.keeps_draft,
    });
    let decline = stored.decline.map(|record| {
        let code = record.code();
        Decline::from_record(record).unwrap_or_else(|_| Decline::with_default_reason(code))
    });
    match stored.ending {
        StoredEnding::Applied => {
            turn.unfinished = decline;
            Outcome::Applied
        }
        StoredEnding::NeedsClarification => Outcome::NeedsClarification,
        StoredEnding::Declined => Outcome::Declined(
            decline.unwrap_or_else(|| Decline::with_default_reason(DeclineCode::NotViable)),
        ),
        StoredEnding::Pending { pending, kind } => {
            let id = PendingId::new(pending);
            if table.pending().get(id).is_some() {
                turn.pending = Some(id);
                Outcome::Pending(kind)
            } else {
                // The pending resolved since: telling the seat it is still waiting would be a
                // pending nobody can answer.
                turn.replayed_readout = None;
                Outcome::Declined(Decline::with_default_reason(DeclineCode::WindowClosed))
            }
        }
    }
}

pub(super) fn stored_outcome(
    turn: &Turn<'_, '_>,
    outcome: &Outcome,
    readout: &str,
) -> StoredOutcome {
    let ending = match (outcome, turn.pending) {
        (Outcome::Pending(kind), Some(id)) => StoredEnding::Pending {
            pending: id.get(),
            kind: *kind,
        },
        (Outcome::Pending(_) | Outcome::Applied, _) => StoredEnding::Applied,
        (Outcome::Declined(_), _) => StoredEnding::Declined,
        (Outcome::NeedsClarification, _) => StoredEnding::NeedsClarification,
    };
    let decline = match outcome {
        Outcome::Declined(decline) => Some(decline.to_record()),
        Outcome::Applied => turn.unfinished.as_ref().map(Decline::to_record),
        Outcome::NeedsClarification | Outcome::Pending(_) => None,
    };
    StoredOutcome {
        ending,
        readout: readout.to_owned(),
        decline,
        clarification: turn
            .clarification
            .as_ref()
            .map(|clarification| StoredClarification {
                options: clarification.options.clone(),
                cause: clarification.cause,
                keeps_draft: clarification.keeps_draft,
            }),
        routed: turn.routed,
    }
}
