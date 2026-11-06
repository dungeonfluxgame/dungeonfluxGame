//! The data of the landing page's ConfigurePanel and its setting cards (render 19, and the session
//! settings of renders 05, 09 and 23), read from a fixture and changed live by `ConfigureEdit`
//! (`home::configure::changes`).
//!
//! The panel is a title, an optional preset selector and a list of cards. A card is one of four
//! components (`CardKind`): a select card (a banner picture, a title and one dropdown), a sliders
//! card, a mixed card (dropdowns and sliders together) and a budget card (labelled amounts). A card
//! holds rows in the order the render draws them, and draws the first `shown_rows` of them; the
//! rest stay in the data and answer to the dev CLI, as the render has no "more" control. The TV
//! only shows state, so none of this is interactive: the phone changes it.
//!
//! Money is only drawn when the data has it: a stat row with no value is left out, and nothing here
//! makes up an amount. Every range is checked at the edge by `ConfigureData::check`, which the
//! fixture reader and each live change call, so there is one copy of each rule. The limits are very
//! large numbers that only keep memory bounded.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::home::checks::HomeError;
use crate::limits::{
    MAXIMUM_AMOUNT_MINOR_UNITS, MAXIMUM_DROPDOWN_OPTIONS, MAXIMUM_MEDIA_NAME_BYTES,
    MAXIMUM_NAME_CHARACTERS, MAXIMUM_PERCENT, MAXIMUM_ROWS_PER_SETTING_CARD, MAXIMUM_SETTING_CARDS,
    MAXIMUM_SLIDER_STEPS,
};
use crate::scene::SceneError;

mod changes;
mod icons;

pub use self::changes::{ConfigureEdit, RowEdit, RowKind};
pub use self::icons::ConfigureIcon;

/// The title of a panel made by a change that gives none.
pub const DEFAULT_TITLE: &str = "Configure Session";
/// The label of the preset selector when the data gives none.
pub const DEFAULT_PRESET_LABEL: &str = "Presets:";
/// The currency sign when the data gives none.
pub const DEFAULT_CURRENCY: &str = "$";
/// How many rows a card draws until its data says: render 19 draws three in each card.
pub const DEFAULT_SHOWN_ROWS: u8 = 3;
/// The segments of a slider's bar: the bar is read from a sofa, so it is ten wide blocks, not a
/// thin track.
pub const SLIDER_SEGMENTS: u8 = 10;

