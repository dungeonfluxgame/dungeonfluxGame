//! The pending book's report and the stored rulings' report (RES26, C.4, RES48).
//!
//! A pending is listed for an administrator in full and for a seat only when the seat opened it or
//! is asked something by it, with only that seat's own offer shown: another seat's choices and
//! answer are not the seat's to read. What a pending will resume (`continuation`, which holds a
//! ruling's options and a roll's sides) is the host's, and the dice it already drew are the
//! operator's, because a hidden roll must not be read off a report before it is revealed. The
//! stored rulings are the host's administration and are not shown to a seat at all.
//!
//! The rulings book has no way to list its entries, so its report is its saved record as data
//! (`Dump`): a record that could not be written as data says so and is never shown as empty. It
//! never changes a book.

use serde::Serialize;
use serde_json::Value;

use crate::envelope::PendingId;
use crate::identity::SceneId;
use crate::identity::SeatId;
use crate::inspect::{Inspect, InspectView, Needs, Visible};
use crate::machine_slot::ErrorClass;
use crate::outcome::PendingKind;
use crate::pending::{
    Completion, Continuation, PendingBook, PendingResolution, PendingStatus, RulingBook, SeatOffer,
    StoredDraw,
};
use crate::ports::{ActionId, ConfirmationReason, RulingRequest};

/// A record written as data, or why it could not be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Dump {
    Json { value: Value },
    Unreadable { class: ErrorClass },
}

impl Dump {
    pub(crate) fn of(record: &impl Serialize) -> Self {
        match serde_json::to_value(record) {
            Ok(value) => Self::Json { value },
            Err(error) => Self::Unreadable {
                class: error.classify().into(),
            },
        }
    }
}

/// One seat's share of a window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfferLine {
    pub seat: SeatId,
    pub choices: Vec<ActionId>,
    pub answer: Option<ActionId>,
}

/// What a pending will do when it is answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "continuation", rename_all = "snake_case")]
pub enum ContinuationDetail {
    Confirm { reason: ConfirmationReason },
    Resolve { nesting: u32, roll_sides: Vec<u16> },
    Ruling { request: RulingRequest, reuse: bool },
}

/// The action a pending holds and what answering it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContinuationLine {
    pub seat: SeatId,
    pub action: ActionId,
    pub detail: ContinuationDetail,
}

/// One open pending resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingLine {
    pub id: PendingId,
    pub kind: PendingKind,
    pub status: PendingStatus,
    pub opened_by: SeatId,
    pub scene: Option<SceneId>,
    pub waited_ms: u64,
    pub nudged: bool,
    pub held: bool,
    pub completion: Completion,
    /// An administrator sees every offer; a seat only its own, and only for a pending it is in.
    pub offers: Vec<OfferLine>,
    pub continuation: Visible<ContinuationLine>,
    /// The dice drawn so far, oldest first.
    pub draws: Visible<Vec<StoredDraw>>,
}

/// The open pendings, in id order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingReport {
    pub open: Visible<usize>,
    pub pendings: Vec<PendingLine>,
}

fn continuation_line(continuation: &Continuation) -> ContinuationLine {
    let detail = match continuation {
        Continuation::Confirm { reason, .. } => ContinuationDetail::Confirm { reason: *reason },
        Continuation::Resolve { nesting, roll, .. } => ContinuationDetail::Resolve {
            nesting: *nesting,
            roll_sides: roll.clone(),
        },
        Continuation::Ruling { request, reuse, .. } => ContinuationDetail::Ruling {
            request: request.clone(),
            reuse: *reuse,
        },
    };
    ContinuationLine {
        seat: continuation.seat(),
        action: continuation.action().clone(),
        detail,
    }
}

fn offer_line(offer: &SeatOffer) -> OfferLine {
    OfferLine {
        seat: offer.seat(),
        choices: offer.choices().to_vec(),
        answer: offer.answer().cloned(),
    }
}

fn involves(pending: &PendingResolution, seat: SeatId) -> bool {
    pending.opened_by() == seat || pending.window().offer_for(seat).is_some()
}

fn pending_line(view: InspectView, pending: &PendingResolution) -> PendingLine {
    let audience = view.audience;
    let offers = pending
        .window()
        .offers()
        .iter()
        .filter(|offer| audience.administers() || audience.seat() == Some(offer.seat()))
        .map(offer_line)
        .collect();
    PendingLine {
        id: pending.id(),
        kind: pending.kind(),
        status: pending.status(),
        opened_by: pending.opened_by(),
        scene: pending.scene(),
        waited_ms: pending.waited_ms(view.now_ms),
        nudged: pending.is_nudged(),
        held: pending.is_held(),
        completion: pending.window().completion(),
        offers,
        continuation: Visible::when(view, Needs::Host, || {
            continuation_line(pending.continuation())
        }),
        draws: Visible::when(view, Needs::Operator, || pending.draws().to_vec()),
    }
}

impl Inspect for PendingBook {
    type Report = PendingReport;

    fn inspect(&self, view: InspectView) -> PendingReport {
        let pendings = self
            .iter()
            .filter(|pending| match view.audience.seat() {
                Some(seat) => involves(pending, seat),
                None => true,
            })
            .map(|pending| pending_line(view, pending))
            .collect();
        PendingReport {
            open: Visible::when(view, Needs::Host, || self.len()),
            pendings,
        }
    }
}

/// The stored rulings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulingsReport {
    pub stored: Visible<usize>,
    /// The saved record: every ruling with its key, scope, answer, tier and source.
    pub entries: Visible<Dump>,
}

impl Inspect for RulingBook {
    type Report = RulingsReport;

    fn inspect(&self, view: InspectView) -> RulingsReport {
        RulingsReport {
            stored: Visible::when(view, Needs::Host, || self.len()),
            entries: Visible::when(view, Needs::Host, || Dump::of(&self.to_record())),
        }
    }
}
