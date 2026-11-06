//! The `scene home` verbs: the live landing page (the Home screen) of the playground's shared
//! screen.
//!
//! `scene home brand|title|top|nav|actions` change the brand mark, the title block, the bell of the
//! top bar, the navigation rail and the action bar; `scene home remove <part>` takes one part away
//! and `scene home clear` the whole page. The settings panel, the adventure card and the campaign
//! strip have their own verbs (`scene home configure ...`, `scene home adventure ...`,
//! `scene home campaigns ...`), which this module only routes. Each verb reads its words into one
//! `HomeEdit` (`df_playground::home::change`). A key left out leaves that field as it is, `none`
//! (or an empty text) takes an optional field away, items are numbered from 1 and the next free
//! number adds one at the end. A range is checked by the host when the change arrives, so the
//! words here only say whether a value is of the right kind.

pub(in crate::command) mod adventure;
pub(in crate::command) mod campaigns;
pub(in crate::command) mod configure;

#[cfg(test)]
mod adventure_tests;
#[cfg(test)]
mod campaigns_tests;
#[cfg(test)]
mod configure_tests;

use df_playground::home::change::{BrandEdit, ButtonEdit, HomeEdit, NavEdit, TitleEdit, TopEdit};
use df_playground::home::chrome::BrandSize;
use df_playground::home::glyph::HomeIcon;
use df_playground::home::model::HomePart;
use df_playground::home::nav::ActionVariant;
use df_playground::overlay::Edit;
use df_playground::scene::SceneChange;

use crate::command::{Command, CommandError, bad_choice, count, keyed, truth};

pub(super) const HOME_USAGE: &str = concat!(
    "scene home brand [wordmark=T] [size=compact|display] [star=on|off] | title [text=T] ",
    "[tagline=T|none] [rule=on|off] | top [bell=on|off] [unread=N|none] | nav <n> [label=T] ",
    "[glyph=NAME] [active=true|false] [badge=T|none] | nav remove <n> | actions <n> [label=T] ",
    "[subtitle=T|none] [glyph=NAME|none] [variant=primary|secondary] [enabled=true|false] | ",
    "actions remove <n> | focus <id>|none | remove <part> | clear | configure ... | adventure ... ",
    "| campaigns ...",
);

/// The help lines of these verbs, as `help` prints them.
pub(in crate::command) const HELP: &str = concat!(
    "\n  scene home ...\n      the landing page: brand, title, top (the bell), nav (by item ",
    "number from 1), actions (by\n      button number from 1), focus <id> (the one element the ",
    "remote is on, such as actions:1),\n      remove <part>, clear; the settings panel, the ",
    "adventure card and the\n      campaign strip have scene home configure|adventure|campaigns ",
    "(scene home with no words\n      lists them all)\n",
);

const BRAND_KEYS: [&str; 3] = ["wordmark", "size", "star"];
const BRAND_KEYS_TEXT: &str = "wordmark=, size=compact|display, star=on|off";
const TITLE_KEYS: [&str; 3] = ["text", "tagline", "rule"];
const TITLE_KEYS_TEXT: &str = "text=, tagline=T|none, rule=on|off";
const TOP_KEYS: [&str; 2] = ["bell", "unread"];
const TOP_KEYS_TEXT: &str = "bell=on|off, unread=N|none";
const NAV_KEYS: [&str; 4] = ["label", "glyph", "active", "badge"];
const NAV_KEYS_TEXT: &str = "label=, glyph=NAME, active=true|false, badge=T|none";
const ACTION_KEYS: [&str; 5] = ["label", "subtitle", "glyph", "variant", "enabled"];
const ACTION_KEYS_TEXT: &str =
    "label=, subtitle=T|none, glyph=NAME|none, variant=primary|secondary, enabled=true|false";

/// The word that takes an optional field away.
const NONE: &str = "none";

fn wrap(edit: HomeEdit) -> Result<Command, CommandError> {
    Ok(Command::Scene(SceneChange::Home { edit }))
}

fn text_edit(value: &str) -> Edit<String> {
    if value == NONE || value.is_empty() {
        Edit::Clear
    } else {
        Edit::Set(value.to_owned())
    }
}

fn icon(key: &str, value: &str) -> Result<HomeIcon, CommandError> {
    HomeIcon::from_key(value).ok_or_else(|| bad_choice(key, value, HomeIcon::keys_text()))
}

fn icon_edit(key: &str, value: &str) -> Result<Edit<HomeIcon>, CommandError> {
    if value == NONE {
        Ok(Edit::Clear)
    } else {
        icon(key, value).map(Edit::Set)
    }
}

fn number(key: &'static str, value: &str) -> Result<usize, CommandError> {
    usize::try_from(count(key, value)?).map_err(|_| CommandError::Usage(HOME_USAGE))
}

