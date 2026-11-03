//! A map that holds at most a fixed number of entries, forgets the oldest first, and is cheap to
//! copy however much it holds.
//!
//! It owns `BoundedMap`, the one structure under the ruling book, the request book, the result
//! book and the table's memory of applied operations, so each bound, each eviction order and each
//! lookup is written once. It is built for the group commit (O.16 S1), where the host takes a
//! checkpoint before every batch and the first change after it must not copy the whole history:
//!
//! - the entries are spread over buckets chosen by a hash of the key, and each bucket is behind an
//!   `Arc`, so a clone copies `bucket_count` pointers and the first change to a bucket after a
//!   clone copies that one bucket (about `len / bucket_count` entries) and no other;
//! - the order the entries were stored in is a queue of chunks behind `Arc`s, so a clone copies one
//!   pointer per chunk and a push copies at most one chunk;
//! - the bucket count grows with the entries (never past `MAXIMUM_BUCKETS`) so a bucket stays
//!   small, and the queue is compacted once stale entries outnumber live ones, so neither grows
//!   without the map growing.
//!
//! A lookup is one hash and one bucket access, an insert or a removal is amortised constant on top
//! of it, and the saved order is read off the queue without a sort. It never decides what an entry
//! means and never hashes one for a digest: it hands back what it removed (a replaced entry, an
//! evicted one) so the caller keeps its own digest exact.