/// Why the panel or a change to it was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigureError {
    /// A text that must not be blank is.
    Empty(&'static str),
    /// A text is longer than its limit.
    TooLong { what: &'static str, limit: usize },
    /// A list is longer than its very large limit.
    TooMany { what: &'static str, limit: usize },
    /// A key is not made of letters, digits, `-` and `_`.
    InvalidKey(String),
    /// Two cards or two rows of one card share a key.
    DuplicateKey { what: &'static str, key: String },
    /// A slider's value is past 100.
    InvalidPercent { row: String, value: u8 },
    /// A card must draw at least one row.
    InvalidShownRows,
    /// There is no panel to change.
    NoPanel,
    /// No card has that key.
    UnknownCard(String),
    /// The card has no row with that key.
    UnknownRow { card: String, row: String },
    /// A dropdown's value is not one of its options.
    NotAnOption { row: String, value: String },
    /// A card of that kind cannot hold that kind of row.
    RowNotAllowed {
        card: String,
        kind: CardKind,
        row: String,
    },
    /// A change names a field the row's kind does not have.
    FieldNotForRow {
        field: &'static str,
        row: String,
        expected: &'static str,
    },
    /// A slider that reads in steps needs at least two.
    TooFewSteps(String),
    /// The low end of an estimate is above its high end.
    InvalidRange { low: u64, high: u64 },
    /// An amount is past the largest.
    InvalidAmount(u64),
    /// A position is 0 or past the end.
    InvalidPosition { position: usize, count: usize },
    /// A change names no field to change.
    NoChange,
}

impl fmt::Display for ConfigureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(what) => write!(formatter, "{what} cannot be empty"),
            Self::TooLong { what, limit } => {
                write!(formatter, "{what} is longer than {limit} characters")
            }
            Self::TooMany { what, limit } => {
                write!(formatter, "{what} holds at most {limit} entries")
            }
            Self::InvalidKey(key) => write!(
                formatter,
                "'{key}' is not a key: use letters, digits, '-' and '_'"
            ),
            Self::DuplicateKey { what, key } => {
                write!(formatter, "{what} '{key}' is already there")
            }
            Self::InvalidPercent { row, value } => write!(
                formatter,
                "the slider '{row}' is a percent from 0 to {MAXIMUM_PERCENT}, not {value}"
            ),
            Self::InvalidShownRows => formatter.write_str("a card draws at least 1 row"),
            Self::NoPanel => formatter.write_str(
                "there is no configure panel on the page: set its title or add a card to make one",
            ),
            Self::UnknownCard(card) => write!(formatter, "there is no card '{card}'"),
            Self::UnknownRow { card, row } => {
                write!(formatter, "the card '{card}' has no row '{row}'")
            }
            Self::NotAnOption { row, value } => write!(
                formatter,
                "'{value}' is not one of the options of '{row}': change the options first"
            ),
            Self::RowNotAllowed { card, kind, row } => write!(
                formatter,
                "the {} card '{card}' cannot hold the row '{row}': it holds {} rows",
                kind.key(),
                kind.holds()
            ),
            Self::FieldNotForRow {
                field,
                row,
                expected,
            } => write!(
                formatter,
                "'{field}' does not apply to the row '{row}': it is for {expected} rows"
            ),
            Self::TooFewSteps(row) => {
                write!(
                    formatter,
                    "the slider '{row}' reads in steps and needs at least 2"
                )
            }
            Self::InvalidRange { low, high } => {
                write!(
                    formatter,
                    "an estimate cannot start at {low} and end at {high}"
                )
            }
            Self::InvalidAmount(cents) => write!(
                formatter,
                "an amount is at most {MAXIMUM_AMOUNT_MINOR_UNITS} hundredths, not {cents}"
            ),
            Self::InvalidPosition { position, count } => {
                write!(formatter, "a position is from 1 to {count}, not {position}")
            }
            Self::NoChange => formatter.write_str("the change names nothing to change"),
        }
    }
}

impl std::error::Error for ConfigureError {}

impl From<ConfigureError> for SceneError {
    fn from(error: ConfigureError) -> Self {
        Self::Home(HomeError::Configure(error))
    }
}

/// Which of the four components a card is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    /// A banner picture, a title and one dropdown (the first row).
    #[default]
    Select,
    /// A title and slider rows.
    Sliders,
    /// A title with dropdown and slider rows.
    Mixed,
    /// A title and labelled amounts, the last of them highlighted.
    Budget,
}

impl CardKind {
    pub const ALL: [Self; 4] = [Self::Select, Self::Sliders, Self::Mixed, Self::Budget];

    /// The name a fixture and the command line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Sliders => "sliders",
            Self::Mixed => "mixed",
            Self::Budget => "budget",
        }
    }

    /// The kind of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// The rows a card of this kind holds, for a message.
    pub fn holds(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Sliders => "slider",
            Self::Mixed => "select and slider",
            Self::Budget => "stat",
        }
    }

    fn allows(self, row: &Row) -> bool {
        matches!(
            (self, row),
            (Self::Select, Row::Select(_))
                | (Self::Sliders, Row::Slider(_))
                | (Self::Mixed, Row::Select(_) | Row::Slider(_))
                | (Self::Budget, Row::Stat(_))
        )
    }
}

