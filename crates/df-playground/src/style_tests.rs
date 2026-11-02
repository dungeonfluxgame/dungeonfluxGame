use crate::style::css_color;

#[test]
fn a_short_or_long_hex_colour_is_let_through_unchanged() {
    assert_eq!(css_color("#4f8cff"), Some("#4f8cff"));
    assert_eq!(css_color("#FFF"), Some("#FFF"));
}

#[test]
fn anything_that_could_carry_style_is_dropped() {
    assert_eq!(css_color("red"), None);
    assert_eq!(css_color("#12"), None);
    assert_eq!(css_color("#1234567"), None);
    assert_eq!(css_color("#ggg"), None);
    assert_eq!(css_color("#fff;background:url(x)"), None);
    assert_eq!(css_color("#fff\"><script>"), None);
    assert_eq!(css_color(""), None);
}
