//! Live changes to the landing page: what the dev CLI's `scene home ...` verbs send, and what each
//! one does to the page.
//!
//! A change is a `HomeEdit`. It is made on a copy of the page as it is drawn now (the overlay's
//! own, or else the fixture's), the whole result is checked (`home_checks`), and only then does the
//! overlay hold it, so a refused change leaves everything as it was. `scene reset` drops it. A
//! field left out stays as it is; a text given empty, or `none`, takes an optional field away.
//! Items are numbered from 1; the next free number adds one at the end. The audit line of a change
//! carries the component and the field, plus numbers, flags and keys, never free text.

use serde::{Deserialize, Serialize};

use crate::fixture::Fixture;
use crate::home_adventure::AdventureEdit;
use crate::home_campaigns::CampaignsEdit;
use crate::home_checks::{self, HomeError};
use crate::home_chrome::{BrandMark, BrandSize, HomeTopBar, TitleBlock};
use crate::home_configure::ConfigureEdit;
use crate::home_glyph::HomeIcon;
use crate::home_model::{HomePart, HomeScene, MAXIMUM_ACTION_BUTTONS, MAXIMUM_NAV_ITEMS};
use crate::home_nav::{ActionBar, ActionButton, ActionVariant, NavItem, NavRail};
use crate::overlay::{Edit, Glyph};
use crate::scene::{Override, SceneError, SceneOverlay};

/// The drawing of a navigation item made by a change that names none.
pub const DEFAULT_NAV_GLYPH: HomeIcon = HomeIcon::Overlay(Glyph::Star);

/// A change to the brand mark. A field left out stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrandEdit {
    pub wordmark: Option<String>,
    pub size: Option<BrandSize>,
    pub star: Option<bool>,
}

/// A change to the title block.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TitleEdit {
    pub title: Option<String>,
    pub tagline: Edit<String>,
    pub rule: Option<bool>,
}

/// A change to the bell of the top bar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TopEdit {
    pub bell: Option<bool>,
    pub unread: Edit<u16>,
}

/// A change to one navigation item.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NavEdit {
    pub label: Option<String>,
    pub glyph: Option<HomeIcon>,
    /// Making an item active makes every other one not.
    pub active: Option<bool>,
    pub badge: Edit<String>,
}

/// A change to one action button.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ButtonEdit {
    pub label: Option<String>,
    pub subtitle: Edit<String>,
    pub glyph: Edit<HomeIcon>,
    pub variant: Option<ActionVariant>,
    pub enabled: Option<bool>,
}

/// One change to the landing page, as the dev CLI sends it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum HomeEdit {
    Brand {
        brand: BrandEdit,
    },
    Title {
        title: TitleEdit,
    },
    Top {
        top: TopEdit,
    },
    /// Items are numbered from 1; the number after the last adds an item (it needs a label).
    Nav {
        number: usize,
        item: NavEdit,
    },
    NavRemove {
        number: usize,
    },
    /// Buttons are numbered from 1; the number after the last adds a button (it needs a label).
    Action {
        number: usize,
        button: ButtonEdit,
    },
    ActionRemove {
        number: usize,
    },
    /// Puts the focus on one element by its id (`actions:1`, `nav:2`); `Clear` returns it to the
    /// first primary action.
    Focus {
        id: Edit<String>,
    },
    /// Takes one part of the page away.
    Remove {
        part: HomePart,
    },
    /// Takes the whole landing page away.
    Clear {},
    Configure {
        edit: ConfigureEdit,
    },
    Adventure {
        edit: AdventureEdit,
    },
    Campaigns {
        edit: CampaignsEdit,
    },
}

fn text(slot: &mut String, value: Option<String>) {
    if let Some(value) = value {
        *slot = value;
    }
}

/// An optional text: empty takes it away.
fn optional(slot: &mut Option<String>, value: Edit<String>) {
    match value {
        Edit::Keep => {}
        Edit::Clear => *slot = None,
        Edit::Set(text) if text.trim().is_empty() => *slot = None,
        Edit::Set(text) => *slot = Some(text),
    }
}

/// The index of item `number`, which may be the one after the last (`len`), and whether it is new.
fn index_of(
    what: &'static str,
    number: usize,
    len: usize,
    limit: usize,
) -> Result<(usize, bool), SceneError> {
    if number == 0 || number > len + 1 {
        return Err(HomeError::Number {
            what,
            number,
            next: len + 1,
        }
        .into());
    }
    if number > limit {
        return Err(HomeError::TooMany { what, limit }.into());
    }
    Ok((number - 1, number == len + 1))
}

fn removal_index(what: &'static str, number: usize, len: usize) -> Result<usize, SceneError> {
    if number == 0 || number > len {
        return Err(HomeError::Number {
            what,
            number,
            next: len + 1,
        }
        .into());
    }
    Ok(number - 1)
}

fn missing(part: &'static str, how: &'static str) -> SceneError {
    HomeError::Missing { part, how }.into()
}

impl BrandEdit {
    fn apply(self, slot: &mut Option<BrandMark>) {
        let brand = slot.get_or_insert_with(BrandMark::default);
        text(&mut brand.wordmark, self.wordmark);
        if let Some(size) = self.size {
            brand.size = size;
        }
        if let Some(star) = self.star {
            brand.star = star;
        }
    }
}

