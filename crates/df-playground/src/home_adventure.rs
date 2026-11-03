//! The data of the landing page's Tonight's Adventure card (render 19, the glass card at the right):
//! a heading, a picture of the adventure's scene art, the adventure's title, a blurb, a list of fact
//! rows (a line drawing, a label and a value each) and the launch button.
//!
//! The card holds presentation values only. Nothing here decides whether a session may start: the
//! `enabled` flag is what the server projected, and the page draws it. Every field is changed live
//! through `AdventureEdit` (`home_adventure_changes`), which `home_change` calls through this
//! contract:
//!
//! - `AdventureData`: the component's data, serde with defaults, `deny_unknown_fields`.
//! - `AdventureData::check(&self)`: every bound of the data (the fixture reader and each edit call it).
//! - `AdventureEdit::apply(self, slot)`, `audit_fields()` and `media_names()`: see the changes file.
//!
//! The limits are very large and only keep memory bounded by design; they are not a design rule.
//! A fact row's drawing is a `HomeIcon` (either icon set), drawn from fixed strings of this crate,
//! so a scene file never carries markup.

use serde::{Deserialize, Serialize};

use crate::home_glyph::{HomeGlyph, HomeIcon};
use crate::limits::{MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_NAME_CHARACTERS};
use crate::media::{MediaKind, media_kind};

#[path = "home_adventure_changes.rs"]
mod changes;

pub use self::changes::{AdventureEdit, AdventureError};

/// Most fact rows on the card: a very large number that only bounds memory.
pub const MAXIMUM_FACT_ROWS: usize = 1_000;
/// The heading of a card that is made by a live change and given none.
pub const DEFAULT_HEADING: &str = "Tonight's Adventure";
/// The launch button's words on a card that is made by a live change and given none.
pub const DEFAULT_LAUNCH_LABEL: &str = "Launch Session";
/// How many fact rows the card draws: a TV density rule (fewer, bigger, readable from a sofa), not a
/// limit on the data. The rows after these stay in the data and are not drawn.
pub const FACTS_SHOWN: usize = 3;
/// The focus id of the launch button, the one control of the card (the remote's focus is the page's
/// `focus` field, which names one control at a time).
pub const LAUNCH_FOCUS_ID: &str = "adventure:launch";
/// The drawing of a fact row that is added without one.
pub const DEFAULT_FACT_ICON: HomeIcon = HomeIcon::Page(HomeGlyph::Target);

/// One fact row of the card: a drawing, what it is about and what it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactRow {
    /// The small drawing at the row's left.
    #[serde(default = "default_fact_icon")]
    pub icon: HomeIcon,
    /// What the fact is about, such as "Region"; the page adds the colon.
    pub label: String,
    /// What it is, such as "Veylmar Expanse".
    pub value: String,
}

fn default_fact_icon() -> HomeIcon {
    DEFAULT_FACT_ICON
}

/// The data of the landing page's Tonight's Adventure card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdventureData {
    /// The italic line over the picture; empty draws none.
    pub heading: String,
    /// The adventure's scene art, a picture in the media library, or none for a neutral plate.
    pub picture: Option<String>,
    /// The adventure's title, drawn over the foot of the picture; empty draws none.
    pub title: String,
    /// The italic paragraph under the picture; empty draws none.
    pub blurb: String,
    pub facts: Vec<FactRow>,
    /// The words on the launch button; empty draws no button.
    pub launch_label: String,
    /// Whether the launch button can be used. The server decides; the page only draws it.
    pub enabled: bool,
}

impl Default for AdventureData {
    fn default() -> Self {
        Self {
            heading: DEFAULT_HEADING.to_owned(),
            picture: None,
            title: String::new(),
            blurb: String::new(),
            facts: Vec::new(),
            launch_label: DEFAULT_LAUNCH_LABEL.to_owned(),
            enabled: true,
        }
    }
}

/// Why a text is too long, or `Ok`.
pub(crate) fn text_fits(
    what: &'static str,
    text: &str,
    limit: usize,
) -> Result<(), AdventureError> {
    if text.chars().count() > limit {
        Err(AdventureError::TooLong { what, limit })
    } else {
        Ok(())
    }
}

impl AdventureData {
    /// Why the data is out of range, or `Ok`.
    pub fn check(&self) -> Result<(), AdventureError> {
        text_fits("the heading", &self.heading, MAXIMUM_NAME_CHARACTERS)?;
        text_fits("the title", &self.title, MAXIMUM_NAME_CHARACTERS)?;
        text_fits("the blurb", &self.blurb, MAXIMUM_CAPTION_CHARACTERS)?;
        text_fits(
            "the launch button's words",
            &self.launch_label,
            MAXIMUM_NAME_CHARACTERS,
        )?;
        if let Some(picture) = &self.picture
            && media_kind(picture) != Some(MediaKind::Image)
        {
            return Err(AdventureError::BadPicture(picture.clone()));
        }
        if self.facts.len() > MAXIMUM_FACT_ROWS {
            return Err(AdventureError::TooManyFacts);
        }
        for fact in &self.facts {
            text_fits("a fact's label", &fact.label, MAXIMUM_NAME_CHARACTERS)?;
            text_fits("a fact's value", &fact.value, MAXIMUM_NAME_CHARACTERS)?;
        }
        Ok(())
    }

    /// The fact rows the card draws: the first `FACTS_SHOWN`.
    pub fn shown_facts(&self) -> &[FactRow] {
        &self.facts[..self.facts.len().min(FACTS_SHOWN)]
    }

    /// The media names the card shows.
    pub fn media_names(&self) -> Vec<&str> {
        self.picture.iter().map(String::as_str).collect()
    }
}

#[cfg(test)]
#[path = "home_adventure_tests.rs"]
mod tests;
