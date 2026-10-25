use super::*;

#[test]
fn a_token_value_is_removed_up_to_the_next_delimiter() {
    assert_eq!(
        scrub("at /app/join#token=0a1b2c&next=1"),
        "at /app/join#[removed]&next=1"
    );
    assert_eq!(scrub("a Token=XYZ b"), "a [removed] b");
}

#[test]
fn a_percent_encoded_marker_is_removed_as_well() {
    assert_eq!(scrub("q=token%3Dabc"), "q=[removed]");
}

#[test]
fn text_without_a_marker_is_kept_and_bounded() {
    assert_eq!(scrub("plain message"), "plain message");
    assert_eq!(
        scrub(&"x".repeat(500)).chars().count(),
        MAXIMUM_DETAIL_CHARS
    );
}

#[test]
fn several_markers_in_one_message_are_all_removed() {
    let cleaned = scrub("token=a token=b token=c");
    assert!(!cleaned.contains("token="), "{cleaned}");
}

#[test]
fn lines_are_area_verb_key_value() {
    assert_eq!(
        line(Event::RequestFailed {
            method: Method::Put,
            status: 503
        }),
        "ACCOUNT_APP request_failed method=PUT status=503"
    );
    assert_eq!(
        line(Event::Booted { route: "dashboard" }),
        "ACCOUNT_APP booted route=dashboard"
    );
}
