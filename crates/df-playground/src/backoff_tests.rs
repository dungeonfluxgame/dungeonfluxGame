use crate::backoff::retry_delay_ms;
use crate::limits::{FIRST_RETRY_MS, LONGEST_RETRY_MS};

#[test]
fn the_wait_starts_short_and_doubles_with_each_failure() {
    assert_eq!(retry_delay_ms(0), FIRST_RETRY_MS);
    assert_eq!(retry_delay_ms(1), 1_000);
    assert_eq!(retry_delay_ms(2), 2_000);
    assert_eq!(retry_delay_ms(3), 4_000);
    assert_eq!(retry_delay_ms(4), 8_000);
}

#[test]
fn the_wait_stops_growing_at_the_ceiling_however_many_failures() {
    assert_eq!(retry_delay_ms(5), LONGEST_RETRY_MS);
    assert_eq!(retry_delay_ms(50), LONGEST_RETRY_MS);
    assert_eq!(retry_delay_ms(u32::MAX), LONGEST_RETRY_MS);
}

#[test]
fn the_wait_never_shrinks_as_failures_pile_up() {
    let mut last = 0;
    for failures in 0..40 {
        let delay = retry_delay_ms(failures);
        assert!(delay >= last, "{failures}");
        last = delay;
    }
}
