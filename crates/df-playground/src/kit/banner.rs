//! The scene-type header banner (component 10) and, in its danger tone, the encounter banner
//! (component 16): one widget. The header sits under the brand mark with a chamfered left end and
//! a feathered right (renders 22, 26 to 35); the encounter banner is a crimson hexagon at the top
//! centre (render 03).

use serde::{Deserialize, Serialize};

use crate::home::glyph::HomeIcon;
use crate::kit::{PanelTone, shown};
use crate::lobby::model::SceneTitle;
use crate::overlay::Glyph;

/// Where the banner sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BannerPlace {
    /// Under the brand mark at the top left.
    Header,
    /// At the top centre, as the combat banner.
    TopCenter,
}

impl BannerPlace {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::TopCenter => "top_center",
        }
    }
}

/// One banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Banner {
    #[serde(default)]
    pub icon: Option<HomeIcon>,
    /// The host's numbering of the scene, drawn before the title ("3."); never read from the text.
    #[serde(default)]
    pub number: Option<u16>,
    pub title: String,
    #[serde(default)]
    pub tone: PanelTone,
    /// A gold banner is a header and a danger banner sits at the top centre when left out.
    #[serde(default)]
    pub placement: Option<BannerPlace>,
}

impl From<&SceneTitle> for Banner {
    fn from(title: &SceneTitle) -> Self {
        Self {
            icon: Some(HomeIcon::Overlay(title.glyph)),
            number: None,
            title: title.text.clone(),
            tone: PanelTone::Gold,
            placement: Some(BannerPlace::Header),
        }
    }
}

impl Banner {
    pub fn title(&self) -> Option<&str> {
        shown(Some(&self.title))
    }

    pub fn placement(&self) -> BannerPlace {
        self.placement.unwrap_or(match self.tone {
            PanelTone::Gold => BannerPlace::Header,
            PanelTone::Danger => BannerPlace::TopCenter,
        })
    }

    pub fn number_text(&self) -> Option<String> {
        self.number.map(|number| format!("{number}."))
    }

    /// Whether the icon is the turning ring of something being made.
    pub fn is_working(&self) -> bool {
        self.icon == Some(HomeIcon::Overlay(Glyph::Spinner))
    }
}
