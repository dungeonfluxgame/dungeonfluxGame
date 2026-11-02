//! Checks for values that reach an inline style.
//!
//! A fixture is data an author edits, and a colour from it ends up inside a `style` attribute, so
//! only a plain `#rgb` or `#rrggbb` colour is allowed through; anything else is dropped and the
//! caller uses its default. It never draws.

/// The colour text if it is `#` followed by three or six hexadecimal digits, otherwise `None`.
pub fn css_color(text: &str) -> Option<&str> {
    let digits = text.strip_prefix('#')?;
    let valid_length = digits.len() == 3 || digits.len() == 6;
    (valid_length && digits.chars().all(|digit| digit.is_ascii_hexdigit())).then_some(text)
}
