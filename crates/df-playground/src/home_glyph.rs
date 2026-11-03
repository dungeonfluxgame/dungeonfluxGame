//! The small line drawings of the landing page, and the name a scene file or the dev CLI gives one.
//!
//! The overlay set (`overlay::Glyph`: a sword, a book, a crown and so on) stays as it is. This set
//! adds the drawings the landing page needs (navigation, actions, settings, facts) in the same
//! style: the inside of a 24 by 24 `svg` made of strokes, drawn in the current text colour.
//! `HomeIcon` is either kind, so every component of the landing page takes one name type and the
//! page draws it from fixed strings of this crate, never from text of a scene file.

use serde::{Deserialize, Serialize};

use crate::overlay::Glyph;

/// The drawings this page adds to the overlay set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeGlyph {
    Home,
    Campaigns,
    Characters,
    Players,
    Assets,
    Settings,
    Play,
    Sparkle,
    Bolt,
    Globe,
    Masks,
    Bell,
    Info,
    Clock,
    ChevronDown,
    ArrowRight,
    Swords,
    Target,
    Coins,
    Mountain,
    Trees,
    Skull,
    Hourglass,
}

impl HomeGlyph {
    /// Every drawing of this set, in the order it is listed to the author.
    pub const ALL: [Self; 23] = [
        Self::Home,
        Self::Campaigns,
        Self::Characters,
        Self::Players,
        Self::Assets,
        Self::Settings,
        Self::Play,
        Self::Sparkle,
        Self::Bolt,
        Self::Globe,
        Self::Masks,
        Self::Bell,
        Self::Info,
        Self::Clock,
        Self::ChevronDown,
        Self::ArrowRight,
        Self::Swords,
        Self::Target,
        Self::Coins,
        Self::Mountain,
        Self::Trees,
        Self::Skull,
        Self::Hourglass,
    ];

