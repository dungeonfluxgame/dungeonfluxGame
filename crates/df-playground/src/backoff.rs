//! How long the page waits before asking the host again after the host could not be reached.
//!
//! The wait doubles with each failure up to a ceiling, so a host that is down costs a request every
//! ten seconds instead of one every second, and a host that comes back is found again within that
//! time. It never reads a clock and never touches the page.

/// The first wait after a failure, in milliseconds.
pub const FIRST_RETRY_MS: u32 = 1_000;
/// The longest wait, in milliseconds.
pub const LONGEST_RETRY_MS: u32 = 10_000;

/// The wait after `failures` failures in a row (1 is the first failure). Zero failures wait the
/// normal, shortest time.
pub fn retry_delay_ms(failures: u32) -> u32 {
    let doublings = failures.saturating_sub(1).min(16);
    FIRST_RETRY_MS
        .saturating_mul(1_u32 << doublings)
        .min(LONGEST_RETRY_MS)
}
