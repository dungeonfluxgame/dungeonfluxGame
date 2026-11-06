//! The `scene home campaigns` verbs: the live Recent Campaigns strip of the landing page.
//!
//! Each verb reads its words into one `CampaignsEdit` (`df_playground::home::campaigns`), wrapped
//! as a `SceneChange::Home`. A key left out leaves that field as it is, a text given empty
//! (`last=`) is not drawn, and `art=none` takes a card's art away. Whether the change is allowed
//! (art that is in the media library, a card that exists, one selected card at most) is decided by
//! the host when the change arrives, so the words here only say whether a value is of the right
//! kind.

use df_playground::home::campaigns::CampaignsEdit;
use df_playground::home::change::HomeEdit;
use df_playground::overlay::Edit;
use df_playground::scene::SceneChange;

use crate::command::{Command, CommandError, bad_choice, count, keyed, truth};

pub(super) const CAMPAIGNS_USAGE: &str = concat!(
    "scene home campaigns set [title=T] [view-all=T] | card-add <title> [art=FILE] [last=T] ",
    "[selected=true|false] | card <n> [art=FILE|none] [title=T] [last=T] [selected=true|false] | ",
    "card-remove <n> | cards-clear | select <n>|none | clear",
);

/// The help lines of these verbs, as `help` prints them.
pub(in crate::command) const HELP: &str = concat!(
    "\n  scene home campaigns ...\n      the Recent Campaigns strip: its title and view-all ",
    "words, and its cards by number from 1\n      (scene home campaigns ",
    "set|card-add|card|card-remove|cards-clear|select|clear); selecting a card\n      un-selects ",
    "the others\n",
);

const SET_KEYS_TEXT: &str = "title=, view-all=";
const ADD_KEYS_TEXT: &str = "art=FILE, last=, selected=true|false";
const CARD_KEYS_TEXT: &str = "art=FILE|none, title=, last=, selected=true|false";

fn wrap(edit: CampaignsEdit) -> Command {
    Command::Scene(SceneChange::Home {
        edit: HomeEdit::Campaigns { edit },
    })
}

fn card_number(text: &str) -> Result<usize, CommandError> {
    count("card", text).map(|number| number as usize)
}

/// `none` or an empty value takes art away; anything else names a picture.
fn art(value: &str) -> Edit<String> {
    if value.is_empty() || value == "none" {
        Edit::Clear
    } else {
        Edit::Set(value.to_owned())
    }
}

fn set(words: &[String]) -> Result<Command, CommandError> {
    if words.is_empty() {
        return Err(CommandError::Usage(CAMPAIGNS_USAGE));
    }
    let (mut title, mut view_all) = (None, None);
    for (key, value) in keyed(words, SET_KEYS_TEXT, &["title", "view-all"])? {
        if key == "title" {
            title = Some(value.to_owned());
        } else {
            view_all = Some(value.to_owned());
        }
    }
    Ok(wrap(CampaignsEdit::Set { title, view_all }))
}

fn card_add(title: &str, rest: &[String]) -> Result<Command, CommandError> {
    let (mut art_name, mut last_played, mut selected) = (None, None, None);
    for (key, value) in keyed(rest, ADD_KEYS_TEXT, &["art", "last", "selected"])? {
        match key {
            "art" => art_name = Some(value.to_owned()),
            "last" => last_played = Some(value.to_owned()),
            _ => selected = Some(truth(key, value)?),
        }
    }
    Ok(wrap(CampaignsEdit::CardAdd {
        art: art_name,
        title: title.to_owned(),
        last_played,
        selected,
    }))
}

fn card(number_text: &str, words: &[String]) -> Result<Command, CommandError> {
    if words.is_empty() {
        return Err(CommandError::Usage(CAMPAIGNS_USAGE));
    }
    let number = card_number(number_text)?;
    let (mut art_edit, mut title, mut last_played, mut selected) = (Edit::Keep, None, None, None);
    for (key, value) in keyed(words, CARD_KEYS_TEXT, &["art", "title", "last", "selected"])? {
        match key {
            "art" => art_edit = art(value),
            "title" => title = Some(value.to_owned()),
            "last" => last_played = Some(value.to_owned()),
            _ => selected = Some(truth(key, value)?),
        }
    }
    Ok(wrap(CampaignsEdit::CardSet {
        number,
        art: art_edit,
        title,
        last_played,
        selected,
    }))
}

fn select(text: &str) -> Result<Command, CommandError> {
    let number = if text == "none" {
        None
    } else {
        Some(
            count("card", text)
                .map(|number| number as usize)
                .map_err(|_| bad_choice("select", text, "a card number from 1, or none"))?,
        )
    };
    Ok(wrap(CampaignsEdit::Select { number }))
}

/// Reads the words after `scene home campaigns` into one live change.
pub(super) fn parse(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb, rest @ ..] if verb == "set" => set(rest),
        [verb, title, rest @ ..] if verb == "card-add" => card_add(title, rest),
        [verb, number, rest @ ..] if verb == "card" => card(number, rest),
        [verb, number] if verb == "card-remove" => Ok(wrap(CampaignsEdit::CardRemove {
            number: card_number(number)?,
        })),
        [verb] if verb == "cards-clear" => Ok(wrap(CampaignsEdit::CardsClear {})),
        [verb, which] if verb == "select" => select(which),
        [verb] if verb == "clear" => Ok(wrap(CampaignsEdit::Clear {})),
        _ => Err(CommandError::Usage(CAMPAIGNS_USAGE)),
    }
}