/// How the panel lays its cards out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelLayout {
    /// Three columns of cards, each with its banner, its heading and up to `shown_rows` rows: the
    /// full settings, for a slot as large as the type needs.
    #[default]
    Cards,
    /// Two columns of tiles for a small slot: each card is its heading and its first drawn row, in
    /// the same large type. What the tile leaves out stays in the data and answers to the CLI.
    Tiles,
}

impl PanelLayout {
    pub const ALL: [Self; 2] = [Self::Cards, Self::Tiles];

    /// The layout of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layout| layout.key() == key)
    }

    /// The name a fixture and the command line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Cards => "cards",
            Self::Tiles => "tiles",
        }
    }
}

/// The colour of a card's rim and glow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Cyan,
    Violet,
    Green,
}

impl Accent {
    pub const ALL: [Self; 3] = [Self::Cyan, Self::Violet, Self::Green];

    /// The name a fixture and the command line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Cyan => "cyan",
            Self::Violet => "violet",
            Self::Green => "green",
        }
    }

    /// The accent of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|accent| accent.key() == key)
    }
}

/// How a slider reads its value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliderUnit {
    /// The value and a percent sign: 80%.
    #[default]
    Percent,
    /// The named step the value falls on: Balanced, High.
    Steps,
}

impl SliderUnit {
    pub const ALL: [Self; 2] = [Self::Percent, Self::Steps];

    /// The name a fixture and the command line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Percent => "percent",
            Self::Steps => "steps",
        }
    }

    /// The unit of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|unit| unit.key() == key)
    }
}

/// A dropdown: the chosen value and what it could be.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SelectRow {
    pub key: String,
    pub label: String,
    pub value: String,
    pub options: Vec<String>,
}

/// A slider: a value from 0 to 100 along the track, read as a percent or as a named step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SliderRow {
    pub key: String,
    pub label: String,
    pub value: u8,
    pub unit: SliderUnit,
    /// The names of the steps, lowest first, when the unit is `Steps`.
    pub steps: Vec<String>,
}

impl SliderRow {
    /// The step the value falls on, when the slider reads in steps.
    pub fn step(&self) -> Option<&str> {
        let last = self.steps.len().checked_sub(1).filter(|last| *last > 0)?;
        let index = (usize::from(self.value) * last + 50) / 100;
        self.steps.get(index).map(String::as_str)
    }

    /// How many of the bar's segments are lit: the value to the nearest tenth.
    pub fn lit_segments(&self) -> u8 {
        let per_segment = MAXIMUM_PERCENT / SLIDER_SEGMENTS;
        (self.value.min(MAXIMUM_PERCENT) + per_segment / 2) / per_segment
    }

    /// What the slider reads out: `80%`, or the name of its step.
    pub fn read_out(&self) -> String {
        match self.unit {
            SliderUnit::Percent => format!("{}%", self.value),
            SliderUnit::Steps => self.step().unwrap_or_default().to_owned(),
        }
    }
}

/// What a stat row shows. Amounts are in hundredths of the currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StatValue {
    /// An amount: `$28.40`.
    Money { cents: u64 },
    /// An amount a hour: `$3.90 / hr`.
    PerHour { cents: u64 },
    /// An estimate: `$7.80 – $11.70`.
    Range { low_cents: u64, high_cents: u64 },
    /// Any other text.
    Text { text: String },
}

/// The amount as the currency sign and the amount to the hundredth, thousands set off by commas.
pub fn money_text(cents: u64, currency: &str) -> String {
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{currency}{grouped}.{:02}", cents % 100)
}

impl StatValue {
    /// The text the row draws.
    pub fn text(&self, currency: &str) -> String {
        match self {
            Self::Money { cents } => money_text(*cents, currency),
            Self::PerHour { cents } => format!("{} / hr", money_text(*cents, currency)),
            Self::Range {
                low_cents,
                high_cents,
            } => format!(
                "{} \u{2013} {}",
                money_text(*low_cents, currency),
                money_text(*high_cents, currency)
            ),
            Self::Text { text } => text.clone(),
        }
    }
}

