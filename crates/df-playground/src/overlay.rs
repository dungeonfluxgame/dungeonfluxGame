//! The data of the narration box and the seat strip of the shared screen, read from a fixture.
//!
//! These are presentation values only. Every field of every type is optional in a scene file
//! (`TvScene` holds them with a default), so a scene that names none of them is read as before.
//! Nothing here decides a rule: who is in the party, how healthy they are and what they are
//! called is what the server projected, and the page draws it. `overlay::rules` turns this data
//! into view decisions and `overlay::checks` says which values are out of range. The lobby,
//! loading, roleplay and top-bar data live in `lobby::model`.

use serde::{Deserialize, Serialize};

pub mod changes;
pub mod checks;
pub mod rules;

#[cfg(test)]
mod changes_tests;
#[cfg(test)]
mod rules_tests;

/// The small picture of a thing, drawn by the page from a fixed set of line drawings. A name that
/// is not in the set is a mistake in the scene file, found when it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Glyph {
    Sword,
    Shield,
    Bow,
    Sun,
    Book,
    Scales,
    Mug,
    Mask,
    Hammer,
    Plus,
    Voice,
    Dice,
    Crown,
    Star,
    Check,
    Lock,
    Pencil,
    /// An open ring a page turns while something is being made.
    Spinner,
}

impl Glyph {
    /// Every glyph, in the order of its drawing.
    pub const ALL: [Self; 18] = [
        Self::Sword,
        Self::Shield,
        Self::Bow,
        Self::Sun,
        Self::Book,
        Self::Scales,
        Self::Mug,
        Self::Mask,
        Self::Hammer,
        Self::Plus,
        Self::Voice,
        Self::Dice,
        Self::Crown,
        Self::Star,
        Self::Check,
        Self::Lock,
        Self::Pencil,
        Self::Spinner,
    ];

    /// The name a scene file and the command line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Sword => "sword",
            Self::Shield => "shield",
            Self::Bow => "bow",
            Self::Sun => "sun",
            Self::Book => "book",
            Self::Scales => "scales",
            Self::Mug => "mug",
            Self::Mask => "mask",
            Self::Hammer => "hammer",
            Self::Plus => "plus",
            Self::Voice => "voice",
            Self::Dice => "dice",
            Self::Crown => "crown",
            Self::Star => "star",
            Self::Check => "check",
            Self::Lock => "lock",
            Self::Pencil => "pencil",
            Self::Spinner => "spinner",
        }
    }

    /// The glyph of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|glyph| glyph.key() == key)
    }

    /// The names of every glyph, comma separated, for a message that lists the choices.
    pub fn keys_text() -> String {
        Self::ALL.map(Self::key).join(", ")
    }

    /// The drawing of the glyph: the inside of a 24 by 24 `svg` made of strokes, drawn in the
    /// current colour. These are fixed strings of this crate, never text from a scene file.
    pub fn markup(self) -> &'static str {
        match self {
            Self::Sword => {
                r#"<path d="M20.5 3.5L10.5 13.5"/><path d="M7 11l6 6"/><path d="M9 15l-4 4"/><circle cx="4.5" cy="19.5" r="1"/>"#
            }
            Self::Shield => {
                r#"<path d="M12 3l7 3v5c0 5-3 8-7 10-4-2-7-5-7-10V6z"/><path d="M12 3v18"/>"#
            }
            Self::Bow => {
                r#"<path d="M6 3c8 2 8 16 0 18"/><path d="M6 3v18"/><path d="M6 12h14M17 9l3 3-3 3"/>"#
            }
            Self::Sun => {
                r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2"/>"#
            }
            Self::Book => {
                r#"<path d="M12 6c-2-1.5-5-2-8-2v14c3 0 6 .5 8 2 2-1.5 5-2 8-2V4c-3 0-6 .5-8 2z"/><path d="M12 6v14"/>"#
            }
            Self::Scales => {
                r#"<path d="M12 4v16M7 20h10M5 7h14"/><path d="M5 7l-3 7h6zM19 7l-3 7h6z"/>"#
            }
            Self::Mug => {
                r#"<path d="M5 6h10v12a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2z"/><path d="M15 9h2a2 2 0 0 1 0 5h-2M8 3v1M12 3v1"/>"#
            }
            Self::Mask => {
                r#"<path d="M4 5c5 2 11 2 16 0v7c0 4-4 7-8 7s-8-3-8-7z"/><path d="M8 11c1 1 2 1 3 0M13 11c1 1 2 1 3 0M9 15c2 1.5 4 1.5 6 0"/>"#
            }
            Self::Hammer => r#"<path d="M14 3l7 7-3 3-7-7z"/><path d="M13 11L4 20"/>"#,
            Self::Plus => r#"<path d="M12 5v14M5 12h14"/>"#,
            Self::Voice => r#"<path d="M3 12h0M6 9v6M9 6v12M12 3v18M15 7v10M18 9v6M21 12h0"/>"#,
            Self::Dice => {
                r#"<path d="M12 2l9 5v10l-9 5-9-5V7z"/><path d="M12 2l-4 9h8zM8 11l-5-4M16 11l5-4M8 11l4 11M16 11l-4 11"/>"#
            }
            Self::Crown => r#"<path d="M3 8l4 4 5-7 5 7 4-4-2 11H5z"/>"#,
            Self::Star => {
                r#"<path d="M12 2l2.5 7.5L22 12l-7.5 2.5L12 22l-2.5-7.5L2 12l7.5-2.5z"/>"#
            }
            Self::Check => r#"<path d="M4 12l5 5L20 6"/>"#,
            Self::Lock => {
                r#"<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>"#
            }
            Self::Pencil => r#"<path d="M4 20l1-5L16 4l4 4L9 19z"/><path d="M14 6l4 4"/>"#,
            Self::Spinner => r#"<path d="M12 3a9 9 0 1 0 9 9"/>"#,
        }
    }
}

