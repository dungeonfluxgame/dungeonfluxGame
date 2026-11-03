//! Every bound of the landing page's data, checked at the edge: when a fixture is read and again
//! on the result of each live change, so the command line, the overlay and a scene file refuse a
//! value in the same words. A check never changes a value and never draws.
//!
//! The typed reasons are `HomeError`. The settings panel, the adventure card and the campaign strip
//! bring their own error types (`home_configure`, `home_adventure`, `home_campaigns`), which
//! `HomeError` wraps, so each component says what is wrong in its own words.

use std::fmt;

use crate::checks;
use crate::home_adventure::AdventureError;
use crate::home_campaigns::CampaignsError;
use crate::home_chrome::{BrandMark, TitleBlock};
use crate::home_configure::ConfigureError;
use crate::home_model::{
    HomeScene, MAXIMUM_ACTION_BUTTONS, MAXIMUM_BADGE_CHARACTERS, MAXIMUM_NAV_ITEMS,
};
use crate::home_nav::{ActionBar, NavRail};
use crate::limits::MAXIMUM_CAPTION_CHARACTERS;
use crate::scene::SceneError;

/// Why a value or a change of the landing page was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeError {
    /// An item is numbered past the end: the next free number is `next`.
    Number {
        what: &'static str,
        number: usize,
        next: usize,
    },
    /// A list holds more than its very large limit.
    TooMany { what: &'static str, limit: usize },
    /// A change needs a part that is not on the page yet.
    Missing {
        part: &'static str,
        how: &'static str,
    },
    /// More than one item of the navigation rail is active.
    ManyActive { count: usize },
    /// A focus id is not made of lower case letters, digits, colons, dashes and underscores.
    BadFocus(String),
    /// The settings panel's own reason.
    Configure(ConfigureError),
    /// The adventure card's own reason.
    Adventure(AdventureError),
    /// The campaign strip's own reason.
    Campaigns(CampaignsError),
}

impl fmt::Display for HomeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number { what, number, next } => write!(
                formatter,
                "there is no {what} number {number}: they are numbered from 1, and the next free number is {next}"
            ),
            Self::TooMany { what, limit } => {
                write!(formatter, "{what} holds at most {limit} entries")
            }
            Self::Missing { part, how } => {
                write!(formatter, "there is no {part} to change yet: {how}")
            }
            Self::ManyActive { count } => write!(
                formatter,
                "{count} navigation items are active: at most one page is the one shown"
            ),
            Self::BadFocus(id) => write!(
                formatter,
                "'{id}' is not a focus id: use lower case letters, digits, colons and dashes, such as actions:1 or nav:2"
            ),
            Self::Configure(error) => write!(formatter, "{error}"),
            Self::Adventure(error) => write!(formatter, "{error}"),
            Self::Campaigns(error) => write!(formatter, "{error}"),
        }
    }
}

impl From<HomeError> for SceneError {
    fn from(error: HomeError) -> Self {
        Self::Home(error)
    }
}

/// The brand mark's wordmark is a name.
pub fn brand(brand: &BrandMark) -> Result<(), SceneError> {
    checks::labelled("the wordmark", &brand.wordmark)
}

/// The title is a name; the tagline, when there is one, a line.
pub fn title(title: &TitleBlock) -> Result<(), SceneError> {
    checks::labelled("the title", &title.title)?;
    if let Some(tagline) = &title.tagline {
        checks::text("the tagline", tagline, MAXIMUM_CAPTION_CHARACTERS)?;
    }
    Ok(())
}

/// Every item has a name, a badge is short, and at most one is active.
pub fn nav(rail: &NavRail) -> Result<(), SceneError> {
    if rail.items.len() > MAXIMUM_NAV_ITEMS {
        return Err(HomeError::TooMany {
            what: "the navigation rail",
            limit: MAXIMUM_NAV_ITEMS,
        }
        .into());
    }
    for item in &rail.items {
        checks::labelled("a navigation label", &item.label)?;
        if let Some(badge) = &item.badge {
            checks::text("a navigation badge", badge, MAXIMUM_BADGE_CHARACTERS)?;
        }
    }
    match rail.active_count() {
        0 | 1 => Ok(()),
        count => Err(HomeError::ManyActive { count }.into()),
    }
}

/// Every button has a label, and a subtitle is a line.
pub fn actions(bar: &ActionBar) -> Result<(), SceneError> {
    if bar.buttons.len() > MAXIMUM_ACTION_BUTTONS {
        return Err(HomeError::TooMany {
            what: "the action bar",
            limit: MAXIMUM_ACTION_BUTTONS,
        }
        .into());
    }
    for button in &bar.buttons {
        checks::labelled("an action's label", &button.label)?;
        if let Some(subtitle) = &button.subtitle {
            checks::labelled("an action's subtitle", subtitle)?;
        }
    }
    Ok(())
}

/// Every component that is present, in the order the page draws them.
pub fn home(home: &HomeScene) -> Result<(), SceneError> {
    if let Some(id) = &home.focus
        && !crate::home_model::is_focus_id(id)
    {
        return Err(HomeError::BadFocus(id.clone()).into());
    }
    if let Some(value) = &home.brand {
        brand(value)?;
    }
    if let Some(value) = &home.title {
        title(value)?;
    }
    if let Some(value) = &home.nav {
        nav(value)?;
    }
    if let Some(value) = &home.actions {
        actions(value)?;
    }
    if let Some(value) = &home.configure {
        value.check()?;
    }
    if let Some(value) = &home.adventure {
        value
            .check()
            .map_err(|error| SceneError::Home(HomeError::Adventure(error)))?;
    }
    if let Some(value) = &home.campaigns {
        value
            .check()
            .map_err(|error| SceneError::Home(HomeError::Campaigns(error)))?;
    }
    Ok(())
}
