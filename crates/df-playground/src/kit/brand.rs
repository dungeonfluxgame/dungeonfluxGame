//! The DungeonFlux brand mark (component 1): the compass star and the wordmark, inline at the top
//! left (renders 02, 04, 12, 13) or stacked as a crest (02, 38), with the place name under it.
//!
//! It is the one surface `view::brand` (the scene's logo) and `view::pills` (the top bar's logo)
//! would both call (inventory F6), so the mark is drawn once.

use serde::{Deserialize, Serialize};

use crate::home::chrome::DEFAULT_WORDMARK;
use crate::kit::shown;
use crate::lobby::model::LogoStyle;

/// What the brand mark shows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrandMark {
    /// Inline when left out.
    pub style: Option<LogoStyle>,
    /// "DungeonFlux" when left out.
    pub wordmark: Option<String>,
    /// The place name drawn under the mark.
    pub place: Option<String>,
}

impl BrandMark {
    /// The mark a scene asks for: its `brand` flag or the top bar's logo, with the scene's place.
    /// `None` when the scene draws no logo.
    pub fn for_scene(brand: bool, logo: Option<LogoStyle>, place: &str) -> Option<Self> {
        let style = logo.or(brand.then_some(LogoStyle::Inline))?;
        Some(Self {
            style: Some(style),
            wordmark: None,
            place: shown(Some(place)).map(str::to_owned),
        })
    }

    pub fn style(&self) -> LogoStyle {
        self.style.unwrap_or(LogoStyle::Inline)
    }

    pub fn wordmark(&self) -> &str {
        shown(self.wordmark.as_deref()).unwrap_or(DEFAULT_WORDMARK)
    }

    pub fn place(&self) -> Option<&str> {
        shown(self.place.as_deref())
    }
}
