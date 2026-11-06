//! The live edits of the landing page's Tonight's Adventure card, and why one is refused.
//!
//! Every field of the card can be changed from the dev CLI: the heading, the picture, the title,
//! the blurb, the launch button's words and whether it is enabled, and each fact row's drawing,
//! label and value, with rows added, removed and cleared. A field left out of a change stays as it
//! is; a text set to nothing is not drawn. An edit is applied to a copy and the whole result is
//! checked with the rules a scene file is checked with, so a refusal leaves the card exactly as it
//! was. The audit line of an edit carries numbers, flags and the field's name, never free text.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{AdventureData, DEFAULT_FACT_ICON, FactRow};
use crate::home::glyph::HomeIcon;
use crate::limits::MAXIMUM_FACT_ROWS;
use crate::overlay::Edit;

/// Why a value or a change of the adventure card was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdventureError {
    /// A text is longer than its very large limit.
    TooLong { what: &'static str, limit: usize },
    /// The picture is not the plain name of an image.
    InvalidPicture(String),
    /// The card holds more fact rows than its very large limit.
    TooManyFacts,
    /// A fact row is numbered 0 or past the last one; `count` is how many there are.
    InvalidFactNumber { number: usize, count: usize },
    /// A change names no field to change.
    NothingToChange,
}

impl fmt::Display for AdventureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { what, limit } => {
                write!(formatter, "{what} is longer than {limit} characters")
            }
            Self::InvalidPicture(name) => write!(
                formatter,
                "'{name}' is not the plain name of a picture (png, jpg, jpeg, webp or gif) of the media library"
            ),
            Self::TooManyFacts => write!(
                formatter,
                "the adventure card holds at most {MAXIMUM_FACT_ROWS} fact rows"
            ),
            Self::InvalidFactNumber { number, count } => write!(
                formatter,
                "there is no fact row {number}: the card has {count} (rows are numbered from 1)"
            ),
            Self::NothingToChange => formatter.write_str(
                "name at least one field to change: heading=, picture=, title=, blurb=, launch=, enabled=",
            ),
        }
    }
}

impl std::error::Error for AdventureError {}

/// One live edit of the adventure card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdventureEdit {
    /// Takes the whole card away.
    Clear {},
    /// Changes the card's own fields, making the card (with the default heading and launch words)
    /// when there is none. A field left out stays as it is.
    Set {
        heading: Option<String>,
        #[serde(default)]
        picture: Edit<String>,
        title: Option<String>,
        blurb: Option<String>,
        launch_label: Option<String>,
        enabled: Option<bool>,
    },
    /// Changes fact row `number` (counted from 1). A part left out stays as it is.
    FactSet {
        number: usize,
        icon: Option<HomeIcon>,
        label: Option<String>,
        value: Option<String>,
    },
    /// Adds a fact row at the end.
    FactAdd {
        icon: Option<HomeIcon>,
        label: String,
        value: String,
    },
    /// Takes fact row `number` away.
    FactRemove { number: usize },
    /// Takes every fact row away.
    FactsClear {},
}

fn fact_index(number: usize, count: usize) -> Result<usize, AdventureError> {
    number
        .checked_sub(1)
        .filter(|index| *index < count)
        .ok_or(AdventureError::InvalidFactNumber { number, count })
}

