//! The `scene home configure` verbs: the live settings panel of the landing page.
//!
//! Each verb reads its words into one `ConfigureEdit` (`df_playground::home_configure`), wrapped as
//! a `SceneChange::Home`. A key left out leaves that field as it is. `icon=none` and `banner=none`
//! (or an empty value) take that field away. Cards and rows are named by their keys, and positions
//! count from 1. A list is written with commas (`options=Fast,Balanced,Thoughtful`), an amount in
//! whole hundredths (`money=2840`) or with a decimal point (`money=28.40`). Whether the change is
//! allowed (a banner that is a picture of the media library, a card or row that exists, a value
//! within its range) is decided by the host when the change arrives, so the words here only say
//! whether a value is of the right kind.

use df_playground::home_change::HomeEdit;
use df_playground::home_configure::{
    Accent, CardKind, ConfigureEdit, ConfigureIcon, PanelLayout, RowEdit, RowKind, SliderUnit,
    StatValue,
};
use df_playground::overlay::Edit;
use df_playground::scene::SceneChange;

use super::{Command, CommandError, bad_choice, count, keyed, split_list, truth};

pub(super) const CONFIGURE_USAGE: &str = "scene home configure panel [title=T] [currency=T] [layout=cards|tiles] | preset [label=T] [value=T] [options=a,b] | preset-remove | card <card> [label=T] [icon=NAME|none] [banner=FILE|none] [accent=cyan|violet|green] [shown=N] | card-add <card> kind=select|sliders|mixed|budget label=T [icon=NAME] [banner=FILE] [accent=A] [shown=N] [at=N] | card-remove <card> | card-move <card> <n> | row <card> <row> [label=T] [value=T] [options=a,b] [level=0-100] [unit=percent|steps] [steps=a,b] [money=N] [per-hour=N] [range=LOW-HIGH] [text=T] [no-value=true] [info=true|false] [highlight=true|false] | row-add <card> <row> kind=select|slider|stat [the row keys] [at=N] | row-remove <card> <row> | row-move <card> <row> <n> | clear";

/// The help lines of these verbs, as `help` prints them.
pub(super) const HELP: &str = "\n  scene home configure ...\n      the settings panel: its title and preset selector, and its cards (select, sliders, mixed,\n      budget) with their rows, by key (scene home configure panel|preset|card|card-add|card-remove|\n      card-move|row|row-add|row-remove|row-move|clear)\n";

const PANEL_KEYS: [&str; 3] = ["title", "currency", "layout"];
const PRESET_KEYS: [&str; 3] = ["label", "value", "options"];
const CARD_KEYS: [&str; 5] = ["label", "icon", "banner", "accent", "shown"];
const CARD_ADD_KEYS: [&str; 7] = ["kind", "label", "icon", "banner", "accent", "shown", "at"];
const ROW_KEYS: [&str; 13] = [
    "label",
    "value",
    "options",
    "level",
    "unit",
    "steps",
    "money",
    "per-hour",
    "range",
    "text",
    "no-value",
    "info",
    "highlight",
];
const ROW_ADD_KEYS: [&str; 15] = [
    "kind",
    "at",
    "label",
    "value",
    "options",
    "level",
    "unit",
    "steps",
    "money",
    "per-hour",
    "range",
    "text",
    "no-value",
    "info",
    "highlight",
];
const KEYS_TEXT: &str = "the keys in the usage of `scene home configure`";
/// The word that takes an optional field away.
const NONE: &str = "none";

fn wrap(edit: ConfigureEdit) -> Result<Command, CommandError> {
    Ok(Command::Scene(SceneChange::Home {
        edit: HomeEdit::Configure { edit },
    }))
}

fn usage() -> CommandError {
    CommandError::Usage(CONFIGURE_USAGE)
}

fn icon(key: &str, value: &str) -> Result<ConfigureIcon, CommandError> {
    ConfigureIcon::from_key(value).ok_or_else(|| bad_choice(key, value, ConfigureIcon::keys_text()))
}

fn icon_edit(value: &str) -> Result<Edit<ConfigureIcon>, CommandError> {
    if value == NONE || value.is_empty() {
        Ok(Edit::Clear)
    } else {
        icon("icon", value).map(Edit::Set)
    }
}

fn banner_edit(value: &str) -> Edit<String> {
    if value == NONE || value.is_empty() {
        Edit::Clear
    } else {
        Edit::Set(value.to_owned())
    }
}

fn accent(value: &str) -> Result<Accent, CommandError> {
    Accent::from_key(value).ok_or_else(|| bad_choice("accent", value, "cyan, violet or green"))
}

fn card_kind(value: &str) -> Result<CardKind, CommandError> {
    CardKind::from_key(value)
        .ok_or_else(|| bad_choice("kind", value, "select, sliders, mixed or budget"))
}

