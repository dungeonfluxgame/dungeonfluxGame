//! How long a page waits before it tries its game connection again.
//!
//! A refused or lost connection is never final: the page keeps trying, slowly enough not to
//! hammer a server that is busy or restarting and quickly enough that a reloaded page does not
//! sit on its splash. The wait doubles from half a second up to a ceiling, with a little jitter
//! so the display and the phones of one room do not all retry in the same instant.

use std::time::Duration;

const FIRST_WAIT: Duration = Duration::from_millis(500);
const CEILING: Duration = Duration::from_secs(10);
/// Retries of one join that the page makes on its own before it leaves the choice to the
/// player. The join is idempotent, so repeating it is safe.
pub(crate) const JOIN_RETRIES: u32 = 3;

/// The wait before retry number `attempt` (0 for the first). `jitter_permille` is 0 to 1000 and
/// moves the wait by up to 20 percent either way; larger values are treated as 1000.
pub(crate) fn delay(attempt: u32, jitter_permille: u32) -> Duration {
    let doubled = FIRST_WAIT
        .checked_mul(1_u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX))
        .unwrap_or(CEILING);
    let base = doubled.min(CEILING);
    let factor = 800 + u64::from(jitter_permille.min(1_000)) * 400 / 1_000;
    Duration::from_millis(u64::try_from(base.as_millis()).unwrap_or(u64::MAX) * factor / 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_and_then_holds_at_the_ceiling() {
        let waits: Vec<u64> = (0..8)
            .map(|attempt| delay(attempt, 500).as_millis() as u64)
            .collect();
        assert_eq!(waits[..5], [500, 1_000, 2_000, 4_000, 8_000]);
        assert!(waits[5..].iter().all(|wait| *wait == 10_000), "{waits:?}");
    }

    #[test]
    fn a_join_is_repeated_a_few_times_on_its_own() {
        assert!((1..=5).contains(&JOIN_RETRIES));
    }

    #[test]
    fn jitter_stays_within_a_fifth_either_way_and_a_huge_attempt_is_safe() {
        assert_eq!(delay(2, 0), Duration::from_millis(1_600));
        assert_eq!(delay(2, 1_000), Duration::from_millis(2_400));
        assert_eq!(delay(2, 9_999), Duration::from_millis(2_400));
        assert_eq!(delay(u32::MAX, 500), Duration::from_secs(10));
    }
}