impl AdventureEdit {
    /// Makes the change in `slot`: all of it, or none when the result is out of range.
    pub fn apply(self, slot: &mut Option<AdventureData>) -> Result<(), AdventureError> {
        if matches!(self, Self::Clear {}) {
            *slot = None;
            return Ok(());
        }
        let mut card = slot.clone().unwrap_or_default();
        match self {
            Self::Clear {} => {}
            Self::Set {
                heading,
                picture,
                title,
                blurb,
                launch_label,
                enabled,
            } => {
                if heading.is_none()
                    && picture == Edit::Keep
                    && title.is_none()
                    && blurb.is_none()
                    && launch_label.is_none()
                    && enabled.is_none()
                {
                    return Err(AdventureError::NothingToChange);
                }
                if let Some(heading) = heading {
                    card.heading = heading;
                }
                picture.apply_to(&mut card.picture);
                if let Some(title) = title {
                    card.title = title;
                }
                if let Some(blurb) = blurb {
                    card.blurb = blurb;
                }
                if let Some(launch_label) = launch_label {
                    card.launch_label = launch_label;
                }
                if let Some(enabled) = enabled {
                    card.enabled = enabled;
                }
            }
            Self::FactSet {
                number,
                icon,
                label,
                value,
            } => {
                let index = fact_index(number, card.facts.len())?;
                let row = &mut card.facts[index];
                if icon.is_none() && label.is_none() && value.is_none() {
                    return Err(AdventureError::NothingToChange);
                }
                if let Some(icon) = icon {
                    row.icon = icon;
                }
                if let Some(label) = label {
                    row.label = label;
                }
                if let Some(value) = value {
                    row.value = value;
                }
            }
            Self::FactAdd { icon, label, value } => card.facts.push(FactRow {
                icon: icon.unwrap_or(DEFAULT_FACT_ICON),
                label,
                value,
            }),
            Self::FactRemove { number } => {
                let index = fact_index(number, card.facts.len())?;
                card.facts.remove(index);
            }
            Self::FactsClear {} => card.facts.clear(),
        }
        card.check()?;
        *slot = Some(card);
        Ok(())
    }

    /// The field and the audit pairs of the change: counts, flags and names of fields.
    pub fn audit_fields(&self) -> (&'static str, Vec<(&'static str, String)>) {
        let length = |text: &String| text.chars().count().to_string();
        match self {
            Self::Clear {} => ("clear", Vec::new()),
            Self::Set {
                heading,
                picture,
                title,
                blurb,
                launch_label,
                enabled,
            } => {
                let mut pairs = Vec::new();
                if let Some(heading) = heading {
                    pairs.push(("heading_chars", length(heading)));
                }
                match picture {
                    Edit::Keep => {}
                    Edit::Clear => pairs.push(("picture", "clear".to_owned())),
                    Edit::Set(_) => pairs.push(("picture", "set".to_owned())),
                }
                if let Some(title) = title {
                    pairs.push(("title_chars", length(title)));
                }
                if let Some(blurb) = blurb {
                    pairs.push(("blurb_chars", length(blurb)));
                }
                if let Some(launch_label) = launch_label {
                    pairs.push(("launch_chars", length(launch_label)));
                }
                if let Some(enabled) = enabled {
                    pairs.push(("enabled", enabled.to_string()));
                }
                ("set", pairs)
            }
            Self::FactSet {
                number,
                icon,
                label,
                value,
            } => {
                let mut pairs = vec![("fact", number.to_string())];
                if let Some(icon) = icon {
                    pairs.push(("icon", icon.key().to_owned()));
                }
                if let Some(label) = label {
                    pairs.push(("label_chars", length(label)));
                }
                if let Some(value) = value {
                    pairs.push(("value_chars", length(value)));
                }
                ("fact", pairs)
            }
            Self::FactAdd { icon, label, value } => (
                "fact-add",
                vec![
                    ("icon", icon.unwrap_or(DEFAULT_FACT_ICON).key().to_owned()),
                    ("label_chars", length(label)),
                    ("value_chars", length(value)),
                ],
            ),
            Self::FactRemove { number } => ("fact-remove", vec![("fact", number.to_string())]),
            Self::FactsClear {} => ("facts-clear", Vec::new()),
        }
    }

    /// The media names the change shows.
    pub fn media_names(&self) -> Vec<&str> {
        match self {
            Self::Set {
                picture: Edit::Set(name),
                ..
            } => vec![name.as_str()],
            _ => Vec::new(),
        }
    }
}
