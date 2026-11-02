//! The one small hasher the books of the pipeline digest themselves with.
//!
//! It owns `Fnv`, FNV-1a over 64 bits: allocation free and the same in every run, unlike a
//! randomly keyed hasher, so a digest can be compared between a recording and its replay. Each
//! book keeps an order-independent digest (the exclusive or of the hash of each entry) that it
//! updates when an entry changes, so the digest of a book costs nothing per input however many
//! entries it holds. It never reads a clock: a digest covers logical state only.

use std::hash::Hasher;

/// FNV-1a, 64 bits.
pub(crate) struct Fnv(u64);

impl Fnv {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub(crate) fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }
}

impl Hasher for Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

// Counts the entries a book walked, so a test can prove that an input costs the same whether a
// book holds a hundred entries or two hundred (the pattern of `owner_queue`'s counter).
#[cfg(test)]
thread_local! {
    static WALKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Notes that `entries` entries were walked.
#[cfg(test)]
pub(crate) fn walked(entries: usize) {
    WALKED.with(|count| count.set(count.get() + entries));
}

#[cfg(not(test))]
pub(crate) fn walked(_entries: usize) {}

/// How many entries were walked since the last reset.
#[cfg(test)]
pub(crate) fn walked_entries() -> usize {
    WALKED.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_walked_entries() {
    WALKED.with(|count| count.set(0));
}

/// Spreads a number over all 64 bits, for folding a counter into an exclusive-or digest.
pub(crate) fn mix(value: u64) -> u64 {
    value.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}
