//! The names of the phone's bar and control drawings. A drawing the shared `overlay::Glyph` set
//! has (the dice, the book, the pencil) is that one; the rest are line drawings of this crate.
//! Every one is built by `dom::line_svg` in the current text colour.

use serde::{Deserialize, Serialize};

use crate::overlay::Glyph;

/// A drawing for a tab or a control of the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneGlyph {
    Story,
    Act,
    Talk,
    Roll,
    Bag,
    Clues,
    Map,
    Hero,
    Notebook,
    More,
    Close,
    Warning,
    Wifi,
    Inbox,
}

impl PhoneGlyph {
    /// The inside of the glyph's 24 by 24 `svg`, drawn with strokes.
    pub fn markup(self) -> &'static str {
        match self {
            Self::Story => Glyph::Book.markup(),
            Self::Act => r#"<path d="M13 2L4.5 13.5h6L9 22l9-12h-6z"/>"#,
            Self::Talk => r#"<path d="M4 5h16v11H11l-4.5 4v-4H4z"/><path d="M8 9.5h8M8 12.5h5"/>"#,
            Self::Roll => Glyph::Dice.markup(),
            Self::Bag => {
                concat!(
                    r#"<path d="M7 8V6a5 5 0 0 1 10 0v2"/>"#,
                    r#"<path d="M5 8h14l1 12H4z"/>"#,
                    r#"<path d="M9 13h6"/>"#,
                )
            }
            Self::Clues => {
                concat!(
                    r#"<circle cx="10.5" cy="10.5" r="6.5"/>"#,
                    r#"<path d="M15.5 15.5L21 21"/>"#,
                    r#"<path d="M8 10.5h5"/>"#,
                )
            }
            Self::Map => {
                concat!(
                    r#"<path d="M3 6l6-2.5 6 2.5 6-2.5v14.5l-6 2.5-6-2.5-6 2.5z"/>"#,
                    r#"<path d="M9 3.5v14.5M15 6v14.5"/>"#,
                )
            }
            Self::Hero => {
                concat!(
                    r#"<circle cx="12" cy="8" r="4.2"/>"#,
                    r#"<path d="M4 21c0-4.8 3.4-7.5 8-7.5s8 2.7 8 7.5z"/>"#,
                )
            }
            Self::Notebook => Glyph::Pencil.markup(),
            Self::More => {
                concat!(
                    r#"<circle cx="5.5" cy="12" r="1.4"/>"#,
                    r#"<circle cx="12" cy="12" r="1.4"/>"#,
                    r#"<circle cx="18.5" cy="12" r="1.4"/>"#,
                )
            }
            Self::Close => r#"<path d="M5.5 5.5l13 13M18.5 5.5l-13 13"/>"#,
            Self::Warning => r#"<path d="M12 3l10 18H2z"/><path d="M12 10v5M12 18v.5"/>"#,
            Self::Wifi => {
                concat!(
                    r#"<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0"/>"#,
                    r#"<path d="M8.8 16a5 5 0 0 1 6.4 0"/>"#,
                    r#"<circle cx="12" cy="19.5" r="1"/>"#,
                )
            }
            Self::Inbox => {
                r#"<path d="M3 13l3-8h12l3 8v6H3z"/><path d="M3 13h5l1.5 2.5h5L16 13h5"/>"#
            }
        }
    }
}
