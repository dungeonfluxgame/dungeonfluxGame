//! The live edits of the landing page's Recent Campaigns strip, and why one is refused.
//!
//! Every field of the strip can be changed from the dev CLI: its title and view-all words, and each
//! card's art, title, last-played line and selection, with cards added, removed, cleared and
//! picked. Picking a card un-picks the others, because at most one is picked. A field left out of a
//! change stays as it is; a text set to nothing is not drawn. An edit is applied to a copy and the
//! whole result is checked with the rules a scene file is checked with, so a refusal leaves the
//! strip as it was. The audit line carries numbers, flags and the field's name, never free text.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{CampaignCard, CampaignsData};
use crate::limits::MAXIMUM_CAMPAIGN_CARDS;
use crate::overlay::Edit;

/// Why a value or a change of the campaign strip was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignsError {
    /// A text is longer than its very large limit.
    TooLong { what: &'static str, limit: usize },
    /// The art is not the plain name of an image.
    InvalidArt(String),
    /// The strip holds more cards than its very large limit.
    TooManyCards,
    /// A card is numbered 0 or past the last one; `count` is how many there are.
    InvalidCardNumber { number: usize, count: usize },
    /// More than one card is selected.
    ManySelected { count: usize },
    /// A change names no field to change.
    NothingToChange,
}

impl fmt::Display for CampaignsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { what, limit } => {
                write!(formatter, "{what} is longer than {limit} characters")
            }
            Self::InvalidArt(name) => write!(
                formatter,
                "'{name}' is not the plain name of a picture (png, jpg, jpeg, webp or gif) of the media library"
            ),
            Self::TooManyCards => write!(
                formatter,
                "the campaign strip holds at most {MAXIMUM_CAMPAIGN_CARDS} cards"
            ),
            Self::InvalidCardNumber { number, count } => write!(
                formatter,
                "there is no campaign card {number}: the strip has {count} (cards are numbered from 1)"
            ),
            Self::ManySelected { count } => {
                write!(
                    formatter,
                    "{count} campaign cards are selected: at most one is"
                )
            }
            Self::NothingToChange => formatter.write_str(
                "name at least one field to change: title=, view-all=, art=, last=, selected=",
            ),
        }
    }
}

impl std::error::Error for CampaignsError {}

/// One live edit of the campaign strip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum CampaignsEdit {
    /// Takes the whole strip away.
    Clear {},
    /// Changes the strip's header, making the strip (with the default title and link) when there is
    /// none. A field left out stays as it is.
    Set {
        title: Option<String>,
        view_all: Option<String>,
    },
    /// Adds a card at the end; selecting it un-selects the others.
    CardAdd {
        art: Option<String>,
        title: String,
        last_played: Option<String>,
        selected: Option<bool>,
    },
    /// Changes card `number` (counted from 1). A part left out stays as it is.
    CardSet {
        number: usize,
        #[serde(default)]
        art: Edit<String>,
        title: Option<String>,
        last_played: Option<String>,
        selected: Option<bool>,
    },
    /// Takes card `number` away.
    CardRemove { number: usize },
    /// Takes every card away.
    CardsClear {},
    /// Selects card `number` and un-selects the others, or un-selects every card when `number` is
    /// left out.
    Select { number: Option<usize> },
}

fn card_index(number: usize, count: usize) -> Result<usize, CampaignsError> {
    number
        .checked_sub(1)
        .filter(|index| *index < count)
        .ok_or(CampaignsError::InvalidCardNumber { number, count })
}

/// Selects card `index` alone, or none.
fn select_only(cards: &mut [CampaignCard], index: Option<usize>) {
    for (position, card) in cards.iter_mut().enumerate() {
        card.selected = Some(position) == index;
    }
}

