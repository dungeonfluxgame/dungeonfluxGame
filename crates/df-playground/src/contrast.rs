//! Colour contrast by the WCAG 2.x rule, so the theme's text pairs can be checked by a test.
//!
//! It reads `#rgb` and `#rrggbb` colours only (the forms the theme's tokens use) and answers a
//! ratio from 1 to 21. It never draws.

/// The red, green and blue of `#rgb` or `#rrggbb` (an alpha of eight digits is refused), or `None`.
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    if !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |index: usize, width: usize| -> Option<u8> {
        let part = digits.get(index * width..(index + 1) * width)?;
        let value = u8::from_str_radix(part, 16).ok()?;
        Some(if width == 1 { value * 17 } else { value })
    };
    match digits.len() {
        3 => Some([channel(0, 1)?, channel(1, 1)?, channel(2, 1)?]),
        6 => Some([channel(0, 2)?, channel(1, 2)?, channel(2, 2)?]),
        _ => None,
    }
}

/// The relative luminance of a colour, from 0 (black) to 1 (white).
pub fn relative_luminance(color: [u8; 3]) -> f64 {
    let linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

/// The contrast ratio of two colours, from 1 to 21, whichever is lighter.
pub fn contrast_ratio(first: [u8; 3], second: [u8; 3]) -> f64 {
    let (a, b) = (relative_luminance(first), relative_luminance(second));
    let (light, dark) = if a >= b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

/// The least ratio for normal text (WCAG AA).
pub const MINIMUM_TEXT_CONTRAST: f64 = 4.5;
