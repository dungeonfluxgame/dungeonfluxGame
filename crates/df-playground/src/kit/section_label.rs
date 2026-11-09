//! The section label (component 15): a vertical gold bar beside a title and a sub-line at the
//! lower left, with an optional gold icon, on the vignette only (renders 17, 20, 36). The journal
//! and chapter cards use it.

use serde::{Deserialize, Serialize};

use crate::home::glyph::HomeIcon;
use crate::kit::shown;

/// The label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionLabel {
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub icon: Option<HomeIcon>,
}

impl SectionLabel {
    /// The title; a label without one is not drawn.
    pub fn title(&self) -> Option<&str> {
        shown(Some(&self.title))
    }

    pub fn subtitle(&self) -> Option<&str> {
        shown(self.subtitle.as_deref())
    }
}
