//! Where a waiting input stands in the total order (K.6: a waiting phone shows its place).
//!
//! It owns the arithmetic of `OwnerQueue::position`: the lanes served before the input's lane, and
//! inside the player and controls lanes the number of serves a round-robin makes before the input,
//! worked out from the seats' line lengths and the cursor. It only reads the queue. The count is
//! exact while no seat is held behind its own free-text line; a held seat is counted as if it were
//! served, so the count can overstate and never understates.

use crate::envelope::Sender;
use crate::identity::{OperationId, SeatId};

use super::counted::Ring;
use super::state::{SeatState, Waiting};
use super::{Lane, OperationStatus, OwnerQueue, QueuePosition};

impl OwnerQueue {
    /// Where a waiting input stands in the total order, or `None` when it is not waiting.
    pub fn position(&self, sender: Sender, operation_id: OperationId) -> Option<QueuePosition> {
        let key = self.key_of(sender, operation_id)?;
        let record = self.operations.get(&key)?;
        let OperationStatus::Queued { lane } = record.status else {
            return None;
        };
        let mut position = QueuePosition {
            lane,
            ahead: 0,
            held_behind_own_text: false,
        };
        let Sender::Seat(seat) = sender else {
            position.ahead = self.engine_position(lane, operation_id);
            return Some(position);
        };
        let state = self.seats.get(&seat)?;
        match lane {
            Lane::SeatControls => {
                let index = index_of(state.controls.iter(), operation_id)?;
                position.ahead = self.round_robin_ahead(&self.control_ring, seat, index, |state| {
                    state.controls.len()
                });
            }
            Lane::Player => {
                let index = index_of(state.lines.iter(), operation_id)?;
                position.ahead = self
                    .controls_waiting
                    .saturating_add(self.engine.waiting.len())
                    .saturating_add(self.timers_and_results.waiting.len())
                    .saturating_add(self.round_robin_ahead(
                        &self.line_ring,
                        seat,
                        index,
                        |state| state.lines.len(),
                    ));
                position.held_behind_own_text = !self.line_ring.contains(seat);
            }
            Lane::Engine | Lane::TimersAndResults => return None,
        }
        Some(position)
    }

    fn engine_position(&self, lane: Lane, operation_id: OperationId) -> usize {
        let (before, fifo) = match lane {
            Lane::Engine => (self.controls_waiting, &self.engine),
            _ => (
                self.controls_waiting
                    .saturating_add(self.engine.waiting.len()),
                &self.timers_and_results,
            ),
        };
        let index = index_of(fifo.waiting.iter(), operation_id).unwrap_or(0);
        before.saturating_add(index)
    }

    /// How many serves come before the input at `index` of `seat`'s line, with no new arrivals:
    /// every earlier round serves each other seat that has an input at that round, and in its own
    /// round the seats before it in the cyclic order from the cursor are served first.
    fn round_robin_ahead(
        &self,
        ring: &Ring,
        seat: SeatId,
        index: usize,
        length_of: impl Fn(&SeatState) -> usize,
    ) -> usize {
        let order = |seat: SeatId| (ring.last().is_some_and(|last| seat <= last), seat);
        let own = order(seat);
        let mut ahead = index;
        for other in ring.iter().filter(|other| *other != seat) {
            let length = self.seats.get(&other).map_or(0, &length_of);
            ahead = ahead.saturating_add(length.min(index));
            if length > index && order(other) < own {
                ahead = ahead.saturating_add(1);
            }
        }
        ahead
    }
}

fn index_of<'a>(
    mut entries: impl Iterator<Item = &'a Waiting>,
    operation_id: OperationId,
) -> Option<usize> {
    entries.position(|waiting| waiting.envelope.operation_id() == operation_id)
}
