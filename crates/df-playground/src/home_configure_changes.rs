//! Live changes to the landing page's ConfigurePanel: what the dev CLI's
//! `scene home configure ...` verbs send, and what each one does to the data.
//!
//! A change is a `ConfigureEdit`. It is applied to a copy of the data and the copy is checked
//! (`ConfigureData::check`) before it replaces what is drawn, so a refused change leaves everything
//! as it was. A field a change leaves out stays as it is. Every field of the panel, a card or a row
//! can be changed, and cards and rows can be added, removed and moved (positions count from 1).
//! The audit line of a change carries the field and the card and row keys, plus numbers and the
//! names of the fields changed, never free text.

use serde::{Deserialize, Serialize};

use crate::home_configure::{
    Accent, CardKind, ConfigureData, ConfigureError, ConfigureIcon, PanelLayout, PresetSelect, Row,
    SelectRow, SettingCard, SliderRow, SliderUnit, StatRow, StatValue, options,
};
use crate::overlay::Edit;

/// Which kind of row `RowAdd` makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    Select,
    Slider,
    Stat,
}

impl RowKind {
    /// The kind of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "select" => Some(Self::Select),
            "slider" => Some(Self::Slider),
            "stat" => Some(Self::Stat),
            _ => None,
        }
    }
}

/// A change to one row. Every field is optional; a field that does not fit the row's kind is
/// refused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RowEdit {
    /// Any row: its label.
    pub label: Option<String>,
    /// A select row: the chosen value.
    pub value: Option<String>,
    /// A select row: the options.
    pub options: Option<Vec<String>>,
    /// A slider row: its value, 0 to 100.
    pub level: Option<u8>,
    /// A slider row: how it reads.
    pub unit: Option<SliderUnit>,
    /// A slider row: the names of its steps.
    pub steps: Option<Vec<String>>,
    /// A stat row: what it shows.
    pub stat: Edit<StatValue>,
    /// A stat row: the info mark.
    pub info: Option<bool>,
    /// A stat row: the total style.
    pub highlight: Option<bool>,
}

impl RowEdit {
    /// The names of the fields the change sets, in a fixed order.
    pub fn fields(&self) -> Vec<&'static str> {
        let mut fields = Vec::new();
        let set = [
            ("label", self.label.is_some()),
            ("value", self.value.is_some()),
            ("options", self.options.is_some()),
            ("level", self.level.is_some()),
            ("unit", self.unit.is_some()),
            ("steps", self.steps.is_some()),
            ("stat", self.stat != Edit::Keep),
            ("info", self.info.is_some()),
            ("highlight", self.highlight.is_some()),
        ];
        for (name, present) in set {
            if present {
                fields.push(name);
            }
        }
        fields
    }

    fn refuse_outside(
        key: &str,
        fields: &[&'static str],
        allowed: &[&str],
        expected: &'static str,
    ) -> Result<(), ConfigureError> {
        match fields.iter().find(|field| !allowed.contains(field)) {
            Some(field) => Err(ConfigureError::FieldNotForRow {
                field,
                row: key.to_owned(),
                expected,
            }),
            None => Ok(()),
        }
    }

    fn apply(self, row: &mut Row) -> Result<(), ConfigureError> {
        let fields = self.fields();
        match row {
            Row::Select(select) => {
                Self::refuse_outside(
                    &select.key,
                    &fields,
                    &["label", "value", "options"],
                    "select",
                )?;
                if let Some(label) = self.label {
                    select.label = label;
                }
                if let Some(value) = self.value {
                    select.value = value;
                }
                if let Some(choices) = self.options {
                    select.options = choices;
                }
            }
            Row::Slider(slider) => {
                Self::refuse_outside(
                    &slider.key,
                    &fields,
                    &["label", "level", "unit", "steps"],
                    "slider",
                )?;
                if let Some(label) = self.label {
                    slider.label = label;
                }
                if let Some(level) = self.level {
                    slider.value = level;
                }
                if let Some(unit) = self.unit {
                    slider.unit = unit;
                }
                if let Some(steps) = self.steps {
                    slider.steps = steps;
                }
            }
            Row::Stat(stat) => {
                Self::refuse_outside(
                    &stat.key,
                    &fields,
                    &["label", "stat", "info", "highlight"],
                    "stat",
                )?;
                if let Some(label) = self.label {
                    stat.label = label;
                }
                self.stat.apply_to(&mut stat.value);
                if let Some(info) = self.info {
                    stat.info = info;
                }
                if let Some(highlight) = self.highlight {
                    stat.highlight = highlight;
                }
            }
        }
        Ok(())
    }
}

