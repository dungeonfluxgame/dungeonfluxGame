//! The map place label (component 20): a route marker on a map plate, a parchment capsule with ink
//! text, a dark glass chip for a night plate, or the current destination with a gold glow
//! (renders 17, 20). It is the look of a `CharacterMarker`; markers keep following the camera.

use serde::{Deserialize, Serialize};

use crate::home::glyph::HomeIcon;
use crate::image_layout::{CharacterMarker, ImageLayout, marker_place};
use crate::kit::{Fill, shown};

/// The most a permille place can be: the far edge of the screen.
pub const PERMILLE_AT_MOST: u16 = 1000;

/// How the label looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceLook {
    #[default]
    Parchment,
    /// Dark glass with a gold rim, for a night plate.
    Chip,
    /// The party's destination, glowing gold.
    Current,
}

impl PlaceLook {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Parchment => "parchment",
            Self::Chip => "chip",
            Self::Current => "current",
        }
    }
}

/// One label, placed by its centre in thousandths of the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceLabel {
    pub text: String,
    #[serde(default)]
    pub glyph: Option<HomeIcon>,
    #[serde(default)]
    pub look: PlaceLook,
    pub x_permille: u16,
    pub y_permille: u16,
}

impl PlaceLabel {
    /// The label of `marker`, placed where the layout puts it; `None` when the layout has no
    /// character of its name (a marker without a character is not drawn).
    pub fn from_marker(marker: &CharacterMarker, layout: Option<&ImageLayout>) -> Option<Self> {
        let (x_permille, y_permille) = marker_place(marker, layout)?;
        Some(Self {
            text: marker.text.clone(),
            glyph: marker.glyph.map(HomeIcon::Overlay),
            look: PlaceLook::Parchment,
            x_permille,
            y_permille,
        })
    }

    pub fn text(&self) -> Option<&str> {
        shown(Some(&self.text))
    }

    /// Only the dark chip is glass that blurs; parchment is paper.
    pub fn fill(&self) -> Fill {
        match self.look {
            PlaceLook::Chip => Fill::Blurred,
            PlaceLook::Parchment | PlaceLook::Current => Fill::Solid,
        }
    }

    /// The centre held on the screen, in tenths of a percent.
    pub fn held_permille(&self) -> (u16, u16) {
        (
            self.x_permille.min(PERMILLE_AT_MOST),
            self.y_permille.min(PERMILLE_AT_MOST),
        )
    }
}