/// A labelled amount, drawn only when it has a value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatRow {
    pub key: String,
    pub label: String,
    pub value: Option<StatValue>,
    /// Draws the round info mark after the label.
    pub info: bool,
    /// Draws the value in the brighter total style.
    pub highlight: bool,
}

/// One row of a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "row", rename_all = "snake_case")]
pub enum Row {
    Select(SelectRow),
    Slider(SliderRow),
    Stat(StatRow),
}

impl Row {
    /// The row's key.
    pub fn key(&self) -> &str {
        match self {
            Self::Select(row) => &row.key,
            Self::Slider(row) => &row.key,
            Self::Stat(row) => &row.key,
        }
    }

    /// The row's label.
    pub fn label(&self) -> &str {
        match self {
            Self::Select(row) => &row.label,
            Self::Slider(row) => &row.label,
            Self::Stat(row) => &row.label,
        }
    }

    /// What kind of row it is, for a message.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Select(_) => "select",
            Self::Slider(_) => "slider",
            Self::Stat(_) => "stat",
        }
    }
}

/// One card of the panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SettingCard {
    pub key: String,
    pub kind: CardKind,
    pub label: String,
    pub icon: Option<ConfigureIcon>,
    /// The scene art behind a select card's top, a picture of the media library.
    pub banner: Option<String>,
    pub accent: Accent,
    /// How many rows the card draws, counted from the first.
    pub shown_rows: u8,
    pub rows: Vec<Row>,
}

impl Default for SettingCard {
    fn default() -> Self {
        Self {
            key: String::new(),
            kind: CardKind::Select,
            label: String::new(),
            icon: None,
            banner: None,
            accent: Accent::Cyan,
            shown_rows: DEFAULT_SHOWN_ROWS,
            rows: Vec::new(),
        }
    }
}

impl SettingCard {
    /// The rows the card draws: a select card its first dropdown, the others the first
    /// `shown_rows`, leaving out a stat row that has no value.
    pub fn drawn_rows(&self) -> Vec<&Row> {
        let limit = match self.kind {
            CardKind::Select => 1,
            _ => usize::from(self.shown_rows),
        };
        self.rows
            .iter()
            .filter(|row| !matches!(row, Row::Stat(StatRow { value: None, .. })))
            .take(limit)
            .collect()
    }
}

/// The preset dropdown at the right of the panel's title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PresetSelect {
    pub label: String,
    pub value: String,
    pub options: Vec<String>,
}

impl Default for PresetSelect {
    fn default() -> Self {
        Self {
            label: DEFAULT_PRESET_LABEL.to_owned(),
            value: String::new(),
            options: Vec::new(),
        }
    }
}

/// The data of the landing page's ConfigurePanel and its setting cards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ConfigureRecord")]
pub struct ConfigureData {
    pub title: String,
    /// The sign before every amount.
    pub currency: String,
    pub layout: PanelLayout,
    pub presets: Option<PresetSelect>,
    pub cards: Vec<SettingCard>,
}

/// `ConfigureData` as a scene file writes it, before its bounds are checked: serde reads the
/// settings panel through this, so a value out of range is refused where it is read.
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ConfigureRecord {
    title: String,
    currency: String,
    layout: PanelLayout,
    presets: Option<PresetSelect>,
    cards: Vec<SettingCard>,
}

impl Default for ConfigureRecord {
    fn default() -> Self {
        let ConfigureData {
            title,
            currency,
            layout,
            presets,
            cards,
        } = ConfigureData::default();
        Self {
            title,
            currency,
            layout,
            presets,
            cards,
        }
    }
}

impl TryFrom<ConfigureRecord> for ConfigureData {
    type Error = ConfigureError;

    fn try_from(record: ConfigureRecord) -> Result<Self, Self::Error> {
        let ConfigureRecord {
            title,
            currency,
            layout,
            presets,
            cards,
        } = record;
        let data = Self {
            title,
            currency,
            layout,
            presets,
            cards,
        };
        data.check()?;
        Ok(data)
    }
}