/// One live edit of the landing page's ConfigurePanel and its setting cards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigureEdit {
    /// Takes the whole component away.
    Clear {},
    /// The panel's title, the currency sign and the layout.
    Panel {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        currency: Option<String>,
        #[serde(default)]
        layout: Option<PanelLayout>,
    },
    /// The preset selector; it is made when there is none.
    Preset {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        options: Option<Vec<String>>,
    },
    /// Takes the preset selector away.
    PresetRemove {},
    /// Changes a card's own fields.
    Card {
        card: String,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        icon: Edit<ConfigureIcon>,
        #[serde(default)]
        banner: Edit<String>,
        #[serde(default)]
        accent: Option<Accent>,
        #[serde(default)]
        shown_rows: Option<u8>,
    },
    /// Adds a card, at a position or at the end.
    CardAdd {
        card: String,
        kind: CardKind,
        label: String,
        #[serde(default)]
        icon: Option<ConfigureIcon>,
        #[serde(default)]
        banner: Option<String>,
        #[serde(default)]
        accent: Option<Accent>,
        #[serde(default)]
        shown_rows: Option<u8>,
        #[serde(default)]
        at: Option<usize>,
    },
    /// Takes a card away.
    CardRemove { card: String },
    /// Moves a card to a position.
    CardMove { card: String, position: usize },
    /// Changes a row of a card.
    Row {
        card: String,
        row: String,
        #[serde(default)]
        edit: RowEdit,
    },
    /// Adds a row to a card, at a position or at the end.
    RowAdd {
        card: String,
        row: String,
        kind: RowKind,
        #[serde(default)]
        edit: RowEdit,
        #[serde(default)]
        at: Option<usize>,
    },
    /// Takes a row away.
    RowRemove { card: String, row: String },
    /// Moves a row to a position within its card.
    RowMove {
        card: String,
        row: String,
        position: usize,
    },
}

fn card_mut<'a>(
    data: &'a mut ConfigureData,
    key: &str,
) -> Result<&'a mut SettingCard, ConfigureError> {
    data.cards
        .iter_mut()
        .find(|card| card.key == key)
        .ok_or_else(|| ConfigureError::UnknownCard(key.to_owned()))
}

fn row_index(card: &SettingCard, row: &str) -> Result<usize, ConfigureError> {
    card.rows
        .iter()
        .position(|candidate| candidate.key() == row)
        .ok_or_else(|| ConfigureError::UnknownRow {
            card: card.key.clone(),
            row: row.to_owned(),
        })
}

/// The index a 1-based position names among `count` places.
fn place(position: usize, count: usize) -> Result<usize, ConfigureError> {
    if (1..=count).contains(&position) {
        Ok(position - 1)
    } else {
        Err(ConfigureError::BadPosition { position, count })
    }
}

/// Where an added item goes among `count` existing ones: a position from 1 up to one past the end.
fn insertion(at: Option<usize>, count: usize) -> Result<usize, ConfigureError> {
    match at {
        None => Ok(count),
        Some(position) => place(position, count + 1),
    }
}

fn move_to<T>(items: &mut Vec<T>, from: usize, position: usize) -> Result<(), ConfigureError> {
    let to = place(position, items.len())?;
    let item = items.remove(from);
    items.insert(to, item);
    Ok(())
}

fn new_row(key: &str, kind: RowKind) -> Row {
    match kind {
        RowKind::Select => Row::Select(SelectRow {
            key: key.to_owned(),
            ..SelectRow::default()
        }),
        RowKind::Slider => Row::Slider(SliderRow {
            key: key.to_owned(),
            ..SliderRow::default()
        }),
        RowKind::Stat => Row::Stat(StatRow {
            key: key.to_owned(),
            ..StatRow::default()
        }),
    }
}

