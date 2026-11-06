//! The `scene` verbs of the narration box, the seat strip and the logo.
//!
//! Every visible field of these components can be changed from the command line without editing a
//! fixture: `scene narration set|edit|clear`, `scene seat set|remove`, `scene seats open|clear` and
//! `scene logo on|off`. A field left out stays as it is, and `none` as a value takes a field
//! away. A change is all or nothing: the playground checks the whole result with the rules a
//! scene file is checked with before it draws anything.

use df_playground::overlay::{Edit, Glyph, NarrationPlacement, Progress};
use df_playground::scene::SceneChange;

use crate::command::usage::{LOGO_USAGE, NARRATION_USAGE, SEAT_USAGE, SEATS_USAGE};
use crate::command::{Command, CommandError, bad_choice, count, keyed, split_list, truth};

/// The word that takes a field away.
const NONE: &str = "none";

const NARRATION_KEYS: [&str; 7] = [
    "text",
    "speaker",
    "badge",
    "continue",
    "voice",
    "placement",
    "scene",
];
const NARRATION_KEYS_TEXT: &str = concat!(
    "text=, speaker=, badge=, continue=, voice=true|false, placement=center|right, ",
    "scene=I/N[:TITLE]",
);
const SEAT_KEYS: [&str; 8] = [
    "class",
    "level",
    "portrait",
    "glyph",
    "accent",
    "hp",
    "max",
    "conditions",
];
const SEAT_KEYS_TEXT: &str =
    "class=, level=N, portrait=FILE, glyph=NAME, accent=#rrggbb, hp=N, max=N, conditions=a,b";

/// A text field: `none` takes it away, anything else sets it.
fn text_edit(value: &str) -> Edit<String> {
    if value == NONE {
        Edit::Clear
    } else {
        Edit::Set(value.to_owned())
    }
}

fn glyph_edit(key: &str, value: &str) -> Result<Edit<Glyph>, CommandError> {
    if value == NONE {
        return Ok(Edit::Clear);
    }
    Glyph::from_key(value)
        .map(Edit::Set)
        .ok_or_else(|| bad_choice(key, value, Glyph::keys_text()))
}

/// `2/5` or `2/5:Introduction`.
fn progress_edit(value: &str) -> Result<Edit<Progress>, CommandError> {
    if value == NONE {
        return Ok(Edit::Clear);
    }
    let allowed = "I/N or I/N:TITLE (scene I of N), or none";
    let (numbers, title) = match value.split_once(':') {
        Some((numbers, title)) => (numbers, Some(title.to_owned())),
        None => (value, None),
    };
    let (index, total) = numbers
        .split_once('/')
        .ok_or_else(|| bad_choice("scene", value, allowed))?;
    let number = |text: &str| {
        text.parse::<u16>()
            .map_err(|_| bad_choice("scene", value, allowed))
    };
    Ok(Edit::Set(Progress {
        index: number(index)?,
        total: number(total)?,
        title,
    }))
}

fn narration_change(
    text: Option<String>,
    pairs: &[(&str, &str)],
) -> Result<SceneChange, CommandError> {
    let mut text = text;
    let mut change = (Edit::Keep, Edit::Keep, Edit::Keep, None, None, Edit::Keep);
    for (key, value) in pairs {
        match *key {
            "text" => text = Some((*value).to_owned()),
            "speaker" => change.0 = text_edit(value),
            "badge" => change.1 = glyph_edit(key, value)?,
            "continue" => change.2 = text_edit(value),
            "voice" => change.3 = Some(truth(key, value)?),
            "placement" => {
                change.4 = Some(
                    NarrationPlacement::from_key(value)
                        .ok_or_else(|| bad_choice(key, value, "center or right"))?,
                );
            }
            _ => change.5 = progress_edit(value)?,
        }
    }
    Ok(SceneChange::Narration {
        text,
        speaker: change.0,
        badge: change.1,
        continue_label: change.2,
        voice: change.3,
        placement: change.4,
        progress: change.5,
    })
}

fn narration(words: &[String]) -> Result<Command, CommandError> {
    let change = match words {
        [verb] if verb == "clear" => SceneChange::ClearNarration {},
        [verb, text, pairs @ ..] if verb == "set" => {
            let pairs = keyed(pairs, NARRATION_KEYS_TEXT, &NARRATION_KEYS[1..])?;
            narration_change(Some(text.clone()), &pairs)?
        }
        [verb, pairs @ ..] if verb == "edit" && !pairs.is_empty() => {
            let pairs = keyed(pairs, NARRATION_KEYS_TEXT, &NARRATION_KEYS)?;
            narration_change(None, &pairs)?
        }
        _ => return Err(CommandError::Usage(NARRATION_USAGE)),
    };
    Ok(Command::Scene(change))
}

fn level(value: &str) -> Result<Edit<u8>, CommandError> {
    if value == NONE {
        return Ok(Edit::Clear);
    }
    value
        .parse::<u8>()
        .map(Edit::Set)
        .map_err(|_| bad_choice("level", value, "a whole number from 0 to 255, or none"))
}

fn seat(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb, name] if verb == "remove" => Ok(Command::Scene(SceneChange::SeatRemove {
            name: name.clone(),
        })),
        [verb, name, pairs @ ..] if verb == "set" => {
            let (mut class, mut level_edit, mut portrait) = (Edit::Keep, Edit::Keep, Edit::Keep);
            let (mut glyph, mut accent) = (Edit::Keep, Edit::Keep);
            let (mut hit_points, mut maximum_hit_points, mut conditions) = (None, None, None);
            for (key, value) in keyed(pairs, SEAT_KEYS_TEXT, &SEAT_KEYS)? {
                match key {
                    "class" => class = text_edit(value),
                    "level" => level_edit = level(value)?,
                    "portrait" => portrait = text_edit(value),
                    "glyph" => glyph = glyph_edit(key, value)?,
                    "accent" => accent = text_edit(value),
                    "hp" => hit_points = Some(count("hp", value)?),
                    "max" => maximum_hit_points = Some(count("max", value)?),
                    _ => conditions = Some(split_list(value)),
                }
            }
            Ok(Command::Scene(SceneChange::SeatSet {
                name: name.clone(),
                class,
                level: level_edit,
                portrait,
                glyph,
                accent,
                hit_points,
                maximum_hit_points,
                conditions,
            }))
        }
        _ => Err(CommandError::Usage(SEAT_USAGE)),
    }
}

fn seats(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb] if verb == "clear" => Ok(Command::Scene(SceneChange::SeatsClear {})),
        [verb, number] if verb == "open" => {
            let open = number
                .parse::<u8>()
                .map_err(|_| bad_choice("open", number, "a whole number of seats from 0 to 255"))?;
            Ok(Command::Scene(SceneChange::SeatsOpen { open }))
        }
        _ => Err(CommandError::Usage(SEATS_USAGE)),
    }
}

fn logo(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [state] => Ok(Command::Scene(SceneChange::Brand {
            on: truth("logo", state)?,
        })),
        _ => Err(CommandError::Usage(LOGO_USAGE)),
    }
}

/// Reads the words of a `scene narration`, `seat`, `seats` or `logo` verb into one live change.
pub(super) fn parse(verb: &str, rest: &[String]) -> Result<Command, CommandError> {
    match verb {
        "narration" => narration(rest),
        "seat" => seat(rest),
        "seats" => seats(rest),
        _ => logo(rest),
    }
}
