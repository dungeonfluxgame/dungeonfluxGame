use super::limits::{DEFAULT_DEADLINE_MS, INTERPRET_DEADLINE_MS, deadline_ms, deadline_ms_or};

#[test]
fn a_deadline_setting_of_whole_seconds_is_read_in_milliseconds() {
    assert_eq!(deadline_ms(Some("90")), 90_000);
}

#[test]
fn a_missing_zero_or_unreadable_deadline_setting_is_the_default() {
    for text in [None, Some("0"), Some("-5"), Some("soon"), Some("1.5")] {
        assert_eq!(deadline_ms(text), DEFAULT_DEADLINE_MS, "{text:?}");
    }
}

#[test]
fn interpret_keeps_its_short_default_so_the_buttons_fallback_still_fires() {
    assert_eq!(INTERPRET_DEADLINE_MS, 15_000);
    assert_eq!(deadline_ms_or(None, INTERPRET_DEADLINE_MS), 15_000);
    assert_eq!(deadline_ms_or(Some("0"), INTERPRET_DEADLINE_MS), 15_000);
    assert_eq!(deadline_ms_or(Some("90"), INTERPRET_DEADLINE_MS), 90_000);
}

#[test]
fn a_huge_deadline_setting_saturates_rather_than_wrapping() {
    assert_eq!(deadline_ms(Some("18446744073709551615")), u64::MAX);
}