fn brand(words: &[String]) -> Result<Command, CommandError> {
    let mut edit = BrandEdit::default();
    for (key, value) in keyed(words, BRAND_KEYS_TEXT, &BRAND_KEYS)? {
        match key {
            "wordmark" => edit.wordmark = Some(value.to_owned()),
            "size" => {
                edit.size = Some(
                    BrandSize::from_key(value)
                        .ok_or_else(|| bad_choice(key, value, "compact or display"))?,
                );
            }
            _ => edit.star = Some(truth(key, value)?),
        }
    }
    wrap(HomeEdit::Brand { brand: edit })
}

fn title(words: &[String]) -> Result<Command, CommandError> {
    let mut edit = TitleEdit::default();
    for (key, value) in keyed(words, TITLE_KEYS_TEXT, &TITLE_KEYS)? {
        match key {
            "text" => edit.title = Some(value.to_owned()),
            "tagline" => edit.tagline = text_edit(value),
            _ => edit.rule = Some(truth(key, value)?),
        }
    }
    wrap(HomeEdit::Title { title: edit })
}

fn top(words: &[String]) -> Result<Command, CommandError> {
    let mut edit = TopEdit::default();
    for (key, value) in keyed(words, TOP_KEYS_TEXT, &TOP_KEYS)? {
        match key {
            "bell" => edit.bell = Some(truth(key, value)?),
            _ if value == NONE => edit.unread = Edit::Clear,
            _ => {
                let unread = value.parse::<u16>().map_err(|_| {
                    bad_choice(key, value, "a whole number from 0 to 65535, or none")
                })?;
                edit.unread = Edit::Set(unread);
            }
        }
    }
    wrap(HomeEdit::Top { top: edit })
}

/// `<n> key=value ...`, or `remove <n>`.
fn numbered(
    words: &[String],
    usage: &'static str,
    what: &'static str,
) -> Result<(usize, bool, Vec<String>), CommandError> {
    match words {
        [verb, n] if verb == "remove" => Ok((number(what, n)?, true, Vec::new())),
        [n, rest @ ..] => Ok((number(what, n)?, false, rest.to_vec())),
        [] => Err(CommandError::Usage(usage)),
    }
}

fn nav(words: &[String]) -> Result<Command, CommandError> {
    let (number, remove, rest) = numbered(words, HOME_USAGE, "the item number")?;
    if remove {
        return wrap(HomeEdit::NavRemove { number });
    }
    let mut item = NavEdit::default();
    for (key, value) in keyed(&rest, NAV_KEYS_TEXT, &NAV_KEYS)? {
        match key {
            "label" => item.label = Some(value.to_owned()),
            "glyph" => item.glyph = Some(icon(key, value)?),
            "active" => item.active = Some(truth(key, value)?),
            _ => item.badge = text_edit(value),
        }
    }
    wrap(HomeEdit::Nav { number, item })
}

fn actions(words: &[String]) -> Result<Command, CommandError> {
    let (number, remove, rest) = numbered(words, HOME_USAGE, "the button number")?;
    if remove {
        return wrap(HomeEdit::ActionRemove { number });
    }
    let mut button = ButtonEdit::default();
    for (key, value) in keyed(&rest, ACTION_KEYS_TEXT, &ACTION_KEYS)? {
        match key {
            "label" => button.label = Some(value.to_owned()),
            "subtitle" => button.subtitle = text_edit(value),
            "glyph" => button.glyph = icon_edit(key, value)?,
            "variant" => {
                button.variant = Some(
                    ActionVariant::from_key(value)
                        .ok_or_else(|| bad_choice(key, value, "primary or secondary"))?,
                );
            }
            _ => button.enabled = Some(truth(key, value)?),
        }
    }
    wrap(HomeEdit::Action { number, button })
}

/// `focus actions:1` puts the focus on one element; `focus none` returns it to the first primary
/// action. An id is checked by the host when the change arrives.
fn focus(words: &[String]) -> Result<Command, CommandError> {
    let [id] = words else {
        return Err(CommandError::Usage(HOME_USAGE));
    };
    wrap(HomeEdit::Focus { id: text_edit(id) })
}

fn remove(words: &[String]) -> Result<Command, CommandError> {
    let [name] = words else {
        return Err(CommandError::Usage(HOME_USAGE));
    };
    let part = HomePart::from_key(name)
        .ok_or_else(|| bad_choice("the part", name, HomePart::keys_text()))?;
    wrap(HomeEdit::Remove { part })
}

/// Reads the words after `scene home`.
pub(super) fn parse(words: &[String]) -> Result<Command, CommandError> {
    let Some((verb, rest)) = words.split_first() else {
        return Err(CommandError::Usage(HOME_USAGE));
    };
    match verb.as_str() {
        "brand" => brand(rest),
        "title" => title(rest),
        "top" => top(rest),
        "nav" => nav(rest),
        "actions" => actions(rest),
        "focus" => focus(rest),
        "remove" => remove(rest),
        "clear" if rest.is_empty() => wrap(HomeEdit::Clear {}),
        "configure" => configure::parse(rest),
        "adventure" => adventure::parse(rest),
        "campaigns" => campaigns::parse(rest),
        _ => Err(CommandError::Usage(HOME_USAGE)),
    }
}