impl ConfigureEdit {
    /// True for the changes that make a panel when there is none.
    fn makes_the_panel(&self) -> bool {
        matches!(
            self,
            Self::Panel { .. } | Self::Preset { .. } | Self::CardAdd { .. }
        )
    }

    /// Makes the change in `slot`. The data is copied, changed and checked first, so a refusal
    /// leaves `slot` as it was.
    pub fn apply(self, slot: &mut Option<ConfigureData>) -> Result<(), ConfigureError> {
        if matches!(self, Self::Clear {}) {
            *slot = None;
            return Ok(());
        }
        let mut data = match slot {
            Some(data) => data.clone(),
            None if self.makes_the_panel() => ConfigureData::default(),
            None => return Err(ConfigureError::NoPanel),
        };
        self.change(&mut data)?;
        data.check()?;
        *slot = Some(data);
        Ok(())
    }

    fn change(self, data: &mut ConfigureData) -> Result<(), ConfigureError> {
        match self {
            Self::Clear {} => {}
            Self::Panel {
                title,
                currency,
                layout,
            } => {
                if title.is_none() && currency.is_none() && layout.is_none() {
                    return Err(ConfigureError::NoChange);
                }
                if let Some(layout) = layout {
                    data.layout = layout;
                }
                if let Some(title) = title {
                    data.title = title;
                }
                if let Some(currency) = currency {
                    data.currency = currency;
                }
            }
            Self::Preset {
                label,
                value,
                options: choices,
            } => {
                if label.is_none() && value.is_none() && choices.is_none() {
                    return Err(ConfigureError::NoChange);
                }
                let presets = data.presets.get_or_insert_with(PresetSelect::default);
                if let Some(label) = label {
                    presets.label = label;
                }
                if let Some(value) = value {
                    presets.value = value;
                }
                if let Some(choices) = choices {
                    options(&choices)?;
                    presets.options = choices;
                }
            }
            Self::PresetRemove {} => data.presets = None,
            Self::Card {
                card,
                label,
                icon,
                banner,
                accent,
                shown_rows,
            } => {
                let target = card_mut(data, &card)?;
                if let Some(label) = label {
                    target.label = label;
                }
                icon.apply_to(&mut target.icon);
                banner.apply_to(&mut target.banner);
                if let Some(accent) = accent {
                    target.accent = accent;
                }
                if let Some(shown) = shown_rows {
                    target.shown_rows = shown;
                }
            }
            Self::CardAdd {
                card,
                kind,
                label,
                icon,
                banner,
                accent,
                shown_rows,
                at,
            } => {
                let index = insertion(at, data.cards.len())?;
                let mut made = SettingCard {
                    key: card,
                    kind,
                    label,
                    icon,
                    banner,
                    ..SettingCard::default()
                };
                if let Some(accent) = accent {
                    made.accent = accent;
                }
                if let Some(shown) = shown_rows {
                    made.shown_rows = shown;
                }
                data.cards.insert(index, made);
            }
            Self::CardRemove { card } => {
                let index = data
                    .cards
                    .iter()
                    .position(|candidate| candidate.key == card)
                    .ok_or(ConfigureError::UnknownCard(card))?;
                data.cards.remove(index);
            }
            Self::CardMove { card, position } => {
                let index = data
                    .cards
                    .iter()
                    .position(|candidate| candidate.key == card)
                    .ok_or(ConfigureError::UnknownCard(card))?;
                move_to(&mut data.cards, index, position)?;
            }
            Self::Row { card, row, edit } => {
                if edit.fields().is_empty() {
                    return Err(ConfigureError::NoChange);
                }
                let target = card_mut(data, &card)?;
                let index = row_index(target, &row)?;
                let changed = target
                    .rows
                    .get_mut(index)
                    .ok_or(ConfigureError::UnknownRow { card, row })?;
                edit.apply(changed)?;
            }
            Self::RowAdd {
                card,
                row,
                kind,
                edit,
                at,
            } => {
                let target = card_mut(data, &card)?;
                let index = insertion(at, target.rows.len())?;
                let mut made = new_row(&row, kind);
                edit.apply(&mut made)?;
                target.rows.insert(index, made);
            }
            Self::RowRemove { card, row } => {
                let target = card_mut(data, &card)?;
                let index = row_index(target, &row)?;
                target.rows.remove(index);
            }
            Self::RowMove {
                card,
                row,
                position,
            } => {
                let target = card_mut(data, &card)?;
                let index = row_index(target, &row)?;
                move_to(&mut target.rows, index, position)?;
            }
        }
        Ok(())
    }