impl Default for ConfigureData {
    fn default() -> Self {
        Self {
            title: DEFAULT_TITLE.to_owned(),
            currency: DEFAULT_CURRENCY.to_owned(),
            layout: PanelLayout::Cards,
            presets: None,
            cards: Vec::new(),
        }
    }
}

fn text(what: &'static str, value: &str) -> Result<(), ConfigureError> {
    if value.trim().is_empty() {
        Err(ConfigureError::Empty(what))
    } else if value.chars().count() > MAXIMUM_NAME_CHARACTERS {
        Err(ConfigureError::TooLong {
            what,
            limit: MAXIMUM_NAME_CHARACTERS,
        })
    } else {
        Ok(())
    }
}

fn key(value: &str) -> Result<(), ConfigureError> {
    text("a key", value)?;
    let plain = value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_');
    if plain {
        Ok(())
    } else {
        Err(ConfigureError::InvalidKey(value.to_owned()))
    }
}

/// The options of a dropdown are named, not repeated, and not too many.
pub(crate) fn options(options: &[String]) -> Result<(), ConfigureError> {
    if options.len() > MAXIMUM_DROPDOWN_OPTIONS {
        return Err(ConfigureError::TooMany {
            what: "a list of options",
            limit: MAXIMUM_DROPDOWN_OPTIONS,
        });
    }
    let mut seen = BTreeSet::new();
    for option in options {
        text("an option", option)?;
        if !seen.insert(option.as_str()) {
            return Err(ConfigureError::DuplicateKey {
                what: "the option",
                key: option.clone(),
            });
        }
    }
    Ok(())
}

fn chosen(row: &str, value: &str, options: &[String]) -> Result<(), ConfigureError> {
    text("a chosen value", value)?;
    if options.is_empty() || options.iter().any(|option| option == value) {
        Ok(())
    } else {
        Err(ConfigureError::NotAnOption {
            row: row.to_owned(),
            value: value.to_owned(),
        })
    }
}

fn amount(cents: u64) -> Result<(), ConfigureError> {
    if cents > MAXIMUM_AMOUNT_MINOR_UNITS {
        Err(ConfigureError::InvalidAmount(cents))
    } else {
        Ok(())
    }
}

fn check_row(row: &Row) -> Result<(), ConfigureError> {
    key(row.key())?;
    text("a row label", row.label())?;
    match row {
        Row::Select(select) => {
            options(&select.options)?;
            chosen(&select.key, &select.value, &select.options)
        }
        Row::Slider(slider) => {
            if slider.value > MAXIMUM_PERCENT {
                return Err(ConfigureError::InvalidPercent {
                    row: slider.key.clone(),
                    value: slider.value,
                });
            }
            if slider.steps.len() > MAXIMUM_SLIDER_STEPS {
                return Err(ConfigureError::TooMany {
                    what: "a list of steps",
                    limit: MAXIMUM_SLIDER_STEPS,
                });
            }
            for step in &slider.steps {
                text("a step name", step)?;
            }
            if slider.unit == SliderUnit::Steps && slider.steps.len() < 2 {
                return Err(ConfigureError::TooFewSteps(slider.key.clone()));
            }
            Ok(())
        }
        Row::Stat(stat) => match &stat.value {
            None => Ok(()),
            Some(StatValue::Money { cents } | StatValue::PerHour { cents }) => amount(*cents),
            Some(StatValue::Range {
                low_cents,
                high_cents,
            }) => {
                amount(*high_cents)?;
                if low_cents > high_cents {
                    Err(ConfigureError::InvalidRange {
                        low: *low_cents,
                        high: *high_cents,
                    })
                } else {
                    Ok(())
                }
            }
            Some(StatValue::Text { text: shown }) => text("a stat value", shown),
        },
    }
}