impl TitleEdit {
    fn apply(self, slot: &mut Option<TitleBlock>) {
        let block = slot.get_or_insert_with(TitleBlock::default);
        text(&mut block.title, self.title);
        optional(&mut block.tagline, self.tagline);
        if let Some(rule) = self.rule {
            block.rule = rule;
        }
    }
}

impl TopEdit {
    fn apply(self, slot: &mut Option<HomeTopBar>) {
        let top = slot.get_or_insert_with(|| HomeTopBar {
            bell: true,
            unread: None,
        });
        if let Some(bell) = self.bell {
            top.bell = bell;
        }
        self.unread.apply_to(&mut top.unread);
    }
}

impl NavEdit {
    fn apply(self, number: usize, slot: &mut Option<NavRail>) -> Result<(), SceneError> {
        let rail = slot.get_or_insert_with(NavRail::default);
        let (index, adding) = index_of(
            "navigation item",
            number,
            rail.items.len(),
            MAXIMUM_NAV_ITEMS,
        )?;
        if adding {
            let label = self
                .label
                .clone()
                .ok_or(SceneError::Empty("a new navigation item's label"))?;
            rail.items.push(NavItem {
                label,
                glyph: self.glyph.unwrap_or(DEFAULT_NAV_GLYPH),
                active: false,
                badge: None,
            });
        }
        if let Some(item) = rail.items.get_mut(index) {
            text(&mut item.label, self.label);
            if let Some(glyph) = self.glyph {
                item.glyph = glyph;
            }
            optional(&mut item.badge, self.badge);
        }
        match self.active {
            Some(true) => rail.activate(index),
            Some(false) => {
                if let Some(item) = rail.items.get_mut(index) {
                    item.active = false;
                }
            }
            None => {}
        }
        Ok(())
    }
}

impl ButtonEdit {
    fn apply(self, number: usize, slot: &mut Option<ActionBar>) -> Result<(), SceneError> {
        let bar = slot.get_or_insert_with(ActionBar::default);
        let (index, adding) = index_of(
            "action button",
            number,
            bar.buttons.len(),
            MAXIMUM_ACTION_BUTTONS,
        )?;
        if adding {
            let label = self
                .label
                .clone()
                .ok_or(SceneError::Empty("a new action's label"))?;
            bar.buttons.push(ActionButton {
                label,
                subtitle: None,
                glyph: None,
                variant: ActionVariant::Secondary,
                enabled: true,
            });
        }
        if let Some(button) = bar.buttons.get_mut(index) {
            text(&mut button.label, self.label);
            optional(&mut button.subtitle, self.subtitle);
            self.glyph.apply_to(&mut button.glyph);
            if let Some(variant) = self.variant {
                button.variant = variant;
            }
            if let Some(enabled) = self.enabled {
                button.enabled = enabled;
            }
        }
        Ok(())
    }
}

impl HomeEdit {
    /// Makes the change on `home`. The caller holds a copy and checks the result.
    fn apply_to(self, home: &mut HomeScene) -> Result<(), SceneError> {
        match self {
            Self::Brand { brand } => brand.apply(&mut home.brand),
            Self::Title { title } => title.apply(&mut home.title),
            Self::Top { top } => top.apply(&mut home.top),
            Self::Nav { number, item } => item.apply(number, &mut home.nav)?,
            Self::NavRemove { number } => {
                let rail = home.nav.as_mut().ok_or_else(|| {
                    missing(
                        "navigation rail",
                        "add an item with `scene home nav 1 label=T`",
                    )
                })?;
                rail.items
                    .remove(removal_index("navigation item", number, rail.items.len())?);
            }
            Self::Action { number, button } => button.apply(number, &mut home.actions)?,
            Self::ActionRemove { number } => {
                let bar = home.actions.as_mut().ok_or_else(|| {
                    missing(
                        "action bar",
                        "add a button with `scene home action 1 label=T`",
                    )
                })?;
                bar.buttons
                    .remove(removal_index("action button", number, bar.buttons.len())?);
            }
            Self::Focus { id } => id.apply_to(&mut home.focus),
            Self::Remove { part } => home.remove(part),
            Self::Clear {} => *home = HomeScene::default(),
            Self::Configure { edit } => edit
                .apply(&mut home.configure)
                .map_err(|error| SceneError::Home(HomeError::Configure(error)))?,
            Self::Adventure { edit } => edit
                .apply(&mut home.adventure)
                .map_err(|error| SceneError::Home(HomeError::Adventure(error)))?,
            Self::Campaigns { edit } => edit
                .apply(&mut home.campaigns)
                .map_err(|error| SceneError::Home(HomeError::Campaigns(error)))?,
        }
        Ok(())
    }
}

impl SceneOverlay {
    /// The landing page as drawn now: the overlay's own, or else the fixture's.
    fn home_now(&self, base: &Fixture) -> Option<HomeScene> {
        match &self.home {
            Some(Override::Set(home)) => Some(home.clone()),
            Some(Override::Clear) => None,
            None => base.tv.home.clone(),
        }
    }

