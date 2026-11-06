//! The small pictures of the landing page's setting cards: a fixed set of line drawings, drawn in
//! the current colour by the page. The data of a card names one (`SettingCard::icon`); the drawing
//! itself is a constant of this crate, never text from a fixture. The shared glyphs of `overlay`
//! are reused where they fit.

use serde::{Deserialize, Serialize};

use crate::overlay::Glyph;

/// The small pictures of the setting cards: line drawings in the current colour, a fixed set of
/// this crate (the shared glyphs are reused where they fit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigureIcon {
    Globe,
    Masks,
    Party,
    Swords,
    Core,
    Coins,
    Dice,
    Book,
    Sparkles,
    Voice,
}

impl ConfigureIcon {
    /// Every icon, in the order of its drawing.
    pub const ALL: [Self; 10] = [
        Self::Globe,
        Self::Masks,
        Self::Party,
        Self::Swords,
        Self::Core,
        Self::Coins,
        Self::Dice,
        Self::Book,
        Self::Sparkles,
        Self::Voice,
    ];

    /// The name a fixture and the command line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Globe => "globe",
            Self::Masks => "masks",
            Self::Party => "party",
            Self::Swords => "swords",
            Self::Core => "core",
            Self::Coins => "coins",
            Self::Dice => "dice",
            Self::Book => "book",
            Self::Sparkles => "sparkles",
            Self::Voice => "voice",
        }
    }

    /// The icon of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|icon| icon.key() == key)
    }

    /// The names of every icon, comma separated, for a message that lists the choices.
    pub fn keys_text() -> String {
        Self::ALL.map(Self::key).join(", ")
    }

    /// The inside of a 24 by 24 `svg` of strokes. These are fixed strings of this crate, never
    /// text from a fixture.
    pub fn markup(self) -> &'static str {
        match self {
            Self::Globe => {
                r#"<circle cx="12" cy="12" r="9"/><path d="M3 12h18M4.6 7.5h14.8M4.6 16.5h14.8M12 3c3.4 3.2 3.4 14.8 0 18M12 3c-3.4 3.2-3.4 14.8 0 18"/>"#
            }
            Self::Masks => {
                r#"<path d="M2.5 4.5c3.2 1.4 6.3 1.4 9.5 0v6.5c0 3.4-2.1 5.8-4.8 5.8S2.5 14.400 2.500 11z"/><path d="M12 8.500c3.200 1.400 6.300 1.400 9.500 0V15c0 3.400-2.100 5.800-4.800 5.800-1.700 0-3.100-.9-4-2.300"/><path d="M4.800 8.500c.8.7 1.700.7 2.500 0M9 11.800c-.8.900-1.900.900-2.700 0"/><path d="M14.800 12.500c.8.700 1.700.700 2.500 0M16.200 17c1 .6 2 .2 2.700-.6"/>"#
            }
            Self::Party => {
                r#"<circle cx="12" cy="7.500" r="2.800"/><path d="M6.500 20c0-3.600 2.400-5.800 5.500-5.800s5.500 2.200 5.500 5.800z"/><circle cx="4.800" cy="9.500" r="2.100"/><path d="M1 19c0-2.700 1.500-4.500 3.800-4.500"/><circle cx="19.200" cy="9.500" r="2.100"/><path d="M23 19c0-2.700-1.500-4.500-3.800-4.500"/>"#
            }
            Self::Swords => {
                r#"<path d="M4 4l9 9M10.500 15.500l5-5M13 13l6 6"/><circle cx="20" cy="20" r="1"/><path d="M20 4l-9 9M13.500 15.500l-5-5M11 13l-6 6"/><circle cx="4" cy="20" r="1"/>"#
            }
            Self::Core => {
                r#"<circle cx="12" cy="12" r="3"/><circle cx="12" cy="12" r="7.500"/><path d="M12 1.500v3M12 19.500v3M1.500 12h3M19.500 12h3"/>"#
            }
            Self::Coins => {
                r#"<ellipse cx="12" cy="6" rx="7" ry="3"/><path d="M5 6v6c0 1.700 3.100 3 7 3s7-1.300 7-3V6"/><path d="M5 12v6c0 1.700 3.100 3 7 3s7-1.300 7-3v-6"/>"#
            }
            Self::Dice => Glyph::Dice.markup(),
            Self::Book => Glyph::Book.markup(),
            Self::Sparkles => Glyph::Star.markup(),
            Self::Voice => Glyph::Voice.markup(),
        }
    }
}
