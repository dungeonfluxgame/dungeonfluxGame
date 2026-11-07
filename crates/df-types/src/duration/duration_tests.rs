use super::*;

#[test]
fn spans_drop_sub_millisecond_time_and_saturate_at_the_largest_count() {
    assert_eq!(saturating_millis(Duration::ZERO), 0);
    assert_eq!(saturating_millis(Duration::from_micros(1999)), 1);
    assert_eq!(saturating_millis(Duration::from_secs(3)), 3000);
    assert_eq!(saturating_millis(Duration::MAX), u64::MAX);
}