    /// Makes one change to the landing page, starting from what is drawn now, and holds the
    /// checked result. `Clear` takes the whole page away.
    pub(crate) fn apply_home(&mut self, edit: HomeEdit, base: &Fixture) -> Result<(), SceneError> {
        if matches!(edit, HomeEdit::Clear {}) {
            self.home = Some(Override::Clear);
            return Ok(());
        }
        let mut home = self.home_now(base).unwrap_or_default();
        edit.apply_to(&mut home)?;
        home_checks::home(&home)?;
        self.home = Some(Override::Set(home));
        Ok(())
    }
}

fn present<T>(pairs: &mut Vec<(&'static str, String)>, key: &'static str, value: &Option<T>) {
    if value.is_some() {
        pairs.push((key, "set".to_owned()));
    }
}

fn edit_pair<T>(pairs: &mut Vec<(&'static str, String)>, key: &'static str, value: &Edit<T>) {
    match value {
        Edit::Keep => {}
        Edit::Clear => pairs.push((key, "none".to_owned())),
        Edit::Set(_) => pairs.push((key, "set".to_owned())),
    }
}

fn flag(pairs: &mut Vec<(&'static str, String)>, key: &'static str, value: Option<bool>) {
    if let Some(value) = value {
        pairs.push((key, value.to_string()));
    }
}

impl HomeEdit {
    /// The component and field of the change for the audit line: `component=nav field=item`, then
    /// numbers, flags and keys, never free text.
    pub fn audit_fields(&self) -> (&'static str, Vec<(&'static str, String)>) {
        let mut pairs: Vec<(&'static str, String)> = Vec::new();
        let (component, field) = match self {
            Self::Brand { brand } => {
                present(&mut pairs, "wordmark", &brand.wordmark);
                if let Some(size) = brand.size {
                    pairs.push(("size", size.key().to_owned()));
                }
                flag(&mut pairs, "star", brand.star);
                ("brand", "set")
            }
            Self::Title { title } => {
                present(&mut pairs, "title", &title.title);
                edit_pair(&mut pairs, "tagline", &title.tagline);
                flag(&mut pairs, "rule", title.rule);
                ("title", "set")
            }
            Self::Top { top } => {
                flag(&mut pairs, "bell", top.bell);
                if let Some(count) = top.unread.set_value() {
                    pairs.push(("unread", count.to_string()));
                } else {
                    edit_pair(&mut pairs, "unread", &top.unread);
                }
                ("top", "bell")
            }
            Self::Nav { number, item } => {
                pairs.push(("item", number.to_string()));
                present(&mut pairs, "label", &item.label);
                if let Some(glyph) = item.glyph {
                    pairs.push(("glyph", glyph.key().to_owned()));
                }
                flag(&mut pairs, "active", item.active);
                edit_pair(&mut pairs, "badge", &item.badge);
                ("nav", "item")
            }
            Self::NavRemove { number } => {
                pairs.push(("item", number.to_string()));
                ("nav", "remove")
            }
            Self::Action { number, button } => {
                pairs.push(("button", number.to_string()));
                present(&mut pairs, "label", &button.label);
                edit_pair(&mut pairs, "subtitle", &button.subtitle);
                if let Some(glyph) = button.glyph.set_value() {
                    pairs.push(("glyph", glyph.key().to_owned()));
                } else {
                    edit_pair(&mut pairs, "glyph", &button.glyph);
                }
                if let Some(variant) = button.variant {
                    pairs.push(("variant", variant.key().to_owned()));
                }
                flag(&mut pairs, "enabled", button.enabled);
                ("actions", "button")
            }
            Self::ActionRemove { number } => {
                pairs.push(("button", number.to_string()));
                ("actions", "remove")
            }
            Self::Focus { id } => {
                match id {
                    Edit::Set(id) => pairs.push(("id", id.clone())),
                    Edit::Clear => pairs.push(("id", "default".to_owned())),
                    Edit::Keep => {}
                }
                ("page", "focus")
            }
            Self::Remove { part } => {
                pairs.push(("part", part.key().to_owned()));
                ("page", "remove")
            }
            Self::Clear {} => ("page", "clear"),
            Self::Configure { edit } => {
                let (field, rest) = edit.audit_fields();
                pairs = rest;
                ("configure", field)
            }
            Self::Adventure { edit } => {
                let (field, rest) = edit.audit_fields();
                pairs = rest;
                ("adventure", field)
            }
            Self::Campaigns { edit } => {
                let (field, rest) = edit.audit_fields();
                pairs = rest;
                ("campaigns", field)
            }
        };
        pairs.insert(0, ("field", field.to_owned()));
        pairs.insert(0, ("component", component.to_owned()));
        ("home", pairs)
    }

    /// The media names the change shows, so the host can check each is a picture it has.
    pub fn media_names(&self) -> Vec<&str> {
        match self {
            Self::Configure { edit } => edit.media_names(),
            Self::Adventure { edit } => edit.media_names(),
            Self::Campaigns { edit } => edit.media_names(),
            _ => Vec::new(),
        }
    }
}