use std::collections::{BTreeMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::digest::Fnv;

/// How many entries one bucket holds before the map doubles its buckets.
const ENTRIES_PER_BUCKET: usize = 8;
/// The fewest buckets a map has.
const MINIMUM_BUCKETS: usize = 16;
/// The most buckets a map has: a clone copies at most this many pointers.
const MAXIMUM_BUCKETS: usize = 4_096;
/// How many keys one chunk of the order queue holds: a push copies at most this many.
const CHUNK_LENGTH: usize = 1_024;

/// What an insert pushed out: the entry it replaced and the entries beyond the bound.
#[derive(Debug)]
pub(crate) struct Removed<V> {
    pub(crate) replaced: Option<V>,
    pub(crate) evicted: Vec<V>,
}

type Bucket<K, V> = Arc<BTreeMap<K, (u64, V)>>;

// Counts the entries a change had to copy because a clone of the map still held their bucket or
// chunk, so a test can prove that the first change after a checkpoint copies a bucket and not the
// history (the pattern of `digest::walked`).
#[cfg(test)]
thread_local! {
    static COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn copied(entries: usize) {
    COPIED.with(|count| count.set(count.get() + entries));
}

/// How many entries were copied since the last reset.
#[cfg(test)]
pub(crate) fn copied_entries() -> usize {
    COPIED.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_copied_entries() {
    COPIED.with(|count| count.set(0));
}

#[derive(Debug, Clone)]
pub(crate) struct BoundedMap<K: Ord + Clone + Hash, V: Clone> {
    buckets: Vec<Bucket<K, V>>,
    /// The keys in the order they were stored, each with the sequence it was stored under; an
    /// entry whose sequence no longer matches the map was replaced or removed and is skipped.
    order: VecDeque<Arc<Vec<(K, u64)>>>,
    /// How many keys of the first chunk were already passed.
    first_chunk_passed: usize,
    order_len: usize,
    live: usize,
    next_sequence: u64,
    maximum: usize,
}

impl<K: Ord + Clone + Hash, V: Clone> BoundedMap<K, V> {
    pub(crate) fn holding_at_most(maximum: usize) -> Self {
        Self {
            buckets: Self::empty_buckets(MINIMUM_BUCKETS),
            order: VecDeque::new(),
            first_chunk_passed: 0,
            order_len: 0,
            live: 0,
            next_sequence: 0,
            maximum,
        }
    }

    fn empty_buckets(count: usize) -> Vec<Bucket<K, V>> {
        let empty: Bucket<K, V> = Arc::new(BTreeMap::new());
        vec![empty; count]
    }

    pub(crate) fn maximum(&self) -> usize {
        self.maximum
    }

    pub(crate) fn len(&self) -> usize {
        self.live
    }

    fn bucket_of(&self, key: &K) -> usize {
        let mut hasher = Fnv::new();
        key.hash(&mut hasher);
        // The bucket count is a power of two, so the low bits of the hash pick the bucket.
        (hasher.finish() as usize) & (self.buckets.len() - 1)
    }

    pub(crate) fn contains(&self, key: &K) -> bool {
        self.buckets[self.bucket_of(key)].contains_key(key)
    }

    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        self.buckets[self.bucket_of(key)]
            .get(key)
            .map(|(_, value)| value)
    }

    /// The value for a key, to change it in place. Only the bucket the key lives in is copied if a
    /// clone of the map is still holding it.
    pub(crate) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let index = self.bucket_of(key);
        if !self.buckets[index].contains_key(key) {
            return None;
        }
        self.bucket_mut(index).get_mut(key).map(|(_, value)| value)
    }

    /// A bucket to change. A bucket a clone of the map still holds is copied first, and only it.
    fn bucket_mut(&mut self, index: usize) -> &mut BTreeMap<K, (u64, V)> {
        #[cfg(test)]
        if Arc::strong_count(&self.buckets[index]) > 1 {
            copied(self.buckets[index].len());
        }
        Arc::make_mut(&mut self.buckets[index])
    }

    /// The live keys and values, oldest stored first, with no sort: the order queue is read in
    /// order and each key is checked against the sequence it is stored under.
    pub(crate) fn iter_oldest_first(&self) -> impl Iterator<Item = (&K, &V)> {
        let mut skip = self.first_chunk_passed;
        self.order
            .iter()
            .flat_map(move |chunk| {
                let from = std::mem::take(&mut skip);
                chunk.iter().skip(from)
            })
            .filter_map(|(key, sequence)| {
                let (held, value) = self.buckets[self.bucket_of(key)].get(key)?;
                (held == sequence).then_some((key, value))
            })
    }

    /// The values, oldest stored first.
    pub(crate) fn oldest_first(&self) -> Vec<&V> {
        self.iter_oldest_first().map(|(_, value)| value).collect()
    }

    #[cfg(test)]
    pub(crate) fn order_len(&self) -> usize {
        self.order_len
    }

    /// How many buckets the map has, which is what a clone copies.
    #[cfg(test)]
    pub(crate) fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.buckets
            .iter()
            .flat_map(|bucket| bucket.values().map(|(_, value)| value))
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
        removed.replaced = self.take_entry(&key);
        self.next_sequence += 1;
        let sequence = self.next_sequence;
        self.push_order(key.clone(), sequence);
        let index = self.bucket_of(&key);
        self.bucket_mut(index).insert(key, (sequence, value));
        self.live += 1;
        while self.live > self.maximum {
            let Some((oldest, passed)) = self.pop_order() else {
                break;
            };
            let current = self.buckets[self.bucket_of(&oldest)]
                .get(&oldest)
                .map(|(held, _)| *held);
            if current == Some(passed)
                && let Some(old) = self.take_entry(&oldest)
            {
                removed.evicted.push(old);
            }
        }
        self.grow_if_needed();
        self.compact();
        removed
    }

    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        self.take_entry(key)
    }

    /// Takes an entry out of its bucket, copying the bucket only if it is shared.
    fn take_entry(&mut self, key: &K) -> Option<V> {
        let index = self.bucket_of(key);
        if !self.buckets[index].contains_key(key) {
            return None;
        }
        let taken = self.bucket_mut(index).remove(key).map(|(_, value)| value);
        if taken.is_some() {
            self.live -= 1;
        }
        taken
    }

    fn push_order(&mut self, key: K, sequence: u64) {
        match self.order.back_mut() {
            Some(chunk) if chunk.len() < CHUNK_LENGTH => {
                #[cfg(test)]
                if Arc::strong_count(chunk) > 1 {
                    copied(chunk.len());
                }
                Arc::make_mut(chunk).push((key, sequence));
            }
            _ => self.order.push_back(Arc::new(vec![(key, sequence)])),
        }
        self.order_len += 1;
    }

    /// The oldest key in the order queue and the sequence it was stored under.
    fn pop_order(&mut self) -> Option<(K, u64)> {
        let first = self.order.front()?;
        let (key, sequence) = first.get(self.first_chunk_passed)?.clone();
        self.first_chunk_passed += 1;
        self.order_len -= 1;
        if self.first_chunk_passed >= first.len() {
            self.order.pop_front();
            self.first_chunk_passed = 0;
        }
        Some((key, sequence))
    }

    /// Doubles the buckets while they hold more than `ENTRIES_PER_BUCKET` entries each, up to
    /// `MAXIMUM_BUCKETS`. The entries are spread again, which copies each once; that happens
    /// once per doubling.
    fn grow_if_needed(&mut self) {
        let wanted = self
            .live
            .div_ceil(ENTRIES_PER_BUCKET)
            .next_power_of_two()
            .clamp(MINIMUM_BUCKETS, MAXIMUM_BUCKETS);
        if wanted <= self.buckets.len() {
            return;
        }
        let old = std::mem::replace(&mut self.buckets, Self::empty_buckets(wanted));
        let mut grown: Vec<BTreeMap<K, (u64, V)>> = (0..wanted).map(|_| BTreeMap::new()).collect();
        for bucket in old {
            for (key, entry) in bucket.iter() {
                let mut hasher = Fnv::new();
                key.hash(&mut hasher);
                let index = (hasher.finish() as usize) & (wanted - 1);
                grown[index].insert(key.clone(), entry.clone());
            }
        }
        self.buckets = grown.into_iter().map(Arc::new).collect();
    }

    /// Drops the stale keys of the order queue once they outnumber the live ones.
    fn compact(&mut self) {
        if self.order_len <= self.live * 2 + CHUNK_LENGTH {
            return;
        }
        let live: Vec<(K, u64)> = self
            .order
            .iter()
            .enumerate()
            .flat_map(|(position, chunk)| {
                let from = if position == 0 {
                    self.first_chunk_passed
                } else {
                    0
                };
                chunk.iter().skip(from)
            })
            .filter(|(key, sequence)| {
                self.buckets[self.bucket_of(key)]
                    .get(key)
                    .is_some_and(|(held, _)| held == sequence)
            })
            .cloned()
            .collect();
        self.order = live
            .chunks(CHUNK_LENGTH)
            .map(|chunk| Arc::new(chunk.to_vec()))
            .collect();
        self.first_chunk_passed = 0;
        self.order_len = live.len();
    }
}

impl<K: Ord + Clone + Hash, V: Clone + PartialEq> PartialEq for BoundedMap<K, V> {
    /// Two maps are equal when they hold the same entries in the same order under the same bound,
    /// however their buckets were grown.
    fn eq(&self, other: &Self) -> bool {
        self.maximum == other.maximum
            && self.live == other.live
            && self.iter_oldest_first().eq(other.iter_oldest_first())
    }
}

impl<K: Ord + Clone + Hash, V: Clone + Eq> Eq for BoundedMap<K, V> {}