    /// The field and the audit pairs of the change: keys, numbers and the names of the fields
    /// changed, never free text.
    pub fn audit_fields(&self) -> (&'static str, Vec<(&'static str, String)>) {
        match self {
            Self::Clear {} => ("clear", Vec::new()),
            Self::Panel {
                title,
                currency,
                layout,
            } => (
                "panel",
                vec![(
                    "fields",
                    names(&[
                        ("title", title.is_some()),
                        ("currency", currency.is_some()),
                        ("layout", layout.is_some()),
                    ]),
                )],
            ),
            Self::Preset {
                label,
                value,
                options,
            } => (
                "preset",
                vec![(
                    "fields",
                    names(&[
                        ("label", label.is_some()),
                        ("value", value.is_some()),
                        ("options", options.is_some()),
                    ]),
                )],
            ),
            Self::PresetRemove {} => ("preset_remove", Vec::new()),
            Self::Card {
                card,
                label,
                icon,
                banner,
                accent,
                shown_rows,
            } => {
                let mut pairs = vec![
                    ("card", card.clone()),
                    (
                        "fields",
                        names(&[
                            ("label", label.is_some()),
                            ("icon", *icon != Edit::Keep),
                            ("banner", *banner != Edit::Keep),
                            ("accent", accent.is_some()),
                            ("shown_rows", shown_rows.is_some()),
                        ]),
                    ),
                ];
                if let Some(shown) = shown_rows {
                    pairs.push(("shown_rows", shown.to_string()));
                }
                ("card", pairs)
            }
            Self::CardAdd { card, kind, at, .. } => {
                let mut pairs = vec![("card", card.clone()), ("kind", kind.key().to_owned())];
                if let Some(at) = at {
                    pairs.push(("at", at.to_string()));
                }
                ("card_add", pairs)
            }
            Self::CardRemove { card } => ("card_remove", vec![("card", card.clone())]),
            Self::CardMove { card, position } => (
                "card_move",
                vec![("card", card.clone()), ("position", position.to_string())],
            ),
            Self::Row { card, row, edit } => {
                let mut pairs = vec![
                    ("card", card.clone()),
                    ("row", row.clone()),
                    ("fields", edit.fields().join(",")),
                ];
                if let Some(level) = edit.level {
                    pairs.push(("level", level.to_string()));
                }
                ("row", pairs)
            }
            Self::RowAdd {
                card,
                row,
                kind,
                at,
                ..
            } => {
                let kind = match kind {
                    RowKind::Select => "select",
                    RowKind::Slider => "slider",
                    RowKind::Stat => "stat",
                };
                let mut pairs = vec![
                    ("card", card.clone()),
                    ("row", row.clone()),
                    ("kind", kind.to_owned()),
                ];
                if let Some(at) = at {
                    pairs.push(("at", at.to_string()));
                }
                ("row_add", pairs)
            }
            Self::RowRemove { card, row } => (
                "row_remove",
                vec![("card", card.clone()), ("row", row.clone())],
            ),
            Self::RowMove {
                card,
                row,
                position,
            } => (
                "row_move",
                vec![
                    ("card", card.clone()),
                    ("row", row.clone()),
                    ("position", position.to_string()),
                ],
            ),
        }
    }

    /// The media names the change shows, so the host can check each is a picture.
    pub fn media_names(&self) -> Vec<&str> {
        match self {
            Self::Card {
                banner: Edit::Set(name),
                ..
            }
            | Self::CardAdd {
                banner: Some(name), ..
            } => vec![name.as_str()],
            _ => Vec::new(),
        }
    }
}

/// The names of the fields that are set, comma separated.
fn names(fields: &[(&'static str, bool)]) -> String {
    fields
        .iter()
        .filter(|(_, set)| *set)
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(",")
}
