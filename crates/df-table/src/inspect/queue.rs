//! The owner queue's report: how deep each lane is, what each seat has waiting and what is in
//! progress.
//!
//! The lanes, the order counter and the remembered operations are the table's administration, so
//! only an administrator sees them. A seat sees its own line and its own operations, never another
//! seat's, and never the engine's: what the engine has in progress would say that something is
//! about to happen. It never changes the queue.

use serde::Serialize;

use crate::envelope::Sender;
use crate::identity::{OperationId, SeatId, TurnSeq};
use crate::inspect::{Inspect, InspectView, Needs, Visible};
use crate::owner_queue::{Lane, OwnerQueue};

/// How many inputs wait in one lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LaneDepth {
    pub lane: Lane,
    pub depth: usize,
}

/// What one seat has admitted and not finished (what `seat_pending_limit` bounds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SeatQueueLine {
    pub seat: SeatId,
    pub pending: usize,
}

/// An input served and not finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InProgressLine {
    /// The seat that sent it, or `None` for the engine.
    pub sender: Option<SeatId>,
    pub operation_id: OperationId,
    pub turn_seq: TurnSeq,
    pub age_ms: u64,
}

/// The state of the owner queue as one audience may see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OwnerQueueReport {
    /// The `turn_seq` of the last input served.
    pub last_turn_seq: Visible<TurnSeq>,
    pub remembered_operations: Visible<usize>,
    /// The lanes in the order they are served.
    pub lanes: Visible<Vec<LaneDepth>>,
    /// The seats at the table, in seat order; a seat sees only its own line.
    pub seats: Vec<SeatQueueLine>,
    /// Oldest first; a seat sees only the operations it sent.
    pub in_progress: Vec<InProgressLine>,
}

impl Inspect for OwnerQueue {
    type Report = OwnerQueueReport;

    fn inspect(&self, view: InspectView) -> OwnerQueueReport {
        let audience = view.audience;
        let seats = self
            .seated()
            .into_iter()
            .filter(|seat| audience.administers() || audience.seat() == Some(*seat))
            .filter_map(|seat| {
                self.seat_pending(seat)
                    .map(|pending| SeatQueueLine { seat, pending })
            })
            .collect();
        let in_progress = self
            .in_progress(view.now_ms)
            .into_iter()
            .filter_map(|operation| {
                let sender = match operation.sender {
                    Sender::Engine => None,
                    Sender::Seat(seat) => Some(seat),
                };
                let visible = match sender {
                    Some(seat) => audience.administers() || audience.seat() == Some(seat),
                    None => audience.administers(),
                };
                visible.then_some(InProgressLine {
                    sender,
                    operation_id: operation.operation_id,
                    turn_seq: operation.turn_seq,
                    age_ms: operation.age_ms,
                })
            })
            .collect();
        OwnerQueueReport {
            last_turn_seq: Visible::when(view, Needs::Host, || self.last_turn_seq()),
            remembered_operations: Visible::when(view, Needs::Host, || {
                self.remembered_operations()
            }),
            lanes: Visible::when(view, Needs::Host, || {
                Lane::ALL
                    .iter()
                    .map(|lane| LaneDepth {
                        lane: *lane,
                        depth: self.depth(*lane),
                    })
                    .collect()
            }),
            seats,
            in_progress,
        }
    }
}