impl CampaignsEdit {
    /// Makes the change in `slot`: all of it, or none when the result is out of range.
    pub fn apply(self, slot: &mut Option<CampaignsData>) -> Result<(), CampaignsError> {
        if matches!(self, Self::Clear {}) {
            *slot = None;
            return Ok(());
        }
        let mut strip = slot.clone().unwrap_or_default();
        match self {
            Self::Clear {} => {}
            Self::Set { title, view_all } => {
                if title.is_none() && view_all.is_none() {
                    return Err(CampaignsError::NothingToChange);
                }
                if let Some(title) = title {
                    strip.title = title;
                }
                if let Some(view_all) = view_all {
                    strip.view_all = view_all;
                }
            }
            Self::CardAdd {
                art,
                title,
                last_played,
                selected,
            } => {
                let selected = selected.unwrap_or(false);
                strip.cards.push(CampaignCard {
                    art,
                    title,
                    last_played: last_played.unwrap_or_default(),
                    selected,
                });
                if selected {
                    let last = strip.cards.len() - 1;
                    select_only(&mut strip.cards, Some(last));
                }
            }
            Self::CardSet {
                number,
                art,
                title,
                last_played,
                selected,
            } => {
                let index = card_index(number, strip.cards.len())?;
                if art == Edit::Keep
                    && title.is_none()
                    && last_played.is_none()
                    && selected.is_none()
                {
                    return Err(CampaignsError::NothingToChange);
                }
                let card = &mut strip.cards[index];
                art.apply_to(&mut card.art);
                if let Some(title) = title {
                    card.title = title;
                }
                if let Some(last_played) = last_played {
                    card.last_played = last_played;
                }
                match selected {
                    Some(true) => select_only(&mut strip.cards, Some(index)),
                    Some(false) => strip.cards[index].selected = false,
                    None => {}
                }
            }
            Self::CardRemove { number } => {
                let index = card_index(number, strip.cards.len())?;
                strip.cards.remove(index);
            }
            Self::CardsClear {} => strip.cards.clear(),
            Self::Select { number } => {
                let index = match number {
                    Some(number) => Some(card_index(number, strip.cards.len())?),
                    None => None,
                };
                select_only(&mut strip.cards, index);
            }
        }
        strip.check()?;
        *slot = Some(strip);
        Ok(())
    }

    /// The field and the audit pairs of the change: counts, flags and names of fields.
    pub fn audit_fields(&self) -> (&'static str, Vec<(&'static str, String)>) {
        let length = |text: &String| text.chars().count().to_string();
        match self {
            Self::Clear {} => ("clear", Vec::new()),
            Self::Set { title, view_all } => {
                let mut pairs = Vec::new();
                if let Some(title) = title {
                    pairs.push(("title_chars", length(title)));
                }
                if let Some(view_all) = view_all {
                    pairs.push(("view_all_chars", length(view_all)));
                }
                ("set", pairs)
            }
            Self::CardAdd {
                art,
                title,
                last_played,
                selected,
            } => {
                let mut pairs = vec![
                    ("art", if art.is_some() { "set" } else { "none" }.to_owned()),
                    ("title_chars", length(title)),
                ];
                if let Some(last_played) = last_played {
                    pairs.push(("last_played_chars", length(last_played)));
                }
                pairs.push(("selected", selected.unwrap_or(false).to_string()));
                ("card-add", pairs)
            }
            Self::CardSet {
                number,
                art,
                title,
                last_played,
                selected,
            } => {
                let mut pairs = vec![("card", number.to_string())];
                match art {
                    Edit::Keep => {}
                    Edit::Clear => pairs.push(("art", "clear".to_owned())),
                    Edit::Set(_) => pairs.push(("art", "set".to_owned())),
                }
                if let Some(title) = title {
                    pairs.push(("title_chars", length(title)));
                }
                if let Some(last_played) = last_played {
                    pairs.push(("last_played_chars", length(last_played)));
                }
                if let Some(selected) = selected {
                    pairs.push(("selected", selected.to_string()));
                }
                ("card", pairs)
            }
            Self::CardRemove { number } => ("card-remove", vec![("card", number.to_string())]),
            Self::CardsClear {} => ("cards-clear", Vec::new()),
            Self::Select { number } => (
                "select",
                vec![(
                    "card",
                    number.map_or_else(|| "none".to_owned(), |number| number.to_string()),
                )],
            ),
        }
    }

    /// The media names the change shows.
    pub fn media_names(&self) -> Vec<&str> {
        match self {
            Self::CardAdd {
                art: Some(name), ..
            }
            | Self::CardSet {
                art: Edit::Set(name),
                ..
            } => vec![name.as_str()],
            _ => Vec::new(),
        }
    }
}
