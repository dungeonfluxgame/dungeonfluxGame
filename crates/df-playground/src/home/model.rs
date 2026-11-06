//! The data of the landing page (the Home screen) of the shared screen: one optional object on the
//! scene that holds one optional field for each component, so a scene without a `home` is read as
//! before and a scene with one draws only the components it names.
//!
//! These are presentation values only. Nothing here decides a rule or works a figure out: what a
//! balance is, which campaign was played last and what a launch costs is what the server projected,
//! and the page draws it. A price, a balance or a rate is drawn only when the scene carries one.
//! Each component has its own pure data type, its own bounds (`home::checks`), its own drawing
//! (`view::home`) and its own live verbs (`home::change`): the brand mark, the title block, the
//! navigation rail and the action bar are in `home::chrome` and `home::nav`; the settings panel,
//! the adventure card and the campaign strip are in `home::configure`, `home::adventure` and
//! `home::campaigns`.

use serde::{Deserialize, Serialize};

use crate::home::adventure::AdventureData;
use crate::home::campaigns::CampaignsData;
use crate::home::chrome::{BrandMark, HomeTopBar, TitleBlock};
use crate::home::configure::ConfigureData;
use crate::home::nav::{ActionBar, NavRail};
use crate::limits::MAXIMUM_FOCUS_CHARACTERS;

// Very large caps. Each only keeps memory bounded by design; the owner's rule is that no limit
// is invented small.

/// The landing page. Every component is optional and is drawn only when present.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HomeScene {
    /// The logo mark and wordmark at the top left.
    pub brand: Option<BrandMark>,
    /// The bell, beside the balance and profile pills of the scene's own `pills`.
    pub top: Option<HomeTopBar>,
    /// The title, its tagline and the diamond rule.
    pub title: Option<TitleBlock>,
    /// The navigation rail at the left.
    pub nav: Option<NavRail>,
    /// The row of actions under the title.
    pub actions: Option<ActionBar>,
    /// The settings panel: presets and the six setting cards.
    pub configure: Option<ConfigureData>,
    /// Tonight's adventure at the right.
    pub adventure: Option<AdventureData>,
    /// The strip of recent campaigns at the bottom.
    pub campaigns: Option<CampaignsData>,
    /// The one element the remote is on, as `<part>:<n>` or `<part>:<name>` (`actions:1` is the
    /// first button, `nav:2` the second item, `adventure:launch` the launch button). Without one
    /// the focus is on the first primary action (Continue Campaign).
    pub focus: Option<String>,
}

/// The parts of the landing page, by the name the dev CLI uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomePart {
    Brand,
    Top,
    Title,
    Nav,
    Actions,
    Configure,
    Adventure,
    Campaigns,
}

impl HomePart {
    /// Every part, in the order the page draws them.
    pub const ALL: [Self; 8] = [
        Self::Brand,
        Self::Top,
        Self::Title,
        Self::Nav,
        Self::Actions,
        Self::Configure,
        Self::Adventure,
        Self::Campaigns,
    ];

    /// The name the dev CLI and the audit line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Brand => "brand",
            Self::Top => "top",
            Self::Title => "title",
            Self::Nav => "nav",
            Self::Actions => "actions",
            Self::Configure => "configure",
            Self::Adventure => "adventure",
            Self::Campaigns => "campaigns",
        }
    }

    /// The part of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|part| part.key() == key)
    }

    /// The names of every part, comma separated.
    pub fn keys_text() -> String {
        Self::ALL.map(Self::key).join(", ")
    }
}

/// Whether `id` is a focus id: lower case letters, digits, colons, dashes and underscores, so it
/// can be looked up on the page without any quoting.
pub fn is_focus_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAXIMUM_FOCUS_CHARACTERS
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, ':' | '-' | '_'))
}

impl HomeScene {
    /// The id of the element that has focus: the scene's own, or else the first primary action.
    pub fn focus_id(&self) -> Option<String> {
        if let Some(id) = &self.focus {
            return Some(id.clone());
        }
        let bar = self.actions.as_ref()?;
        let index = bar
            .buttons
            .iter()
            .position(|button| button.variant == crate::home::nav::ActionVariant::Primary)?;
        Some(format!("actions:{}", index + 1))
    }

    /// Takes one part away.
    pub fn remove(&mut self, part: HomePart) {
        match part {
            HomePart::Brand => self.brand = None,
            HomePart::Top => self.top = None,
            HomePart::Title => self.title = None,
            HomePart::Nav => self.nav = None,
            HomePart::Actions => self.actions = None,
            HomePart::Configure => self.configure = None,
            HomePart::Adventure => self.adventure = None,
            HomePart::Campaigns => self.campaigns = None,
        }
    }

    /// The parts that are drawn now.
    pub fn present(&self) -> Vec<HomePart> {
        HomePart::ALL
            .into_iter()
            .filter(|part| match part {
                HomePart::Brand => self.brand.is_some(),
                HomePart::Top => self.top.is_some(),
                HomePart::Title => self.title.is_some(),
                HomePart::Nav => self.nav.is_some(),
                HomePart::Actions => self.actions.is_some(),
                HomePart::Configure => self.configure.is_some(),
                HomePart::Adventure => self.adventure.is_some(),
                HomePart::Campaigns => self.campaigns.is_some(),
            })
            .collect()
    }
}