fn check_card(card: &SettingCard) -> Result<(), ConfigureError> {
    key(&card.key)?;
    text("a card label", &card.label)?;
    if let Some(banner) = &card.banner {
        if banner.trim().is_empty() {
            return Err(ConfigureError::Empty("a banner picture name"));
        }
        if banner.chars().count() > MAXIMUM_MEDIA_NAME_BYTES {
            return Err(ConfigureError::TooLong {
                what: "a banner picture name",
                limit: MAXIMUM_MEDIA_NAME_BYTES,
            });
        }
    }
    if card.shown_rows == 0 {
        return Err(ConfigureError::InvalidShownRows);
    }
    if card.rows.len() > MAXIMUM_ROWS_PER_SETTING_CARD {
        return Err(ConfigureError::TooMany {
            what: "a card",
            limit: MAXIMUM_ROWS_PER_SETTING_CARD,
        });
    }
    let mut keys = BTreeSet::new();
    for row in &card.rows {
        check_row(row)?;
        if !card.kind.allows(row) {
            return Err(ConfigureError::RowNotAllowed {
                card: card.key.clone(),
                kind: card.kind,
                row: row.key().to_owned(),
            });
        }
        if !keys.insert(row.key()) {
            return Err(ConfigureError::DuplicateKey {
                what: "the row",
                key: row.key().to_owned(),
            });
        }
    }
    Ok(())
}

impl ConfigureData {
    /// Why the data is out of range, or `Ok`.
    pub fn check(&self) -> Result<(), ConfigureError> {
        text("the panel title", &self.title)?;
        text("the currency sign", &self.currency)?;
        if let Some(presets) = &self.presets {
            text("the presets label", &presets.label)?;
            options(&presets.options)?;
            chosen("the presets", &presets.value, &presets.options)?;
        }
        if self.cards.len() > MAXIMUM_SETTING_CARDS {
            return Err(ConfigureError::TooMany {
                what: "the panel",
                limit: MAXIMUM_SETTING_CARDS,
            });
        }
        let mut keys = BTreeSet::new();
        for card in &self.cards {
            check_card(card)?;
            if !keys.insert(card.key.as_str()) {
                return Err(ConfigureError::DuplicateKey {
                    what: "the card",
                    key: card.key.clone(),
                });
            }
        }
        Ok(())
    }

    /// The id of the preset selector, for the focus.
    pub const PRESET_FOCUS_ID: &'static str = "configure:preset";

    /// The focus id of the card in place `number` (counted from 1): `configure:<number>`.
    pub fn card_focus_id(number: usize) -> String {
        format!("configure:{number}")
    }

    /// The focus id of a drawn row (counted from 1 among the rows its card draws) of the card in
    /// place `card`: `configure:<card>:<row>`.
    pub fn row_focus_id(card: usize, row: usize) -> String {
        format!("configure:{card}:{row}")
    }

    /// Every id that can hold the focus, in reading order: the preset selector (when there is
    /// one), then each card followed by the rows it draws. Ids are lower case, so the page can look
    /// one up without quoting (`home::model::is_focus_id`). The page's remote walks these.
    pub fn focus_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        if self.presets.is_some() {
            ids.push(Self::PRESET_FOCUS_ID.to_owned());
        }
        for (index, card) in self.cards.iter().enumerate() {
            let number = index + 1;
            ids.push(Self::card_focus_id(number));
            if card.kind != CardKind::Select && self.layout == PanelLayout::Cards {
                let drawn = card.drawn_rows().len();
                ids.extend((1..=drawn).map(|row| Self::row_focus_id(number, row)));
            }
        }
        ids
    }

    /// The card with that key.
    pub fn card(&self, key: &str) -> Option<&SettingCard> {
        self.cards.iter().find(|card| card.key == key)
    }

    /// The media names the data shows: every card's banner.
    pub fn media_names(&self) -> Vec<&str> {
        self.cards
            .iter()
            .filter_map(|card| card.banner.as_deref())
            .collect()
    }
}
