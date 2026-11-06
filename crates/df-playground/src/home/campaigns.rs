//! The data of the landing page's Recent Campaigns strip (render 19, along the bottom): a header
//! with a title and a see-all button, and a row of campaign cards, each an art thumbnail, a title
//! and a last-played line, one of which can be selected.
//!
//! The strip holds presentation values only. Which campaigns were played, and when, is what the
//! server projected: the last-played line is a text the server worded ("Last played: Yesterday"),
//! never a time the page works out. Every field is changed live through `CampaignsEdit`
//! (`home::campaigns::changes`), which `home::change` calls through this contract:
//!
//! - `CampaignsData`: the component's data, read through `CampaignsRecord` (defaults,
//!   `deny_unknown_fields`), so a value out of range is refused where it is read.
//! - `CampaignsData::check(&self)`: every bound of the data (the reader and each edit call it).
//! - `CampaignsEdit::apply(self, slot)`, `audit_fields()` and `media_names()`: see the changes
//!   file.
//!
//! The limits are very large and only keep memory bounded by design; they are not a design rule.

use serde::{Deserialize, Serialize};

use crate::limits::{MAXIMUM_CAMPAIGN_CARDS, MAXIMUM_NAME_CHARACTERS};
use crate::media::{MediaKind, media_kind};

mod changes;

pub use self::changes::{CampaignsEdit, CampaignsError};

/// The heading of a strip that is made by a live change and given none.
pub const DEFAULT_STRIP_TITLE: &str = "Recent Campaigns";
/// The words of the see-all button, on a strip that is made by a live change and given none. The
/// television layout has no room to draw the button; the words stay in the data.
pub const DEFAULT_VIEW_ALL: &str = "See all campaigns";
/// How many campaign cards the strip draws: a TV density rule (fewer, bigger, readable from a
/// sofa), not a limit on the data. The cards after these stay in the data.
pub const CARDS_SHOWN: usize = 3;

/// The focus id of campaign card `number`, counted from 1 (`campaigns:2`).
pub fn card_focus_id(number: usize) -> String {
    format!("campaigns:{number}")
}

/// One campaign card.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CampaignCard {
    /// The campaign's art, a picture in the media library, or none for a neutral plate.
    pub art: Option<String>,
    pub title: String,
    /// The line under the title, worded by the server ("Last played: Yesterday"); empty draws none.
    pub last_played: String,
    /// Whether this is the campaign that is picked. At most one card is.
    pub selected: bool,
}

/// The data of the landing page's Recent Campaigns strip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CampaignsRecord")]
pub struct CampaignsData {
    /// The strip's heading; empty draws none.
    pub title: String,
    /// The words of the see-all button at the header's right; empty draws no button.
    pub view_all: String,
    pub cards: Vec<CampaignCard>,
}

/// `CampaignsData` as a scene file writes it, before its bounds are checked: serde reads the
/// recent-campaigns strip through this, so a value out of range is refused where it is read.
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CampaignsRecord {
    title: String,
    view_all: String,
    cards: Vec<CampaignCard>,
}

impl Default for CampaignsRecord {
    fn default() -> Self {
        let CampaignsData {
            title,
            view_all,
            cards,
        } = CampaignsData::default();
        Self {
            title,
            view_all,
            cards,
        }
    }
}

impl TryFrom<CampaignsRecord> for CampaignsData {
    type Error = CampaignsError;

    fn try_from(record: CampaignsRecord) -> Result<Self, Self::Error> {
        let CampaignsRecord {
            title,
            view_all,
            cards,
        } = record;
        let data = Self {
            title,
            view_all,
            cards,
        };
        data.check()?;
        Ok(data)
    }
}

impl Default for CampaignsData {
    fn default() -> Self {
        Self {
            title: DEFAULT_STRIP_TITLE.to_owned(),
            view_all: DEFAULT_VIEW_ALL.to_owned(),
            cards: Vec::new(),
        }
    }
}

fn text_fits(what: &'static str, text: &str) -> Result<(), CampaignsError> {
    if text.chars().count() > MAXIMUM_NAME_CHARACTERS {
        Err(CampaignsError::TooLong {
            what,
            limit: MAXIMUM_NAME_CHARACTERS,
        })
    } else {
        Ok(())
    }
}

impl CampaignsData {
    /// Why the data is out of range, or `Ok`.
    pub fn check(&self) -> Result<(), CampaignsError> {
        text_fits("the strip's title", &self.title)?;
        text_fits("the see-all button's words", &self.view_all)?;
        if self.cards.len() > MAXIMUM_CAMPAIGN_CARDS {
            return Err(CampaignsError::TooManyCards);
        }
        for card in &self.cards {
            text_fits("a campaign's title", &card.title)?;
            text_fits("a campaign's last-played line", &card.last_played)?;
            if let Some(art) = &card.art
                && media_kind(art) != Some(MediaKind::Image)
            {
                return Err(CampaignsError::InvalidArt(art.clone()));
            }
        }
        match self.selected_count() {
            0 | 1 => Ok(()),
            count => Err(CampaignsError::ManySelected { count }),
        }
    }

    /// How many cards are selected.
    pub fn selected_count(&self) -> usize {
        self.cards.iter().filter(|card| card.selected).count()
    }

    /// The cards the strip draws: the first `CARDS_SHOWN`.
    pub fn shown_cards(&self) -> &[CampaignCard] {
        &self.cards[..self.cards.len().min(CARDS_SHOWN)]
    }
}
