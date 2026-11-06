//! The landing page's brand mark, title block and top bar: the pure data of three components.
//!
//! Each field is optional in a scene file or has a default, and each bound is checked at the edge
//! (`home::checks`). The wordmark is words from the scene; it is drawn as text, never as markup.

use serde::{Deserialize, Serialize};

/// The wordmark when a scene does not give one.
pub const DEFAULT_WORDMARK: &str = "DungeonFlux";

/// How large a brand mark is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrandSize {
    /// The logo at the top left of a screen: the star over the wordmark.
    #[default]
    Compact,
    /// The big wordmark of a title.
    Display,
}

impl BrandSize {
    pub const ALL: [Self; 2] = [Self::Compact, Self::Display];

    /// The name the dev CLI uses.
    pub fn key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Display => "display",
        }
    }

    /// The size of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|size| size.key() == key)
    }
}

fn default_true() -> bool {
    true
}

fn default_wordmark() -> String {
    DEFAULT_WORDMARK.to_owned()
}

/// The logo: a compass star and the wordmark, in one of two sizes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrandMark {
    /// The name under the star.
    pub wordmark: String,
    pub size: BrandSize,
    /// Whether the compass star is drawn.
    pub star: bool,
}

impl Default for BrandMark {
    fn default() -> Self {
        Self {
            wordmark: default_wordmark(),
            size: BrandSize::Compact,
            star: default_true(),
        }
    }
}

/// The title of the page: the big name, a line under it and a diamond rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TitleBlock {
    pub title: String,
    /// The line under the title, such as "Forge a world. Gather your party. Begin the legend."
    pub tagline: Option<String>,
    /// Whether the rule with a diamond in the middle is drawn under the tagline.
    pub rule: bool,
}

impl Default for TitleBlock {
    fn default() -> Self {
        Self {
            title: default_wordmark(),
            tagline: None,
            rule: default_true(),
        }
    }
}

/// The top bar's own part: the bell. The balance, the rate and the profile mark are the scene's
/// `pills` (`lobby::model::TopBar`), which this bar composes at its right.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HomeTopBar {
    /// Whether the bell is drawn.
    pub bell: bool,
    /// How many notices are unread; the bell shows a count when it is present and not zero.
    pub unread: Option<u16>,
}

/// The text of the unread count on the bell: the number, or "99+" past ninety-nine, or `None`.
pub fn unread_text(unread: Option<u16>) -> Option<String> {
    match unread {
        None | Some(0) => None,
        Some(count) if count > 99 => Some("99+".to_owned()),
        Some(count) => Some(count.to_string()),
    }
}

/// The words a screen reader says for the bell.
pub fn bell_label(unread: Option<u16>) -> String {
    match unread {
        None | Some(0) => "Notifications".to_owned(),
        Some(1) => "Notifications, 1 unread".to_owned(),
        Some(count) => format!("Notifications, {count} unread"),
    }
}