fn small(key: &'static str, value: &str) -> Result<u8, CommandError> {
    u8::try_from(count(key, value)?).map_err(|_| {
        bad_choice(
            key,
            value,
            "a whole number from 0 to 255 (the host says the range)",
        )
    })
}

fn position(key: &'static str, value: &str) -> Result<usize, CommandError> {
    usize::try_from(count(key, value)?).map_err(|_| usage())
}

/// An amount in hundredths: `2840`, or with a decimal point, `28.40`.
fn cents(key: &str, value: &str) -> Result<u64, CommandError> {
    let wrong = || bad_choice(key, value, "an amount such as 2840 (hundredths) or 28.40");
    match value.split_once('.') {
        None => value.parse::<u64>().map_err(|_| wrong()),
        Some((whole, fraction)) => {
            let whole = whole.parse::<u64>().map_err(|_| wrong())?;
            let digits = fraction.len();
            if digits == 0 || digits > 2 || !fraction.chars().all(|c| c.is_ascii_digit()) {
                return Err(wrong());
            }
            let fraction = fraction.parse::<u64>().map_err(|_| wrong())?;
            let scale = if digits == 1 { 10 } else { 1 };
            whole
                .checked_mul(100)
                .and_then(|hundredths| hundredths.checked_add(fraction * scale))
                .ok_or_else(wrong)
        }
    }
}

fn panel(words: &[String]) -> Result<Command, CommandError> {
    if words.is_empty() {
        return Err(usage());
    }
    let (mut title, mut currency, mut layout) = (None, None, None);
    for (key, value) in keyed(
        words,
        "title=T, currency=T, layout=cards|tiles",
        &PANEL_KEYS,
    )? {
        match key {
            "title" => title = Some(value.to_owned()),
            "currency" => currency = Some(value.to_owned()),
            _ => {
                layout = Some(
                    PanelLayout::from_key(value)
                        .ok_or_else(|| bad_choice(key, value, "cards or tiles"))?,
                );
            }
        }
    }
    wrap(ConfigureEdit::Panel {
        title,
        currency,
        layout,
    })
}

fn preset(words: &[String]) -> Result<Command, CommandError> {
    if words.is_empty() {
        return Err(usage());
    }
    let (mut label, mut value_text, mut options) = (None, None, None);
    for (key, value) in keyed(words, "label=T, value=T, options=a,b", &PRESET_KEYS)? {
        match key {
            "label" => label = Some(value.to_owned()),
            "value" => value_text = Some(value.to_owned()),
            _ => options = Some(split_list(value)),
        }
    }
    wrap(ConfigureEdit::Preset {
        label,
        value: value_text,
        options,
    })
}

fn card(card: &str, words: &[String]) -> Result<Command, CommandError> {
    if words.is_empty() {
        return Err(usage());
    }
    let (mut label, mut icon_slot, mut banner, mut accent_slot, mut shown) =
        (None, Edit::Keep, Edit::Keep, None, None);
    let keys_text = "label=T, icon=NAME|none, banner=FILE|none, accent=A, shown=N";
    for (key, value) in keyed(words, keys_text, &CARD_KEYS)? {
        match key {
            "label" => label = Some(value.to_owned()),
            "icon" => icon_slot = icon_edit(value)?,
            "banner" => banner = banner_edit(value),
            "accent" => accent_slot = Some(accent(value)?),
            _ => shown = Some(small("shown", value)?),
        }
    }
    wrap(ConfigureEdit::Card {
        card: card.to_owned(),
        label,
        icon: icon_slot,
        banner,
        accent: accent_slot,
        shown_rows: shown,
    })
}

fn card_add(card: &str, words: &[String]) -> Result<Command, CommandError> {
    let (mut kind, mut label, mut icon_slot, mut banner, mut accent_slot, mut shown, mut at) =
        (None, None, None, None, None, None, None);
    for (key, value) in keyed(words, KEYS_TEXT, &CARD_ADD_KEYS)? {
        match key {
            "kind" => kind = Some(card_kind(value)?),
            "label" => label = Some(value.to_owned()),
            "icon" => icon_slot = Some(icon(key, value)?),
            "banner" => banner = Some(value.to_owned()).filter(|name| !name.is_empty()),
            "accent" => accent_slot = Some(accent(value)?),
            "shown" => shown = Some(small("shown", value)?),
            _ => at = Some(position("at", value)?),
        }
    }
    let (Some(kind), Some(label)) = (kind, label) else {
        return Err(usage());
    };
    wrap(ConfigureEdit::CardAdd {
        card: card.to_owned(),
        kind,
        label,
        icon: icon_slot,
        banner,
        accent: accent_slot,
        shown_rows: shown,
        at,
    })
}

