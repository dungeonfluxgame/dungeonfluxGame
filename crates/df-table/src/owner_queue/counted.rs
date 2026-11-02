//! The queue's collections, which count every entry they touch when the crate is under test.
//!
//! `Book`, `Fifo` and `Ring` are the only containers the owner queue keeps its state in, and each
//! method that reaches an entry (a lookup, a push, a pop, one step of an iteration) adds one to a
//! per-thread meter in test builds. The queue cannot scan its state without going through them, so
//! a test that reads the meter measures work the queue did and cannot under-report it. In
//! production builds the meter does not exist and the calls compile to nothing.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::identity::SeatId;

#[cfg(test)]
pub(crate) use meter::{read as touched_entries, reset as reset_touched_entries};

#[cfg(test)]
fn touch() {
    meter::add(1);
}

#[cfg(not(test))]
fn touch() {}

/// An ordered map that counts each entry it looks at.
pub(super) struct Book<K, V> {
    entries: BTreeMap<K, V>,
}

impl<K: Ord, V> Book<K, V> {
    pub(super) fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn get(&self, key: &K) -> Option<&V> {
        touch();
        self.entries.get(key)
    }

    pub(super) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        touch();
        self.entries.get_mut(key)
    }

    pub(super) fn contains_key(&self, key: &K) -> bool {
        touch();
        self.entries.contains_key(key)
    }

    pub(super) fn insert(&mut self, key: K, value: V) -> Option<V> {
        touch();
        self.entries.insert(key, value)
    }

    pub(super) fn remove(&mut self, key: &K) -> Option<V> {
        touch();
        self.entries.remove(key)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().inspect(|_| touch())
    }
}

/// A first-in first-out queue that counts each entry it pushes, pops, looks at or walks past.
pub(super) struct Fifo<T> {
    entries: VecDeque<T>,
}

impl<T> Fifo<T> {
    pub(super) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn push_back(&mut self, entry: T) {
        touch();
        self.entries.push_back(entry);
    }

    pub(super) fn pop_front(&mut self) -> Option<T> {
        touch();
        self.entries.pop_front()
    }

    pub(super) fn front(&self) -> Option<&T> {
        touch();
        self.entries.front()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().inspect(|_| touch())
    }
}

/// The seats that may be served next, with the seat served last. `next` is the seat after the
/// cursor in seat order, wrapping to the smallest, so each eligible seat is served once per pass.
pub(super) struct Ring {
    eligible: BTreeSet<SeatId>,
    last: Option<SeatId>,
}

impl Ring {
    pub(super) fn new() -> Self {
        Self {
            eligible: BTreeSet::new(),
            last: None,
        }
    }

    pub(super) fn set_eligible(&mut self, seat: SeatId, eligible: bool) {
        touch();
        if eligible {
            self.eligible.insert(seat);
        } else {
            self.eligible.remove(&seat);
        }
    }

    pub(super) fn contains(&self, seat: SeatId) -> bool {
        touch();
        self.eligible.contains(&seat)
    }

    /// The seat to serve next, without moving the cursor.
    pub(super) fn next(&self) -> Option<SeatId> {
        touch();
        let after_cursor = self.last.and_then(|last| {
            self.eligible
                .range((std::ops::Bound::Excluded(last), std::ops::Bound::Unbounded))
                .next()
        });
        after_cursor.or_else(|| self.eligible.first()).copied()
    }

    /// Moves the cursor to the seat that was just served.
    pub(super) fn served(&mut self, seat: SeatId) {
        self.last = Some(seat);
    }

    pub(super) fn last(&self) -> Option<SeatId> {
        self.last
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = SeatId> {
        self.eligible.iter().inspect(|_| touch()).copied()
    }
}

#[cfg(test)]
mod meter {
    use std::cell::Cell;

    thread_local! {
        static TOUCHED: Cell<u64> = const { Cell::new(0) };
    }

    pub(super) fn add(entries: u64) {
        TOUCHED.with(|touched| touched.set(touched.get().saturating_add(entries)));
    }

    pub(crate) fn read() -> u64 {
        TOUCHED.with(Cell::get)
    }

    pub(crate) fn reset() {
        TOUCHED.with(|touched| touched.set(0));
    }
}
