//! The italic scene caption bar (component 9): a band of ink glass above the party strip with
//! hairline gold rules top and bottom and an italic centred line (renders 07, 22, 26 to 35). It
//! draws the roleplay card's caption; on a close-up the title card takes its place (the rule of
//! `closeup::scene_closeup`).

use serde::{Deserialize, Serialize};

use crate::kit::shown;
use crate::lobby::model::RoleplayScene;

/// The caption bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptionBar {
    pub text: String,
}

impl CaptionBar {
    /// The bar of a roleplay card, when its caption has words.
    pub fn from_roleplay(roleplay: &RoleplayScene) -> Option<Self> {
        shown(roleplay.caption.as_deref()).map(|text| Self {
            text: text.to_owned(),
        })
    }

    /// The line; a bar without one is not drawn.
    pub fn text(&self) -> Option<&str> {
        shown(Some(&self.text))
    }
}
