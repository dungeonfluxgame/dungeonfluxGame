//! The four class queues: bounded, first in first out, served in class order.
//!
//! It owns where waiting requests sit and what a full queue does: a class that may drop discards
//! its oldest waiting request and counts it, and the live class refuses the new one instead (H.7).
//! A preempted or backing-off request re-enters at the front and may exceed its bound by the
//! number of provider slots, because it was already admitted. It never decides what to claim:
//! `scheduler` does.

use std::collections::VecDeque;

use df_table::ports::ResourceHandle;

use super::limits::{
    MAXIMUM_QUEUED_BACKGROUND, MAXIMUM_QUEUED_LIVE, MAXIMUM_QUEUED_LOOKAHEAD, MAXIMUM_QUEUED_PLATES,
};
use super::purpose::QueueClass;
use super::scheduler::BrokerLimitsError;

/// The most requests each class may have waiting; every bound is at least one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueBounds {
    live: usize,
    plate: usize,
    lookahead: usize,
    background: usize,
}

impl Default for QueueBounds {
    fn default() -> Self {
        Self {
            live: MAXIMUM_QUEUED_LIVE,
            plate: MAXIMUM_QUEUED_PLATES,
            lookahead: MAXIMUM_QUEUED_LOOKAHEAD,
            background: MAXIMUM_QUEUED_BACKGROUND,
        }
    }
}

impl QueueBounds {
    /// The bounds of the four classes. A bound of zero is refused: a live queue of zero would
    /// refuse every request, and a dropping queue of zero would still keep one.
    pub fn new(
        live: usize,
        plate: usize,
        lookahead: usize,
        background: usize,
    ) -> Result<Self, BrokerLimitsError> {
        Self::default()
            .with_bound(QueueClass::Live, live)?
            .with_bound(QueueClass::Plate, plate)?
            .with_bound(QueueClass::Lookahead, lookahead)?
            .with_bound(QueueClass::Background, background)
    }

    /// These bounds with `class` bounded at `bound`.
    pub fn with_bound(
        mut self,
        class: QueueClass,
        bound: usize,
    ) -> Result<Self, BrokerLimitsError> {
        if bound == 0 {
            return Err(BrokerLimitsError::ZeroQueueBound { class });
        }
        *match class {
            QueueClass::Live => &mut self.live,
            QueueClass::Plate => &mut self.plate,
            QueueClass::Lookahead => &mut self.lookahead,
            QueueClass::Background => &mut self.background,
        } = bound;
        Ok(self)
    }

    /// The most requests `class` may have waiting.
    pub fn of(self, class: QueueClass) -> usize {
        match class {
            QueueClass::Live => self.live,
            QueueClass::Plate => self.plate,
            QueueClass::Lookahead => self.lookahead,
            QueueClass::Background => self.background,
        }
    }
}

/// What pushing a request did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pushed {
    Queued,
    /// The queue was full, so its oldest request was discarded to make room.
    DroppedOldest(ResourceHandle),
}

/// The queue of a class that cannot drop is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Full;

#[derive(Debug)]
pub(super) struct ClassQueues {
    queues: [VecDeque<ResourceHandle>; 4],
    bounds: QueueBounds,
    dropped: [u64; 4],
}

impl ClassQueues {
    pub(super) fn new(bounds: QueueBounds) -> Self {
        Self {
            queues: Default::default(),
            bounds,
            dropped: [0; 4],
        }
    }

    fn queue(&self, class: QueueClass) -> &VecDeque<ResourceHandle> {
        &self.queues[class.rank()]
    }

    fn queue_mut(&mut self, class: QueueClass) -> &mut VecDeque<ResourceHandle> {
        &mut self.queues[class.rank()]
    }

    /// Whether a new request of this class would be refused.
    pub(super) fn refuses(&self, class: QueueClass) -> bool {
        !class.drops_oldest_when_full() && self.depth(class) >= self.bounds.of(class)
    }

    pub(super) fn push_back(
        &mut self,
        class: QueueClass,
        handle: ResourceHandle,
    ) -> Result<Pushed, Full> {
        if self.refuses(class) {
            return Err(Full);
        }
        let bound = self.bounds.of(class);
        let dropped = if self.depth(class) >= bound {
            self.dropped[class.rank()] += 1;
            self.queue_mut(class).pop_front()
        } else {
            None
        };
        self.queue_mut(class).push_back(handle);
        Ok(dropped.map_or(Pushed::Queued, Pushed::DroppedOldest))
    }

    /// Puts an already admitted request back in front of its class, past the bound if need be.
    pub(super) fn push_front(&mut self, class: QueueClass, handle: ResourceHandle) {
        self.queue_mut(class).push_front(handle);
    }

    pub(super) fn remove(&mut self, class: QueueClass, handle: ResourceHandle) -> bool {
        let queue = self.queue_mut(class);
        match queue.iter().position(|queued| *queued == handle) {
            Some(index) => queue.remove(index).is_some(),
            None => false,
        }
    }

    pub(super) fn depth(&self, class: QueueClass) -> usize {
        self.queue(class).len()
    }

    /// The waiting handles of a class, oldest first.
    pub(super) fn iter(&self, class: QueueClass) -> impl Iterator<Item = ResourceHandle> + '_ {
        self.queue(class).iter().copied()
    }

    pub(super) fn dropped(&self, class: QueueClass) -> u64 {
        self.dropped[class.rank()]
    }
}
