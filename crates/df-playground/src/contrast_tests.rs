//! The theme's text pairs reach WCAG AA, read from the tokens in `theme.css` itself.

use crate::contrast::{MINIMUM_TEXT_CONTRAST, contrast_ratio, parse_hex, relative_luminance};

const THEME: &str = include_str!("../../../assets/playground/theme.css");

/// The colour a `--ui-` token is set to in the theme.
fn token(name: &str) -> [u8; 3] {
    let key = format!("--{name}:");
    let start = THEME
        .find(&key)
        .unwrap_or_else(|| panic!("the theme has no {name}"))
        + key.len();
    let value = THEME[start..]
        .split(';')
        .next()
        .expect("a token ends")
        .trim();
    parse_hex(value).unwrap_or_else(|| panic!("{name} is {value}, not a plain colour"))
}

#[test]
fn black_on_white_is_twenty_one_and_a_colour_on_itself_is_one() {
    let (black, white) = ([0, 0, 0], [255, 255, 255]);
    assert!((contrast_ratio(black, white) - 21.0).abs() < 0.001);
    assert!((contrast_ratio(white, black) - 21.0).abs() < 0.001);
    assert!((contrast_ratio(white, white) - 1.0).abs() < 0.001);
    assert!(relative_luminance(black) < relative_luminance(white));
}

#[test]
fn hex_colours_of_three_and_six_digits_are_read_and_anything_else_is_not() {
    assert_eq!(parse_hex("#fff"), Some([255, 255, 255]));
    assert_eq!(parse_hex("#09131d"), Some([9, 19, 29]));
    assert_eq!(parse_hex("#09131dff"), None);
    assert_eq!(parse_hex("09131d"), None);
    assert_eq!(parse_hex("#12g456"), None);
    assert_eq!(parse_hex("#12"), None);
}

#[test]
fn every_text_colour_reaches_aa_on_the_glass_it_is_drawn_on() {
    let pairs = [
        ("ui-text-strong", "ui-glass-warm"),
        ("ui-text-strong", "ui-glass-warm-hi"),
        ("ui-text-strong", "ui-glass-navy"),
        ("ui-text-strong", "ui-glass-navy-hi"),
        ("ui-text", "ui-glass-warm"),
        ("ui-text", "ui-glass-navy-hi"),
        ("ui-muted-cool", "ui-glass-navy"),
        ("ui-muted-cool", "ui-glass-navy-hi"),
        ("ui-muted-warm", "ui-glass-warm"),
        ("ui-muted-warm", "ui-glass-warm-hi"),
        ("ui-brass", "ui-glass-warm"),
        ("ui-brass-hi", "ui-glass-warm"),
        ("ui-brass-hi", "ui-glass-navy"),
        ("ui-ready", "ui-ready-fill"),
        ("ui-creating-text", "ui-creating-fill"),
        ("ui-open", "ui-open-fill"),
        ("ui-money", "ui-glass-navy"),
        ("ui-hp-high", "ui-glass-warm"),
        ("ui-warn", "ui-glass-warm"),
        ("ui-danger", "ui-glass-warm"),
        ("ui-info", "ui-glass-warm"),
        ("ui-rarity-common", "ui-glass-warm"),
        ("ui-rarity-uncommon", "ui-glass-warm"),
        ("ui-rarity-rare", "ui-glass-warm"),
        ("ui-rarity-legendary", "ui-glass-warm"),
        ("ui-tone-hero", "ui-glass-warm"),
        ("ui-tone-ally", "ui-glass-warm"),
        ("ui-tone-mystic", "ui-glass-warm"),
        ("ui-tone-npc", "ui-glass-warm"),
        ("ui-tone-narrator", "ui-glass-warm"),
        ("ui-tone-danger", "ui-glass-warm"),
    ];
    for (text, ground) in pairs {
        let ratio = contrast_ratio(token(text), token(ground));
        assert!(
            ratio >= MINIMUM_TEXT_CONTRAST,
            "{text} on {ground} is {ratio:.2}, under {MINIMUM_TEXT_CONTRAST}"
        );
    }
}
