//! The `scene home adventure` verbs: the live Tonight's Adventure card of the landing page.
//!
//! Each verb reads its words into one `AdventureEdit` (`df_playground::home::adventure`), wrapped
//! as a `SceneChange::Home`. A key left out leaves that field as it is, a text given empty
//! (`blurb=`) is not drawn, and `picture=none` takes the picture away. Whether the change is
//! allowed (a picture that is in the media library, a fact row that exists, a text within its
//! limit) is decided by the host when the change arrives, so the words here only say whether a
//! value is of the right kind.

use df_playground::home::adventure::AdventureEdit;
use df_playground::home::change::HomeEdit;
use df_playground::home::glyph::HomeIcon;
use df_playground::overlay::Edit;
use df_playground::scene::SceneChange;

use crate::command::{Command, CommandError, bad_choice, count, keyed, truth};

pub(super) const ADVENTURE_USAGE: &str = concat!(
    "scene home adventure set [heading=T] [picture=FILE|none] [title=T] [blurb=T] [launch=T] ",
    "[enabled=true|false] | fact <n> [icon=NAME] [label=T] [value=T] | fact-add <label> <value> ",
    "[icon=NAME] | fact-remove <n> | facts-clear | clear",
);

/// The help lines of these verbs, as `help` prints them.
pub(in crate::command) const HELP: &str = concat!(
    "\n  scene home adventure ...\n      the Tonight's Adventure card: its heading, picture, ",
    "title, blurb, launch button (words and\n      enabled) and fact rows, by number from 1 ",
    "(scene home adventure set|fact|fact-add|fact-remove|facts-clear|clear)\n",
);

const SET_KEYS: [&str; 6] = ["heading", "picture", "title", "blurb", "launch", "enabled"];
const SET_KEYS_TEXT: &str =
    "heading=, picture=FILE|none, title=, blurb=, launch=, enabled=true|false";
const FACT_KEYS: [&str; 3] = ["icon", "label", "value"];
const FACT_KEYS_TEXT: &str = "icon=NAME, label=, value=";

fn wrap(edit: AdventureEdit) -> Command {
    Command::Scene(SceneChange::Home {
        edit: HomeEdit::Adventure { edit },
    })
}

fn icon(value: &str) -> Result<HomeIcon, CommandError> {
    HomeIcon::from_key(value).ok_or_else(|| bad_choice("icon", value, HomeIcon::keys_text()))
}

/// `none` or an empty value takes a picture away; anything else names one.
fn picture(value: &str) -> Edit<String> {
    if value.is_empty() || value == "none" {
        Edit::Clear
    } else {
        Edit::Set(value.to_owned())
    }
}

fn number(key: &'static str, text: &str) -> Result<usize, CommandError> {
    count(key, text).map(|number| number as usize)
}

fn set(words: &[String]) -> Result<Command, CommandError> {
    if words.is_empty() {
        return Err(CommandError::Usage(ADVENTURE_USAGE));
    }
    let (mut heading, mut title, mut blurb, mut launch_label, mut enabled) =
        (None, None, None, None, None);
    let mut picture_edit = Edit::Keep;
    for (key, value) in keyed(words, SET_KEYS_TEXT, &SET_KEYS)? {
        match key {
            "heading" => heading = Some(value.to_owned()),
            "picture" => picture_edit = picture(value),
            "title" => title = Some(value.to_owned()),
            "blurb" => blurb = Some(value.to_owned()),
            "launch" => launch_label = Some(value.to_owned()),
            _ => enabled = Some(truth(key, value)?),
        }
    }
    Ok(wrap(AdventureEdit::Set {
        heading,
        picture: picture_edit,
        title,
        blurb,
        launch_label,
        enabled,
    }))
}

fn fact(number_text: &str, words: &[String]) -> Result<Command, CommandError> {
    if words.is_empty() {
        return Err(CommandError::Usage(ADVENTURE_USAGE));
    }
    let number = number("fact", number_text)?;
    let (mut icon_edit, mut label, mut value_text) = (None, None, None);
    for (key, value) in keyed(words, FACT_KEYS_TEXT, &FACT_KEYS)? {
        match key {
            "icon" => icon_edit = Some(icon(value)?),
            "label" => label = Some(value.to_owned()),
            _ => value_text = Some(value.to_owned()),
        }
    }
    Ok(wrap(AdventureEdit::FactSet {
        number,
        icon: icon_edit,
        label,
        value: value_text,
    }))
}

fn fact_add(label: &str, value: &str, rest: &[String]) -> Result<Command, CommandError> {
    let mut icon_edit = None;
    for (_, value) in keyed(rest, "icon=NAME", &["icon"])? {
        icon_edit = Some(icon(value)?);
    }
    Ok(wrap(AdventureEdit::FactAdd {
        icon: icon_edit,
        label: label.to_owned(),
        value: value.to_owned(),
    }))
}

/// Reads the words after `scene home adventure` into one live change.
pub(super) fn parse(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb, rest @ ..] if verb == "set" => set(rest),
        [verb, number, rest @ ..] if verb == "fact" => fact(number, rest),
        [verb, label, value, rest @ ..] if verb == "fact-add" => fact_add(label, value, rest),
        [verb, number] if verb == "fact-remove" => Ok(wrap(AdventureEdit::FactRemove {
            number: self::number("fact", number)?,
        })),
        [verb] if verb == "facts-clear" => Ok(wrap(AdventureEdit::FactsClear {})),
        [verb] if verb == "clear" => Ok(wrap(AdventureEdit::Clear {})),
        _ => Err(CommandError::Usage(ADVENTURE_USAGE)),
    }
}
