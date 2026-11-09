//! The title card family: the clue or item close-up card (component 13) and, at a larger scale,
//! the chapter or prologue card (component 19). A compass-rose ornament over a short rule, the
//! title in gold gradient text between flank rules, an optional rarity line tight under it, and
//! an italic line, centred on the lower third or in a right column (renders 01, 07, 17, 20, 38).
//!
//! The lobby's loading title (`lb-*`) has the same look; both should call this one builder.

use serde::{Deserialize, Serialize};

use crate::closeup::CloseUp;
use crate::home::glyph::HomeIcon;
use crate::kit::{Fill, Rarity, shown};

/// Where the card sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardAlign {
    #[default]
    Center,
    /// A right column, vertically centred, star rules above and below.
    Right,
}

impl CardAlign {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// How large the card is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardScale {
    /// An item or a clue under inspection.
    #[default]
    Item,
    /// A chapter opening: a letter-spaced kicker and a large italic title.
    Chapter,
}

impl CardScale {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Chapter => "chapter",
        }
    }
}

/// One title card.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TitleCard {
    /// A small letter-spaced line above the title ("Entering", "Prologue").
    pub kicker: Option<String>,
    pub title: Option<String>,
    /// The italic line under the title.
    pub text: Option<String>,
    pub rarity: Option<Rarity>,
    pub align: CardAlign,
    pub scale: CardScale,
    /// The drawing over the title; the eight-point compass rose when left out.
    pub ornament: Option<HomeIcon>,
}

impl From<&CloseUp> for TitleCard {
    fn from(closeup: &CloseUp) -> Self {
        Self {
            title: closeup.title.clone(),
            text: Some(closeup.description.clone()),
            ..Self::default()
        }
    }
}

impl TitleCard {
    pub fn kicker(&self) -> Option<&str> {
        shown(self.kicker.as_deref())
    }

    pub fn title(&self) -> Option<&str> {
        shown(self.title.as_deref())
    }

    pub fn text(&self) -> Option<&str> {
        shown(self.text.as_deref())
    }

    /// An item card blurs the plate under it; a chapter card's ground is a vignette only, since a
    /// blur that large would repaint the whole screen.
    pub fn fill(&self) -> Fill {
        match self.scale {
            CardScale::Item => Fill::Blurred,
            CardScale::Chapter => Fill::Solid,
        }
    }

    /// A card with neither a title nor a line is not drawn.
    pub fn is_drawn(&self) -> bool {
        self.title().is_some() || self.text().is_some()
    }
}
