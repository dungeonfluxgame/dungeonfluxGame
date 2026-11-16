//! The phone UI kit: every piece of the player's phone in the concept art's look (dark glass,
//! brass hairlines, chamfered corners, a high-contrast serif), beside the TV kit in `kit`.
//!
//! Each piece is a module holding its typed input (serde-ready, sample data today) and the view
//! decisions made from it as plain functions that native tests cover. The drawing is `draw`
//! (browser only), styled by `assets/playground/phone.css`. These are presentation types of the
//! playground, not wire contracts: nothing here decides a rule or names a server call.

use serde::{Deserialize, Serialize};

pub mod act;
#[cfg(debug_assertions)]
pub mod gallery;
pub mod glyph;
pub mod nav;
pub mod page;
pub mod roll;
pub mod shell;
pub mod talk;

#[cfg(target_arch = "wasm32")]
pub mod draw;

#[cfg(test)]
mod act_tests;
#[cfg(all(test, debug_assertions))]
mod gallery_tests;
#[cfg(test)]
mod nav_tests;
#[cfg(test)]
mod page_tests;
#[cfg(test)]
mod roll_tests;
#[cfg(test)]
mod shell_tests;
#[cfg(test)]
mod talk_tests;

use crate::phone_kit::act::{ActPanel, ComposerSample};
use crate::phone_kit::page::Block;
use crate::phone_kit::roll::RollPanel;
use crate::phone_kit::shell::{
    ButtonSpec, EmptyState, HpBar, LoadingState, PhoneHeader, Pill, TabBar, TabKey, Toast,
};
use crate::phone_kit::talk::TalkPanel;

/// Everything one phone screen can show at once. A panel's own screen sets `header`, the panel
/// and `tabs`; the shell board sets the loose pieces.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PhonePanels {
    /// The picture of the media library drawn full-bleed behind the screen: the scene, or the
    /// room or adventure before one.
    pub scene: Option<String>,
    pub header: Option<PhoneHeader>,
    /// Whether the line that says the link to the host is down is up.
    pub link_down: bool,
    pub status: Option<Pill>,
    pub pills: Vec<Pill>,
    pub hp: Vec<HpBar>,
    pub buttons: Vec<ButtonSpec>,
    pub empty: Option<EmptyState>,
    pub loading: Option<LoadingState>,
    pub toast: Option<Toast>,
    pub act: Option<ActPanel>,
    pub talk: Option<TalkPanel>,
    pub roll: Option<RollPanel>,
    /// The blocks of a page, in order, after the panels above.
    pub blocks: Vec<Block>,
    pub composer: Option<ComposerSample>,
    pub tabs: Option<TabBar>,
}

/// A page that follows another after a time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoNext {
    pub page: String,
    pub after_ms: u32,
}

/// One phone screen of a board, with the name of its state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhoneScreen {
    pub title: String,
    /// The page's name in the demo's navigation (`nav::page_action`).
    #[serde(default)]
    pub key: Option<String>,
    /// The panel of the tab bar this page belongs to, which lights its tab.
    #[serde(default)]
    pub tab: Option<TabKey>,
    /// A sample state that gives way to another page by itself, such as a picture being made.
    #[serde(default)]
    pub auto: Option<AutoNext>,
    pub panels: PhonePanels,
}