/// Reads the row keys of `pairs` into one `RowEdit`; the keys the caller reads itself (`kind`, `at`)
/// are skipped here.
fn row_edit(pairs: &[(&str, &str)]) -> Result<RowEdit, CommandError> {
    let mut edit = RowEdit::default();
    let mut stat_given = 0;
    for (key, value) in pairs {
        match *key {
            "label" => edit.label = Some((*value).to_owned()),
            "value" => edit.value = Some((*value).to_owned()),
            "options" => edit.options = Some(split_list(value)),
            "level" => edit.level = Some(small("level", value)?),
            "unit" => {
                edit.unit = Some(
                    SliderUnit::from_key(value)
                        .ok_or_else(|| bad_choice("unit", value, "percent or steps"))?,
                );
            }
            "steps" => edit.steps = Some(split_list(value)),
            "money" => {
                edit.stat = Edit::Set(StatValue::Money {
                    cents: cents(key, value)?,
                });
                stat_given += 1;
            }
            "per-hour" => {
                edit.stat = Edit::Set(StatValue::PerHour {
                    cents: cents(key, value)?,
                });
                stat_given += 1;
            }
            "range" => {
                let (low, high) = value.split_once('-').ok_or_else(|| {
                    bad_choice(
                        key,
                        value,
                        "two amounts joined by a dash, such as 7.80-11.70",
                    )
                })?;
                edit.stat = Edit::Set(StatValue::Range {
                    low_cents: cents(key, low)?,
                    high_cents: cents(key, high)?,
                });
                stat_given += 1;
            }
            "text" => {
                edit.stat = Edit::Set(StatValue::Text {
                    text: (*value).to_owned(),
                });
                stat_given += 1;
            }
            "no-value" => {
                if truth(key, value)? {
                    edit.stat = Edit::Clear;
                    stat_given += 1;
                }
            }
            "info" => edit.info = Some(truth(key, value)?),
            "highlight" => edit.highlight = Some(truth(key, value)?),
            _ => {}
        }
    }
    if stat_given > 1 {
        return Err(usage());
    }
    Ok(edit)
}

fn row(card: &str, row: &str, words: &[String]) -> Result<Command, CommandError> {
    if words.is_empty() {
        return Err(usage());
    }
    let pairs = keyed(words, KEYS_TEXT, &ROW_KEYS)?;
    wrap(ConfigureEdit::Row {
        card: card.to_owned(),
        row: row.to_owned(),
        edit: row_edit(&pairs)?,
    })
}

fn row_add(card: &str, row: &str, words: &[String]) -> Result<Command, CommandError> {
    let pairs = keyed(words, KEYS_TEXT, &ROW_ADD_KEYS)?;
    let (mut kind, mut at) = (None, None);
    for (key, value) in &pairs {
        match *key {
            "kind" => {
                kind = Some(
                    RowKind::from_key(value)
                        .ok_or_else(|| bad_choice("kind", value, "select, slider or stat"))?,
                );
            }
            "at" => at = Some(position("at", value)?),
            _ => {}
        }
    }
    let Some(kind) = kind else {
        return Err(usage());
    };
    wrap(ConfigureEdit::RowAdd {
        card: card.to_owned(),
        row: row.to_owned(),
        kind,
        edit: row_edit(&pairs)?,
        at,
    })
}

/// Reads the words after `scene home configure` into one live change.
pub(super) fn parse(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb, rest @ ..] if verb == "panel" => panel(rest),
        [verb, rest @ ..] if verb == "preset" => preset(rest),
        [verb] if verb == "preset-remove" => wrap(ConfigureEdit::PresetRemove {}),
        [verb, name, rest @ ..] if verb == "card" => card(name, rest),
        [verb, name, rest @ ..] if verb == "card-add" => card_add(name, rest),
        [verb, name] if verb == "card-remove" => {
            wrap(ConfigureEdit::CardRemove { card: name.clone() })
        }
        [verb, name, to] if verb == "card-move" => wrap(ConfigureEdit::CardMove {
            card: name.clone(),
            position: position("the position", to)?,
        }),
        [verb, name, key, rest @ ..] if verb == "row" => row(name, key, rest),
        [verb, name, key, rest @ ..] if verb == "row-add" => row_add(name, key, rest),
        [verb, name, key] if verb == "row-remove" => wrap(ConfigureEdit::RowRemove {
            card: name.clone(),
            row: key.clone(),
        }),
        [verb, name, key, to] if verb == "row-move" => wrap(ConfigureEdit::RowMove {
            card: name.clone(),
            row: key.clone(),
            position: position("the position", to)?,
        }),
        [verb] if verb == "clear" => wrap(ConfigureEdit::Clear {}),
        _ => Err(usage()),
    }
}

#[cfg(test)]
#[path = "scene_home_configure_tests.rs"]
mod tests;
