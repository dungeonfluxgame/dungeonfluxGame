//! The landing page's navigation rail and action bar: the pure data of two components and the
//! rule that keeps one item active.

use serde::{Deserialize, Serialize};

use crate::home::glyph::HomeIcon;

fn default_true() -> bool {
    true
}

/// One entry of the navigation rail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavItem {
    pub label: String,
    pub glyph: HomeIcon,
    /// Whether the page the item leads to is the one shown. At most one item is active.
    #[serde(default)]
    pub active: bool,
    /// A small count or word at the item's right, such as "2".
    #[serde(default)]
    pub badge: Option<String>,
}

/// The navigation rail: Home, Campaigns, Characters and so on, each with its drawing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NavRail {
    pub items: Vec<NavItem>,
}

impl NavRail {
    /// Makes the item at `index` the only active one.
    pub fn activate(&mut self, index: usize) {
        for (position, item) in self.items.iter_mut().enumerate() {
            item.active = position == index;
        }
    }

    /// How many items are active.
    pub fn active_count(&self) -> usize {
        self.items.iter().filter(|item| item.active).count()
    }
}

/// How an action button looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionVariant {
    /// The gold button: the one thing the page most wants done.
    Primary,
    /// A glass button.
    #[default]
    Secondary,
}

impl ActionVariant {
    pub const ALL: [Self; 2] = [Self::Primary, Self::Secondary];

    /// The name the dev CLI uses.
    pub fn key(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
        }
    }

    /// The variant of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|variant| variant.key() == key)
    }
}

/// One button of the action bar. Whether it may be pressed is the server's word; the shared
/// screen takes no input, so a button is a picture of what the phones do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionButton {
    pub label: String,
    /// A line under the label, such as the campaign's name.
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub glyph: Option<HomeIcon>,
    #[serde(default)]
    pub variant: ActionVariant,
    /// A button that cannot be used now is drawn dimmed and says so.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// The row of actions under the title.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActionBar {
    pub buttons: Vec<ActionButton>,
}
