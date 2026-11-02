//! A map that holds at most a fixed number of entries and forgets the oldest first.
//!
//! It owns `BoundedMap`, the one structure under the ruling book, the request book and the result
//! book, so each bound, each eviction order and each lookup is written once: a lookup is one map
//! access, an insert or a removal is amortised constant on top of it, and the order list is
//! compacted once stale entries outnumber live ones, so it never grows without the map growing.
//! It never decides what an entry means and never hashes one: it hands back what it removed (a
//! replaced entry, an evicted one) so the caller keeps its own digest exact.

use std::collections::{BTreeMap, VecDeque};

/// What an insert pushed out: the entry it replaced and the entries beyond the bound.
#[derive(Debug)]
pub(crate) struct Removed<V> {
    pub(crate) replaced: Option<V>,
    pub(crate) evicted: Vec<V>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundedMap<K: Ord + Clone, V> {
    entries: BTreeMap<K, (u64, V)>,
    /// The keys in the order they were stored, each with the sequence it was stored under; an
    /// entry whose sequence no longer matches the map was replaced or removed and is skipped.
    order: VecDeque<(K, u64)>,
    next_sequence: u64,
    maximum: usize,
}

impl<K: Ord + Clone, V> BoundedMap<K, V> {
    pub(crate) fn holding_at_most(maximum: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            next_sequence: 0,
            maximum,
        }
    }

    pub(crate) fn maximum(&self) -> usize {
        self.maximum
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|(_, value)| value)
    }

    pub(crate) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.entries.get_mut(key).map(|(_, value)| value)
    }

    /// The values, oldest stored first.
    pub(crate) fn oldest_first(&self) -> Vec<&V> {
        let mut live: Vec<&(u64, V)> = self.entries.values().collect();
        live.sort_by_key(|(sequence, _)| *sequence);
        live.into_iter().map(|(_, value)| value).collect()
    }

    #[cfg(test)]
    pub(crate) fn order_len(&self) -> usize {
        self.order.len()
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.values().map(|(_, value)| value)
    }

    /// Stores `value` under `key` as the newest entry. A bound of zero stores nothing.
    pub(crate) fn insert(&mut self, key: K, value: V) -> Removed<V> {
        let mut removed = Removed {
            replaced: None,
            evicted: Vec::new(),
        };
        if self.maximum == 0 {
            removed.evicted.push(value);
            return removed;
        }
        removed.replaced = self.entries.remove(&key).map(|(_, old)| old);
        self.next_sequence += 1;
        self.order.push_back((key.clone(), self.next_sequence));
        self.entries.insert(key, (self.next_sequence, value));
        while self.entries.len() > self.maximum {
            let Some((oldest, sequence)) = self.order.pop_front() else {
                break;
            };
            let current = self.entries.get(&oldest).map(|(held, _)| *held);
            if current == Some(sequence)
                && let Some((_, old)) = self.entries.remove(&oldest)
            {
                removed.evicted.push(old);
            }
        }
        self.compact();
        removed
    }

    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        self.entries.remove(key).map(|(_, value)| value)
    }

    fn compact(&mut self) {
        if self.order.len() <= self.entries.len() * 2 + 16 {
            return;
        }
        let entries = &self.entries;
        self.order
            .retain(|(key, sequence)| entries.get(key).is_some_and(|(held, _)| held == sequence));
    }
}
