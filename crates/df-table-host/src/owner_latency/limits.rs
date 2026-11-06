//! The latency spike's size, count and time bounds. They only keep a run from stopping or
//! refusing for a reason that is not what is measured, so they are far above any figure the run
//! reaches.

/// The ms the commit may take before the owner gives the batch up: far above any figure here.
pub const COMMIT_DEADLINE_MS: u64 = 60_000;
/// The most narration lines the stand-in state keeps.
pub const NARRATION_LIMIT: usize = 200;
/// The pending bound per seat (J.4's default of 3), so the queue is as deep as the design allows.
pub const SEAT_PENDING_LIMIT: u32 = 3;
/// A rate so high that the token bucket never refuses; admission refusals are not what is measured.
pub const UNLIMITED_RATE: u32 = 1_000_000;