    /// The name a scene file and the command line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Campaigns => "campaigns",
            Self::Characters => "characters",
            Self::Players => "players",
            Self::Assets => "assets",
            Self::Settings => "settings",
            Self::Play => "play",
            Self::Sparkle => "sparkle",
            Self::Bolt => "bolt",
            Self::Globe => "globe",
            Self::Masks => "masks",
            Self::Bell => "bell",
            Self::Info => "info",
            Self::Clock => "clock",
            Self::ChevronDown => "chevron-down",
            Self::ArrowRight => "arrow-right",
            Self::Swords => "swords",
            Self::Target => "target",
            Self::Coins => "coins",
            Self::Mountain => "mountain",
            Self::Trees => "trees",
            Self::Skull => "skull",
            Self::Hourglass => "hourglass",
        }
    }

    /// The drawing of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|glyph| glyph.key() == key)
    }

    /// The inside of the drawing's 24 by 24 `svg`. Fixed strings of this crate.
    pub fn markup(self) -> &'static str {
        match self {
            Self::Home => {
                r#"<path d="M3 11.5l9-8.5 9 8.5"/><path d="M5.5 10v10.5H10v-6h4v6h4.5V10"/>"#
            }
            Self::Campaigns => {
                r#"<path d="M12 6c-2-1.5-5-2-9-2v14c4 0 7 .5 9 2 2-1.5 5-2 9-2V4c-4 0-7 .5-9 2z"/><path d="M12 6v14M5.5 8.5c1.6 0 3 .2 4.2.7M5.5 12c1.6 0 3 .2 4.2.7M14.3 9.2c1.2-.5 2.600-.7 4.200-.7M14.3 12.7c1.2-.5 2.600-.7 4.200-.7"/>"#
            }
            Self::Characters => {
                r#"<path d="M12 3c-3.500 0-6 2.600-6 6v4h12V9c0-3.400-2.500-6-6-6z"/><path d="M6 13v3c0 2 2 3 6 5 4-2 6-3 6-5v-3"/><path d="M9 10h6M12 10v5"/>"#
            }
            Self::Players => {
                r#"<circle cx="9" cy="8" r="3.200"/><path d="M3 20c0-3.800 2.700-6 6-6s6 2.200 6 6z"/><circle cx="17.500" cy="9" r="2.600"/><path d="M16.500 14.200c.3-.1.700-.2 1-.2 2.800 0 4.500 1.800 4.500 5.500h-5"/>"#
            }
            Self::Assets => {
                r#"<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="10" r="2"/><path d="M3 18l5-5 4 4 3-3 6 6"/>"#
            }
            Self::Settings => {
                r#"<circle cx="12" cy="12" r="3.200"/><path d="M12 2.500l1.600 2.600 3-.6.800 3 2.800 1.400-1 2.900 1 2.900-2.800 1.400-.8 3-3-.6L12 21.500l-1.600-2.600-3 .6-.8-3-2.800-1.400 1-2.900-1-2.900 2.800-1.400.8-3 3 .6z"/>"#
            }
            Self::Play => r#"<path d="M6 3.500l14 8.500-14 8.500z"/>"#,
            Self::Sparkle => {
                r#"<path d="M12 2v6M12 16v6M2 12h6M16 12h6M5 5l3.500 3.500M15.500 15.500L19 19M19 5l-3.500 3.500M8.500 15.500L5 19"/><circle cx="12" cy="12" r="2"/>"#
            }
            Self::Bolt => r#"<path d="M13.500 2L5 13.500h6L10 22l9-12h-6z"/>"#,
            Self::Globe => {
                r#"<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18"/>"#
            }
            Self::Masks => {
                r#"<path d="M3 5c3 1.500 6 1.500 9 0v6c0 3.500-2 6-4.500 6S3 14.500 3 11z"/><path d="M12 9c3 1.500 6 1.500 9 0v6c0 3.500-2 6-4.500 6-2 0-3.500-1.500-4.200-3.500"/><path d="M5.500 9.500c.8.600 1.700.6 2.500 0M15 12.500c.8.600 1.700.6 2.500 0M6 13c1 .8 2 .8 3 0"/>"#
            }
            Self::Bell => {
                r#"<path d="M6 17v-6a6 6 0 0 1 12 0v6l2 2H4z"/><path d="M10 21a2 2 0 0 0 4 0"/>"#
            }
            Self::Info => r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.500v.5"/>"#,
            Self::Clock => r#"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.500 2"/>"#,
            Self::ChevronDown => r#"<path d="M5 9l7 7 7-7"/>"#,
            Self::ArrowRight => r#"<path d="M4 12h16M14 6l6 6-6 6"/>"#,
            Self::Swords => {
                r#"<path d="M4 4l9 9M20 4l-9 9"/><path d="M3 16l5 5M21 16l-5 5M8 13l-4 4M16 13l4 4"/>"#
            }
            Self::Target => {
                r#"<circle cx="12" cy="12" r="3"/><circle cx="12" cy="12" r="8"/><path d="M12 1.500v3M12 19.500v3M1.500 12h3M19.500 12h3"/>"#
            }
            Self::Coins => {
                r#"<ellipse cx="12" cy="6" rx="7" ry="3"/><path d="M5 6v6c0 1.700 3.100 3 7 3s7-1.300 7-3V6"/><path d="M5 12v6c0 1.700 3.100 3 7 3s7-1.300 7-3v-6"/>"#
            }
            Self::Mountain => r#"<path d="M2 20l7-13 4 7 3-4 6 10z"/>"#,
            Self::Trees => r#"<path d="M12 3l5 8h-3l4 6H6l4-6H7z"/><path d="M12 17v4"/>"#,
            Self::Skull => {
                r#"<path d="M12 3a8 8 0 0 0-5 14v3h10v-3a8 8 0 0 0-5-14z"/><circle cx="9" cy="12" r="1.600"/><circle cx="15" cy="12" r="1.600"/><path d="M12 15l-1 2h2z"/>"#
            }
            Self::Hourglass => {
                r#"<path d="M6 3h12M6 21h12M7 3c0 5 5 6 5 9s-5 4-5 9M17 3c0 5-5 6-5 9s5 4 5 9"/>"#
            }
        }
    }
}

/// A drawing of either set, by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeIcon {
    /// A drawing this page adds.
    Page(HomeGlyph),
    /// A drawing of the overlay set.
    Overlay(Glyph),
}

impl HomeIcon {
    /// The name a scene file and the command line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Page(glyph) => glyph.key(),
            Self::Overlay(glyph) => glyph.key(),
        }
    }

    /// The drawing of that name, from this page's set first, then the overlay's.
    pub fn from_key(key: &str) -> Option<Self> {
        HomeGlyph::from_key(key)
            .map(Self::Page)
            .or_else(|| Glyph::from_key(key).map(Self::Overlay))
    }

    /// The names of every drawing, comma separated, for a message that lists the choices.
    pub fn keys_text() -> String {
        HomeGlyph::ALL
            .iter()
            .map(|glyph| glyph.key())
            .chain(Glyph::ALL.iter().map(|glyph| glyph.key()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Serialize for HomeIcon {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for HomeIcon {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        Self::from_key(&key).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "'{key}' is not an icon of the landing page; the icons are: {}",
                Self::keys_text()
            ))
        })
    }
}