/// What a live change does to one optional field: leave it as it is, remove it, or set it.
///
/// This is the only keep/clear/set type of a live change. `apply_to` holds the one rule of what
/// each variant does to a field, and `apply_to_value` and `drawn` are built on it; `set_value` and
/// `is_keep` only ask which variant it is. A plain `Option<T>` in a change message is for a field
/// that is only given or not (such as `hit_points`, `title` or `voice`), never for keep/clear/set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edit<T> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

impl<T> Edit<T> {
    /// The value this edit sets, if it sets one.
    pub fn set_value(&self) -> Option<&T> {
        match self {
            Self::Set(value) => Some(value),
            Self::Keep | Self::Clear => None,
        }
    }

    /// Whether the edit leaves the field as it is.
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

impl<T: Clone> Edit<T> {
    /// Applies the edit to `slot`: the one rule of keep, clear and set.
    pub fn apply_to(&self, slot: &mut Option<T>) {
        match self {
            Self::Keep => {}
            Self::Clear => *slot = None,
            Self::Set(value) => *slot = Some(value.clone()),
        }
    }

    /// Applies the edit to a field that is always present: `Clear` leaves its empty value.
    pub fn apply_to_value(&self, slot: &mut T)
    where
        T: Default,
    {
        let mut value = Some(std::mem::take(slot));
        self.apply_to(&mut value);
        *slot = value.unwrap_or_default();
    }

    /// What is shown now when this edit is drawn over `base`.
    pub fn drawn(&self, base: Option<&T>) -> Option<T> {
        let mut value = base.cloned();
        self.apply_to(&mut value);
        value
    }
}

/// Where a scene is in its run, for a line such as "Scene 2 of 5 - Introduction".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    /// The scene's number, counted from 1.
    pub index: u16,
    pub total: u16,
    pub title: Option<String>,
}

/// Where the narration box sits across the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrationPlacement {
    /// In the middle of the screen.
    #[default]
    Center,
    /// Against the right edge, leaving the left for a figure in the plate (render 13).
    Right,
}

impl NarrationPlacement {
    pub const ALL: [Self; 2] = [Self::Center, Self::Right];

    pub fn key(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Right => "right",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|placement| placement.key() == key)
    }
}

/// The narrator's box: it takes the place of the caption in the caption layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrationBox {
    /// Who is speaking; the page says "Dungeon Master" when this is left out.
    #[serde(default)]
    pub speaker: Option<String>,
    pub text: String,
    /// The word beside the continue mark, or `None` to draw no mark. The mark is a hint only: the
    /// shared screen takes no input and the dwell rules decide how long a line stays up.
    #[serde(default)]
    pub continue_label: Option<String>,
    /// The round medallion at the box's left (a die, for the Dungeon Master), or none.
    #[serde(default)]
    pub badge: Option<Glyph>,
    /// Whether the line is being spoken aloud, which draws the voice mark.
    #[serde(default = "default_true")]
    pub voice: bool,
    #[serde(default)]
    pub progress: Option<Progress>,
    #[serde(default)]
    pub placement: NarrationPlacement,
}

fn default_true() -> bool {
    true
}

/// How one hero of the party strip looks. The hit points and conditions are the party member's own;
/// this adds the picture and the class line, matched to the member by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartyLook {
    pub name: String,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub level: Option<u8>,
    /// A picture in the media library.
    #[serde(default)]
    pub portrait: Option<String>,
    #[serde(default)]
    pub glyph: Option<Glyph>,
    /// A `#rrggbb` colour for the card's edge and bar.
    #[serde(default)]
    pub accent: Option<String>,
}

/// The looks of the heroes in the party strip and how many open seats follow them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SeatStrip {
    pub looks: Vec<PartyLook>,
    pub open: u8,
}
