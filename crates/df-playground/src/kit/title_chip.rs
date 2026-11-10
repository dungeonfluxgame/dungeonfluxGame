//! The episode or quest title chip (component 2): a night-glass capsule at the top right with an
//! icon, the title, an optional sub-line or inline parts, and an optional dot track (renders 02,
//! 03, 04, 12, 13).
//!
//! The top bar's pills (session, party, balance, rate) are chips of one line, so `top_bar_chips`
//! turns a `TopBar` into chips and the pills keep their data and their words.

use serde::{Deserialize, Serialize};

use crate::home::glyph::{HomeGlyph, HomeIcon};
use crate::kit::{Step, shown, steps};
use crate::lobby::model::{TopBar, party_text, session_text};
use crate::overlay::{Glyph, Progress, StoryMark};

/// One chip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleChip {
    /// The chip's drawing; the kit's outlined compass in a ring when left out (render 02).
    #[serde(default)]
    pub icon: Option<HomeIcon>,
    pub title: String,
    /// A second line under the title, such as "Episode 1 — A Fragile Alliance".
    #[serde(default)]
    pub subtitle: Option<String>,
    /// Parts on the title's line after a bullet each, worded by the host ("Scene 3 of 10").
    #[serde(default)]
    pub parts: Vec<String>,
    /// The dots under the line, one per scene.
    #[serde(default)]
    pub track: Option<Progress>,
}

impl TitleChip {
    fn line(icon: HomeIcon, title: String) -> Self {
        Self {
            icon: Some(icon),
            title,
            subtitle: None,
            parts: Vec::new(),
            track: None,
        }
    }

    /// The title; a chip without one is not drawn.
    pub fn title(&self) -> Option<&str> {
        shown(Some(&self.title))
    }

    pub fn subtitle(&self) -> Option<&str> {
        shown(self.subtitle.as_deref())
    }

    /// The inline parts that have words.
    pub fn parts(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter_map(|part| shown(Some(part)))
            .collect()
    }

    /// The track's dots; none without a track of at least two scenes.
    pub fn track(&self) -> Vec<Step> {
        self.track.as_ref().map(steps).unwrap_or_default()
    }
}

impl From<&StoryMark> for TitleChip {
    /// The story's title alone, under the kit's compass; where the party is stands in the progress
    /// line.
    fn from(mark: &StoryMark) -> Self {
        Self {
            icon: None,
            title: mark.title.clone(),
            subtitle: None,
            parts: Vec::new(),
            track: None,
        }
    }
}

/// The top bar's pills as chips, in the bar's order: session, party, balance, rate.
pub fn top_bar_chips(bar: &TopBar) -> Vec<TitleChip> {
    let mut chips = Vec::new();
    if let Some(session) = shown(bar.session.as_deref()) {
        chips.push(TitleChip::line(
            HomeIcon::Overlay(Glyph::Book),
            session_text(session),
        ));
    }
    if let Some(count) = bar.party {
        chips.push(TitleChip::line(
            HomeIcon::Page(HomeGlyph::Players),
            party_text(count),
        ));
    }
    if let Some(balance) = shown(bar.balance.as_deref()) {
        chips.push(TitleChip::line(
            HomeIcon::Page(HomeGlyph::Coins),
            balance.to_owned(),
        ));
    }
    if let Some(rate) = shown(bar.rate.as_deref()) {
        chips.push(TitleChip::line(
            HomeIcon::Page(HomeGlyph::Clock),
            rate.to_owned(),
        ));
    }
    chips
}
