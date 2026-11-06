//! Every size and count bound of the production adapters, in one place.

/// The most draws a `RecordedDice` keeps. Deliberately huge so it never limits play; scope it
/// down later (owner, 2026-11-02). A draw past it is refused rather than forgotten, because a
/// forgotten draw could not be replayed.
pub const MAXIMUM_RECORDED_DRAWS: usize = 1_000_000;

/// How many random numbers one draw may reject while removing modulo bias before the source is
/// called broken. A sound source rejects with a probability below 2^-48, so this is never reached.
pub const MAXIMUM_REJECTIONS_PER_DRAW: u32 = 1_000;
