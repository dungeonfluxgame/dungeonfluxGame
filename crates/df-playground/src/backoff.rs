//! How long the page waits before asking the host again after the host could not be reached.
//!
//! The wait doubles with each failure up to a ceiling, so a host that is down costs a request every
//! ten seconds instead of one every second, and a host that comes back is found again within that
//! time. It never reads a clock and never touches the page.

use crate::limits::{FIRST_RETRY_MS, LONGEST_RETRY_MS};

/// The wait after `failures` failures in a row (1 is the first failure). Zero failures wait the
/// normal, shortest time.
pub fn retry_delay_ms(failures: u32) -> u64 {
    let doublings = failures.saturating_sub(1).min(16);
    FIRST_RETRY_MS
        .saturating_mul(1_u64 << doublings)
        .min(LONGEST_RETRY_MS)
}
