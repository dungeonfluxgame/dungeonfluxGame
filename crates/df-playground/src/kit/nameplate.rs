//! The speaker subtitle or nameplate (component 8): no box, the name in gold between hairline
//! rules with diamond ends and the italic line under it, on the bottom vignette (renders 07, 14,
//! 20). It draws the scene's spoken caption and keeps the caption's contract.

use serde::{Deserialize, Serialize};

use crate::fixture::Caption;
use crate::kit::shown;

/// The nameplate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nameplate {
    #[serde(default)]
    pub speaker: Option<String>,
    pub text: String,
}

impl From<&Caption> for Nameplate {
    fn from(caption: &Caption) -> Self {
        Self {
            speaker: caption.speaker.clone(),
            text: caption.text.clone(),
        }
    }
}

impl Nameplate {
    /// The name between the rules; a line alone draws no rules.
    pub fn speaker(&self) -> Option<&str> {
        shown(self.speaker.as_deref())
    }

    /// The line; a nameplate without one is not drawn.
    pub fn text(&self) -> Option<&str> {
        shown(Some(&self.text))
    }
}
