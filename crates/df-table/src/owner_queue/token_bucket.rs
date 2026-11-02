//! The per-seat token bucket of the owner queue (C.1: 3 inputs a second in a burst, 1 sustained).
//!
//! It owns the arithmetic only: whole milliseconds in, one token out or the exact wait. It never
//! reads a clock (the caller passes `now_ms`) and never decides what a refused token means.

use std::num::NonZeroU32;

const MILLITOKENS_PER_INPUT: u64 = 1_000;

/// A token bucket in thousandths of a token, so a rate of whole tokens per second refills exactly.
/// Arithmetic saturates: no elapsed time, however large, overflows or wraps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TokenBucket {
    capacity_millitokens: u64,
    refill_millitokens_per_ms: u64,
    held_millitokens: u64,
    refilled_at_ms: u64,
}

impl TokenBucket {
    /// A bucket holding `burst` tokens at `now_ms`, refilling `sustained_per_s` tokens a second.
    pub(crate) fn full(burst: NonZeroU32, sustained_per_s: NonZeroU32, now_ms: u64) -> Self {
        let capacity_millitokens = u64::from(burst.get()).saturating_mul(MILLITOKENS_PER_INPUT);
        Self {
            capacity_millitokens,
            refill_millitokens_per_ms: u64::from(sustained_per_s.get()),
            held_millitokens: capacity_millitokens,
            refilled_at_ms: now_ms,
        }
    }

    #[cfg(test)]
    pub(crate) fn held_millitokens(&self) -> u64 {
        self.held_millitokens
    }

    fn refill(&mut self, now_ms: u64) {
        let elapsed_ms = now_ms.saturating_sub(self.refilled_at_ms);
        if elapsed_ms == 0 {
            return;
        }
        self.refilled_at_ms = now_ms;
        let added = elapsed_ms.saturating_mul(self.refill_millitokens_per_ms);
        self.held_millitokens = self
            .held_millitokens
            .saturating_add(added)
            .min(self.capacity_millitokens);
    }

    /// Takes one token, or says how many milliseconds until one is held. A clock that moves back
    /// adds nothing and does not move the refill point back.
    pub(crate) fn try_take(&mut self, now_ms: u64) -> Result<(), u64> {
        self.refill(now_ms);
        if self.held_millitokens >= MILLITOKENS_PER_INPUT {
            self.held_millitokens -= MILLITOKENS_PER_INPUT;
            return Ok(());
        }
        let missing = MILLITOKENS_PER_INPUT - self.held_millitokens;
        Err(missing.div_ceil(self.refill_millitokens_per_ms))
    }
}
